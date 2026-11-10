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
}

/// Java class/method call sites for the effective source configuration. The
/// optional JDK backend reports an unavailable status instead of failing import
/// analysis. Results are reused only while source *contents* and paths match.
/// Thus changes inside method bodies invalidate calls even if imports do not.
/// The single cached project bounds memory and serves the live viewer's polls.
#[tracing::instrument(skip_all)]
pub fn analyze_java_calls(root: &Path, config: &Config) -> CallAnalysis {
    let input = match prepare(root, config) {
        Ok(input) => input,
        Err(error) => {
            return unavailable(0, format!("Java call sources could not be read: {error}"));
        }
    };
    if input.files == 0 {
        return CallAnalysis::default();
    }
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    cached_result(cache, input.fingerprint, || {
        let analysis = run_helper(&input, Path::new("java"))
            .unwrap_or_else(|error| unavailable(input.files, error));
        match prepare(root, config) {
            Ok(current) if current.fingerprint == input.fingerprint => analysis,
            _ => unavailable(input.files,
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
    if input.files == 0 {
        return CallAnalysis::default();
    }
    cached_result(
        CACHE.get_or_init(|| Mutex::new(None)),
        input.fingerprint,
        || {
            let result = run_helper(&input, Path::new("java"))
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

/// Content fingerprint used by the live viewer before reusing a rendered page.
/// Includes compiler settings, build descriptors and explicit reference files.
pub fn java_calls_fingerprint(root: &Path, config: &Config) -> Result<[u8; 32], String> {
    prepare(root, config).map(|input| input.fingerprint)
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

fn hash_part(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_le_bytes());
    hash.update(value);
}

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
        let content = fs::read_to_string(&file.path).map_err(|e| e.to_string())?;
        let stripped = strip_root(&file.rel_path, &config.project.source_roots);
        let extraction = JavaExtractor.extract(&stripped, &content);
        let node = crate::pipeline::file_node_name(file, &stripped, &extraction.declarations);
        let absolute = file.path.to_str().ok_or("Java source path is not UTF-8")?;
        for directory in file
            .path
            .ancestors()
            .skip(1)
            .take_while(|path| path.starts_with(&root))
        {
            directories.insert(directory.to_path_buf());
        }
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

    #[test]
    fn no_java_does_not_require_a_jdk_or_change_import_analysis() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("index.ts"), "export const answer=42;").unwrap();
        assert_eq!(
            analyze_java_calls(root.path(), &Config::default()).status,
            "not_applicable"
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

    #[test]
    fn missing_java_is_reported_and_temporary_directory_is_removed() {
        let input = Input {
            manifest: Vec::new(),
            context: Vec::new(),
            fingerprint: [0; 32],
            files: 1,
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
