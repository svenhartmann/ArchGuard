//! Explicit Trivy dependency scans and offline, project-local result snapshots.
use crate::project_files::read_repo_file;
use archguard_core::{ArchguardError, Config};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct SecurityScanOptions {
    pub offline: bool,
    pub cache_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityTarget {
    pub path: String,
    pub ecosystem: String,
    pub packages: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityFinding {
    pub id: String,
    pub package: String,
    pub installed_version: String,
    pub fixed_version: Option<String>,
    pub ecosystem: String,
    pub severity: String,
    pub title: String,
    pub url: Option<String>,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityReport {
    pub schema_version: u32,
    pub status: String,
    pub scanner: String,
    pub scanner_version: Option<String>,
    /// Timestamp supplied by Trivy's CreatedAt field, without a guessed value.
    pub scanned_at: Option<String>,
    pub database_updated_at: Option<String>,
    pub fingerprint: String,
    /// Distinct ecosystem/package/version tuples across all reported targets.
    pub packages_scanned: usize,
    pub targets: Vec<SecurityTarget>,
    pub findings: Vec<SecurityFinding>,
    pub diagnostics: Vec<String>,
}

impl SecurityReport {
    fn empty(status: &str, fingerprint: String) -> Self {
        Self {
            schema_version: 1,
            status: status.into(),
            scanner: "trivy".into(),
            scanner_version: None,
            scanned_at: None,
            database_updated_at: None,
            fingerprint,
            packages_scanned: 0,
            targets: Vec::new(),
            findings: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

pub(crate) const SNAPSHOT: &str = ".archguard/security.json";
pub(crate) const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FILES: usize = 2048;
const MAX_WALK_ENTRIES: usize = 100_000;
const PROCESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(125);

fn error(message: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("dependency security: {message}"))
}

#[derive(Clone, Copy)]
pub(crate) struct DependencyKind {
    ecosystem: &'static str,
    resolved: bool,
}

pub(crate) fn dependency_kind(path: &Path) -> Option<DependencyKind> {
    let name = path.file_name()?.to_str()?;
    let (ecosystem, resolved) = match name {
        "Cargo.lock" => ("cargo", true),
        "Cargo.toml" => ("cargo", false),
        "package-lock.json"
        | "npm-shrinkwrap.json"
        | "yarn.lock"
        | "pnpm-lock.yaml"
        | "bun.lock"
        | "bun.lockb" => ("npm", true),
        "package.json" => ("npm", false),
        "packages.lock.json" => ("nuget", true),
        "packages.config" => ("nuget", false),
        "pom.xml" | "build.gradle" | "build.gradle.kts" => ("maven", false),
        "gradle.lockfile" => ("maven", true),
        "Pipfile.lock" | "poetry.lock" | "uv.lock" | "pdm.lock" => ("pypi", true),
        "Pipfile" | "pyproject.toml" | "setup.py" | "setup.cfg" => ("pypi", false),
        "composer.lock" => ("composer", true),
        "composer.json" => ("composer", false),
        "go.mod" => ("go", false),
        "go.sum" => ("go", true),
        "Gemfile.lock" => ("rubygems", true),
        "Gemfile" => ("rubygems", false),
        "mix.lock" => ("hex", true),
        "mix.exs" => ("hex", false),
        "pubspec.lock" => ("pub", true),
        "pubspec.yaml" => ("pub", false),
        "Package.resolved" => ("swift", true),
        "Package.swift" => ("swift", false),
        "conan.lock" => ("conan", true),
        "conanfile.txt" | "conanfile.py" => ("conan", false),
        "Manifest.toml" => ("julia", true),
        "Project.toml" => ("julia", false),
        _ if name.ends_with(".deps.json") => ("nuget", true),
        _ if name.ends_with("Packages.props")
            || name.ends_with(".csproj")
            || name.ends_with(".fsproj")
            || name.ends_with(".vbproj") =>
        {
            ("nuget", false)
        }
        _ if name.ends_with(".lockfile")
            && path
                .components()
                .any(|c| c.as_os_str() == "dependency-locks") =>
        {
            ("maven", true)
        }
        _ if (name.starts_with("requirements")
            && (name.ends_with(".txt") || name.ends_with(".in")))
            || (path.parent().is_some_and(|p| p.ends_with("requirements"))
                && name.ends_with(".txt")) =>
        {
            ("pypi", false)
        }
        _ if name.ends_with(".gemspec") => ("rubygems", false),
        _ => return None,
    };
    Some(DependencyKind {
        ecosystem,
        resolved,
    })
}

pub(crate) struct Inputs {
    pub(crate) files: std::collections::BTreeMap<String, Vec<u8>>,
    fingerprint: String,
    pub(crate) diagnostics: Vec<String>,
}

fn dependency_path(path: &Path) -> Result<String, ArchguardError> {
    let mut names = Vec::new();
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(error(
                "dependency path must be relative and contain no traversal",
            ));
        };
        let name = name
            .to_str()
            .ok_or_else(|| error("dependency path is not valid UTF-8"))?;
        // Unix permits literal backslashes. Do not reinterpret such a filename
        // as a separator when serializing portable, project-relative targets.
        if name.contains('\\') {
            return Err(error("dependency path contains an ambiguous backslash"));
        }
        names.push(name);
    }
    if names.is_empty() {
        return Err(error("dependency path must not be empty"));
    }
    Ok(names.join("/"))
}

pub(crate) fn collect_inputs(root: &Path, config: &Config) -> Result<Inputs, ArchguardError> {
    use sha2::{Digest, Sha256};
    if !root.is_dir() {
        return Err(error("a project directory is required"));
    }
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in &config.project.exclude {
        builder.add(
            globset::Glob::new(pattern).map_err(|e| error(format!("exclude {pattern:?}: {e}")))?,
        );
    }
    let excludes = builder.build().map_err(error)?;
    let walk_root = root.to_path_buf();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .git_global(false)
        .filter_entry(move |entry| {
            if entry.depth() == 0 {
                return true;
            }
            let name = entry.file_name();
            name != ".git"
                && name != ".archguard"
                && !entry
                    .path()
                    .strip_prefix(&walk_root)
                    .is_ok_and(|path| excludes.is_match(path))
        })
        .build();
    let mut files = std::collections::BTreeMap::new();
    let mut diagnostics = Vec::new();
    let mut total = 0usize;
    for (index, entry) in walker.enumerate() {
        if index >= MAX_WALK_ENTRIES {
            return Err(error("dependency discovery exceeded 100000 entries"));
        }
        let entry = entry.map_err(error)?;
        if entry.depth() > 64 {
            return Err(error("dependency discovery exceeded 64 directory levels"));
        }
        if entry.depth() == 0 {
            continue;
        }
        let relative = entry.path().strip_prefix(root).map_err(error)?;
        let relative_name = dependency_path(relative)?;
        if entry.file_type().is_some_and(|kind| kind.is_symlink()) {
            if diagnostics.len() >= 200 {
                return Err(error("dependency discovery exceeded 200 symlink omissions"));
            }
            diagnostics.push(format!(
                "Symlink omitted from dependency scope: {relative_name}"
            ));
            continue;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file())
            || dependency_kind(relative).is_none()
        {
            continue;
        }
        if files.len() >= MAX_FILES {
            return Err(error("dependency discovery exceeded 2048 files"));
        }
        let bytes = read_repo_file(root, relative, MAX_FILE_BYTES)
            .map_err(|e| error(format!("{relative_name}: {e}")))?;
        total += bytes.len();
        if total > MAX_INPUT_BYTES {
            return Err(error("dependency inputs exceeded 64 MiB"));
        }
        files.insert(relative_name, bytes);
    }
    diagnostics.sort();
    let mut hash = Sha256::new();
    hash.update(b"archguard-security-dependencies/v1\0");
    hash.update(serde_json::to_vec(&config.project.exclude).map_err(error)?);
    for (name, bytes) in &files {
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    hash.update(serde_json::to_vec(&diagnostics).map_err(error)?);
    Ok(Inputs {
        files,
        fingerprint: format!("{:x}", hash.finalize()),
        diagnostics,
    })
}

/// Hash dependency bytes and the exclusion scope; source roots do not restrict this scan.
pub fn security_fingerprint(root: &Path, config: &Config) -> Result<String, ArchguardError> {
    Ok(collect_inputs(root, config)?.fingerprint)
}

/// Read only the last explicit scan. This function never starts Trivy or network work.
pub fn read_security(root: &Path, config: &Config) -> SecurityReport {
    let inputs = match collect_inputs(root, config) {
        Ok(inputs) => inputs,
        Err(err) => {
            let mut report = SecurityReport::empty("error", String::new());
            report.diagnostics.push(err.to_string());
            return report;
        }
    };
    let bytes = match read_repo_file(root, Path::new(SNAPSHOT), MAX_OUTPUT_BYTES) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let mut report = SecurityReport::empty("not_scanned", inputs.fingerprint);
            report.diagnostics = inputs.diagnostics;
            report
                .diagnostics
                .push("No dependency security snapshot. Run archguard security explicitly.".into());
            return report;
        }
        Err(err) => {
            let mut report = SecurityReport::empty("error", inputs.fingerprint);
            report
                .diagnostics
                .push(format!("Cannot read {SNAPSHOT}: {err}"));
            return report;
        }
    };
    let mut report: SecurityReport = match serde_json::from_slice(&bytes) {
        Ok(report) => report,
        Err(err) => {
            let mut report = SecurityReport::empty("error", inputs.fingerprint);
            report
                .diagnostics
                .push(format!("Invalid security snapshot: {err}"));
            return report;
        }
    };
    if report.schema_version != 1
        || report.scanner != "trivy"
        || !matches!(report.status.as_str(), "ready" | "partial")
        || report.fingerprint.len() != 64
        || !report.fingerprint.bytes().all(|c| c.is_ascii_hexdigit())
    {
        let mut report = SecurityReport::empty("error", inputs.fingerprint);
        report
            .diagnostics
            .push("Unsupported or inconsistent security snapshot; run an explicit scan.".into());
        return report;
    }
    if report.fingerprint != inputs.fingerprint {
        report.status = "stale".into();
        report.diagnostics.push("Dependency files or exclusions changed since this scan; findings describe the previous input snapshot.".into());
    }
    report
}

/// Explicit scan; online mode permits only Trivy database updates. Dependency
/// identification stays offline and source/build commands are never executed.
pub fn scan_security(
    root: &Path,
    config: &Config,
    options: &SecurityScanOptions,
) -> Result<SecurityReport, ArchguardError> {
    scan_security_with(root, config, options, Path::new("trivy"), PROCESS_TIMEOUT)
}

pub(crate) fn scan_security_with(
    root: &Path,
    config: &Config,
    options: &SecurityScanOptions,
    executable: &Path,
    timeout: std::time::Duration,
) -> Result<SecurityReport, ArchguardError> {
    let inputs = collect_inputs(root, config)?;
    let stage = tempfile::tempdir()?;
    for (name, bytes) in &inputs.files {
        if dependency_path(Path::new(name))? != *name {
            return Err(error(
                "staging requires a normalized relative dependency path",
            ));
        }
        let target = stage.path().join(name);
        std::fs::create_dir_all(target.parent().expect("staged relative file"))?;
        std::fs::write(target, bytes)?;
    }
    let mut command = trivy_command(executable, stage.path(), options, false)?;
    let output = run_command(&mut command, timeout, MAX_OUTPUT_BYTES)?;
    let paths: Vec<_> = inputs.files.keys().map(String::as_str).collect();
    let mut report = parse_trivy(&output, &paths)?;
    report.fingerprint = inputs.fingerprint;
    if !inputs.diagnostics.is_empty() {
        report.status = "partial".into();
        report.diagnostics.extend(inputs.diagnostics);
    }
    let mut version = trivy_command(executable, stage.path(), options, true)?;
    match run_command(
        &mut version,
        std::time::Duration::from_secs(10).min(timeout),
        1024 * 1024,
    ) {
        Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(value) => {
                if let Some(version) = optional_string(&value, "Version")? {
                    report.scanner_version = Some(version);
                }
                report.database_updated_at = value
                    .get("VulnerabilityDB")
                    .and_then(|db| db.get("UpdatedAt"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
            }
            Err(_) => report
                .diagnostics
                .push("Trivy version metadata was not valid JSON.".into()),
        },
        Err(err) => report.diagnostics.push(format!(
            "Trivy version/database metadata unavailable: {err}"
        )),
    }
    if report.database_updated_at.is_none() {
        report
            .diagnostics
            .push("Vulnerability database update time is unknown.".into());
    }
    let json = serde_json::to_vec_pretty(&report).map_err(error)?;
    crate::project_files::write_project_file(root, Path::new("security.json"), &json, true)?;
    Ok(report)
}

pub(crate) fn clear_trivy_environment(command: &mut std::process::Command) {
    let inherited = std::env::vars_os().map(|(key, _)| key);
    let explicit: Vec<_> = command.get_envs().map(|(key, _)| key.to_owned()).collect();
    for key in inherited.chain(explicit) {
        if key.to_string_lossy().starts_with("TRIVY_") {
            command.env_remove(key);
        }
    }
}

pub(crate) fn trivy_command(
    executable: &Path,
    stage: &Path,
    options: &SecurityScanOptions,
    version: bool,
) -> Result<std::process::Command, ArchguardError> {
    let mut command = std::process::Command::new(executable);
    if version {
        command.args(["version", "--format", "json", "--config", "", "--quiet"]);
    } else {
        command.args([
            "fs",
            "--scanners",
            "vuln",
            "--pkg-types",
            "library",
            "--format",
            "json",
            "--list-all-pkgs",
            "--include-dev-deps",
            "--offline-scan",
            "--disable-telemetry",
            "--skip-version-check",
            "--ignorefile",
            "",
            "--config",
            "",
            "--timeout",
            "2m",
            "--quiet",
        ]);
        if options.offline {
            command.args(["--skip-db-update", "--skip-java-db-update"]);
        }
    }
    if let Some(path) = &options.cache_dir {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()?.join(path)
        };
        command.arg("--cache-dir").arg(path);
    }
    if !version {
        command.arg(".");
    }
    command
        .current_dir(stage)
        .stdin(std::process::Stdio::null());
    clear_trivy_environment(&mut command);
    Ok(command)
}

pub(crate) fn run_command(
    command: &mut std::process::Command,
    timeout: std::time::Duration,
    output_limit: u64,
) -> Result<Vec<u8>, ArchguardError> {
    use std::io::Read;
    use std::time::{Duration, Instant};
    let output = tempfile::tempdir()?;
    let stdout = output.path().join("stdout");
    let stderr = output.path().join("stderr");
    command
        .stdout(std::fs::File::create(&stdout)?)
        .stderr(std::fs::File::create(&stderr)?);
    let mut child = command.spawn().map_err(|e| {
        error(format!(
            "cannot start Trivy; install trivy and make it available on PATH: {e}"
        ))
    })?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error(err));
            }
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error("Trivy exceeded its process timeout"));
        }
        if std::fs::metadata(&stdout)?.len() > output_limit
            || std::fs::metadata(&stderr)?.len() > output_limit
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error("Trivy exceeded its output limit"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // A fast process can exit before the polling loop observes its output.
    if std::fs::metadata(&stdout)?.len() > output_limit
        || std::fs::metadata(&stderr)?.len() > output_limit
    {
        return Err(error("Trivy exceeded its output limit"));
    }
    if !status.success() {
        let mut bytes = Vec::new();
        std::fs::File::open(stderr)?
            .take(4096)
            .read_to_end(&mut bytes)?;
        return Err(error(format!(
            "Trivy failed ({status}): {}",
            String::from_utf8_lossy(&bytes)
        )));
    }
    read_repo_file(output.path(), Path::new("stdout"), output_limit).map_err(error)
}

fn optional_string(value: &serde_json::Value, key: &str) -> Result<Option<String>, ArchguardError> {
    match value.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(text)) => {
            Ok((!text.trim().is_empty()).then(|| text.clone()))
        }
        _ => Err(error(format!("Trivy field {key} must be a string or null"))),
    }
}
fn required_string(value: &serde_json::Value, key: &str) -> Result<String, ArchguardError> {
    optional_string(value, key)?.ok_or_else(|| error(format!("Trivy field {key} is required")))
}
fn array<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Result<&'a [serde_json::Value], ArchguardError> {
    match value.get(key) {
        None | Some(serde_json::Value::Null) => Ok(&[]),
        Some(serde_json::Value::Array(values)) => Ok(values),
        _ => Err(error(format!("Trivy field {key} must be an array or null"))),
    }
}
fn ecosystem(name: &str) -> String {
    match name {
        "npm" | "yarn" | "pnpm" | "bun" | "node-pkg" => "npm",
        "dotnet-core" | "nuget" | "dotnet" | "packages-props" => "nuget",
        "pip" | "pipenv" | "poetry" | "uv" | "pdm" | "python-pkg" => "pypi",
        "jar" | "pom" | "gradle" => "maven",
        "gomod" | "gobinary" => "go",
        "gemspec" | "bundler" => "rubygems",
        other => other,
    }
    .into()
}

pub(crate) fn parse_trivy(bytes: &[u8], paths: &[&str]) -> Result<SecurityReport, ArchguardError> {
    use std::collections::{BTreeMap, BTreeSet};
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| error(format!("invalid Trivy JSON: {e}")))?;
    if value.get("SchemaVersion").and_then(|v| v.as_u64()) != Some(2) {
        return Err(error("unsupported Trivy report schema"));
    }
    let mut report = SecurityReport::empty("ready", String::new());
    report.scanned_at = optional_string(&value, "CreatedAt")?;
    report.scanner_version = value
        .get("Trivy")
        .map(|trivy| optional_string(trivy, "Version"))
        .transpose()?
        .flatten();
    let mut packages = BTreeSet::new();
    let mut target_packages: BTreeMap<(String, String), BTreeSet<(String, String)>> =
        BTreeMap::new();
    let mut findings: BTreeMap<(String, String, String, String), SecurityFinding> = BTreeMap::new();
    for result in array(&value, "Results")? {
        let path = required_string(result, "Target")?;
        let path = path.strip_prefix("./").unwrap_or(&path).replace('\\', "/");
        if !paths.contains(&path.as_str()) {
            return Err(error(format!("Trivy returned an unstaged target: {path}")));
        }
        let ecosystem = ecosystem(&required_string(result, "Type")?);
        if result
            .get("Class")
            .and_then(|v| v.as_str())
            .is_some_and(|class| class != "lang-pkgs")
        {
            report
                .diagnostics
                .push(format!("Non-library target omitted: {path}"));
            continue;
        }
        let target = target_packages
            .entry((path.clone(), ecosystem.clone()))
            .or_default();
        for package in array(result, "Packages")? {
            let name = optional_string(package, "Name")?;
            let version = optional_string(package, "Version")?;
            // Trivy 0.76 emits an anonymous graph root for a Cargo workspace.
            // Its DependsOn links describe workspace members; the synthetic
            // root itself has no installed package name or version to count.
            if ecosystem == "cargo"
                && package.get("Relationship").and_then(|value| value.as_str()) == Some("root")
                && name.is_none()
                && version.is_none()
            {
                continue;
            }
            let name = name.ok_or_else(|| error("Trivy field Name is required"))?;
            let version = version.ok_or_else(|| error("Trivy field Version is required"))?;
            packages.insert((ecosystem.clone(), name.clone(), version.clone()));
            target.insert((name, version));
        }
        if target.is_empty() {
            report
                .diagnostics
                .push(format!("No package inventory reported for {path}."));
        }
        for vulnerability in array(result, "Vulnerabilities")? {
            let id = required_string(vulnerability, "VulnerabilityID")?;
            let package = required_string(vulnerability, "PkgName")?;
            let installed_version = required_string(vulnerability, "InstalledVersion")?;
            let severity = optional_string(vulnerability, "Severity")?
                .unwrap_or_default()
                .to_ascii_lowercase();
            let severity = if matches!(
                severity.as_str(),
                "critical" | "high" | "medium" | "low" | "unknown"
            ) {
                severity
            } else {
                "unknown".into()
            };
            let fixed_version = optional_string(vulnerability, "FixedVersion")?;
            let url = optional_string(vulnerability, "PrimaryURL")?.filter(|url| {
                (url.starts_with("https://") || url.starts_with("http://"))
                    && !url.chars().any(char::is_whitespace)
            });
            let title = optional_string(vulnerability, "Title")?.unwrap_or_else(|| id.clone());
            let key = (
                id.clone(),
                package.clone(),
                installed_version.clone(),
                ecosystem.clone(),
            );
            if let Some(existing) = findings.get_mut(&key) {
                if existing.severity != severity || existing.fixed_version != fixed_version {
                    report.diagnostics.push(format!("Conflicting source severity/fix for {id} in {package}; inspect each target with Trivy."));
                    existing.fixed_version = None;
                }
                if !existing.targets.contains(&path) {
                    existing.targets.push(path.clone());
                    existing.targets.sort();
                }
            } else {
                findings.insert(
                    key,
                    SecurityFinding {
                        id,
                        package,
                        installed_version,
                        fixed_version,
                        ecosystem: ecosystem.clone(),
                        severity,
                        title,
                        url,
                        targets: vec![path.clone()],
                    },
                );
            }
        }
    }
    let covered: BTreeSet<_> = target_packages
        .iter()
        .filter(|(_, packages)| !packages.is_empty())
        .map(|((path, _), _)| path.clone())
        .collect();
    for path in paths {
        let Some(kind) = dependency_kind(Path::new(path)) else {
            continue;
        };
        let resolved_here = if kind.resolved {
            covered.contains(*path)
        } else {
            covered.iter().any(|target| {
                dependency_kind(Path::new(target))
                    .is_some_and(|other| other.resolved && other.ecosystem == kind.ecosystem)
                    && Path::new(target).parent() == Path::new(path).parent()
            })
        };
        if !resolved_here {
            report.diagnostics.push(if covered.contains(*path) {
                format!("{path}: manifest/direct-dependency coverage only; a resolved lockfile is needed for transitive coverage.")
            } else {
                format!("{path}: no package inventory reported by Trivy; dependency coverage is incomplete.")
            });
            target_packages
                .entry((path.to_string(), kind.ecosystem.into()))
                .or_default();
        }
    }
    report.packages_scanned = packages.len();
    report.targets = target_packages
        .into_iter()
        .map(|((path, ecosystem), packages)| SecurityTarget {
            path,
            ecosystem,
            packages: packages.len(),
        })
        .collect();
    report.findings = findings.into_values().collect();
    if report.packages_scanned == 0 {
        report.diagnostics.push(
            "No packages were inventoried; this is not evidence of an absence of vulnerabilities."
                .into(),
        );
    }
    if !report.diagnostics.is_empty() {
        report.status = "partial".into();
    }
    Ok(report)
}
