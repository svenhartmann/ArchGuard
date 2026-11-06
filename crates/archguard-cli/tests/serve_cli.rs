//! Real loopback HTTP, a temporary project, and the built CLI.
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime};

use serde_json::Value;
use tempfile::{TempDir, tempdir};

const CONFIG: &str = "[project]\nsource_roots = []\nlanguages = ['ts']\n";

struct Server {
    child: Child,
    address: String,
    root: TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn start() -> Self {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("one")).unwrap();
        fs::create_dir(root.path().join("two")).unwrap();
        fs::write(root.path().join("one/A.ts"), "export const a = 1;\n").unwrap();
        fs::write(root.path().join("two/B.ts"), "export const b = 2;\n").unwrap();
        fs::write(root.path().join("archguard.toml"), CONFIG).unwrap();
        let (child, address) = Self::spawn(root.path());
        Self {
            child,
            address,
            root,
        }
    }

    fn spawn(root: &Path) -> (Child, String) {
        Self::spawn_with_config(root, None)
    }

    fn spawn_with_config(root: &Path, config: Option<&Path>) -> (Child, String) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_archguard"));
        if let Some(config) = config {
            command.arg("--config").arg(config);
        }
        let mut child = command
            .arg("serve")
            .arg(root)
            .args(["--port", "0"])
            .env("GIT_TRACE", root.join(".git-trace"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        if !line.starts_with("http://127.0.0.1:") {
            let output = child.wait_with_output().unwrap();
            panic!(
                "server did not start: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        (
            child,
            line.trim()
                .trim_start_matches("http://")
                .trim_end_matches('/')
                .into(),
        )
    }

    fn restart(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
        (self.child, self.address) = Self::spawn(self.root.path());
    }

    fn request(&self, method: &str, path: &str, host: Option<&str>) -> (u16, String) {
        let mut socket = TcpStream::connect(&self.address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(
            socket,
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            host.unwrap_or(&self.address)
        )
        .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").expect("HTTP response");
        assert!(headers.contains("Cache-Control: no-store"));
        let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, body.to_owned())
    }

    fn post_json(&self, path: &str, body: &str) -> (u16, String) {
        let mut socket = TcpStream::connect(&self.address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(
            socket,
            "POST {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            self.address,
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").expect("HTTP response");
        let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, body.to_owned())
    }

    fn version(&self) -> Value {
        // The browser's version probe is intentionally non-blocking. Existing
        // invalidation assertions need a completed fresh check, not a transient
        // loading flag or an older cached revision.
        let (status, body) = self.request("GET", "/__archguard/check", None);
        assert_eq!(status, 200);
        serde_json::from_str(&body).unwrap()
    }

    fn payload(&self) -> Value {
        self.version();
        let (status, html) = self.request("GET", "/", None);
        assert_eq!(status, 200);
        assert!(html.contains("/__archguard/version"));
        let payload = html
            .split_once("<script id=\"archguard-data\" type=\"application/json\">")
            .unwrap()
            .1
            .split_once("</script>")
            .unwrap()
            .0;
        serde_json::from_str(payload).unwrap()
    }
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn workspace_sources_load_in_background_and_refresh_independently_of_overview() {
    let server = Server::start();
    fs::write(
        server.root.path().join("one/package.json"),
        r#"{"name":"one","version":"1.0.0","dependencies":{"two":"file:../two"}}"#,
    )
    .unwrap();
    fs::write(
        server.root.path().join("two/package.json"),
        r#"{"name":"two","version":"1.0.0"}"#,
    )
    .unwrap();
    let overview = server.version();
    let (status, first) = server.request("GET", "/__archguard/check?workspace=sources", None);
    assert_eq!(status, 200, "{first}");
    let first: Value = serde_json::from_str(&first).unwrap();
    let (status, page) = server.request("GET", "/?workspace=sources", None);
    assert_eq!(status, 200);
    assert!(page.contains("/__archguard/version?workspace=sources"));
    assert!(page.contains("project:one/package.json") && page.contains("workspace_sources"));
    assert!(
        server
            .root
            .path()
            .join(".archguard/reports/workspace.html")
            .exists()
    );
    let (_, unchanged) = server.request("GET", "/__archguard/check?workspace=sources", None);
    assert_eq!(serde_json::from_str::<Value>(&unchanged).unwrap(), first);
    fs::write(server.root.path().join("two/C.ts"), "export const c = 3;\n").unwrap();
    let (status, changed) = server.request("GET", "/__archguard/check?workspace=sources", None);
    assert_eq!(status, 200, "{changed}");
    assert_ne!(serde_json::from_str::<Value>(&changed).unwrap(), first);
    assert_eq!(
        server.version(),
        overview,
        "source edits must not replace the lightweight repository overview"
    );
    assert_eq!(
        server
            .request("GET", "/?workspace=sources&project=one", None)
            .0,
        400
    );
}

fn history_scans(root: &Path) -> usize {
    fs::read_to_string(root.join(".git-trace"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("built-in: git log "))
        .count()
}

const SECURITY_LOCK: &str = r#"{"name":"serve-security-test","lockfileVersion":3,"packages":{"node_modules/example-library":{"version":"1.0.0"}}}"#;

fn write_security_snapshot(root: &Path, id: &str) -> archguard_engine::SecurityReport {
    let config = archguard_engine::load_config(None, root).unwrap();
    let report = archguard_engine::SecurityReport {
        schema_version: 1,
        status: "ready".into(),
        scanner: "trivy".into(),
        scanner_version: Some("test-scanner-1".into()),
        scanned_at: Some("2026-10-27T12:00:00Z".into()),
        database_updated_at: Some("2026-10-27T09:00:00Z".into()),
        fingerprint: archguard_engine::security_fingerprint(root, &config.config).unwrap(),
        packages_scanned: 1,
        targets: vec![archguard_engine::SecurityTarget {
            path: "package-lock.json".into(),
            ecosystem: "npm".into(),
            packages: 1,
        }],
        findings: vec![archguard_engine::SecurityFinding {
            id: id.into(),
            package: "example-library".into(),
            installed_version: "1.0.0".into(),
            fixed_version: Some("1.0.1".into()),
            ecosystem: "npm".into(),
            severity: "high".into(),
            title: "Test advisory from a saved scanner report".into(),
            url: Some("https://example.invalid/advisory".into()),
            targets: vec!["package-lock.json".into()],
        }],
        diagnostics: vec![],
    };
    fs::create_dir_all(root.join(".archguard")).unwrap();
    fs::write(
        root.join(".archguard/security.json"),
        serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    report
}

#[test]
fn serve_security_includes_saved_snapshot_in_html_payload() {
    let server = Server::start();
    let root = server.root.path();
    fs::write(root.join("package-lock.json"), SECURITY_LOCK).unwrap();
    let saved = write_security_snapshot(root, "CVE-2099-0001");
    let expected = serde_json::to_value(saved).unwrap();
    assert_eq!(
        server.payload()["insights"]["security"],
        expected,
        "the HTML payload carries saved scanner evidence, timestamps and coverage"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(root.join(".archguard/security.json")).unwrap())
            .unwrap(),
        expected,
        "serving does not rewrite the scanner snapshot"
    );
}

#[test]
fn serve_security_snapshot_creation_and_replacement_update_the_live_version() {
    let server = Server::start();
    let root = server.root.path();
    fs::write(root.join("package-lock.json"), SECURITY_LOCK).unwrap();
    let original_payload = server.payload();
    let initial = server.version();
    write_security_snapshot(root, "CVE-2099-0001");
    let created = server.version();
    assert_ne!(
        initial, created,
        "a new security snapshot must invalidate a warm page without source or dependency changes"
    );
    let first = server.payload();
    assert_eq!(first["insights"]["security"]["status"], "ready");
    assert_eq!(
        first["insights"]["security"]["findings"][0]["id"],
        "CVE-2099-0001"
    );
    assert_eq!(first["nodes"], original_payload["nodes"]);
    assert_eq!(first["edges"], original_payload["edges"]);
    assert_eq!(
        created,
        server.version(),
        "unchanged snapshots keep a stable live version"
    );

    write_security_snapshot(root, "CVE-2099-0002");
    let replaced = server.version();
    assert_ne!(
        created, replaced,
        "replacement scanner findings refresh the warm page"
    );
    let second = server.payload();
    assert_eq!(
        second["insights"]["security"]["findings"][0]["id"],
        "CVE-2099-0002"
    );
    assert_eq!(second["nodes"], original_payload["nodes"]);
    assert_eq!(second["edges"], original_payload["edges"]);
    assert_eq!(
        fs::read_to_string(root.join("package-lock.json")).unwrap(),
        SECURITY_LOCK
    );
    assert_eq!(
        fs::read_to_string(root.join("one/A.ts")).unwrap(),
        "export const a = 1;\n"
    );
    assert_eq!(replaced, server.version());
}

#[test]
fn serve_security_dependency_edits_mark_saved_findings_stale() {
    let server = Server::start();
    let root = server.root.path();
    fs::write(root.join("package-lock.json"), SECURITY_LOCK).unwrap();
    let saved = write_security_snapshot(root, "CVE-2099-0001");
    let initial_payload = server.payload();
    let initial = server.version();
    let snapshot_bytes = fs::read(root.join(".archguard/security.json")).unwrap();
    fs::write(
        root.join("package-lock.json"),
        SECURITY_LOCK.replace("1.0.0", "1.0.1"),
    )
    .unwrap();
    let changed = server.version();
    assert_ne!(
        initial, changed,
        "changed dependency inputs must invalidate the saved security state even without import changes"
    );
    let payload = server.payload();
    let security = &payload["insights"]["security"];
    assert_eq!(security["status"], "stale");
    assert_eq!(
        security["fingerprint"], saved.fingerprint,
        "stale evidence retains the fingerprint of the scanned inputs"
    );
    assert_eq!(
        security["findings"],
        serde_json::to_value(saved.findings).unwrap()
    );
    assert!(!security["diagnostics"].as_array().unwrap().is_empty());
    assert_eq!(payload["nodes"], initial_payload["nodes"]);
    assert_eq!(payload["edges"], initial_payload["edges"]);
    assert_eq!(
        fs::read(root.join(".archguard/security.json")).unwrap(),
        snapshot_bytes,
        "serve never rescans or rewrites stale evidence"
    );
    assert_eq!(changed, server.version());
}

#[test]
fn serve_reuses_rendered_page_and_invalidates_source_git_and_ownership_inputs() {
    let server = Server::start();
    let root = server.root.path();
    git(root, &["init", "--quiet", "--template="]);
    git(root, &["config", "user.name", "Serve Test"]);
    git(root, &["config", "user.email", "serve@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    git(root, &["add", "one/A.ts", "two/B.ts", "archguard.toml"]);
    git(root, &["commit", "--quiet", "-m", "initial"]);

    let initial = server.version();
    let scans = history_scans(root);
    assert!(scans > 0, "the first render reads Git history");
    assert_eq!(initial, server.version());
    assert_eq!(
        history_scans(root),
        scans,
        "an unchanged version probe must reuse the rendered page without rescanning Git history"
    );

    fs::write(
        root.join("one/A.ts"),
        "export const a = 1;\n// another line\n\n",
    )
    .unwrap();
    let source_changed = server.version();
    assert_ne!(
        initial, source_changed,
        "line count changes invalidate even when graph topology is unchanged"
    );
    assert!(history_scans(root) > scans);
    let scans = history_scans(root);
    git(root, &["add", "one/A.ts"]);
    git(root, &["commit", "--quiet", "-m", "comment only"]);
    let committed = server.version();
    assert_ne!(
        source_changed, committed,
        "a new commit updates churn even with a warm source cache"
    );
    assert!(history_scans(root) > scans);
    let scans = history_scans(root);

    fs::write(root.join("CODEOWNERS"), "one/* @org/one\ntwo/* @org/two\n").unwrap();
    let owned = server.version();
    assert_ne!(
        committed, owned,
        "CODEOWNERS is an input even without ownership gate rules"
    );
    assert!(history_scans(root) > scans);
    let scans = history_scans(root);
    assert_eq!(owned, server.version());
    assert_eq!(history_scans(root), scans);
}

#[test]
fn serve_updates_graph_without_manual_html_generation_and_recovers_from_bad_config() {
    let server = Server::start();
    let calls = &server.payload()["calls"];
    assert_eq!(calls["status"], "unavailable");
    assert_eq!(calls["analyzers"][0]["language"], "typescript");
    assert_eq!(calls["analyzers"][0]["status"], "unavailable");
    assert!(calls["calls"].as_array().unwrap().is_empty());
    assert!(server.payload()["edges"].as_array().unwrap().is_empty());
    let initial = server.version();
    assert_eq!(
        initial,
        server.version(),
        "unchanged project keeps its version"
    );
    fs::write(
        server.root.path().join("one/A.ts"),
        "import { b } from '../two/B';\n",
    )
    .unwrap();
    assert_ne!(
        initial,
        server.version(),
        "updated payload: {}",
        server.payload()
    );
    let payload = server.payload();
    assert_eq!(payload["edges"].as_array().unwrap().len(), 1);
    assert_eq!(payload["view"]["depth"], 0);
    assert!(
        payload["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "one" && n["kind"] == "module")
    );
    let changed = server.version();
    fs::write(
        server.root.path().join("archguard.toml"),
        "unknown_gate = true\n",
    )
    .unwrap();
    assert_ne!(changed, server.version());
    let (_, error_page) = server.request("GET", "/", None);
    assert!(error_page.contains("unknown_gate"));
    assert!(
        error_page.contains("/__archguard/version"),
        "error pages keep polling for recovery"
    );
    fs::write(server.root.path().join("archguard.toml"), CONFIG).unwrap();
    assert_eq!(server.payload()["edges"].as_array().unwrap().len(), 1);
    assert!(!server.root.path().join("index.html").exists());
    assert!(!server.root.path().join("zyklen.html").exists());
}

#[test]
fn serve_saves_standalone_project_snapshots_and_preserves_last_successful_analysis() {
    let mut server = Server::start();
    let root = server.root.path().to_path_buf();
    let graph_path = root.join(".archguard/graph.json");
    let html_path = root.join(".archguard/reports/graph.html");
    let config_path = root.join(".archguard/effective-config.json");
    let initial = server.version();
    assert!(
        graph_path.is_file(),
        "serve saves the source graph in the analyzed project"
    );
    assert!(
        html_path.is_file(),
        "serve saves a standalone HTML report in the analyzed project"
    );
    let saved_config = fs::read(&config_path).expect("serve saves the effective analysis config");
    let config: Value = serde_json::from_slice(&saved_config).unwrap();
    assert_eq!(
        config["config"]["project"]["languages"],
        serde_json::json!(["ts"])
    );
    assert!(
        config["sources"].is_object(),
        "effective values retain their provenance"
    );

    let saved_html = fs::read_to_string(&html_path).unwrap();
    assert!(saved_html.contains("archguard-data"));
    assert!(
        !saved_html.contains("/__archguard/version"),
        "saved reports do not poll a live server"
    );
    let (_, live_html) = server.request("GET", "/", None);
    assert!(live_html.contains("/__archguard/version"));
    assert!(live_html.starts_with(saved_html.split_once("</body>").unwrap().0));
    let graph: archguard_engine::core::Graph =
        serde_json::from_slice(&fs::read(&graph_path).unwrap()).unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == archguard_engine::core::NodeKind::File)
            .count(),
        2,
        "the saved graph includes files even though the viewer initially displays modules"
    );
    assert!(graph.edges.is_empty());

    // Set a known old timestamp so even a fast identical rewrite is observable.
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    for path in [&graph_path, &html_path, &config_path] {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    assert_eq!(initial, server.version());
    server.restart();
    assert_eq!(initial, server.version());
    for path in [&graph_path, &html_path, &config_path] {
        assert_eq!(
            fs::metadata(path).unwrap().modified().unwrap(),
            old,
            "unchanged probes and server restarts do not rewrite saved artifacts"
        );
    }

    fs::write(root.join("one/A.ts"), "import { b } from '../two/B';\n").unwrap();
    let changed = server.version();
    assert_ne!(initial, changed);
    let saved_graph = fs::read(&graph_path).unwrap();
    let graph: archguard_engine::core::Graph = serde_json::from_slice(&saved_graph).unwrap();
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.node_name(graph.edges[0].from), Some("one/A.ts"));
    assert_eq!(graph.node_name(graph.edges[0].to), Some("two/B.ts"));
    let changed_html = fs::read(&html_path).unwrap();
    let changed_config = fs::read(&config_path).unwrap();
    assert_ne!(saved_html.as_bytes(), changed_html);

    fs::write(root.join("archguard.toml"), "unknown_gate = true\n").unwrap();
    assert_ne!(changed, server.version());
    let (_, error_html) = server.request("GET", "/", None);
    assert!(error_html.contains("unknown_gate"));
    assert_eq!(
        fs::read(&graph_path).unwrap(),
        saved_graph,
        "analysis errors preserve the last good graph"
    );
    assert_eq!(
        fs::read(&html_path).unwrap(),
        changed_html,
        "analysis errors preserve the last good HTML"
    );
    assert_eq!(
        fs::read(&config_path).unwrap(),
        changed_config,
        "analysis errors preserve the last good effective config"
    );

    fs::write(root.join("two/B.ts"), "import { a } from '../one/A';\n").unwrap();
    fs::write(root.join("archguard.toml"), CONFIG).unwrap();
    assert_ne!(changed, server.version());
    let graph: archguard_engine::core::Graph =
        serde_json::from_slice(&fs::read(&graph_path).unwrap()).unwrap();
    assert_eq!(
        graph.edges.len(),
        2,
        "recovered analyses refresh saved snapshots"
    );
    assert_ne!(fs::read(&html_path).unwrap(), changed_html);
    assert_eq!(server.request("GET", "/.archguard/graph.json", None).0, 404);
    assert_eq!(
        server
            .request("GET", "/.archguard/reports/graph.html", None)
            .0,
        404
    );
}

#[test]
fn serve_keeps_live_report_available_and_warns_when_project_snapshot_cannot_be_saved() {
    let mut server = Server::start();
    fs::create_dir_all(server.root.path().join(".archguard")).unwrap();
    fs::write(
        server.root.path().join(".archguard/reports"),
        "existing file",
    )
    .unwrap();
    let payload = server.payload();
    assert!(!payload["nodes"].as_array().unwrap().is_empty());
    server.child.kill().unwrap();
    server.child.wait().unwrap();
    let mut stderr = String::new();
    server
        .child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(
        stderr.contains("could not save project graph"),
        "save failure must be explicit: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(server.root.path().join(".archguard/reports")).unwrap(),
        "existing file"
    );
}

#[test]
fn serve_only_exposes_its_loopback_report_routes() {
    let server = Server::start();
    assert_eq!(server.request("GET", "/one/A.ts", None).0, 404);
    assert_eq!(server.request("GET", "/../Cargo.toml", None).0, 404);
    assert_eq!(server.request("POST", "/", None).0, 405);
    assert_eq!(server.request("GET", "/", Some("untrusted.example")).0, 403);
    // Query parameters now select analysis scopes; unsupported parameters fail explicitly.
    assert_eq!(server.request("GET", "/?fresh=1", None).0, 400);
    // Deferred payload sections: only known names of the current revision.
    assert_eq!(
        server
            .request("GET", "/__archguard/payload/../Cargo.toml/x", None)
            .0,
        404
    );
    assert_eq!(
        server.request("GET", "/__archguard/payload/calls", None).0,
        404
    );
    assert_eq!(
        server
            .request(
                "GET",
                "/",
                Some(&format!("{}\r\nHost: untrusted.example", server.address))
            )
            .0,
        403
    );
}

#[test]
fn served_version_detects_edits_while_the_server_was_stopped() {
    let mut server = Server::start();
    let initial = server.version();
    fs::write(
        server.root.path().join("one/A.ts"),
        "import { b } from '../two/B';\n",
    )
    .unwrap();
    server.restart();
    assert_ne!(
        initial,
        server.version(),
        "an open browser must notice the changed project after reconnecting"
    );
}

#[test]
fn serve_detects_compose_creation_edits_and_removal_without_source_changes() {
    let server = Server::start();
    let root = server.root.path();
    let initial = server.version();
    let compose = "services:\n  one:\n    build: ./one\n    environment:\n      TWO_URL: http://two:8080/api\n  two:\n    build: ./two\n    environment:\n      ONE_URL: http://one:8080/api\n";
    fs::write(root.join("compose.yaml"), compose).unwrap();
    let detected = server.payload();
    assert_eq!(
        detected["services"]["services"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(detected["services"]["monorepo"], true);
    assert_eq!(
        detected["services"]["cycles"].as_array().map(Vec::len),
        Some(1)
    );
    let added = server.version();
    assert_ne!(
        initial, added,
        "new Compose files invalidate a warm source cache"
    );
    assert_eq!(
        added,
        server.version(),
        "unchanged topology keeps the revision stable"
    );
    fs::write(
        root.join("compose.yaml"),
        compose.replace("http://one:8080/api", "https://outside.invalid/api"),
    )
    .unwrap();
    let changed = server.payload();
    assert!(changed["services"]["cycles"].as_array().unwrap().is_empty());
    let edited = server.version();
    assert_ne!(
        added, edited,
        "API configuration changes invalidate independently of imports"
    );
    fs::remove_file(root.join("compose.yaml")).unwrap();
    assert!(
        server.payload()["services"]["services"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_ne!(edited, server.version());
}

#[test]
fn serve_can_show_a_compose_only_project_without_source_imports() {
    let mut server = Server::start();
    let root = server.root.path();
    fs::remove_file(root.join("one/A.ts")).unwrap();
    fs::remove_file(root.join("two/B.ts")).unwrap();
    fs::remove_file(root.join("archguard.toml")).unwrap();
    fs::write(root.join("compose.yml"), "services:\n  api:\n    image: example-api\n    environment:\n      DATABASE_URL: http://db:8080\n  db:\n    image: example-db\n").unwrap();
    server.restart();
    let payload = server.payload();
    assert_eq!(
        payload["services"]["services"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(
        payload["services"]["dependencies"].as_array().map(Vec::len),
        Some(1)
    );
}

#[test]
fn serve_monorepo_inventory_reloads_when_project_manifests_change() {
    let server = Server::start();
    let before = server.payload();
    let version = server.version();
    let app = server.root.path().join("functions/search");
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("pom.xml"),"<project><artifactId>search</artifactId><build><plugins><plugin><groupId>com.microsoft.azure</groupId><artifactId>azure-functions-maven-plugin</artifactId></plugin></plugins></build></project>").unwrap();
    let changed = server.version();
    assert_ne!(
        changed, version,
        "project inventory is an independent live-view input"
    );
    let payload = server.payload();
    assert_eq!(
        payload["services"]["projects"][0]["path"],
        "functions/search"
    );
    assert_eq!(payload["services"]["services"].as_array().unwrap().len(), 1);
    assert_eq!(payload["nodes"], before["nodes"]);
    assert_eq!(payload["edges"], before["edges"]);
    assert_eq!(
        changed,
        server.version(),
        "unchanged inventory keeps the live version stable"
    );
    fs::remove_file(app.join("pom.xml")).unwrap();
    assert_ne!(changed, server.version());
    assert!(
        server.payload()["services"]["projects"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

fn declare_live_projects(root: &Path) {
    for name in ["one", "two"] {
        fs::write(
            root.join(name).join("package.json"),
            format!(r#"{{"name":"{name}","scripts":{{"start":"node A.ts"}}}}"#),
        )
        .unwrap();
    }
}
fn request_payload(server: &Server, path: &str) -> Value {
    let check = path.split_once('?').map_or_else(
        || "/__archguard/check".to_owned(),
        |(_, query)| format!("/__archguard/check?{query}"),
    );
    let (status, result) = server.request("GET", &check, None);
    assert_eq!(status, 200, "{result}");
    let (status, html) = server.request("GET", path, None);
    assert_eq!(status, 200, "{html}");
    serde_json::from_str(
        html.split_once("<script id=\"archguard-data\" type=\"application/json\">")
            .unwrap()
            .1
            .split_once("</script>")
            .unwrap()
            .0,
    )
    .unwrap()
}
fn request_version(server: &Server, project: &str) -> Value {
    let (status, body) = server.request(
        "GET",
        &format!("/__archguard/check?project={project}"),
        None,
    );
    assert_eq!(status, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

#[test]
fn serve_monorepo_overview_never_merges_source_models_or_replaces_saved_graph() {
    let server = Server::start();
    let root = server.root.path();
    declare_live_projects(root);
    fs::create_dir_all(root.join(".archguard")).unwrap();
    fs::write(
        root.join(".archguard/graph.json"),
        "previous source graph sentinel",
    )
    .unwrap();
    let payload = server.payload();
    assert_eq!(payload["insights"]["repository"]["mode"], "overview");
    assert_eq!(payload["services"]["projects"].as_array().unwrap().len(), 2);
    assert!(
        payload["nodes"].as_array().unwrap().is_empty(),
        "repository overview must not merge source models"
    );
    assert!(payload["edges"].as_array().unwrap().is_empty());
    assert!(
        payload["calls"].is_null(),
        "the root overview must not execute a combined semantic analysis"
    );
    assert!(payload["reviewContext"].is_null());
    assert!(
        !root.join(".archguard/cache.json").exists(),
        "overview must bypass the source extraction pipeline"
    );
    assert_eq!(
        fs::read_to_string(root.join(".archguard/graph.json")).unwrap(),
        "previous source graph sentinel"
    );
    let html = fs::read_to_string(root.join(".archguard/reports/services.html")).unwrap();
    assert!(
        !html.contains("/__archguard/version"),
        "saved overview is standalone"
    );
    assert_eq!(server.version(), server.version());
}

#[test]
fn serve_project_query_isolates_sources_own_config_versions_and_encoded_paths() {
    let server = Server::start();
    let root = server.root.path();
    declare_live_projects(root);
    fs::write(
        root.join("archguard.toml"),
        "[project]\nlanguages=['ts']\nexclude=['**/*.ts']\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("one/.archguard")).unwrap();
    fs::write(root.join("one/.archguard/archguard.toml"), CONFIG).unwrap();
    let one = request_payload(&server, "/?project=%6fne");
    let two = request_payload(&server, "/?project=two");
    assert_eq!(
        one["root"],
        root.join("one").canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(one["insights"]["repository"]["active_project"], "one");
    assert_eq!(one["insights"]["repository"]["mode"], "project");
    assert!(
        one["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "A.ts"),
        "project config takes precedence over root policy"
    );
    assert!(
        !one["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "B.ts")
    );
    assert!(
        two["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "B.ts")
    );
    let first = request_version(&server, "one");
    let other = request_version(&server, "two");
    assert_ne!(first, other);
    assert_eq!(first, request_version(&server, "one"));
    fs::write(
        root.join("one/A.ts"),
        "export const a = 1;\n// updated project\n",
    )
    .unwrap();
    assert_ne!(first, request_version(&server, "one"));
    assert_eq!(
        other,
        request_version(&server, "two"),
        "editing one project must not change an unrelated project's revision"
    );
    let encoded = root.join("apps/search api");
    fs::create_dir_all(&encoded).unwrap();
    fs::write(
        encoded.join("package.json"),
        r#"{"name":"search","scripts":{"start":"node index.js"}}"#,
    )
    .unwrap();
    fs::write(encoded.join("index.js"), "export const search = 1;\n").unwrap();
    let chosen = request_payload(&server, "/?project=apps%2Fsearch%20api");
    assert_eq!(
        chosen["root"],
        encoded.canonicalize().unwrap().to_str().unwrap()
    );
    let (status, html) = server.request("GET", "/?project=apps%2Fsearch%20api", None);
    assert_eq!(status, 200);
    assert!(html.contains("/__archguard/version?project=apps%2Fsearch%20api"));
}

#[test]
fn serve_project_query_rejects_unrecognized_and_unsafe_scope_requests() {
    let server = Server::start();
    declare_live_projects(server.root.path());
    for query in [
        "project=",
        "project=..%2Foutside",
        "project=%2Ftmp",
        "project=one%2F..%2Ftwo",
        "project=%ZZ",
        "project=%FF",
        "project=one&project=two",
        "other=one",
        "project=one%00",
        "project=one%5C..%5Ctwo",
    ] {
        let (status, _) = server.request("GET", &format!("/?{query}"), None);
        assert_eq!(status, 400, "invalid query {query}");
    }
    for project in ["missing", "one%2FA.ts"] {
        assert_eq!(
            server
                .request("GET", &format!("/?project={project}"), None)
                .0,
            404
        );
    }
    let workspace = server.root.path().join("tools");
    fs::create_dir(&workspace).unwrap();
    fs::write(
        workspace.join("package.json"),
        r#"{"name":"workspace","workspaces":["packages/*"]}"#,
    )
    .unwrap();
    assert_eq!(
        server.request("GET", "/?project=tools", None).0,
        404,
        "workspace containers are not analysis projects"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = tempdir().unwrap();
        fs::write(
            outside.path().join("package.json"),
            r#"{"name":"external","scripts":{"start":"node index.js"}}"#,
        )
        .unwrap();
        symlink(outside.path(), server.root.path().join("external")).unwrap();
        assert_eq!(server.request("GET", "/?project=external", None).0, 404);
        assert!(!outside.path().join(".archguard").exists());
    }
    assert!(
        !server.root.path().join(".archguard/cache.json").exists(),
        "invalid scopes never run analysis"
    );
}

#[test]
fn serve_selected_parent_excludes_nested_projects_without_changing_their_configs() {
    let server = Server::start();
    let root = server.root.path();
    declare_live_projects(root);
    let nested = root.join("one/nested[app]");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        nested.join("package.json"),
        r#"{"name":"nested","scripts":{"start":"node Nested.ts"}}"#,
    )
    .unwrap();
    fs::write(nested.join("Nested.ts"), "export const nested=1;\n").unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"root-app","scripts":{"start":"node Root.ts"}}"#,
    )
    .unwrap();
    fs::write(root.join("Root.ts"), "export const rootApp=1;\n").unwrap();
    let original = fs::read(root.join("archguard.toml")).unwrap();
    let parent = request_payload(&server, "/?project=one");
    assert_eq!(
        parent["insights"]["repository"]["excluded_projects"],
        serde_json::json!(["nested[app]"])
    );
    assert!(
        parent["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "A.ts")
    );
    assert!(
        !parent["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"].as_str().unwrap_or("").contains("Nested")),
        "literal metacharacters in nested project paths must not bypass scope isolation"
    );
    let outer = request_payload(&server, "/?project=.");
    assert!(
        outer["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "Root.ts")
    );
    assert!(!outer["nodes"].as_array().unwrap().iter().any(|n| {
        ["A.ts", "B.ts", "Nested.ts"]
            .iter()
            .any(|file| n["name"].as_str().unwrap_or("").ends_with(file))
    }));
    assert_eq!(
        fs::read(root.join("archguard.toml")).unwrap(),
        original,
        "request-specific exclusions must not rewrite user configuration"
    );
    assert!(!root.join("one/archguard.toml").exists());
    assert_eq!(
        server.payload()["insights"]["repository"]["mode"],
        "overview"
    );
}

#[test]
fn serve_project_selector_updates_when_inventory_changes_with_a_warm_source_cache() {
    let server = Server::start();
    let root = server.root.path();
    declare_live_projects(root);
    let initial = request_version(&server, "one");
    fs::write(
        root.join("two/package.json"),
        r#"{"name":"renamed-app","scripts":{"start":"node B.ts"}}"#,
    )
    .unwrap();
    let changed = request_version(&server, "one");
    assert_ne!(initial, changed);
    let payload = request_payload(&server, "/?project=one");
    assert!(
        payload["insights"]["repository"]["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "renamed-app")
    );
    fs::remove_file(root.join("two/package.json")).unwrap();
    assert_ne!(changed, request_version(&server, "one"));
    assert_eq!(
        server.request("GET", "/?project=two", None).0,
        404,
        "cached pages do not authorize removed projects"
    );
}

#[test]
fn serve_selected_project_does_not_inherit_the_explicit_repository_config() {
    let mut server = Server::start();
    declare_live_projects(server.root.path());
    let explicit = server.root.path().join("repository-only.toml");
    fs::write(&explicit, "[project]\nlanguages=['ts']\nexclude=['**']\n").unwrap();
    server.child.kill().unwrap();
    server.child.wait().unwrap();
    (server.child, server.address) = Server::spawn_with_config(server.root.path(), Some(&explicit));
    assert_eq!(
        server.payload()["insights"]["repository"]["mode"],
        "overview"
    );
    let selected = request_payload(&server, "/?project=one");
    assert!(
        selected["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "A.ts"),
        "root --config cannot exclude a selected project's own sources"
    );
    assert!(!server.root.path().join("one/archguard.toml").exists());
}

#[test]
fn staged_comparison_endpoint_uses_index_sources() {
    let server = Server::start();
    let root = server.root.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    git(root, &["add", "archguard.toml", "one", "two"]);
    git(root, &["commit", "-qm", "base"]);
    fs::write(root.join("one/A.ts"), "import '../two/B';\n").unwrap();
    git(root, &["add", "one/A.ts"]);
    fs::write(root.join("one/A.ts"), "export const unstaged = 2;\n").unwrap();
    let (status, html) = server.request("GET", "/__archguard/diff", None);
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("architecture-diff-data"));
    assert!(html.contains("import '../two/B'"));
    assert!(!html.contains("export const unstaged"));
    assert_eq!(
        server
            .request("GET", "/__archguard/diff?file=../../elsewhere", None)
            .0,
        400
    );
    assert_eq!(
        server
            .request("GET", "/__archguard/diff", Some("external.invalid"))
            .0,
        403
    );
    server.version();
    // Comparison endpoints remain available, but the viewer no longer links this UI.
    assert!(
        !server
            .request("GET", "/", None)
            .1
            .contains("/__archguard/compare")
    );
    assert!(
        server
            .request("GET", "/__archguard/compare", None)
            .1
            .contains("compare-index")
    );
}

#[test]
fn package_comparison_endpoint_lists_branches_and_compares_committed_sources() {
    let server = Server::start();
    let root = server.root.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    fs::write(root.join("one/A.ts"), "import '../two/B';\n").unwrap();
    fs::write(
        root.join("one/package.json"),
        r#"{"name":"one","private":true}"#,
    )
    .unwrap();
    fs::write(
        root.join("two/package.json"),
        r#"{"name":"two","private":true}"#,
    )
    .unwrap();
    git(root, &["add", "archguard.toml", "one", "two"]);
    git(root, &["commit", "-qm", "base"]);
    git(root, &["switch", "-qc", "feature/packages"]);
    fs::write(root.join("two/B.ts"), "import '../one/A';\n").unwrap();
    git(root, &["add", "two/B.ts"]);
    git(root, &["commit", "-qm", "cycle"]);
    git(root, &["switch", "-q", "main"]);
    fs::write(root.join("two/B.ts"), "export const dirty=1;\n").unwrap();
    let index = fs::read(root.join(".git/index")).unwrap();
    let (status, body) = server.request("GET", "/__archguard/refs", None);
    assert_eq!(status, 200, "{body}");
    let refs: Value = serde_json::from_str(&body).unwrap();
    assert!(
        refs["branches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reference"] == "refs/heads/feature/packages")
    );
    let (status, shell) = server.request("GET", "/__archguard/compare", None);
    assert_eq!(status, 200, "{shell}");
    assert!(
        shell.contains("compare-from")
            && shell.contains("compare-to")
            && shell.contains("comparison-frame")
    );
    let (status, html) = server.request(
        "GET",
        "/__archguard/diff?from=refs%2Fheads%2Fmain&to=refs%2Fheads%2Ffeature%2Fpackages",
        None,
    );
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("diff-level") && html.contains("diff-cycles"));
    let data = html
        .split_once("<script id=\"architecture-diff-data\" type=\"application/json\">")
        .unwrap()
        .1
        .split_once("</script>")
        .unwrap()
        .0;
    let diff: Value = serde_json::from_str(data).unwrap();
    assert_eq!(diff["target_ref"], "refs/heads/feature/packages");
    assert_eq!(diff["added_edges"].as_array().unwrap().len(), 1);
    assert!(!data.contains("dirty"));
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    let (status, scoped) = server.request(
        "GET",
        "/__archguard/diff?from=main&to=feature%2Fpackages&project=one",
        None,
    );
    assert_eq!(status, 200, "{scoped}");
    let scoped_data = scoped
        .split_once("<script id=\"architecture-diff-data\" type=\"application/json\">")
        .unwrap()
        .1
        .split_once("</script>")
        .unwrap()
        .0;
    let scoped_diff: Value = serde_json::from_str(scoped_data).unwrap();
    assert!(
        scoped_diff["base_graph"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "A.ts")
    );
    assert!(
        !scoped_diff["target_graph"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "two/B.ts" || n["name"] == "B.ts")
    );
    for query in [
        "from=main",
        "to=main",
        "from=main&to=main&to=HEAD",
        "from=main&to=%00",
        "from=%xx&to=main",
        "from=main&to=main&file=../../other",
        "from=main&to=main&project=..%2Foutside",
    ] {
        assert_eq!(
            server
                .request("GET", &format!("/__archguard/diff?{query}"), None)
                .0,
            400,
            "{query}"
        );
    }
    assert_eq!(
        server
            .request("GET", "/__archguard/diff?from=main&to=missing", None)
            .0,
        422
    );
    assert_eq!(
        server
            .request("GET", "/__archguard/refs", Some("external.invalid"))
            .0,
        403
    );
    assert_eq!(server.request("POST", "/__archguard/compare", None).0, 405);
    server.version();
    // Comparison endpoints remain available, but the viewer no longer links this UI.
    assert!(
        !server
            .request("GET", "/", None)
            .1
            .contains("/__archguard/compare")
    );
}

// ---- Removed review dashboard endpoints ----------------------------------

#[test]
fn serve_has_no_review_dashboard_or_decision_endpoints() {
    // The viewer's Review Dashboard tab is gone; decisions are recorded
    // through `archguard review-decision` and the MCP tools only.
    let server = Server::start();
    let (status, _) = server.request("GET", "/api/review-dashboard", None);
    assert_eq!(status, 404);
    let decision = serde_json::json!({
        "id": "d1", "finding_ref": "cycle-ab", "decision": "defer",
        "note": "served", "author": "e2e", "ts": ""
    })
    .to_string();
    let (status, _) = server.post_json("/api/decisions", &decision);
    assert_eq!(status, 405);
    assert!(!server.root.path().join(".archguard/reviews.json").exists());
    assert!(
        !server
            .request("GET", "/", None)
            .1
            .contains("/api/review-dashboard")
    );
}
