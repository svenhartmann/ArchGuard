//! Offline Go AST/type analysis. Only the bundled helper is compiled/executed.
use archguard_core::{CallAnalysis, CallAnalyzer, Config};
use archguard_extract::{Lang, strip_root};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const HELPER: &str = include_str!("ArchguardCallScanner.go");
static BUILD_CACHE: OnceLock<tempfile::TempDir> = OnceLock::new();
type CachedAnalysis = ([u8; 32], CallAnalysis);
static CACHE: OnceLock<Mutex<Option<CachedAnalysis>>> = OnceLock::new();

fn prepare(root: &Path, config: &Config) -> Result<(Vec<serde_json::Value>, [u8; 32]), String> {
    let files = crate::pipeline::walk_files(root, config).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    hash.update(HELPER);
    hash.update(root.to_string_lossy().as_bytes());
    hash.update(crate::go_sources::fingerprint(root, &files));
    hash.update(serde_json::to_vec(&config.project).map_err(|e| e.to_string())?);
    hash.update(std::env::var("ARCHGUARD_GO").unwrap_or_default());
    hash.update(std::env::var("PATH").unwrap_or_default());
    hash.update(std::env::var("GOROOT").unwrap_or_default());
    let executable = std::env::var_os("ARCHGUARD_GO").unwrap_or_else(|| {
        if cfg!(windows) {
            "go.exe".into()
        } else {
            "go".into()
        }
    });
    let candidates = if Path::new(&executable).components().count() > 1 {
        vec![std::path::PathBuf::from(&executable)]
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join(&executable))
            .collect()
    };
    for candidate in candidates {
        if let Ok(metadata) = fs::metadata(&candidate) {
            hash.update(candidate.to_string_lossy().as_bytes());
            hash.update(metadata.len().to_le_bytes());
            hash.update(format!("{:?}", metadata.modified()));
            break;
        }
    }
    let mut input = Vec::new();
    let mut bytes = 0;
    for file in files.iter().filter(|f| f.lang == Lang::Go) {
        let content = crate::service_projects::read_manifest(root, Path::new(&file.rel_path))
            .map_err(|e| format!("{}: {e}", file.rel_path))?;
        bytes += content.len();
        if bytes > 64 * 1024 * 1024 || input.len() >= 20000 {
            return Err("Go source budget exceeded".into());
        }
        input.push(json!({"path":file.rel_path, "content":content, "package_path":crate::go_sources::package_path(root, &file.rel_path)}));
    }
    hash.update(serde_json::to_vec(&input).map_err(|e| e.to_string())?);
    Ok((input, hash.finalize().into()))
}
pub(crate) fn go_calls_fingerprint(root: &Path, config: &Config) -> Result<[u8; 32], String> {
    prepare(root, config).map(|(_, fp)| fp)
}
fn unavailable(files: usize, message: String) -> CallAnalysis {
    CallAnalysis {
        status: "unavailable".into(),
        java_release: 0,
        files,
        analyzers: vec![CallAnalyzer {
            language: "go".into(),
            backend: "go/parser + go/types".into(),
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
pub(crate) fn analyze_go_calls(root: &Path, config: &Config) -> CallAnalysis {
    let (files, fp) = match prepare(root, config) {
        Ok(v) => v,
        Err(e) => return unavailable(0, e),
    };
    if files.is_empty() {
        return CallAnalysis {
            java_release: 0,
            ..Default::default()
        };
    }
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    // Serialize helper runs: several viewer endpoints can request the same analysis.
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((old, result)) = &*guard
        && *old == fp
    {
        return result.clone();
    }
    let result = run(&files)
        .and_then(|mut result| {
            if prepare(root, config)?.1 != fp {
                return Err("Go sources changed during analysis; refresh to retry".into());
            }
            for class in &mut result.classes {
                class.graph_node = strip_root(&class.file, &config.project.source_roots);
            }
            Ok(result)
        })
        .unwrap_or_else(|e| unavailable(files.len(), e));
    // A failure may be transient (timeout, concurrent edit): keep it retryable.
    if result.status != "unavailable" {
        *guard = Some((fp, result.clone()));
    }
    result
}
fn execute(
    command: &mut Command,
    dir: &Path,
    input: Option<&Path>,
    label: &str,
) -> Result<Vec<u8>, String> {
    let output = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    let error = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    command
        .current_dir(dir)
        .stdout(output.reopen().map_err(|e| e.to_string())?)
        .stderr(error.reopen().map_err(|e| e.to_string())?);
    if let Some(input) = input {
        command.stdin(fs::File::open(input).map_err(|e| e.to_string())?);
    } else {
        command.stdin(Stdio::null());
    }
    let mut child = command.spawn().map_err(|e| {
        format!("{label}: {e}. Install the Go SDK or set ARCHGUARD_GO to its executable.")
    })?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(120)
            || [output.path(), error.path()]
                .iter()
                .any(|p| fs::metadata(p).is_ok_and(|m| m.len() > 64 * 1024 * 1024))
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{label}: time/output budget exceeded"));
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    if !status.success() {
        return Err(format!(
            "{label}: {}",
            String::from_utf8_lossy(&fs::read(error.path()).map_err(|e| e.to_string())?)
                .chars()
                .take(4000)
                .collect::<String>()
        ));
    }
    let metadata = fs::metadata(output.path()).map_err(|e| e.to_string())?;
    if metadata.len() > 64 * 1024 * 1024 {
        return Err("Go output budget exceeded".into());
    }
    fs::read(output.path()).map_err(|e| e.to_string())
}
fn run(files: &[serde_json::Value]) -> Result<CallAnalysis, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let source = dir.path().join("scanner.go");
    let binary = dir.path().join(if cfg!(windows) {
        "scanner.exe"
    } else {
        "scanner"
    });
    fs::write(&source, HELPER).map_err(|e| e.to_string())?;
    if BUILD_CACHE.get().is_none() {
        let _ = BUILD_CACHE.set(tempfile::tempdir().map_err(|e| e.to_string())?);
    }
    let build_cache = BUILD_CACHE
        .get()
        .ok_or("Go cache initialization failed")?
        .path();
    let executable = std::env::var_os("ARCHGUARD_GO").unwrap_or_else(|| "go".into());
    let mut compile = Command::new(&executable);
    compile
        .args(["build", "-o"])
        .arg(&binary)
        .arg(&source)
        .env("GOTOOLCHAIN", "local")
        .env("GOPROXY", "off")
        .env("GOSUMDB", "off")
        .env("GOENV", "off")
        .env("GOWORK", "off")
        .env("GO111MODULE", "off")
        .env_remove("GOOS")
        .env_remove("GOARCH")
        .env_remove("GOTOOLDIR")
        .env("CGO_ENABLED", "0")
        .env("GOFLAGS", "")
        .env("GOCACHE", build_cache);
    execute(&mut compile, dir.path(), None, "Go helper compilation")?;
    let input = dir.path().join("input.json");
    fs::write(
        &input,
        serde_json::to_vec(&json!({"files":files})).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut scanner = Command::new(&binary);
    scanner
        .env("GOTOOLCHAIN", "local")
        .env("GOPROXY", "off")
        .env("GOSUMDB", "off")
        .env("GOENV", "off")
        .env("GOWORK", "off")
        .env("GO111MODULE", "off")
        .env_remove("GOOS")
        .env_remove("GOARCH")
        .env_remove("GOTOOLDIR")
        .env("CGO_ENABLED", "0")
        .env("GOFLAGS", "")
        .env("GOCACHE", build_cache);
    if let Some(bin) = Path::new(&executable)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        let mut paths = vec![bin.to_path_buf()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        scanner.env(
            "PATH",
            std::env::join_paths(paths).map_err(|e| e.to_string())?,
        );
    }
    let bytes = execute(&mut scanner, dir.path(), Some(&input), "Go source analysis")?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid Go analysis: {e}"))
}
