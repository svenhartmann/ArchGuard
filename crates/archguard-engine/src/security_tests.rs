use crate::security::*;
use archguard_core::Config;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[test]
fn real_trivy_synthetic_cargo_workspace_root_is_not_an_installed_package() {
    let mut data = inventory("Cargo.lock", "cargo");
    data["Results"][0]["Packages"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "ID":"52a8d28e6c5562ea", "Identifier":{"UID":"f7405d60028163ba"},
            "Relationship":"root", "DependsOn":["demo@1.0"], "AnalyzedBy":"cargo"
        }));
    let report = parse_trivy(&serde_json::to_vec(&data).unwrap(), &["Cargo.lock"]).unwrap();
    assert_eq!(report.packages_scanned, 1);
    assert_eq!(report.targets[0].packages, 1);
    assert_eq!(report.status, "ready");
    data["Results"][0]["Packages"][1]["Relationship"] = json!("direct");
    assert!(parse_trivy(&serde_json::to_vec(&data).unwrap(), &["Cargo.lock"]).is_err());
}

#[test]
fn real_trivy_packages_props_is_one_nuget_target_with_direct_coverage() {
    let report = parse_trivy(
        &serde_json::to_vec(&inventory("Directory.Packages.props", "packages-props")).unwrap(),
        &["Directory.Packages.props"],
    )
    .unwrap();
    assert_eq!(report.targets.len(), 1);
    assert_eq!(report.targets[0].ecosystem, "nuget");
    assert_eq!(report.targets[0].packages, 1);
    assert_eq!(report.status, "partial");
}

#[cfg(unix)]
#[test]
fn unix_backslash_names_cannot_escape_staging_when_normalized() {
    let root = tempfile::tempdir().unwrap();
    put(
        root.path(),
        "..\\escaped/Cargo.lock",
        b"untrusted dependency",
    );
    assert!(
        collect_inputs(root.path(), &Config::default()).is_err(),
        "a literal Unix backslash must never turn into a traversal separator"
    );
}

#[test]
fn one_resolved_lockfile_cannot_hide_an_unscanned_sibling_lockfile() {
    let report = parse_trivy(
        &serde_json::to_vec(&inventory("package-lock.json", "npm")).unwrap(),
        &["package-lock.json", "yarn.lock", "package.json"],
    )
    .unwrap();
    assert_eq!(report.status, "partial");
    assert!(
        report
            .targets
            .iter()
            .any(|target| target.path == "yarn.lock" && target.packages == 0)
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|message| message.contains("yarn.lock"))
    );
}

fn put(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn inventory(target: &str, kind: &str) -> serde_json::Value {
    json!({"SchemaVersion":2,"Trivy":{"Version":"0.76.0"},"CreatedAt":"2026-10-27T12:00:00Z","Results":[{"Target":target,"Type":kind,"Class":"lang-pkgs","Packages":[{"Name":"demo","Version":"1.0"}],"Vulnerabilities":null}]})
}

#[test]
fn manifests_without_resolved_locks_have_explicit_partial_coverage() {
    for (path, kind) in [
        ("pom.xml", "pom"),
        ("requirements.txt", "pip"),
        ("Directory.Packages.props", "nuget"),
        ("go.mod", "gomod"),
    ] {
        let report = parse_trivy(
            &serde_json::to_vec(&inventory(path, kind)).unwrap(),
            &[path],
        )
        .unwrap();
        assert_eq!(report.status, "partial", "{path}");
        assert_eq!(report.packages_scanned, 1);
    }
    let report = parse_trivy(
        &serde_json::to_vec(&inventory("Cargo.lock", "cargo")).unwrap(),
        &["Cargo.lock", "Cargo.toml"],
    )
    .unwrap();
    assert_eq!(report.status, "ready");
    assert_eq!(report.scanner_version.as_deref(), Some("0.76.0"));
}

#[test]
fn fingerprint_covers_dependency_bytes_additions_deletions_and_excludes_not_source_roots() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), ".gitignore", b"ignored/\n");
    put(dir.path(), "Cargo.lock", b"first");
    put(dir.path(), "ignored/Cargo.lock", b"ignored");
    put(dir.path(), "excluded/pom.xml", b"excluded");
    put(dir.path(), "src/main.rs", b"source");
    let mut config = Config::default();
    config.project.exclude = vec!["excluded/**".into()];
    config.project.source_roots = vec!["src".into()];
    let original = security_fingerprint(dir.path(), &config).unwrap();
    put(dir.path(), "ignored/Cargo.lock", b"changed ignored");
    put(dir.path(), "excluded/pom.xml", b"changed excluded");
    put(dir.path(), "src/main.rs", b"changed source");
    assert_eq!(original, security_fingerprint(dir.path(), &config).unwrap());
    put(dir.path(), "nested/package-lock.json", b"another lock");
    assert_ne!(original, security_fingerprint(dir.path(), &config).unwrap());
    fs::remove_file(dir.path().join("nested/package-lock.json")).unwrap();
    assert_eq!(original, security_fingerprint(dir.path(), &config).unwrap());
    put(dir.path(), "Cargo.lock", b"other");
    assert_ne!(original, security_fingerprint(dir.path(), &config).unwrap());
    config.project.exclude.clear();
    assert!(
        collect_inputs(dir.path(), &config)
            .unwrap()
            .files
            .contains_key("excluded/pom.xml")
    );
}

#[test]
fn snapshot_reader_never_scans_and_marks_changed_dependencies_stale() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::default();
    put(dir.path(), "Cargo.lock", b"dependencies");
    assert_eq!(read_security(dir.path(), &config).status, "not_scanned");
    assert!(!dir.path().join(".archguard").exists());
    let mut report = parse_trivy(
        &serde_json::to_vec(&inventory("Cargo.lock", "cargo")).unwrap(),
        &["Cargo.lock"],
    )
    .unwrap();
    report.fingerprint = security_fingerprint(dir.path(), &config).unwrap();
    put(dir.path(), SNAPSHOT, &serde_json::to_vec(&report).unwrap());
    assert_eq!(read_security(dir.path(), &config), report);
    put(dir.path(), "Cargo.lock", b"edited dependencies");
    let stale = read_security(dir.path(), &config);
    assert_eq!(stale.status, "stale");
    assert_eq!(stale.fingerprint, report.fingerprint);
    assert_eq!(stale.targets, report.targets);
    put(dir.path(), SNAPSHOT, b"null");
    assert_eq!(read_security(dir.path(), &config).status, "error");
}

#[test]
fn dependency_file_recognition_includes_supported_ecosystems_and_unsupported_manifests() {
    for name in [
        "Cargo.lock",
        "package-lock.json",
        "npm-shrinkwrap.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "bun.lock",
        "bun.lockb",
        "packages.lock.json",
        "packages.config",
        "Directory.Packages.props",
        "bin/App.deps.json",
        "pom.xml",
        "gradle.lockfile",
        "gradle/dependency-locks/runtime.lockfile",
        "requirements-dev.txt",
        "requirements/test.txt",
        "Pipfile.lock",
        "poetry.lock",
        "uv.lock",
        "pdm.lock",
        "composer.lock",
        "go.mod",
        "go.sum",
        "Gemfile.lock",
        "mix.lock",
        "pubspec.lock",
        "Package.resolved",
        "App.csproj",
        "package.json",
    ] {
        assert!(dependency_kind(Path::new(name)).is_some(), "{name}");
    }
    for name in [
        "trivy.yaml",
        ".trivyignore",
        ".npmrc",
        "src/main.rs",
        "application.dll",
        "application.jar",
        ".env",
    ] {
        assert!(dependency_kind(Path::new(name)).is_none(), "{name}");
    }
}

#[test]
fn runner_flags_are_fixed_and_trivy_environment_cannot_override_scope_or_filtering() {
    let options = SecurityScanOptions {
        offline: true,
        cache_dir: Some(PathBuf::from("relative-cache")),
    };
    let mut command = trivy_command(
        Path::new("trivy"),
        Path::new("/tmp/staged-security"),
        &options,
        false,
    )
    .unwrap();
    let args: Vec<_> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    for pair in [
        ["--scanners", "vuln"],
        ["--pkg-types", "library"],
        ["--format", "json"],
        ["--ignorefile", ""],
        ["--config", ""],
        ["--timeout", "2m"],
    ] {
        assert!(args.windows(2).any(|window| window == pair));
    }
    for flag in [
        "--offline-scan",
        "--list-all-pkgs",
        "--include-dev-deps",
        "--disable-telemetry",
        "--skip-version-check",
        "--skip-db-update",
        "--skip-java-db-update",
    ] {
        assert!(args.iter().any(|a| a == flag));
    }
    assert_eq!(args.last().unwrap(), ".");
    assert_eq!(
        command.get_current_dir(),
        Some(Path::new("/tmp/staged-security"))
    );
    assert!(args.iter().any(|a| {
        a == &std::env::current_dir()
            .unwrap()
            .join("relative-cache")
            .display()
            .to_string()
    }));
    for name in [
        "TRIVY_SERVER",
        "TRIVY_SEVERITY",
        "TRIVY_IGNORE_UNFIXED",
        "TRIVY_CONFIG",
        "TRIVY_CACHE_DIR",
    ] {
        command.env(name, "untrusted");
    }
    clear_trivy_environment(&mut command);
    assert!(
        command
            .get_envs()
            .filter(|(key, _)| key.to_string_lossy().starts_with("TRIVY_"))
            .all(|(_, value)| value.is_none())
    );
    let online = trivy_command(
        Path::new("trivy"),
        Path::new("/tmp"),
        &SecurityScanOptions::default(),
        false,
    )
    .unwrap();
    assert!(!online.get_args().any(|a| a == "--skip-db-update"));
}

#[cfg(unix)]
fn executable(root: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("fake-trivy");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn explicit_scan_stages_only_allowed_files_and_preserves_snapshot_on_failure() {
    let root = tempfile::tempdir().unwrap();
    let runner = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    put(root.path(), "Cargo.lock", b"lock");
    put(root.path(), "Cargo.toml", b"manifest");
    put(root.path(), "src/main.rs", b"source");
    put(root.path(), ".gitignore", b"ignored/\n");
    put(root.path(), "ignored/Cargo.lock", b"ignored");
    put(root.path(), "trivy.yaml", b"severity: [LOW]");
    put(root.path(), ".trivyignore", b"CVE-2024-0001");
    put(outside.path(), "Cargo.lock", b"external");
    std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
    let json = inventory("Cargo.lock", "cargo").to_string();
    let script = executable(
        runner.path(),
        &format!(
            r#"
if [ "$1" = version ]; then
  printf '%s' '{{"Version":"0.76.0","VulnerabilityDB":{{"UpdatedAt":"2026-10-27T00:00:00Z"}}}}'
  exit 0
fi
[ -f Cargo.lock ] && [ -f Cargo.toml ] || exit 31
[ ! -e src/main.rs ] && [ ! -e trivy.yaml ] && [ ! -e .trivyignore ] && [ ! -e .gitignore ] && [ ! -e ignored ] && [ ! -e linked ] || exit 32
printf '%s' '{json}'
"#
        ),
    );
    let report = scan_security_with(
        root.path(),
        &Config::default(),
        &SecurityScanOptions::default(),
        &script,
        Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(report.status, "partial", "omitted symlink stays explicit");
    assert_eq!(report.packages_scanned, 1);
    assert_eq!(report.scanner_version.as_deref(), Some("0.76.0"));
    assert_eq!(
        report.database_updated_at.as_deref(),
        Some("2026-10-27T00:00:00Z")
    );
    assert_eq!(read_security(root.path(), &Config::default()), report);
    let previous = fs::read(root.path().join(SNAPSHOT)).unwrap();
    for body in [
        "printf 'not json'",
        "printf 'scanner unavailable' >&2; exit 7",
    ] {
        executable(runner.path(), body);
        assert!(
            scan_security_with(
                root.path(),
                &Config::default(),
                &SecurityScanOptions::default(),
                &script,
                Duration::from_secs(2)
            )
            .is_err()
        );
        assert_eq!(fs::read(root.path().join(SNAPSHOT)).unwrap(), previous);
    }
    fs::remove_file(&script).unwrap();
    assert!(
        scan_security_with(
            root.path(),
            &Config::default(),
            &SecurityScanOptions::default(),
            &script,
            Duration::from_secs(2)
        )
        .unwrap_err()
        .to_string()
        .contains("install trivy")
    );
    assert_eq!(fs::read(root.path().join(SNAPSHOT)).unwrap(), previous);
}

#[cfg(unix)]
#[test]
fn scanner_timeout_is_enforced_and_no_snapshot_is_published() {
    let root = tempfile::tempdir().unwrap();
    let runner = tempfile::tempdir().unwrap();
    put(root.path(), "Cargo.lock", b"lock");
    let path = executable(runner.path(), "exec /bin/sleep 5");
    let start = Instant::now();
    let result = scan_security_with(
        root.path(),
        &Config::default(),
        &SecurityScanOptions::default(),
        &path,
        Duration::from_millis(25),
    );
    assert!(result.unwrap_err().to_string().contains("timeout"));
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(!root.path().join(SNAPSHOT).exists());
}

#[cfg(unix)]
#[test]
fn finished_scanner_output_is_bounded_on_stdout_and_stderr() {
    let runner = tempfile::tempdir().unwrap();
    for body in [
        "printf '12345678901234567890'",
        "printf '12345678901234567890' >&2",
    ] {
        let executable = executable(runner.path(), body);
        let mut command = std::process::Command::new(executable);
        assert!(
            run_command(&mut command, Duration::from_secs(2), 16)
                .unwrap_err()
                .to_string()
                .contains("output limit")
        );
    }
}

#[cfg(unix)]
#[test]
fn running_scanner_is_stopped_past_its_output_limit() {
    let runner = tempfile::tempdir().unwrap();
    // Past the limit, then quiet: only the polling budget can end it early.
    let executable = executable(runner.path(), "head -c 4096 /dev/zero; exec /bin/sleep 5");
    let mut command = std::process::Command::new(executable);
    let start = Instant::now();
    let error = run_command(&mut command, Duration::from_secs(4), 16).unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(
        error
            .to_string()
            .contains("Trivy exceeded its output limit"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn snapshot_and_dependency_symlinks_are_never_read_through() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    put(outside.path(), "Cargo.lock", b"private external");
    symlink(
        outside.path().join("Cargo.lock"),
        root.path().join("Cargo.lock"),
    )
    .unwrap();
    let inputs = collect_inputs(root.path(), &Config::default()).unwrap();
    assert!(inputs.files.is_empty());
    assert!(!inputs.diagnostics.is_empty());
    symlink(outside.path(), root.path().join(".archguard")).unwrap();
    let report = read_security(root.path(), &Config::default());
    assert_eq!(report.status, "error");
    assert!(!outside.path().join("security.json").exists());
}

#[test]
fn oversized_dependency_file_is_an_explicit_error() {
    let root = tempfile::tempdir().unwrap();
    let file = fs::File::create(root.path().join("Cargo.lock")).unwrap();
    file.set_len(MAX_FILE_BYTES + 1).unwrap();
    assert!(security_fingerprint(root.path(), &Config::default()).is_err());
    assert_eq!(
        read_security(root.path(), &Config::default()).status,
        "error"
    );
}

#[test]
fn parser_deduplicates_findings_and_preserves_unknown_severity_and_fixes() {
    let vulnerability = json!({"VulnerabilityID":"CVE-2024-0001","PkgName":"demo","InstalledVersion":"1.0","FixedVersion":"1.2, 2.0","Severity":"HIGH","Title":"Demo issue","PrimaryURL":"https://example.invalid/advisory"});
    let data = json!({"SchemaVersion":2,"CreatedAt":"2026-10-27T12:00:00Z","Results":[
        {"Target":"Cargo.lock","Class":"lang-pkgs","Type":"cargo","Packages":[{"Name":"demo","Version":"1.0"}],"Vulnerabilities":[vulnerability.clone(),{"VulnerabilityID":"GHSA-demo","PkgName":"demo","InstalledVersion":"1.0","Severity":"UNRECOGNIZED","FixedVersion":null}]},
        {"Target":"nested/Cargo.lock","Class":"lang-pkgs","Type":"cargo","Packages":[{"Name":"demo","Version":"1.0"}],"Vulnerabilities":[vulnerability]}
    ]});
    let report = parse_trivy(
        &serde_json::to_vec(&data).unwrap(),
        &["Cargo.lock", "nested/Cargo.lock"],
    )
    .unwrap();
    assert_eq!(report.findings.len(), 2);
    assert_eq!(report.packages_scanned, 1);
    assert_eq!(report.findings[0].severity, "high");
    assert_eq!(
        report.findings[0].fixed_version.as_deref(),
        Some("1.2, 2.0")
    );
    assert_eq!(
        report.findings[0].targets,
        ["Cargo.lock", "nested/Cargo.lock"]
    );
    assert_eq!(report.findings[1].severity, "unknown");
    assert_eq!(report.findings[1].fixed_version, None);
    assert_eq!(report.scanned_at.as_deref(), Some("2026-10-27T12:00:00Z"));
}

#[test]
fn empty_and_uncovered_scans_are_partial_not_clean() {
    for data in [
        json!({"SchemaVersion":2,"Results":null}),
        json!({"SchemaVersion":2,"Results":[]}),
    ] {
        let report = parse_trivy(&serde_json::to_vec(&data).unwrap(), &["App.csproj"]).unwrap();
        assert_eq!(report.status, "partial");
        assert!(!report.diagnostics.is_empty());
    }
}

#[test]
fn malformed_output_cannot_be_mistaken_for_no_vulnerabilities() {
    for data in [
        json!(null),
        json!({}),
        json!({"SchemaVersion":2,"Results":"bad"}),
        json!({"SchemaVersion":2,"Results":[{"Target":"Cargo.lock","Type":"cargo","Packages":null,"Vulnerabilities":[{"PkgName":"demo"}]}]}),
    ] {
        assert!(
            parse_trivy(&serde_json::to_vec(&data).unwrap(), &["Cargo.lock"]).is_err(),
            "accepted {data}"
        );
    }
}
