//! Optional compiler-backed Java calls, separate from the import graph.
//!
//! The helper parses/attributes an explicit source list with annotation
//! processing disabled; it never generates or executes project classes.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use archguard_core::{CallAnalysis, CallAnalyzer, Config};
use archguard_extract::{Extractor, JavaExtractor, Lang, strip_root};
use archguard_flow_common::text_decode::{Decoded, decode_text};
use sha2::{Digest, Sha256};

const HELPER: &str = include_str!("ArchguardCallScanner.java");
// Large source snapshots include calls, type uses and method metrics. Keep a
// hard bound while allowing their streamed output beyond the former 64 MiB cap.
const MAX_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;
// Elasticsearch (16,714 files) needed 43 s on a 16-core laptop; 10 ms per
// file keeps that headroom on slower machines.
const MIN_PROCESS_SECS: u64 = 45;
// javac keeps every attributed tree: Elasticsearch (16,714 files) exhausted
// 512 MiB and reached 3.7 GiB RSS. -Xmx caps the heap without reserving it.
const MIN_HEAP_MIB: usize = 512;
const MAX_HEAP_MIB: usize = 8192;
// The helper analyzes one top-level type at a time to isolate javac crashes;
// only the internal JavacTaskImpl offers that.
const JAVAC_API_EXPORT: &str = "jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED";
const JAVA_ENV: [&str; 4] = [
    "JDK_JAVA_OPTIONS",
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "CLASSPATH",
];
static TEMP_ID: AtomicU64 = AtomicU64::new(0);
type CacheEntry = Option<([u8; 32], CallAnalysis)>;
static CACHE: OnceLock<Mutex<CacheEntry>> = OnceLock::new();

struct Input {
    manifest: Vec<u8>,
    context: Vec<u8>,
    fingerprint: [u8; 32],
    files: usize,
    /// One message per source file left out of the analysis.
    skipped: Vec<String>,
}

/// Java class/method call sites for the effective source configuration. The
/// optional JDK backend reports an unavailable status instead of failing import
/// analysis. Results are reused only while source *contents* and paths match.
/// Thus changes inside method bodies invalidate calls even if imports do not.
/// The single cached project bounds memory for long-running MCP sessions.
#[tracing::instrument(skip_all)]
pub fn analyze_java_calls(root: &Path, config: &Config) -> CallAnalysis {
    let scopes = match prepare_scopes(root, config) {
        Ok(scopes) => scopes,
        Err(error) => {
            return unavailable(0, format!("Java call sources could not be read: {error}"));
        }
    };
    let files = scopes.iter().map(|(_, input)| input.files).sum();
    let skipped: Vec<String> = scopes.iter().flat_map(|(_, i)| i.skipped.clone()).collect();
    if files == 0 && !skipped.is_empty() {
        return unavailable(0, skipped.join("; "));
    }
    if files == 0 {
        return CallAnalysis::default();
    }
    let fingerprint = scopes_fingerprint(&scopes);
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    cached_result(cache, fingerprint, || {
        let analysis = analyze_scopes(&scopes);
        match prepare_scopes(root, config) {
            Ok(current) if scopes_fingerprint(&current) == fingerprint => analysis,
            _ => unavailable(files,
                "Java sources changed during analysis; the next refresh will analyze the new contents.".into()),
        }
    })
}

/// A caller-validated, explicit source closure. Workspace analysis supplies only
/// sources belonging to the current project and its declared visible projects.
pub(crate) fn analyze_java_workspace_calls(
    root: &Path,
    config: &Config,
    files: &[archguard_extract::SourceFile],
) -> CallAnalysis {
    let input = match prepare_sources(root, config, files) {
        Ok(input) => input,
        Err(error) => return unavailable(files.len(), error),
    };
    if input.files == 0 && !input.skipped.is_empty() {
        return unavailable(0, input.skipped.join("; "));
    }
    if input.files == 0 {
        return CallAnalysis::default();
    }
    cached_result(
        CACHE.get_or_init(|| Mutex::new(None)),
        input.fingerprint,
        || {
            let result = run_helper(&input, Path::new("java"))
                .map(|analysis| with_skipped(analysis, &input.skipped))
                .unwrap_or_else(|error| unavailable(input.files, error));
            match prepare_sources(root, config, files) {
                Ok(current) if current.fingerprint == input.fingerprint => result,
                _ => unavailable(
                    input.files,
                    "Workspace Java sources changed during analysis.".into(),
                ),
            }
        },
    )
}

/// Content fingerprint of the Java call inputs.
/// Includes compiler settings, build descriptors and explicit reference files.
pub fn java_calls_fingerprint(root: &Path, config: &Config) -> Result<[u8; 32], String> {
    prepare_scopes(root, config).map(|scopes| scopes_fingerprint(&scopes))
}

/// One scope's own fingerprint, or a digest over all scopes.
fn scopes_fingerprint(scopes: &[(String, Input)]) -> [u8; 32] {
    if let [(_, input)] = scopes {
        return input.fingerprint;
    }
    let mut hash = Sha256::new();
    for (project, input) in scopes {
        hash_part(&mut hash, project.as_bytes());
        hash_part(&mut hash, &input.fingerprint);
    }
    hash.finalize().into()
}

/// One compiler run per scope. Several scopes are projects whose classes
/// share names; their identities get the project directory as a prefix.
fn analyze_scopes(scopes: &[(String, Input)]) -> CallAnalysis {
    let run = |input: &Input| {
        run_helper(input, Path::new("java"))
            .map(|analysis| with_skipped(analysis, &input.skipped))
            .unwrap_or_else(|error| unavailable(input.files, error))
    };
    if let [(_, input)] = scopes {
        return run(input);
    }
    let mut result = CallAnalysis::default();
    let mut statuses = Vec::new();
    for (project, input) in scopes {
        let mut part = run(input);
        part.reidentify(&|id| format!("{project}::{id}"));
        statuses.push(part.status.clone());
        absorb(&mut result, project, part);
    }
    result.status = if statuses.iter().all(|s| s == "ready") {
        "ready".into()
    } else if statuses.iter().any(|s| s == "ready" || s == "partial") {
        "partial".into()
    } else {
        statuses.swap_remove(0)
    };
    result
}

/// Append one scope's report to the merged one.
fn absorb(result: &mut CallAnalysis, project: &str, part: CallAnalysis) {
    result.files += part.files;
    result.java_release = result.java_release.max(part.java_release);
    result.analyzers.extend(part.analyzers);
    result.classes.extend(part.classes);
    result.methods.extend(part.methods);
    result.calls.extend(part.calls);
    let diagnostics = part.diagnostics.into_iter();
    result
        .diagnostics
        .extend(diagnostics.map(|d| format!("{project}: {d}")));
    let Some(details) = part.details else { return };
    let merged = result
        .details
        .get_or_insert_with(|| archguard_core::SemanticDetails {
            definition: details.definition.clone(),
            enumeration_complete: Some(true),
            ..Default::default()
        });
    merged.enumeration_complete = Some(
        merged.enumeration_complete == Some(true) && details.enumeration_complete == Some(true),
    );
    merged.type_uses.extend(details.type_uses);
    merged.class_states.extend(details.class_states);
    merged.method_metrics.extend(details.method_metrics);
    merged.inheritance.extend(details.inheritance);
}

fn cached_result(
    cache: &Mutex<CacheEntry>,
    fingerprint: [u8; 32],
    analyze: impl FnOnce() -> CallAnalysis,
) -> CallAnalysis {
    if let Ok(guard) = cache.lock()
        && let Some((previous, analysis)) = &*guard
        && previous == &fingerprint
    {
        return analysis.clone();
    }
    let analysis = analyze();
    // Cache tool failures too: an unchanged project must not rerun a timed-out
    // compiler on every live poll. Source changes or a server restart retry it.
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((fingerprint, analysis.clone()));
    }
    analysis
}

fn unavailable(files: usize, message: String) -> CallAnalysis {
    CallAnalysis {
        status: "unavailable".into(),
        java_release: 0,
        files,
        diagnostics: vec![message.clone()],
        analyzers: vec![CallAnalyzer {
            language: "java".into(),
            backend: "javac".into(),
            status: "unavailable".into(),
            files,
            version: None,
            project: None,
            diagnostics: vec![message],
        }],
        ..CallAnalysis::default()
    }
}

/// The source's directories up to the root; their build descriptors are inputs.
fn add_directories(directories: &mut BTreeSet<PathBuf>, path: &Path, root: &Path) {
    for directory in path
        .ancestors()
        .skip(1)
        .take_while(|path| path.starts_with(root))
    {
        directories.insert(directory.to_path_buf());
    }
}

/// The text of a source file, or `None` after recording why it is left out.
/// The fingerprint sees the skip, so fixing the file invalidates the cache.
fn read_or_skip(
    path: &Path,
    rel: &str,
    hash: &mut Sha256,
    skipped: &mut Vec<String>,
) -> Option<String> {
    let why = match fs::read(path)
        .map_err(|e| e.to_string())
        .map(|b| decode_text(&b))
    {
        Ok(Decoded::Clean(text) | Decoded::Lossy(text)) => return Some(text),
        Ok(Decoded::Unreadable) => "not decodable (broken UTF-16)".to_string(),
        Err(why) => why,
    };
    hash_part(hash, format!("skipped:{}:{why}", path.display()).as_bytes());
    skipped.push(format!("Java source {rel} skipped: {why}"));
    None
}

/// Skipped files make a ready analysis partial and are named in the diagnostics.
fn with_skipped(mut analysis: CallAnalysis, skipped: &[String]) -> CallAnalysis {
    if skipped.is_empty() {
        return analysis;
    }
    if analysis.status == "ready" {
        analysis.status = "partial".into();
    }
    analysis.diagnostics.extend_from_slice(skipped);
    for analyzer in &mut analysis.analyzers {
        if analyzer.status == "ready" {
            analyzer.status = "partial".into();
        }
        analyzer.diagnostics.extend_from_slice(skipped);
    }
    analysis
}

fn hash_part(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_le_bytes());
    hash.update(value);
}

/// The compiler inputs: one for all sources, or one per project when
/// projects declare the same class, which javac rejects as a duplicate
/// class in one compilation (ADR 0010).
fn prepare_scopes(root: &Path, config: &Config) -> Result<Vec<(String, Input)>, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let files = crate::pipeline::walk_files(&root, config).map_err(|e| e.to_string())?;
    let Some(groups) = conflict_free_groups(&files, config) else {
        return Ok(vec![(
            String::new(),
            prepare_sources(&root, config, &files)?,
        )]);
    };
    let mut scopes = Vec::new();
    for group in groups {
        let own: Vec<_> = files
            .iter()
            .filter(|f| group.contains(&f.project))
            .cloned()
            .collect();
        let name = group.first().cloned().unwrap_or_default();
        scopes.push((name, prepare_sources(&root, config, &own)?));
    }
    Ok(scopes)
}

/// The projects of the Java sources in groups that each compile together,
/// when two projects declare a class of the same qualified name (javac
/// rejects that as a duplicate), else `None`. A project joins the first group
/// holding none of its twins, so modules that merely depend on each other
/// stay together and their calls resolve; only the twin modules (alternative
/// implementations) end up in different groups.
/// shortcut: calls between two groups are not resolved (they stay `unresolved`,
/// which the service reader reports as a gap); upgrade to per-application
/// groups once a repository needs the twin modules and their callers resolved.
fn conflict_free_groups(
    files: &[archguard_extract::SourceFile],
    config: &Config,
) -> Option<Vec<Vec<String>>> {
    let mut owners: std::collections::BTreeMap<String, BTreeSet<&str>> = Default::default();
    for file in files.iter().filter(|f| f.lang == Lang::Java) {
        // An unreadable file is reported by `prepare_sources`, which reads
        // every Java file of the scope again; here it only has no owner entry.
        let Some(Decoded::Clean(content) | Decoded::Lossy(content)) =
            fs::read(&file.path).ok().map(|bytes| decode_text(&bytes))
        else {
            continue;
        };
        let stripped = strip_root(&file.rel_path, &config.project.source_roots);
        let declarations = JavaExtractor.extract(&stripped, &content).declarations;
        let package = declarations.first().map_or("", |d| d.module_path.as_str());
        let name = file.rel_path.rsplit('/').next().unwrap_or_default();
        owners
            .entry(format!("{package}/{name}"))
            .or_default()
            .insert(&file.project);
    }
    let twins = |a: &str, b: &str| owners.values().any(|p| p.contains(a) && p.contains(b));
    if !owners.values().any(|projects| projects.len() > 1) {
        return None;
    }
    let all: BTreeSet<&String> = (files.iter())
        .filter(|f| f.lang == Lang::Java)
        .map(|f| &f.project)
        .collect();
    let mut groups: Vec<Vec<String>> = Vec::new();
    for project in all {
        match groups
            .iter_mut()
            .find(|g| !g.iter().any(|m| twins(m, project)))
        {
            Some(group) => group.push(project.clone()),
            None => groups.push(vec![project.clone()]),
        }
    }
    Some(groups)
}

#[cfg(test)]
fn prepare(root: &Path, config: &Config) -> Result<Input, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let files = crate::pipeline::walk_files(&root, config).map_err(|e| e.to_string())?;
    prepare_sources(&root, config, &files)
}

fn prepare_sources(
    root: &Path,
    config: &Config,
    files: &[archguard_extract::SourceFile],
) -> Result<Input, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !root.is_dir() {
        return Err("project path is not a directory".into());
    }
    let mut hash = Sha256::new();
    hash_part(&mut hash, HELPER.as_bytes());
    hash_part(&mut hash, root.to_string_lossy().as_bytes());
    let mut input = Input {
        manifest: Vec::new(),
        context: Vec::new(),
        fingerprint: [0; 32],
        files: 0,
        skipped: Vec::new(),
    };
    if !config.project.languages.is_empty()
        && !config
            .project
            .languages
            .iter()
            .any(|lang| lang == "auto" || lang == "java")
    {
        input.fingerprint = hash.finalize().into();
        return Ok(input);
    }
    let mut directories = BTreeSet::from([root.clone()]);
    for file in files {
        // Module descriptors hold no calls, and several of them in one
        // classpath compilation crash javac (Enter.moduleEnv assertion).
        if file.lang != Lang::Java
            || file
                .path
                .file_name()
                .is_some_and(|n| n == "module-info.java")
        {
            continue;
        }
        let Some(content) = read_or_skip(&file.path, &file.rel_path, &mut hash, &mut input.skipped)
        else {
            continue;
        };
        let stripped = strip_root(&file.rel_path, &config.project.source_roots);
        let extraction = JavaExtractor.extract(&stripped, &content);
        let node = crate::pipeline::file_node_name(file, &stripped, &extraction.declarations);
        let absolute = file.path.to_str().ok_or("Java source path is not UTF-8")?;
        add_directories(&mut directories, &file.path, &root);
        for field in [absolute, &stripped, &node] {
            input.manifest.extend_from_slice(field.as_bytes());
            input.manifest.push(0);
        }
        hash_part(&mut hash, absolute.as_bytes());
        hash_part(&mut hash, stripped.as_bytes());
        hash_part(&mut hash, node.as_bytes());
        hash_part(&mut hash, content.as_bytes());
        input.files += 1;
    }
    if input.files == 0 {
        input.fingerprint = hash.finalize().into();
        return Ok(input);
    }
    let settings = &config.semantic.java;
    let mut classpath = Vec::new();
    let mut entries = settings.classpath.clone();
    // Standard dependency-plugin/Gradle-export output, explicitly placed in
    // ArchGuard's data directory. Read references only; never run a build.
    match crate::project_files::read_repo_file(
        &root,
        Path::new(".archguard/java-classpath.txt"),
        1024 * 1024,
    ) {
        Ok(bytes) => {
            hash_part(&mut hash, &bytes);
            let text = std::str::from_utf8(&bytes)
                .map_err(|e| format!("Java classpath file: {e}"))?
                .trim();
            if text.contains(['\n', '\r', '\0']) {
                return Err(
                    "Java classpath file must contain one platform-separated path list".into(),
                );
            }
            if !text.is_empty() {
                for path in std::env::split_paths(text) {
                    entries.push(
                        path.to_str()
                            .ok_or("Java classpath entry is not UTF-8")?
                            .to_owned(),
                    );
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("Java classpath file: {e}")),
    }
    for entry in &entries {
        let path = root
            .join(entry)
            .canonicalize()
            .map_err(|e| format!("Java classpath {entry}: {e}"))?;
        // Configured references are data, not executable compiler plugins. Hash
        // every relevant file, including same-size/same-mtime replacements.
        hash_reference(&mut hash, &path, &path)?;
        classpath.push(path);
    }
    let classpath =
        std::env::join_paths(&classpath).map_err(|e| format!("Invalid Java classpath: {e}"))?;
    let classpath = classpath.to_str().ok_or("Java classpath is not UTF-8")?;
    let release = settings
        .release
        .map(|value| value.to_string())
        .unwrap_or_else(|| "auto".into());
    for field in [&release, classpath] {
        input.context.extend_from_slice(field.as_bytes());
        input.context.push(0);
    }
    let mut descriptors = BTreeSet::new();
    for directory in directories {
        for name in [
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
            "gradle.properties",
        ] {
            let path = directory.join(name);
            if !path.exists() {
                continue;
            }
            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
            if !canonical.starts_with(&root) {
                return Err("Java project descriptor escapes the analysis root".into());
            }
            let bytes = fs::read(&canonical).map_err(|e| e.to_string())?;
            hash_part(&mut hash, path.to_string_lossy().as_bytes());
            hash_part(&mut hash, &bytes);
            if matches!(name, "pom.xml" | "build.gradle" | "build.gradle.kts") {
                descriptors.insert(canonical);
            }
        }
    }
    for path in descriptors {
        input.context.extend_from_slice(
            path.to_str()
                .ok_or("Java project path is not UTF-8")?
                .as_bytes(),
        );
        input.context.push(0);
    }
    hash_part(&mut hash, &input.context);
    input.fingerprint = hash.finalize().into();
    Ok(input)
}

fn hash_reference(hash: &mut Sha256, base: &Path, path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Java classpath contains a symlink: {}",
            path.display()
        ));
    }
    hash_part(hash, path.to_string_lossy().as_bytes());
    if metadata.is_dir() {
        let mut entries = fs::read_dir(path)
            .map_err(|e| e.to_string())?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort();
        for entry in entries {
            hash_reference(hash, base, &entry)?;
        }
    } else if metadata.is_file()
        && (path == base
            || path
                .extension()
                .is_some_and(|extension| extension == "class" || extension == "jar"))
    {
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        hash.update(metadata.len().to_le_bytes());
        let mut buffer = [0; 65536];
        loop {
            let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if read == 0 {
                break;
            }
            hash.update(&buffer[..read]);
        }
    }
    Ok(())
}

struct WorkingDirectory(PathBuf);
impl WorkingDirectory {
    fn create() -> io::Result<Self> {
        for _ in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "archguard-java-{}-{}",
                std::process::id(),
                TEMP_ID.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other(
            "could not create isolated Java analysis directory",
        ))
    }
}
impl Drop for WorkingDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read_bounded(path: &Path, maximum: u64) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    if file.metadata()?.len() > maximum {
        return Err(io::Error::other(
            "Java analysis output exceeds its size limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(io::Error::other(
            "Java analysis output exceeds its size limit",
        ));
    }
    Ok(bytes)
}

fn heap_mib(files: usize) -> usize {
    (files / 4).clamp(MIN_HEAP_MIB, MAX_HEAP_MIB)
}

fn process_time(files: usize) -> Duration {
    Duration::from_secs((files as u64 / 100).max(MIN_PROCESS_SECS))
}

#[tracing::instrument(skip_all)]
fn run_helper(input: &Input, executable: &Path) -> Result<CallAnalysis, String> {
    let directory = WorkingDirectory::create().map_err(|e| e.to_string())?;
    let helper = directory.0.join("ArchguardCallScanner.java");
    let manifest = directory.0.join("sources");
    let context = directory.0.join("context");
    let empty = directory.0.join("empty");
    let output = directory.0.join("output.json");
    let errors = directory.0.join("errors.txt");
    fs::write(&helper, HELPER).map_err(|e| e.to_string())?;
    fs::write(&manifest, &input.manifest).map_err(|e| e.to_string())?;
    fs::write(&context, &input.context).map_err(|e| e.to_string())?;
    fs::create_dir(&empty).map_err(|e| e.to_string())?;
    let stdout = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .map_err(|e| e.to_string())?;
    let stderr = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&errors)
        .map_err(|e| e.to_string())?;
    let mut command = Command::new(executable);
    command
        .arg(format!("-Xmx{}m", heap_mib(input.files)))
        .arg("--add-exports")
        .arg(JAVAC_API_EXPORT)
        .arg("--class-path")
        .arg(&empty)
        .arg(&helper)
        .arg(&manifest)
        .arg(&empty)
        .arg(&context)
        .current_dir(&directory.0)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    for name in JAVA_ENV {
        command.env_remove(name);
    }
    let mut child = command.spawn().map_err(|e| {
        format!("Java call analysis requires JDK 17+ available as java on PATH: {e}")
    })?;
    let limit = process_time(input.files);
    let outputs = [output.as_path(), errors.as_path()];
    let status = match crate::go_calls::wait(&mut child, &outputs, limit, MAX_OUTPUT_BYTES) {
        Ok(status) => status,
        Err(crate::go_calls::Stop::Failed(error)) => {
            return Err(format!("Java call analysis process failed: {error}"));
        }
        Err(crate::go_calls::Stop::Time) => {
            return Err(format!(
                "Java call analysis exceeded its {} second limit; import analysis remains available.",
                limit.as_secs()
            ));
        }
        Err(crate::go_calls::Stop::Output) => {
            return Err(
                "Java call analysis exceeded its output limit; import analysis remains available."
                    .into(),
            );
        }
    };
    if !status.success() {
        let detail = read_bounded(&errors, MAX_OUTPUT_BYTES).unwrap_or_default();
        let detail: String = String::from_utf8_lossy(&detail)
            .chars()
            .take(1500)
            .collect();
        if detail.contains("java.lang.OutOfMemoryError") {
            return Err(format!(
                "Java call analysis ran out of its {} MiB heap for {} files; import analysis remains available.",
                heap_mib(input.files),
                input.files
            ));
        }
        return Err(format!("Java call analysis failed: {detail}"));
    }
    let bytes = read_bounded(&output, MAX_OUTPUT_BYTES).map_err(|e| e.to_string())?;
    let result: CallAnalysis = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Java call analysis returned invalid data: {e}"))?;
    if result.files != input.files || !matches!(result.status.as_str(), "ready" | "partial") {
        return Err("Java call analysis returned inconsistent source coverage".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-javac tests run only where a JDK is installed.
    fn project_with(files: &[(&str, &[u8])]) -> Option<tempfile::TempDir> {
        let jdk = |tool: &str| {
            Command::new(tool)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !(jdk("java") && jdk("javac")) {
            return None;
        }
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("src/main/java/de/x");
        fs::create_dir_all(&dir).unwrap();
        for (name, bytes) in files {
            fs::write(dir.join(name), bytes).unwrap();
        }
        Some(root)
    }
    const GOOD: &[u8] =
        b"package de.x;\npublic class Good { void run() { help(); } void help() {} }\n";

    #[test]
    fn an_undecodable_file_skips_only_itself_and_says_so() {
        // Odd byte count after a UTF-16LE mark: decode_text yields Unreadable.
        let Some(root) = project_with(&[("Bad.java", b"\xFF\xFEa"), ("Good.java", GOOD)]) else {
            return;
        };
        let analysis = analyze_java_calls(root.path(), &Config::default());
        assert_eq!(analysis.status, "partial", "{:?}", analysis.diagnostics);
        assert!(analysis.calls.iter().any(|c| c.code == "help()"));
        assert!(analysis.diagnostics.iter().any(|d| d.contains("Bad.java")));
        assert!(
            analysis.analyzers[0]
                .diagnostics
                .iter()
                .any(|d| d.contains("Bad.java"))
        );
    }

    #[test]
    fn only_unreadable_files_give_unavailable_with_their_messages() {
        let Some(root) = project_with(&[("Bad.java", b"\xFF\xFEa")]) else {
            return;
        };
        let analysis = analyze_java_calls(root.path(), &Config::default());
        assert_eq!(analysis.status, "unavailable", "{:?}", analysis.diagnostics);
        assert!(analysis.diagnostics.iter().any(|d| d.contains("Bad.java")));
    }

    #[test]
    fn a_latin1_file_is_read_and_not_reported_as_skipped() {
        let latin1: &[u8] = b"package de.x;\n// \xE4\npublic class Latin { void run() { help(); } void help() {} }\n";
        let Some(root) = project_with(&[("Latin.java", latin1), ("Good.java", GOOD)]) else {
            return;
        };
        let analysis = analyze_java_calls(root.path(), &Config::default());
        assert_eq!(analysis.files, 2, "{:?}", analysis.diagnostics);
        assert!(analysis.calls.len() >= 2);
        assert!(!analysis.diagnostics.iter().any(|d| d.contains("skipped")));
    }

    #[test]
    fn grouping_reads_latin1_and_utf16_like_the_scan() {
        // Same-named class in two projects forces conflict_free_groups; the
        // Latin-1 twin must still count as an owner, the broken one must not panic.
        let root = tempfile::tempdir().unwrap();
        for (project, bytes) in [
            ("a", &b"package de.x;\n// \xE4\nclass T {}\n"[..]),
            ("b", GOOD_T),
        ] {
            let dir = root.path().join(project).join("src/main/java/de/x");
            fs::create_dir_all(&dir).unwrap();
            fs::write(root.path().join(project).join("pom.xml"), "<project/>").unwrap();
            fs::write(dir.join("T.java"), bytes).unwrap();
            fs::write(dir.join("Bad.java"), b"\xFF\xFEa").unwrap();
        }
        let config = Config::default();
        let files = crate::pipeline::walk_files(root.path(), &config).unwrap();
        let groups = conflict_free_groups(&files, &config).expect("twin T.java");
        assert_eq!(groups.len(), 2, "{groups:?}");
    }
    const GOOD_T: &[u8] = b"package de.x;\nclass T {}\n";

    #[test]
    fn no_java_does_not_require_a_jdk_or_change_import_analysis() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("index.ts"), "export const answer=42;").unwrap();
        assert_eq!(
            analyze_java_calls(root.path(), &Config::default()).status,
            "not_applicable"
        );
    }

    /// Two projects declare the same class: each project's calls reach its
    /// own copy, and the copies keep apart identities (ADR 0010).
    #[test]
    fn projects_sharing_a_class_name_call_their_own_copy() {
        let root = tempfile::tempdir().unwrap();
        for project in ["functions/a", "functions/b"] {
            let java = root.path().join(project).join("src/main/java/de/x");
            fs::create_dir_all(java.join("common")).unwrap();
            fs::write(root.path().join(project).join("pom.xml"), "<project/>").unwrap();
            fs::write(
                java.join("App.java"),
                "package de.x;\nimport de.x.common.Log;\npublic class App { void run() { Log.write(); } }\n",
            )
            .unwrap();
            fs::write(
                java.join("common/Log.java"),
                "package de.x.common;\npublic class Log { public static void write() {} }\n",
            )
            .unwrap();
        }
        let result = analyze_java_calls(root.path(), &Config::default());
        assert_eq!(result.status, "ready", "{:?}", result.diagnostics);
        let node = |id: &str| {
            let class = result.classes.iter().find(|c| c.id == id);
            class.map(|c| c.graph_node.clone()).unwrap_or_default()
        };
        let mut edges: Vec<(String, String)> = result
            .calls
            .iter()
            .filter(|c| c.to_method.as_deref().is_some_and(|m| m.contains("write")))
            .map(|c| {
                (
                    node(&c.from_class),
                    node(c.to_class.as_deref().unwrap_or("")),
                )
            })
            .collect();
        edges.sort();
        assert_eq!(
            edges,
            [
                (
                    "functions/a/de/x/App.java".into(),
                    "functions/a/de/x/common/Log.java".into()
                ),
                (
                    "functions/b/de/x/App.java".into(),
                    "functions/b/de/x/common/Log.java".into()
                ),
            ]
        );
        let ids: BTreeSet<&str> = result.classes.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), result.classes.len(), "one identity per class");
    }

    /// Two alternative modules declare the same class, a third module calls
    /// a class of the first: the call is still resolved to its target.
    #[test]
    fn a_duplicate_in_two_modules_does_not_cut_calls_between_the_others() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let file = root.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        };
        for p in ["a", "b", "c"] {
            write(&format!("{p}/pom.xml"), "<project/>");
        }
        for p in ["a", "b"] {
            write(
                &format!("{p}/src/main/java/de/x/Log.java"),
                "package de.x;\npublic class Log {}\n",
            );
        }
        write(
            "a/src/main/java/de/x/Svc.java",
            "package de.x;\npublic interface Svc { void run(); }\n",
        );
        write(
            "c/src/main/java/de/y/Ctl.java",
            "package de.y;\nimport de.x.Svc;\npublic class Ctl { Svc svc; void go() { svc.run(); } }\n",
        );
        let result = analyze_java_calls(root.path(), &Config::default());
        let resolved: Vec<_> = (result.calls.iter())
            .filter(|c| c.target_label.ends_with("run()"))
            .map(|c| c.resolution.as_str())
            .collect();
        assert_eq!(resolved, ["project"], "{:?}", result.diagnostics);
    }

    /// A project without its dependency jars reports thousands of compiler
    /// errors; calls in files without an error must still resolve.
    #[test]
    fn many_errors_elsewhere_do_not_unresolve_a_clean_call() {
        let root = tempfile::tempdir().unwrap();
        let java = root.path().join("src/main/java/de/x");
        fs::create_dir_all(&java).unwrap();
        let fields: String = (0..12_000).map(|i| format!("Nope{i} f{i};\n")).collect();
        fs::write(
            java.join("Bad.java"),
            format!("package de.x;\nclass Bad {{\n{fields}}}\n"),
        )
        .unwrap();
        fs::write(
            java.join("Ok.java"),
            "package de.x;\npublic class Ok { void a() { b(); } void b() {} }\n",
        )
        .unwrap();
        let result = analyze_java_calls(root.path(), &Config::default());
        let resolved: Vec<_> = (result.calls.iter())
            .filter(|c| c.code == "b()")
            .map(|c| c.resolution.as_str())
            .collect();
        assert_eq!(resolved, ["project"], "{:?}", result.diagnostics);
    }

    /// A project that declares a newer Java release than the installed JDK
    /// supports (syncope: 25 on a JDK 23) is still analysed, with a diagnostic.
    #[test]
    fn a_release_newer_than_the_jdk_falls_back_to_the_jdk_release() {
        let root = tempfile::tempdir().unwrap();
        let java = root.path().join("src/main/java/de/x");
        fs::create_dir_all(&java).unwrap();
        fs::write(
            java.join("Ok.java"),
            "package de.x;\npublic class Ok { void a() { b(); } void b() {} }\n",
        )
        .unwrap();
        let mut config = Config::default();
        config.semantic.java.release = Some(99);
        let result = analyze_java_calls(root.path(), &config);
        assert_ne!(result.status, "unavailable", "{:?}", result.diagnostics);
        assert!(
            (result.diagnostics.iter()).any(|d| d.contains("release 99")),
            "{:?}",
            result.diagnostics
        );
    }

    #[test]
    fn no_java_ignores_unavailable_java_references() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("index.ts"), "export const answer=42;").unwrap();
        let mut config = Config::default();
        config.semantic.java.classpath = vec!["missing.jar".into()];
        assert_eq!(
            analyze_java_calls(root.path(), &config).status,
            "not_applicable"
        );
    }

    #[test]
    fn context_fingerprint_covers_descriptors_settings_and_reference_contents() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("A.java"), "class A {}").unwrap();
        let jar = root.path().join("api.jar");
        fs::write(&jar, b"version-one").unwrap();
        let mut config = Config::default();
        config.semantic.java.classpath = vec!["api.jar".into()];
        let first = prepare(root.path(), &config).unwrap();
        let context = String::from_utf8(first.context).unwrap();
        assert!(context.starts_with("auto\0"));
        assert!(context.contains(jar.to_str().unwrap()));
        fs::write(&jar, b"version-two").unwrap();
        let second = java_calls_fingerprint(root.path(), &config).unwrap();
        assert_ne!(
            first.fingerprint, second,
            "same-size reference edits invalidate calls"
        );
        let pom = root.path().join("pom.xml");
        fs::write(&pom, "<project><properties><maven.compiler.release>17</maven.compiler.release></properties></project>").unwrap();
        let third = java_calls_fingerprint(root.path(), &config).unwrap();
        assert_ne!(second, third);
        fs::write(&pom, "<project><properties><maven.compiler.release>21</maven.compiler.release></properties></project>").unwrap();
        assert_ne!(third, java_calls_fingerprint(root.path(), &config).unwrap());
        config.semantic.java.release = Some(11);
        let explicit = prepare(root.path(), &config).unwrap();
        assert!(
            String::from_utf8(explicit.context)
                .unwrap()
                .starts_with("11\0")
        );
    }

    #[test]
    fn nested_project_descriptor_is_part_of_context_and_cache() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("app/src/main/java")).unwrap();
        fs::write(root.path().join("app/src/main/java/A.java"), "class A {}").unwrap();
        let descriptor = root.path().join("app/build.gradle.kts");
        fs::write(
            &descriptor,
            "java { toolchain { languageVersion.set(JavaLanguageVersion.of(17)) } }",
        )
        .unwrap();
        let first = prepare(root.path(), &Config::default()).unwrap();
        assert!(
            String::from_utf8(first.context)
                .unwrap()
                .contains(descriptor.to_str().unwrap())
        );
        fs::write(root.path().join("app/gradle.properties"), "javaVersion=21").unwrap();
        assert_ne!(
            first.fingerprint,
            java_calls_fingerprint(root.path(), &Config::default()).unwrap()
        );
    }

    #[test]
    fn missing_project_is_an_explained_unavailable_result() {
        let root = tempfile::tempdir().unwrap();
        let result = analyze_java_calls(&root.path().join("missing"), &Config::default());
        assert_eq!(result.status, "unavailable");
        assert!(!result.diagnostics.is_empty());
    }

    #[test]
    fn source_manifest_obeys_excludes_roots_and_language_and_hashes_method_bodies() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("src/main")).unwrap();
        fs::create_dir_all(root.path().join("src/tests")).unwrap();
        let source = root.path().join("src/main/A.java");
        fs::write(&source, "package app;\nclass A { void a() { a(); } }").unwrap();
        fs::write(root.path().join("src/tests/Test.java"), "class Test {}").unwrap();
        let mut config = Config::default();
        config.project.source_roots = vec!["src/main".into()];
        let first = prepare(root.path(), &config).unwrap();
        assert_eq!(first.files, 1);
        let manifest = String::from_utf8(first.manifest).unwrap();
        assert!(manifest.ends_with("\0A.java\0app/A.java\0"));
        fs::write(&source, "package app;\nclass A { void a() { } }").unwrap();
        let second = prepare(root.path(), &config).unwrap();
        assert_ne!(
            first.fingerprint, second.fingerprint,
            "method body changes invalidate even identical imports"
        );
        assert_eq!(
            second.fingerprint,
            prepare(root.path(), &config).unwrap().fingerprint
        );
        config.project.languages = vec!["ts".into()];
        assert_eq!(prepare(root.path(), &config).unwrap().files, 0);
    }

    #[test]
    fn module_descriptors_stay_out_of_the_single_module_compiler_run() {
        // javac asserts in Enter.moduleEnv when several module-info.java files
        // share one classpath compilation (Elasticsearch has 115).
        let root = tempfile::tempdir().unwrap();
        for module in ["a", "b"] {
            let dir = root.path().join(module);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("module-info.java"),
                format!("module {module} {{}}"),
            )
            .unwrap();
            fs::write(
                dir.join("A.java"),
                format!("package {module};\nclass A {{}}"),
            )
            .unwrap();
        }
        let input = prepare(root.path(), &Config::default()).unwrap();
        assert_eq!(input.files, 2);
        assert!(
            !String::from_utf8(input.manifest)
                .unwrap()
                .contains("module-info")
        );
    }

    #[test]
    fn heap_and_time_limits_grow_with_the_source_count() {
        assert_eq!((heap_mib(367), process_time(367).as_secs()), (512, 45));
        assert_eq!(
            (heap_mib(16_714), process_time(16_714).as_secs()),
            (4_178, 167)
        );
        assert_eq!(heap_mib(1_000_000), MAX_HEAP_MIB);
    }

    #[cfg(unix)]
    #[test]
    fn heap_exhaustion_is_reported_as_memory_not_as_a_missing_jdk() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let java = root.path().join("java");
        fs::write(
            &java,
            "#!/bin/sh\necho 'java.lang.OutOfMemoryError: Java heap space' >&2\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&java, fs::Permissions::from_mode(0o755)).unwrap();
        let input = Input {
            manifest: Vec::new(),
            context: Vec::new(),
            fingerprint: [0; 32],
            files: 20_000,
            skipped: Vec::new(),
        };
        let error = run_helper(&input, &java).unwrap_err();
        assert!(!error.contains("JDK 17+"), "{error}");
        assert!(error.contains("MiB heap"), "{error}");
    }

    #[test]
    fn unchanged_unavailable_analysis_is_cached_without_respawning_java() {
        let cache = Mutex::new(None);
        let invocations = std::cell::Cell::new(0);
        let analyze = || {
            invocations.set(invocations.get() + 1);
            unavailable(1, "Java analysis timed out".into())
        };
        assert_eq!(
            cached_result(&cache, [1; 32], analyze).status,
            "unavailable"
        );
        assert_eq!(
            cached_result(&cache, [1; 32], analyze).status,
            "unavailable"
        );
        assert_eq!(
            invocations.get(),
            1,
            "a failed Java process must not repeat on every live poll"
        );
        cached_result(&cache, [2; 32], analyze);
        assert_eq!(
            invocations.get(),
            2,
            "changed source contents retry the optional analysis"
        );
    }

    /// A repository-supplied classpath jar must stay data: javac would otherwise
    /// load its `com.sun.source.util.Plugin` service and run it (autoStart),
    /// because without a processor path it searches the classpath.
    #[test]
    fn repository_classpath_never_loads_compiler_plugins() {
        let jdk = |tool: &str| {
            Command::new(tool)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !(jdk("java") && jdk("javac") && jdk("jar")) {
            eprintln!("skipped: no JDK (java, javac, jar) on PATH");
            return;
        }
        let build = tempfile::tempdir().unwrap();
        let marker = build.path().join("plugin-ran");
        let plugin = build.path().join("src/evil/Evil.java");
        fs::create_dir_all(plugin.parent().unwrap()).unwrap();
        fs::write(
            &plugin,
            format!(
                "package evil;\npublic class Evil implements com.sun.source.util.Plugin {{\n\
                 static {{ try {{ java.nio.file.Files.writeString(java.nio.file.Path.of({:?}), \"x\"); }} catch (Exception e) {{}} }}\n\
                 public String getName() {{ return \"Evil\"; }}\n\
                 public boolean autoStart() {{ return true; }}\n\
                 public void init(com.sun.source.util.JavacTask task, String... args) {{}}\n}}\n",
                marker.to_str().unwrap()
            ),
        )
        .unwrap();
        let classes = build.path().join("classes");
        let services = classes.join("META-INF/services");
        fs::create_dir_all(&services).unwrap();
        fs::write(services.join("com.sun.source.util.Plugin"), "evil.Evil\n").unwrap();
        let run = |command: &mut Command| assert!(command.status().unwrap().success());
        run(Command::new("javac").arg("-d").arg(&classes).arg(&plugin));
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("A.java"),
            "class A { String a() { return new evil.Evil().getName(); } }",
        )
        .unwrap();
        run(Command::new("jar")
            .arg("cf")
            .arg(root.path().join("plugin.jar"))
            .arg("-C")
            .arg(&classes)
            .arg("."));
        let mut config = Config::default();
        config.semantic.java.classpath = vec!["plugin.jar".into()];
        let result = analyze_java_calls(root.path(), &config);
        assert!(
            !marker.exists(),
            "javac ran a plugin from the repository classpath"
        );
        assert_eq!(result.status, "ready", "{:?}", result.diagnostics);
        assert!(
            result
                .calls
                .iter()
                .any(|c| c.to_class.as_deref() == Some("evil.Evil")),
            "classpath symbols still resolve: {:?}",
            result.calls
        );
    }

    #[test]
    fn missing_java_is_reported_and_temporary_directory_is_removed() {
        let input = Input {
            manifest: Vec::new(),
            context: Vec::new(),
            fingerprint: [0; 32],
            files: 1,
            skipped: Vec::new(),
        };
        let root = tempfile::tempdir().unwrap();
        assert!(
            run_helper(&input, &root.path().join("no-java-executable"))
                .unwrap_err()
                .contains("JDK 17+")
        );
        let path;
        {
            let directory = WorkingDirectory::create().unwrap();
            path = directory.0.clone();
            assert!(path.is_dir());
        }
        assert!(!path.exists());
    }
}
#[test]
fn exported_classpath_file_is_read_and_hashed_without_executing_a_build() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("A.java"), "class A {}").unwrap();
    fs::create_dir(root.path().join(".archguard")).unwrap();
    fs::write(root.path().join("dependency.jar"), "one").unwrap();
    fs::write(
        root.path().join(".archguard/java-classpath.txt"),
        "dependency.jar\n",
    )
    .unwrap();
    let config = Config::default();
    let first = prepare(root.path(), &config).unwrap();
    assert!(
        String::from_utf8(first.context.clone())
            .unwrap()
            .contains("dependency.jar")
    );
    fs::write(root.path().join("dependency.jar"), "two").unwrap();
    assert_ne!(
        first.fingerprint,
        prepare(root.path(), &config).unwrap().fingerprint
    );
    fs::write(
        root.path().join(".archguard/java-classpath.txt"),
        "missing.jar",
    )
    .unwrap();
    assert!(
        prepare(root.path(), &config).is_err(),
        "invalid classpath must not silently fall back"
    );
}
