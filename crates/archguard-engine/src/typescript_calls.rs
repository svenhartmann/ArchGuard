//! Installed TypeScript compiler API, with explicit source membership and no emit.
//! The helper loads no project sources, plugins or package scripts. The compiler
//! itself is JavaScript found through the analyzed repository, so it only runs
//! after the operator opts in via [`TRUST_ENV`]; scanning alone executes nothing.
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use archguard_core::{CallAnalysis, CallAnalyzer, Config};
use archguard_extract::{Lang, strip_root};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const HELPER: &str = include_str!("ArchguardCallScanner.cjs");
const LIMIT: u64 = 64 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(45);
/// Set to `1` outside the repository to run the compiler the repository names.
const TRUST_ENV: &str = "ARCHGUARD_TRUST_PROJECT_COMPILER";
static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();

#[derive(Clone, Deserialize)]
struct TrackedInput {
    file: PathBuf,
    sha256: Option<String>,
}
#[derive(Deserialize)]
struct HelperResult {
    analysis: CallAnalysis,
    inputs: Vec<TrackedInput>,
}
struct Cached {
    fingerprint: [u8; 32],
    inputs: Vec<TrackedInput>,
    analysis: CallAnalysis,
}
struct Input {
    manifest: Value,
    fingerprint: [u8; 32],
    files: usize,
    root: PathBuf,
    compiler: Option<PathBuf>,
    /// A compiler exists but was not selected because [`TRUST_ENV`] is unset.
    blocked_compiler: bool,
}

fn project_compiler_trusted() -> bool {
    crate::compiler_trust::trusted()
}

pub fn analyze_typescript_calls(root: &Path, config: &Config) -> CallAnalysis {
    analyze(root, config, project_compiler_trusted())
}

fn analyze(root: &Path, config: &Config, trusted: bool) -> CallAnalysis {
    let input = match prepare(root, config, trusted) {
        Ok(input) => input,
        Err(error) => return unavailable(0, error),
    };
    if input.files == 0 {
        return CallAnalysis {
            java_release: 0,
            ..Default::default()
        };
    }
    if input.blocked_compiler {
        return unavailable(
            input.files,
            format!(
                "A TypeScript compiler was found through this repository but not loaded, because running it would execute repository code. Set {TRUST_ENV}=1 for repositories you trust."
            ),
        );
    }
    if input.compiler.is_none() {
        return unavailable(input.files, "No installed TypeScript compiler found. Install TypeScript in the project or configure semantic.typescript.compiler; ArchGuard never downloads packages.".into());
    }
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock()
        && let Some(previous) = &*guard
        && previous.fingerprint == input.fingerprint
        && inputs_current(&previous.inputs, &input)
    {
        return previous.analysis.clone();
    }
    let result = run_helper(&input).and_then(|result| {
        let current = prepare(root, config, trusted)?;
        if current.fingerprint != input.fingerprint || !inputs_current(&result.inputs, &input) {
            return Err(
                "TypeScript source/compiler context changed during analysis; refresh to retry."
                    .into(),
            );
        }
        Ok(result)
    });
    // A failure may be transient (timeout, concurrent edit): keep it retryable.
    let (analysis, inputs) = match result {
        Ok(result) => (result.analysis, result.inputs),
        Err(error) => return unavailable(input.files, error),
    };
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(Cached {
            fingerprint: input.fingerprint,
            inputs,
            analysis: analysis.clone(),
        });
    }
    analysis
}

/// Includes project sources and the compiler inputs observed by the cached run.
pub fn typescript_calls_fingerprint(root: &Path, config: &Config) -> Result<[u8; 32], String> {
    let input = prepare(root, config, project_compiler_trusted())?;
    let mut hash = Sha256::new();
    part(&mut hash, &input.fingerprint);
    if let Some(cache) = CACHE.get()
        && let Ok(guard) = cache.lock()
        && let Some(previous) = &*guard
        && previous.fingerprint == input.fingerprint
    {
        for dependency in &previous.inputs {
            part(&mut hash, dependency.file.to_string_lossy().as_bytes());
            part(
                &mut hash,
                approved_hash(&dependency.file, &input)
                    .unwrap_or_else(Some)
                    .as_deref()
                    .unwrap_or("missing")
                    .as_bytes(),
            );
        }
    }
    Ok(hash.finalize().into())
}

fn unavailable(files: usize, message: String) -> CallAnalysis {
    CallAnalysis {
        status: "unavailable".into(),
        java_release: 0,
        files,
        analyzers: vec![CallAnalyzer {
            language: "typescript".into(),
            backend: "typescript".into(),
            status: "unavailable".into(),
            files,
            version: None,
            project: None,
            diagnostics: vec![message.clone()],
        }],
        diagnostics: vec![message],
        ..Default::default()
    }
}
fn part(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}
fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    if !file
        .metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= limit)
    {
        return Err("TypeScript input/output size limit exceeded".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("TypeScript input/output size limit exceeded".into());
    }
    Ok(bytes)
}
fn current_hash(path: &Path) -> Result<Option<String>, String> {
    match fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
        Ok(metadata) if !metadata.is_file() => Err("compiler input is not a file".into()),
        Ok(_) => Ok(Some(format!(
            "{:x}",
            Sha256::digest(bounded(path, 16 * 1024 * 1024)?)
        ))),
    }
}
fn approved_hash(path: &Path, input: &Input) -> Result<Option<String>, String> {
    if let Ok(canonical) = path.canonicalize() {
        let compiler_root = input
            .compiler
            .as_ref()
            .and_then(|p| p.parent())
            .and_then(Path::parent);
        if !canonical.starts_with(&input.root)
            && !compiler_root.is_some_and(|root| canonical.starts_with(root))
        {
            return Err("Cached TypeScript input leaves the analysis/compiler roots".into());
        }
    }
    current_hash(path)
}
fn inputs_current(inputs: &[TrackedInput], input: &Input) -> bool {
    inputs
        .iter()
        .all(|file| approved_hash(&file.file, input).is_ok_and(|hash| hash == file.sha256))
}
fn compiler_candidate(root: &Path, config: &Config) -> Option<PathBuf> {
    if let Some(configured) = &config.semantic.typescript.compiler {
        Some(root.join(configured))
    } else {
        root.ancestors()
            .map(|dir| dir.join("node_modules/typescript/lib/typescript.js"))
            .find(|file| file.is_file())
    }
}
fn validated_compiler(selected: &Path) -> Result<PathBuf, String> {
    let selected = selected
        .canonicalize()
        .map_err(|e| format!("TypeScript compiler: {e}"))?;
    if selected
        .file_name()
        .is_none_or(|name| name != "typescript.js")
        || selected
            .parent()
            .and_then(Path::file_name)
            .is_none_or(|name| name != "lib")
    {
        return Err(
            "semantic.typescript.compiler must name an installed typescript/lib/typescript.js"
                .into(),
        );
    }
    let manifest = selected
        .parent()
        .and_then(Path::parent)
        .ok_or("Invalid TypeScript compiler location")?
        .join("package.json");
    let package: Value =
        serde_json::from_slice(&bounded(&manifest, 1024 * 1024)?).map_err(|e| e.to_string())?;
    if package["name"] != "typescript" || !package["version"].is_string() {
        return Err("Configured compiler is not an installed TypeScript package".into());
    }
    Ok(selected)
}
fn prepare(root: &Path, config: &Config, trusted: bool) -> Result<Input, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    part(&mut hash, HELPER.as_bytes());
    part(&mut hash, root.to_string_lossy().as_bytes());
    part(
        &mut hash,
        &serde_json::to_vec(&config.semantic.typescript).map_err(|e| e.to_string())?,
    );
    let mut sources = Vec::new();
    let mut directories = BTreeSet::from([root.clone()]);
    let mut total_bytes = 0;
    for file in crate::pipeline::walk_files(&root, config).map_err(|e| e.to_string())? {
        if !matches!(file.lang, Lang::Ts | Lang::Js) {
            continue;
        }
        let absolute = file.path.canonicalize().map_err(|e| e.to_string())?;
        if !absolute.starts_with(&root) {
            return Err("TypeScript source leaves the analysis root".into());
        }
        let bytes = bounded(&absolute, 8 * 1024 * 1024)?;
        total_bytes += bytes.len();
        if total_bytes as u64 > LIMIT || sources.len() >= 20000 {
            return Err("TypeScript source budget exceeded".into());
        }
        let stripped = strip_root(&file.rel_path, &config.project.source_roots);
        part(&mut hash, absolute.to_string_lossy().as_bytes());
        part(&mut hash, stripped.as_bytes());
        part(&mut hash, &bytes);
        sources.push(json!({"path":absolute,"file":stripped,"graph_node":stripped,"language":if file.lang==Lang::Ts {"typescript"} else {"javascript"}}));
        for dir in absolute
            .ancestors()
            .skip(1)
            .take_while(|dir| dir.starts_with(&root))
        {
            directories.insert(dir.to_path_buf());
        }
    }
    let candidate = if sources.is_empty() {
        None
    } else {
        compiler_candidate(&root, config)
    };
    let blocked_compiler = candidate.is_some() && !trusted;
    let compiler = match candidate {
        Some(candidate) if trusted => Some(validated_compiler(&candidate)?),
        _ => None,
    };
    if let Some(compiler) = &compiler {
        part(&mut hash, compiler.to_string_lossy().as_bytes());
        part(&mut hash, &bounded(compiler, 16 * 1024 * 1024)?);
        let package = compiler
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("package.json");
        part(&mut hash, &bounded(&package, 1024 * 1024)?);
    }
    if !sources.is_empty() {
        for dir in directories {
            for name in [
                "tsconfig.json",
                "package.json",
                "package-lock.json",
                "pnpm-lock.yaml",
                "yarn.lock",
            ] {
                let file = dir.join(name);
                if !file.exists() {
                    continue;
                }
                let canonical = file.canonicalize().map_err(|e| e.to_string())?;
                if !canonical.starts_with(&root) {
                    return Err("TypeScript context leaves the analysis root".into());
                }
                part(&mut hash, file.to_string_lossy().as_bytes());
                part(&mut hash, &bounded(&canonical, 8 * 1024 * 1024)?);
            }
        }
        if let Some(project) = &config.semantic.typescript.project {
            let project = root
                .join(project)
                .canonicalize()
                .map_err(|e| format!("TypeScript project: {e}"))?;
            if !project.starts_with(&root) || !project.is_file() {
                return Err(
                    "TypeScript project must be an existing file inside the analysis root".into(),
                );
            }
            part(&mut hash, &bounded(&project, 8 * 1024 * 1024)?);
        }
    }
    Ok(Input {
        files: sources.len(),
        manifest: json!({"root":root,"sources":sources,"compiler":compiler,"project":config.semantic.typescript.project}),
        fingerprint: hash.finalize().into(),
        root,
        compiler,
        blocked_compiler,
    })
}
/// Node running the helper, confined to the project and `scratch`
/// (ADR 0007), without inherited Node options.
fn node(root: &Path, scratch: &Path, script: &Path, manifest: &Path) -> std::process::Command {
    let mut command = crate::compiler_sandbox::command(OsStr::new("node"), root, scratch);
    command
        .arg("--max-old-space-size=512")
        .arg(script)
        .arg(manifest)
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH")
        .current_dir(scratch);
    command
}

fn run_helper(input: &Input) -> Result<HelperResult, String> {
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let script = directory.path().join("scanner.cjs");
    let manifest = directory.path().join("input.json");
    let output = directory.path().join("output.json");
    let errors = directory.path().join("errors.txt");
    fs::write(&script, HELPER).map_err(|e| e.to_string())?;
    fs::write(
        &manifest,
        serde_json::to_vec(&input.manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut child = node(&input.root, directory.path(), &script, &manifest)
        .stdin(Stdio::null())
        .stdout(File::create(&output).map_err(|e| e.to_string())?)
        .stderr(File::create(&errors).map_err(|e| e.to_string())?)
        .spawn()
        .map_err(|e| format!("TypeScript analysis requires Node.js on PATH: {e}"))?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
        if start.elapsed() > TIMEOUT
            || [&output, &errors]
                .iter()
                .any(|path| fs::metadata(path).is_ok_and(|m| m.len() > LIMIT))
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err("TypeScript analysis exceeded its 45-second/output budget".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !status.success() {
        return Err(format!(
            "TypeScript compiler analysis failed: {}",
            String::from_utf8_lossy(&bounded(&errors, LIMIT).unwrap_or_default())
                .chars()
                .take(1500)
                .collect::<String>()
        ));
    }
    let result: HelperResult = serde_json::from_slice(&bounded(&output, LIMIT)?)
        .map_err(|e| format!("Invalid TypeScript analysis: {e}"))?;
    if result.analysis.files != input.files
        || !matches!(
            result.analysis.status.as_str(),
            "ready" | "partial" | "unavailable"
        )
        || result.inputs.len() > 20000
    {
        return Err("TypeScript compiler returned inconsistent coverage".into());
    }
    let compiler_root = input
        .compiler
        .as_ref()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    if result
        .inputs
        .iter()
        .any(|file| !file.file.starts_with(&input.root) && !file.file.starts_with(compiler_root))
    {
        return Err("TypeScript compiler inputs leave the allowed roots".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    fn failing_compiler(root: &Path, counter: &Path) {
        let lib = root.join("node_modules/typescript/lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(
            root.join("node_modules/typescript/package.json"),
            r#"{"name":"typescript","version":"5.8.2"}"#,
        )
        .unwrap();
        fs::write(
            lib.join("typescript.js"),
            format!(
                // The counter is outside the sandbox's reach where it confines the
                // compiler (ADR 0007); the error names its run either way.
                "try{{require('fs').appendFileSync({},'x')}}catch(e){{}}throw new Error('planted failure '+process.hrtime.bigint());",
                serde_json::to_string(counter.to_str().unwrap()).unwrap()
            ),
        )
        .unwrap();
        fs::write(root.join("A.ts"), "export const a=1;").unwrap();
    }
    #[test]
    fn failed_helper_runs_are_retried_instead_of_served_from_the_cache() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("runs");
        failing_compiler(dir.path(), &counter);
        let config = Config::default();
        let (first, second) = (
            analyze(dir.path(), &config, true),
            analyze(dir.path(), &config, true),
        );
        assert_eq!(
            (first.status.as_str(), second.status.as_str()),
            ("unavailable", "unavailable")
        );
        let text = |a: &CallAnalysis| a.diagnostics.join(" ");
        assert!(text(&first).contains("planted failure"), "{}", text(&first));
        assert_ne!(
            text(&first),
            text(&second),
            "the second run ran the compiler again"
        );
    }
    #[test]
    fn project_compilers_stay_unselected_without_trust() {
        let dir = tempfile::tempdir().unwrap();
        failing_compiler(dir.path(), &dir.path().join("runs"));
        let config = Config::default();
        let input = prepare(dir.path(), &config, false).unwrap();
        assert!(input.compiler.is_none());
        assert!(input.blocked_compiler);
        let trusted = prepare(dir.path(), &config, true).unwrap();
        assert!(trusted.compiler.is_some());
        assert!(!trusted.blocked_compiler);
    }
    #[test]
    fn unrelated_languages_do_not_require_a_typescript_toolchain() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("A.java"), "class A {}").unwrap();
        let mut config = Config::default();
        config.semantic.typescript.compiler = Some("missing/typescript.js".into());
        assert_eq!(
            analyze_typescript_calls(dir.path(), &config).status,
            "not_applicable"
        );
    }
    #[test]
    fn missing_compiler_is_explicit_and_never_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("A.ts"), "export function run() {}").unwrap();
        let mut config = Config::default();
        config.semantic.typescript.compiler = Some("missing/typescript.js".into());
        let result = analyze_typescript_calls(dir.path(), &config);
        assert_eq!(result.status, "unavailable");
        assert!(result.methods.is_empty());
        assert!(!result.diagnostics.is_empty());
        assert!(!dir.path().join("node_modules").exists());
    }
    #[test]
    fn source_and_project_content_changes_invalidate_the_input() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("A.ts");
        fs::write(&source, "export const a=1;").unwrap();
        let config = Config::default();
        let initial = prepare(dir.path(), &config, true).unwrap().fingerprint;
        fs::write(&source, "export const a=2;").unwrap();
        let changed = prepare(dir.path(), &config, true).unwrap().fingerprint;
        assert_ne!(initial, changed);
        fs::write(
            dir.path().join("tsconfig.json"),
            "{\"compilerOptions\":{\"strict\":true}}",
        )
        .unwrap();
        assert_ne!(
            changed,
            prepare(dir.path(), &config, true).unwrap().fingerprint
        );
    }
    #[test]
    fn tracked_context_catches_changed_and_new_dependency_files() {
        let dir = tempfile::tempdir().unwrap();
        let input = prepare(dir.path(), &Config::default(), true).unwrap();
        let file = dir.path().join("api.d.ts");
        let mut inputs = vec![TrackedInput {
            file: file.clone(),
            sha256: None,
        }];
        assert!(inputs_current(&inputs, &input));
        fs::write(&file, "export const a: number;").unwrap();
        assert!(!inputs_current(&inputs, &input));
        inputs[0].sha256 = current_hash(&file).unwrap();
        assert!(inputs_current(&inputs, &input));
        fs::write(&file, "export const b: number;").unwrap();
        assert!(!inputs_current(&inputs, &input));
    }
    #[cfg(unix)]
    #[test]
    fn cached_inputs_cannot_follow_a_retargeted_link_outside_the_project() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("api.d.ts"), "export const a: number;").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        let file = root.path().join("link/api.d.ts");
        let input = prepare(root.path(), &Config::default(), true).unwrap();
        let inputs = vec![TrackedInput {
            file: file.clone(),
            sha256: current_hash(&file).unwrap(),
        }];
        assert!(
            !inputs_current(&inputs, &input),
            "cached compiler inputs must remain in their approved roots"
        );
    }
}
