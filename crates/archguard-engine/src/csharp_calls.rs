//! Optional Roslyn analysis with SDK-local assemblies and explicit source input.
//! Only our helper is compiled. No project build, restore, analyzer or generator
//! is executed. Unsupported project context is represented as partial coverage.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use archguard_core::{CallAnalysis, CallAnalyzer, Config};
use archguard_extract::{CsExtractor, Extractor, Lang, strip_root};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const HELPER: &str = include_str!("ArchguardCallScanner.cs");
const LIMIT: u64 = 64 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(45);
/// Larger package assemblies are identified by their size alone.
const REFERENCE_LIMIT: u64 = 256 * 1024 * 1024;
type CacheEntry = Option<([u8; 32], CallAnalysis)>;
static CACHE: OnceLock<Mutex<CacheEntry>> = OnceLock::new();

struct Input {
    manifest: Value,
    fingerprint: [u8; 32],
    files: usize,
}
#[derive(Debug)]
struct Sdk {
    executable: PathBuf,
    path: PathBuf,
    home: PathBuf,
    major: u32,
    references: PathBuf,
    version: String,
}

pub fn analyze_csharp_calls(root: &Path, config: &Config) -> CallAnalysis {
    let mut input = match prepare(root, config) {
        Ok(input) => input,
        Err(error) => return unavailable(0, error),
    };
    if input.files == 0 {
        return CallAnalysis {
            java_release: 0,
            ..CallAnalysis::default()
        };
    }
    let sdk = match sdk() {
        Ok(sdk) => sdk,
        Err(error) => return unavailable(input.files, error),
    };
    input.manifest["dotnet_root"] = json!(sdk.home);
    input.manifest["sdk_version"] = json!(sdk.version);
    let fingerprint = with_sdk(input.fingerprint, &sdk);
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock()
        && let Some((previous, analysis)) = &*guard
        && *previous == fingerprint
    {
        return analysis.clone();
    }
    let analysis = run_helper(&input, &sdk).unwrap_or_else(|error| unavailable(input.files, error));
    let analysis = match prepare(root, config) {
        Ok(after) if after.fingerprint == input.fingerprint => analysis,
        _ => unavailable(
            input.files,
            "C# source or project context changed during analysis; refresh to retry.".into(),
        ),
    };
    // A failure may be transient (timeout, concurrent edit): keep it retryable.
    if analysis.status != "unavailable"
        && let Ok(mut guard) = cache.lock()
    {
        *guard = Some((fingerprint, analysis.clone()));
    }
    analysis
}

/// Live-view inputs include method bodies and project metadata even when the
/// source import graph remains unchanged. Missing toolchains remain observable.
pub fn csharp_calls_fingerprint(root: &Path, config: &Config) -> Result<[u8; 32], String> {
    let input = prepare(root, config)?;
    if input.files == 0 {
        return Ok(input.fingerprint);
    }
    match sdk() {
        Ok(sdk) => Ok(with_sdk(input.fingerprint, &sdk)),
        Err(error) => {
            let mut hash = Sha256::new();
            part(&mut hash, &input.fingerprint);
            part(&mut hash, error.as_bytes());
            Ok(hash.finalize().into())
        }
    }
}

fn unavailable(files: usize, message: String) -> CallAnalysis {
    CallAnalysis {
        status: "unavailable".into(),
        java_release: 0,
        files,
        diagnostics: vec![message.clone()],
        analyzers: vec![CallAnalyzer {
            language: "csharp".into(),
            backend: "roslyn".into(),
            status: "unavailable".into(),
            files,
            version: None,
            project: None,
            diagnostics: vec![message],
        }],
        ..CallAnalysis::default()
    }
}
fn part(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}
fn prepare(root: &Path, config: &Config) -> Result<Input, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !root.is_dir() {
        return Err("C# project path is not a directory".into());
    }
    let mut hash = Sha256::new();
    part(&mut hash, HELPER.as_bytes());
    part(&mut hash, root.to_string_lossy().as_bytes());
    part(
        &mut hash,
        &serde_json::to_vec(&config.semantic.csharp).map_err(|e| e.to_string())?,
    );
    let enabled = config.project.languages.is_empty()
        || config
            .project
            .languages
            .iter()
            .any(|s| s == "auto" || s == "cs" || s == "csharp");
    let mut sources = Vec::new();
    if enabled {
        for file in crate::pipeline::walk_files(&root, config).map_err(|e| e.to_string())? {
            if file.lang != Lang::CSharp {
                continue;
            }
            let path = file.path.canonicalize().map_err(|e| e.to_string())?;
            if !path.starts_with(&root) {
                return Err("C# source resolves outside the analysis root".into());
            }
            let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let stripped = strip_root(&file.rel_path, &config.project.source_roots);
            let extraction = CsExtractor.extract(&stripped, &content);
            let node = crate::pipeline::file_node_name(&stripped, &extraction.declarations);
            part(&mut hash, path.to_string_lossy().as_bytes());
            part(&mut hash, stripped.as_bytes());
            part(&mut hash, node.as_bytes());
            part(&mut hash, content.as_bytes());
            sources.push(json!({"path":path,"file":stripped,"graph_node":node}));
        }
    }
    if !sources.is_empty() {
        for path in context_files(&root)? {
            part(
                &mut hash,
                path.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .as_bytes(),
            );
            let bytes =
                fs::read(&path).map_err(|e| format!("C# context {}: {e}", path.display()))?;
            part(&mut hash, &bytes);
            if path.file_name().is_some_and(|n| n == "project.assets.json") {
                for reference in package_reference_paths(&bytes) {
                    part(&mut hash, reference.to_string_lossy().as_bytes());
                    hash_package_reference(&mut hash, &reference)?;
                }
            }
        }
    }
    Ok(Input {
        files: sources.len(),
        manifest: json!({"root":root, "sources":sources,
            "project":config.semantic.csharp.project,
            "configuration":config.semantic.csharp.configuration,
            "target_framework":config.semantic.csharp.target_framework}),
        fingerprint: hash.finalize().into(),
    })
}

/// The path comes from a repository file: only bounded regular files are read,
/// so a device, FIFO or sparse file cannot stall the analysis.
fn hash_package_reference(hash: &mut Sha256, reference: &Path) -> Result<(), String> {
    let metadata = match fs::metadata(reference) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            part(hash, b"missing");
            return Ok(());
        }
        Err(error) => {
            part(hash, format!("unreadable:{error}").as_bytes());
            return Ok(());
        }
    };
    if !metadata.is_file() {
        part(hash, b"not-a-file");
        return Ok(());
    }
    hash.update(metadata.len().to_le_bytes());
    if metadata.len() > REFERENCE_LIMIT {
        part(hash, b"oversized");
        return Ok(());
    }
    match File::open(reference) {
        Ok(mut file) => {
            part(hash, b"present");
            let mut buffer = [0; 64 * 1024];
            loop {
                let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
                if read == 0 {
                    break;
                }
                hash.update(&buffer[..read]);
            }
        }
        Err(error) => part(hash, format!("unreadable:{error}").as_bytes()),
    }
    Ok(())
}

fn package_reference_paths(bytes: &[u8]) -> Vec<PathBuf> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    let Some(folders) = value["packageFolders"].as_object() else {
        return Vec::new();
    };
    let Some(targets) = value["targets"].as_object() else {
        return Vec::new();
    };
    let mut paths = std::collections::BTreeSet::new();
    for target in targets.values().filter_map(Value::as_object) {
        for (name, entry) in target {
            let library = &value["libraries"][name];
            if library["type"] != "package" {
                continue;
            }
            let Some(references) = entry["compile"].as_object() else {
                continue;
            };
            let fallback = name.to_lowercase();
            let directory = library["path"].as_str().unwrap_or(&fallback);
            for reference in references.keys().filter(|p| !p.ends_with("/_._")) {
                for folder in folders.keys() {
                    paths.insert(Path::new(folder).join(directory).join(reference));
                }
            }
        }
    }
    paths.into_iter().collect()
}

fn context_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            let name = entry.file_name();
            if kind.is_dir() {
                if matches!(
                    name.to_str(),
                    Some(".git" | ".archguard" | "bin" | "node_modules" | "target")
                ) {
                    continue;
                }
                pending.push(path);
            } else if kind.is_file() {
                let extension = path.extension().and_then(|e| e.to_str());
                if matches!(
                    extension,
                    Some("csproj" | "props" | "targets" | "sln" | "dll")
                ) || matches!(
                    name.to_str(),
                    Some("project.assets.json" | "global.json" | "packages.lock.json")
                ) {
                    files.push(path);
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

fn version(name: &str) -> Option<(u32, u32, u32)> {
    let mut pieces = name.split('.').map(str::parse::<u32>);
    let value = (
        pieces.next()?.ok()?,
        pieces.next()?.ok()?,
        pieces.next()?.ok()?,
    );
    if pieces.next().is_some() {
        return None;
    }
    Some(value)
}
fn sdk() -> Result<Sdk, String> {
    let executable = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| {
            p.join(if cfg!(windows) {
                "dotnet.exe"
            } else {
                "dotnet"
            })
        })
        .find(|p| p.is_file())
        .ok_or("C# semantic analysis requires .NET SDK 8+ available as dotnet on PATH")?;
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut command = Command::new(&executable);
    command.arg("--list-sdks");
    let output = bounded(&mut command, temporary.path(), "sdks")?;
    let mut choices = Vec::new();
    for line in String::from_utf8_lossy(&output).lines() {
        let Some((name, parent)) = line.split_once(" [") else {
            continue;
        };
        let Some(v) = version(name) else { continue };
        if v.0 < 8 {
            continue;
        }
        let path = Path::new(parent.trim_end_matches(']')).join(name);
        if path.join("Roslyn/bincore/csc.dll").is_file() {
            choices.push((v, path, name.to_owned()));
        }
    }
    let (v, path, name) = choices.into_iter().max_by_key(|(v, _, _)| *v).ok_or(
        "C# semantic analysis requires an installed .NET SDK 8+ with Roslyn compiler assemblies",
    )?;
    let home = path
        .parent()
        .and_then(Path::parent)
        .ok_or("Invalid .NET SDK directory")?
        .to_path_buf();
    let packs = home.join("packs/Microsoft.NETCore.App.Ref");
    let references = fs::read_dir(packs)
        .map_err(|e| format!(".NET SDK reference packs unavailable: {e}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let ver = version(name.to_str()?)?;
            let reference = entry.path().join(format!("ref/net{}.0", v.0));
            (ver.0 == v.0 && reference.is_dir()).then_some((ver, reference))
        })
        .max_by_key(|(v, _)| *v)
        .map(|(_, p)| p)
        .ok_or("Matching .NET SDK reference pack unavailable")?;
    Ok(Sdk {
        executable,
        path,
        home,
        major: v.0,
        references,
        version: name,
    })
}
fn with_sdk(input: [u8; 32], sdk: &Sdk) -> [u8; 32] {
    let mut hash = Sha256::new();
    part(&mut hash, &input);
    for path in [&sdk.path, &sdk.references, &sdk.home.join("packs")] {
        part(&mut hash, path.to_string_lossy().as_bytes());
        if let Ok(meta) = fs::metadata(path) {
            part(
                &mut hash,
                format!("{:?}:{}", meta.modified(), meta.len()).as_bytes(),
            );
        }
    }
    part(&mut hash, sdk.version.as_bytes());
    hash.finalize().into()
}

fn bounded(command: &mut Command, directory: &Path, phase: &str) -> Result<Vec<u8>, String> {
    let stdout = directory.join(format!("{phase}.out"));
    let stderr = directory.join(format!("{phase}.err"));
    command
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(File::create(&stdout).map_err(|e| e.to_string())?)
        .stderr(File::create(&stderr).map_err(|e| e.to_string())?)
        .env("DOTNET_CLI_HOME", directory)
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
    for name in [
        "DOTNET_STARTUP_HOOKS",
        "DOTNET_ADDITIONAL_DEPS",
        "DOTNET_SHARED_STORE",
    ] {
        command.env_remove(name);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("C# {phase} could not start: {e}"))?;
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
            || [&stdout, &stderr]
                .iter()
                .any(|path| fs::metadata(path).is_ok_and(|m| m.len() > LIMIT))
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "C# {phase} exceeded its 45 second or 64 MiB output limit; import analysis remains available"
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut bytes = Vec::new();
    File::open(&stdout)
        .map_err(|e| e.to_string())?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("C# helper output exceeds 64 MiB".into());
    }
    if !status.success() {
        let mut errors = String::new();
        File::open(&stderr)
            .map_err(|e| e.to_string())?
            .take(3000)
            .read_to_string(&mut errors)
            .map_err(|e| e.to_string())?;
        return Err(format!(
            "C# {phase} failed: {} {errors}",
            String::from_utf8_lossy(&bytes)
                .chars()
                .take(1500)
                .collect::<String>()
        ));
    }
    Ok(bytes)
}
fn run_helper(input: &Input, sdk: &Sdk) -> Result<CallAnalysis, String> {
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let helper = directory.path().join("ArchguardCallScanner.cs");
    let binary = directory.path().join("scanner.dll");
    let manifest = directory.path().join("sources.json");
    fs::write(&helper, HELPER).map_err(|e| e.to_string())?;
    fs::write(
        &manifest,
        serde_json::to_vec(&input.manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let roslyn = sdk.path.join("Roslyn/bincore");
    let mut compile = Command::new(&sdk.executable);
    compile
        .arg(roslyn.join("csc.dll"))
        .args([
            "/nologo",
            "/noconfig",
            "/nostdlib+",
            "/target:exe",
            "/nullable:enable",
        ])
        .arg(format!("/out:{}", binary.display()));
    let mut references = fs::read_dir(&sdk.references)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "dll"))
        .collect::<Vec<_>>();
    for name in [
        "Microsoft.CodeAnalysis.dll",
        "Microsoft.CodeAnalysis.CSharp.dll",
    ] {
        let assembly = roslyn.join(name);
        references.push(assembly.clone());
        fs::copy(&assembly, directory.path().join(name)).map_err(|e| e.to_string())?;
    }
    references.sort();
    for reference in references {
        compile.arg(format!("/reference:{}", reference.display()));
    }
    compile.arg(&helper);
    bounded(&mut compile, directory.path(), "compile-helper")?;
    fs::write(binary.with_extension("runtimeconfig.json"), serde_json::to_vec(&json!({
        "runtimeOptions":{"tfm":format!("net{}.0",sdk.major),"framework":{"name":"Microsoft.NETCore.App","version":format!("{}.0.0",sdk.major)}}})).unwrap()).map_err(|e| e.to_string())?;
    let mut run = Command::new(&sdk.executable);
    run.arg(&binary).arg(&manifest);
    let bytes = bounded(&mut run, directory.path(), "analyze")?;
    let result = decode_helper(&bytes)?;
    if result.files > input.files || !matches!(result.status.as_str(), "ready" | "partial") {
        return Err("Inconsistent C# source coverage".into());
    }
    Ok(result)
}

fn decode_helper(bytes: &[u8]) -> Result<CallAnalysis, String> {
    let mut value: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid C# helper result: {e}"))?;
    if value.get("encoding").is_some() {
        if value["encoding"] != "string-references-v1" {
            return Err("Unsupported C# transport encoding".into());
        }
        let mut analysis = value["analysis"].take();
        let strings = value["strings"]
            .as_array()
            .ok_or("Missing C# transport string table")?;
        let mut expanded = 0usize;
        fn expand(
            value: &mut Value,
            strings: &[Value],
            expanded: &mut usize,
        ) -> Result<(), String> {
            match value {
                Value::Object(object) if object.len() == 1 && object.contains_key("$str") => {
                    let index = object["$str"]
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or("Invalid C# string index")?;
                    let text = strings
                        .get(index)
                        .and_then(Value::as_str)
                        .ok_or("Invalid C# string reference")?;
                    *expanded = expanded.saturating_add(text.len());
                    if *expanded > 256 * 1024 * 1024 {
                        return Err("Expanded C# transport exceeds 256 MiB".into());
                    }
                    *value = Value::String(text.into());
                }
                Value::Object(object) => {
                    for child in object.values_mut() {
                        expand(child, strings, expanded)?;
                    }
                }
                Value::Array(items) => {
                    for child in items {
                        expand(child, strings, expanded)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        expand(&mut analysis, strings, &mut expanded)?;
        value = analysis;
    }
    serde_json::from_value(value).map_err(|e| format!("Invalid C# helper analysis: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_helper_transport_preserves_source_strings_and_rejects_invalid_indices() {
        let report = json!({"status":"ready","files":1,"classes":[],"methods":[],"calls":[],"diagnostics":[{"$str":0}]});
        let bytes=serde_json::to_vec(&json!({"encoding":"string-references-v1","strings":["source </script> with Unicode ä"],"analysis":report})).unwrap();
        assert_eq!(
            decode_helper(&bytes).unwrap().diagnostics,
            vec!["source </script> with Unicode ä"]
        );
        let invalid = serde_json::to_vec(
            &json!({"encoding":"string-references-v1","strings":[],"analysis":report}),
        )
        .unwrap();
        assert!(decode_helper(&invalid).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn package_references_never_block_on_special_or_huge_files() {
        let packages = tempfile::tempdir().unwrap();
        let fifo = packages.path().join("fifo");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let huge = File::create(packages.path().join("huge")).unwrap();
        huge.set_len(1 << 40).unwrap();
        for (folder, reference) in [
            ("/dev".to_owned(), "zero"),
            (packages.path().display().to_string(), "fifo"),
            (packages.path().display().to_string(), "huge"),
        ] {
            let root = tempfile::tempdir().unwrap();
            fs::write(root.path().join("A.cs"), "class A {}").unwrap();
            fs::create_dir(root.path().join("obj")).unwrap();
            fs::write(
                root.path().join("obj/project.assets.json"),
                json!({
                    "packageFolders": {folder: {}},
                    "targets": {"t": {"x/1": {"compile": {reference: {}}}}},
                    "libraries": {"x/1": {"type": "package", "path": "."}},
                })
                .to_string(),
            )
            .unwrap();
            let (send, receive) = std::sync::mpsc::channel();
            let path = root.path().to_path_buf();
            std::thread::spawn(move || {
                let _ = send.send(prepare(&path, &Config::default()).map(|input| input.files));
            });
            let files = receive
                .recv_timeout(Duration::from_secs(20))
                .unwrap_or_else(|_| panic!("{reference} blocked the fingerprint"));
            assert_eq!(files, Ok(1));
        }
    }

    #[test]
    fn csharp_input_uses_import_node_mapping_and_hashes_project_context() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("src/tests")).unwrap();
        fs::write(
            root.path().join("src/A.cs"),
            "namespace App;\nclass A { void Run() {} }",
        )
        .unwrap();
        fs::write(root.path().join("src/tests/A.cs"), "class Ignored {}").unwrap();
        fs::write(
            root.path().join("App.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk\" />",
        )
        .unwrap();
        let config = Config::default();
        let before = prepare(root.path(), &config).unwrap();
        assert_eq!(before.files, 1);
        assert_eq!(before.manifest["sources"][0]["graph_node"], "App/A.cs");
        fs::write(root.path().join("App.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><DefineConstants>CHANGED</DefineConstants></PropertyGroup></Project>").unwrap();
        let after = prepare(root.path(), &config).unwrap();
        assert_ne!(before.fingerprint, after.fingerprint);
        fs::write(
            root.path().join("src/A.cs"),
            "namespace App;\nclass A { void Run() { Run(); } }",
        )
        .unwrap();
        assert_ne!(
            after.fingerprint,
            prepare(root.path(), &config).unwrap().fingerprint
        );
        let mut config = config;
        config.project.languages = vec!["java".into()];
        assert_eq!(prepare(root.path(), &config).unwrap().files, 0);
    }
    #[test]
    fn no_csharp_sources_needs_no_toolchain() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            analyze_csharp_calls(root.path(), &Config::default()).status,
            "not_applicable"
        );
        let failed = analyze_csharp_calls(&root.path().join("absent"), &Config::default());
        assert_eq!(failed.status, "unavailable");
        assert_eq!(failed.java_release, 0);
        assert_eq!(failed.analyzers[0].language, "csharp");
    }
    #[test]
    fn selected_csharp_configuration_changes_fingerprint() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("A.cs"), "class A {}").unwrap();
        let mut config = Config::default();
        let a = prepare(root.path(), &config).unwrap();
        config.semantic.csharp.configuration = "Release".into();
        assert_ne!(
            a.fingerprint,
            prepare(root.path(), &config).unwrap().fingerprint
        );
    }
    #[test]
    fn package_reference_content_and_missing_files_invalidate_cache() {
        let root = tempfile::tempdir().unwrap();
        let packages = tempfile::tempdir().unwrap();
        fs::write(root.path().join("A.cs"), "class A {}").unwrap();
        fs::create_dir(root.path().join("obj")).unwrap();
        fs::write(
            root.path().join("obj/project.assets.json"),
            serde_json::to_vec(&json!({
                "targets":{"net8.0":{"Example/1.0":{"compile":{"lib/net8.0/Example.dll":{}}}}},
                "libraries":{"Example/1.0":{"type":"package","path":"example/1.0"}},
                "packageFolders":{packages.path().to_str().unwrap():{}}
            }))
            .unwrap(),
        )
        .unwrap();
        let config = Config::default();
        let missing = prepare(root.path(), &config).unwrap().fingerprint;
        let dll = packages.path().join("example/1.0/lib/net8.0/Example.dll");
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"version1").unwrap();
        let present = prepare(root.path(), &config).unwrap().fingerprint;
        assert_ne!(
            missing, present,
            "a restored reference must invalidate missing-reference analysis"
        );
        fs::write(&dll, b"version2").unwrap();
        assert_ne!(
            present,
            prepare(root.path(), &config).unwrap().fingerprint,
            "changed assembly bytes must invalidate semantic evidence"
        );
    }
}
