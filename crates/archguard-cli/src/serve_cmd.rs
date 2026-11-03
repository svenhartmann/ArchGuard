//! Local live viewer. Only the generated report and a version probe are served;
//! project files are never exposed as static HTTP paths. The browser polls a
//! published snapshot while one background worker checks sources and config.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use archguard_engine::{Level, analyze_run_quiet, load_config};

const MAX_HEADER_BYTES: usize = 16 * 1024;
/// R41: bound for `POST /api/decisions` bodies; a decision record is far
/// smaller, so exceeding the bound is abuse or a client bug, never intent.
const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_CACHED_PROJECTS: usize = 8;
const MAX_PAGE_AGE: Duration = Duration::from_secs(60);
const MAX_INPUT_BYTES: u64 = 3 * 1024 * 1024;
const MIN_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const HTTP_WORKERS: usize = 4;
const RELOAD_JS: &str = include_str!("serve_reload.js");
// This internal cache key cannot be supplied through a project path.
const WORKSPACE_KEY: &str = "\0workspace";

#[derive(Default, Clone)]
struct Page {
    html: Arc<str>,
    revision: String,
    warnings: Vec<String>,
    rendered: Option<(u64, Instant)>,
    inputs: Option<u64>,
    scheduled: Option<u64>,
    completed: u64,
    checked: Option<Instant>,
    check_interval: Duration,
}

impl Page {
    fn refresh(
        &mut self,
        scope: &Scope,
        config: Option<&Path>,
        depth: u32,
        services: &archguard_engine::core::ServiceTopology,
    ) {
        let root = scope.root.as_path();
        let result = load_config(config, root).and_then(|mut loaded| {
            // A selected parent owns its own source tree, not nested projects.
            // These are request-local bounds; user configuration is not rewritten.
            for directory in &scope.exclude_dirs {
                let escaped: String = directory
                    .chars()
                    .flat_map(|c| {
                        if "?*[]{}\\".contains(c) {
                            vec!['\\', c]
                        } else {
                            vec![c]
                        }
                    })
                    .collect();
                loaded.config.project.exclude.push(escaped.clone());
                loaded.config.project.exclude.push(format!("{escaped}/**"));
            }
            if scope.overview {
                let security = archguard_engine::read_security(root, &loaded.config);
                let workspace = archguard_engine::analyze_workspace_with_topology(root, services)?;
                let mut hash = DefaultHasher::new();
                serde_json::to_vec(&(
                    &loaded.config,
                    &loaded.sources,
                    services,
                    &security,
                    &scope.repository,
                    &workspace,
                    archguard_engine::api_contracts_fingerprint(root),
                    archguard_engine::pattern_inputs_fingerprint(root, &loaded.config),
                ))
                .expect("repository inputs serialize")
                .hash(&mut hash);
                let key = hash.finish();
                if self
                    .rendered
                    .is_some_and(|(previous, at)| previous == key && at.elapsed() < MAX_PAGE_AGE)
                {
                    return Ok(None);
                }
                let html = crate::review_cmd::repository_html(
                    root,
                    &loaded,
                    services,
                    &security,
                    scope.repository.as_ref().expect("overview context"),
                    &workspace,
                )?;
                let mut warnings = Vec::new();
                if let Err(error) = crate::project_files::write_report(root, "services.html", &html)
                {
                    warnings.push(format!("could not save repository overview: {error}"));
                }
                if let Err(error) = archguard_engine::write_project_file(
                    root,
                    Path::new("services.json"),
                    &serde_json::to_vec_pretty(services).expect("services serialize"),
                    true,
                ) {
                    warnings.push(format!("could not save repository topology: {error}"));
                }
                if let Err(error) = crate::project_files::write_effective_config(root, &loaded) {
                    warnings.push(format!("could not save effective project config: {error}"));
                }
                self.rendered = Some((key, Instant::now()));
                return Ok(Some((html, warnings)));
            }
            let input_key = source_inputs(scope, depth, &loaded, services)?;
            if self.inputs == Some(input_key)
                && self
                    .rendered
                    .is_some_and(|(_, at)| at.elapsed() < MAX_PAGE_AGE)
            {
                return Ok(None);
            }
            let run = analyze_run_quiet(root, &loaded, Level::Module)?;
            let security = archguard_engine::read_security(root, &run.config);
            let key = page_inputs(
                root,
                depth,
                &run,
                services,
                &security,
                scope.repository.as_ref(),
            );
            if run.cache_hit
                && self
                    .rendered
                    .is_some_and(|(previous, at)| previous == key && at.elapsed() < MAX_PAGE_AGE)
            {
                self.inputs = Some(input_key);
                return Ok(None);
            }
            let findings: Vec<_> = crate::cycle_findings(&run.report).collect();
            let cyclic = crate::cyclic_edges(&run.graph, &findings);
            let effective = run.effective_config();
            let html = crate::review_cmd::html_with_services(
                &run.graph,
                &effective,
                None,
                depth,
                &cyclic,
                &run.report.findings,
                None,
                None,
                services,
                &security,
                scope.repository.as_ref(),
            )?;
            let mut warnings = Vec::new();
            if let Err(error) =
                crate::project_files::write_project_graph(root, &run.graph, Some(&html))
            {
                warnings.push(format!("could not save project graph: {error}"));
            }
            if let Err(error) = crate::project_files::write_effective_config(root, &effective) {
                warnings.push(format!("could not save effective project config: {error}"));
            }
            warnings.extend(run.warnings);
            self.rendered = Some((key, Instant::now()));
            self.inputs = Some(input_key);
            Ok(Some((html, warnings)))
        });
        let (html, warnings) = match result {
            Ok(Some(page)) => page,
            Ok(None) => return,
            Err(error) => {
                self.rendered = None;
                let message = error.to_string();
                (error_html(&message), vec![message])
            }
        };
        self.publish(&scope.root, scope.project.as_deref(), html, warnings);
    }

    fn publish(&mut self, root: &Path, project: Option<&str>, html: String, warnings: Vec<String>) {
        if warnings != self.warnings {
            for warning in warnings.iter().take(20) {
                eprintln!("warning: {warning}");
            }
            if warnings.len() > 20 {
                eprintln!(
                    "warning: ...and {} more unresolved imports",
                    warnings.len() - 20
                );
            }
            self.warnings = warnings;
        }
        if html != self.html.as_ref() {
            // Content identity also detects edits made while this process was
            // stopped. A counter restarted at 1 could leave an open page stale.
            let mut hash = DefaultHasher::new();
            root.hash(&mut hash);
            project.hash(&mut hash);
            html.hash(&mut hash);
            self.revision = format!("{:016x}", hash.finish());
            self.html = html.into();
        }
    }

    fn live_script(&self, project: Option<&str>) -> String {
        let version_path = match project {
            Some(WORKSPACE_KEY) => "/__archguard/version?workspace=sources".to_owned(),
            Some(project) => format!("/__archguard/version?project={}", encode_project(project)),
            None => "/__archguard/version".to_owned(),
        };
        RELOAD_JS
            .replace(
                "__ARCHGUARD_VERSION__",
                &serde_json::to_string(&self.revision).expect("version serializes"),
            )
            .replace(
                "__ARCHGUARD_VERSION_PATH__",
                &serde_json::to_string(&version_path).expect("version URL serializes"),
            )
    }
}

fn error_html(message: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>ArchGuard — analysis error</title><body style=\"font:16px system-ui;max-width:800px;margin:60px auto;padding:24px\"><h1>The project could not be analyzed</h1><pre style=\"white-space:pre-wrap\">{}</pre><p>Correct the source configuration. This page updates automatically after the fix.</p><p id=\"feedback\" role=\"status\"></p></body></html>",
        message
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    )
}

const LOADING_HTML: &str = "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>ArchGuard — Loading</title><body style=\"font:16px system-ui;max-width:800px;margin:60px auto;padding:24px;background:#0f1722;color:#e4edf7\"><h1>ArchGuard</h1><p id=\"feedback\" role=\"status\" aria-live=\"polite\">Preparing the graph in the background… / Graph wird im Hintergrund vorbereitet…</p><p>This page opens automatically when the analysis is ready.<br>Die Ansicht öffnet sich automatisch, sobald die Analyse bereit ist.</p></body></html>";

/// A content-based preflight avoids repeating metrics, semantic projection and
/// HTML generation for unchanged inputs. Relative Git windows still expire.
fn source_inputs(
    scope: &Scope,
    depth: u32,
    loaded: &archguard_engine::LoadedConfig,
    services: &archguard_engine::core::ServiceTopology,
) -> Result<u64, archguard_engine::core::ArchguardError> {
    let root = &scope.root;
    let mut hash = DefaultHasher::new();
    serde_json::to_vec(&(depth, &loaded.config, &loaded.sources))
        .expect("source inputs serialize")
        .hash(&mut hash);
    archguard_engine::source_fingerprint(root, &loaded.config)?.hash(&mut hash);
    if loaded.sources.get("project") != Some(&archguard_engine::core::ConfigSource::Declared) {
        // Match effective.rs derivation, which also sees excluded nested source
        // directories when deciding roots/languages. Its output is an input too.
        archguard_engine::derive_project_hints(root)?.hash(&mut hash);
    }
    // PHP autoload/source-root derivation depends on this non-source manifest.
    match fs::read(root.join("composer.json")) {
        Ok(bytes) => Some(bytes).hash(&mut hash),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None::<Vec<u8>>.hash(&mut hash),
        Err(error) => return Err(error.into()),
    }
    let security = archguard_engine::read_security(root, &loaded.config);
    context_inputs(
        root,
        &loaded.config,
        services,
        &security,
        scope.repository.as_ref(),
        &mut hash,
    );
    Ok(hash.finish())
}

fn page_inputs(
    root: &Path,
    depth: u32,
    run: &archguard_engine::AnalysisRun,
    services: &archguard_engine::core::ServiceTopology,
    security: &archguard_engine::SecurityReport,
    repository: Option<&serde_json::Value>,
) -> u64 {
    let mut hash = DefaultHasher::new();
    serde_json::to_vec(&(depth, &run.graph, &run.config, &run.report, &run.warnings))
        .expect("analysis inputs serialize")
        .hash(&mut hash);
    context_inputs(root, &run.config, services, security, repository, &mut hash);
    hash.finish()
}

fn context_inputs(
    root: &Path,
    config: &archguard_engine::core::Config,
    services: &archguard_engine::core::ServiceTopology,
    security: &archguard_engine::SecurityReport,
    repository: Option<&serde_json::Value>,
    hash: &mut DefaultHasher,
) {
    // Compose files live outside language extraction roots. Hash the complete
    // evidence model so new/deleted services and endpoint edits reload too.
    serde_json::to_vec(services)
        .expect("service evidence serializes")
        .hash(hash);
    // Explicit security scans can change database findings without source edits.
    serde_json::to_vec(security)
        .expect("security snapshot serializes")
        .hash(hash);
    serde_json::to_vec(&repository)
        .expect("repository context serializes")
        .hash(hash);
    let workspace_root = repository
        .and_then(|r| r["root"].as_str())
        .map(Path::new)
        .unwrap_or(root);
    match archguard_engine::analyze_workspace(workspace_root) {
        Ok(workspace) => serde_json::to_vec(&workspace)
            .expect("workspace serializes")
            .hash(hash),
        Err(error) => error.to_string().hash(hash),
    }
    archguard_engine::semantic_fingerprints(root, config).hash(hash);
    archguard_engine::pattern_inputs_fingerprint(root, config).hash(hash);
    // Importing a new test report must refresh priorities without a source edit.
    archguard_engine::test_coverage_fingerprint(root).hash(hash);
    archguard_engine::quality_findings_fingerprint(root).hash(hash);
    archguard_engine::quality_history_fingerprint(root).hash(hash);
    archguard_engine::test_evidence_fingerprint(root).hash(hash);
    archguard_engine::measurement_runs_fingerprint(root).hash(hash);
    archguard_engine::mutation_fingerprint(root).hash(hash);
    archguard_engine::duplication_fingerprint(root).hash(hash);
    archguard_engine::api_contracts_fingerprint(root).hash(hash);
    let review_now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    archguard_engine::review_session_fingerprint(root, review_now).hash(hash);
    let head = git_state(root, &["rev-parse", "--verify", "HEAD"]);
    let shallow = git_state(root, &["rev-parse", "--is-shallow-repository"]);
    head.hash(hash);
    shallow.hash(hash);
    if shallow.as_deref() == Some(b"true\n") {
        // A fetch can deepen a still-shallow repository without changing HEAD.
        if let Some(path) = git_path(root, &["rev-parse", "--git-path", "shallow"]) {
            hash_file(&path, hash);
        }
    }
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let repository = git_path(root, &["rev-parse", "--show-toplevel"])
        .and_then(|path| path.canonicalize().ok())
        .filter(|path| canonical_root.starts_with(path))
        .unwrap_or(canonical_root);
    repository.hash(hash);
    for name in [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"] {
        hash_file(&repository.join(name), hash);
    }
}

fn git_state(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn git_path(root: &Path, args: &[&str]) -> Option<PathBuf> {
    let value = String::from_utf8(git_state(root, args)?).ok()?;
    // Only remove Git's line terminator; whitespace may belong to a real path.
    Some(root.join(value.strip_suffix('\n').unwrap_or(&value)))
}

fn hash_file(path: &Path, hash: &mut DefaultHasher) {
    path.hash(hash);
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            metadata.is_file().hash(hash);
            metadata.file_type().is_symlink().hash(hash);
            metadata.len().hash(hash);
            metadata.modified().ok().hash(hash);
            if metadata.is_file() {
                // Match the bounded CODEOWNERS reader; ignore symlink entries.
                let contents = fs::File::open(path).and_then(|file| {
                    let mut bytes = Vec::new();
                    file.take(MAX_INPUT_BYTES).read_to_end(&mut bytes)?;
                    Ok(bytes)
                });
                match contents {
                    Ok(bytes) => bytes.hash(hash),
                    Err(error) => error.kind().hash(hash),
                }
            }
        }
        Err(error) => error.kind().hash(hash),
    }
}

#[derive(Default)]
struct RefreshState {
    pages: PageCache,
    jobs: VecDeque<(String, u64)>,
    ticket: u64,
    stopping: bool,
}

/// One analysis worker publishes complete snapshots. HTTP handlers only clone
/// the Arc-backed HTML; neither analysis nor socket writes hold this mutex.
struct BackgroundPages {
    shared: Arc<(Mutex<RefreshState>, Condvar)>,
}

impl BackgroundPages {
    fn new(root: PathBuf, config: Option<PathBuf>, depth: u32) -> io::Result<Self> {
        let shared = Arc::new((Mutex::new(RefreshState::default()), Condvar::new()));
        let worker = Arc::clone(&shared);
        thread::Builder::new().name("archguard-analysis".into()).spawn(move || {
            loop {
                let (key, ticket, mut page) = {
                    let (lock, changed) = &*worker;
                    let mut state = lock.lock().expect("refresh state");
                    while state.jobs.is_empty() && !state.stopping {
                        state = changed.wait(state).expect("refresh state");
                    }
                    if state.stopping { return; }
                    let (key, ticket) = state.jobs.pop_front().expect("queued refresh");
                    let Some((page, _)) = state.pages.pages.get(&key) else { continue };
                    (key, ticket, page.clone())
                };
                let selected = (!key.is_empty()).then_some(key.as_str());
                    let started = Instant::now();
                // Revalidate manifests and every directory component after a
                // queued request: a cached page never authorizes a new scope.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    refresh_project(&mut page, &root, config.as_deref(), depth, selected)
                }));
                let error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error),
                    Err(_) => Some("The background analysis stopped unexpectedly. Retrying on the next source check.".into()),
                };
                if let Some(error) = error {
                    page.rendered = None;
                    page.publish(&root, selected, error_html(&error), vec![error]);
                }
                page.scheduled = None;
                page.completed = ticket;
                page.checked = Some(Instant::now());
                    // HTTP probes are cheap now; pace the actual analysis by
                    // its cost so frequent polling cannot keep a slow scan busy.
                    page.check_interval = started.elapsed().saturating_mul(2)
                        .clamp(MIN_CHECK_INTERVAL, Duration::from_secs(10));
                let (lock, changed) = &*worker;
                let mut state = lock.lock().expect("refresh state");
                if let Some((current, _)) = state.pages.pages.get_mut(&key)
                    && current.scheduled == Some(ticket)
                {
                    *current = page;
                }
                changed.notify_all();
            }
        })?;
        Ok(Self { shared })
    }

    fn snapshot(&self, project: Option<&str>, wait: bool) -> io::Result<Page> {
        self.snapshot_until(project, wait, Instant::now() + Duration::from_secs(120))
    }

    fn snapshot_until(
        &self,
        project: Option<&str>,
        wait: bool,
        deadline: Instant,
    ) -> io::Result<Page> {
        let key = project.unwrap_or("");
        let (lock, changed) = &*self.shared;
        if wait {
            let mut state = lock.lock().expect("refresh state");
            if state.pages.waiters.values().sum::<usize>() >= HTTP_WORKERS / 2 {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Too many synchronous checks; retry shortly",
                ));
            }
            if !state.pages.available(key) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "All project analysis slots are busy; retry shortly",
                ));
            }
            state.pages.get(key);
            *state.pages.waiters.entry(key.to_owned()).or_default() += 1;
        }
        let result = (|| {
            let mut state = lock.lock().expect("refresh state");
            // The explicit automation endpoint waits for a check started after its
            // request. Browser status probes never wait or queue duplicate work.
            if wait {
                while state
                    .pages
                    .pages
                    .get(key)
                    .is_some_and(|(page, _)| page.scheduled.is_some())
                {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Analysis is still running",
                        ));
                    }
                    state = changed
                        .wait_timeout(state, remaining)
                        .expect("refresh state")
                        .0;
                }
            }
            if !state.pages.available(key) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "All project analysis slots are busy; retry shortly",
                ));
            }
            let page = state.pages.get(key);
            let schedule = page.scheduled.is_none()
                && (wait
                    || page.checked.is_none_or(|at| {
                        at.elapsed() >= page.check_interval.max(MIN_CHECK_INTERVAL)
                    }));
            let ticket = if schedule {
                state.ticket += 1;
                let ticket = state.ticket;
                state.pages.get(key).scheduled = Some(ticket);
                state.jobs.push_back((key.to_owned(), ticket));
                changed.notify_all();
                Some(ticket)
            } else {
                None
            };
            if wait && let Some(ticket) = ticket {
                loop {
                    if let Some((page, _)) = state.pages.pages.get(key)
                        && page.completed >= ticket
                    {
                        return Ok(page.clone());
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Analysis is still running",
                        ));
                    }
                    state = changed
                        .wait_timeout(state, remaining)
                        .expect("refresh state")
                        .0;
                }
            }
            Ok(state.pages.get(key).clone())
        })();
        // A completed job is no longer scheduled, but its result must survive
        // cache pressure until every waiting check copied it. Clean up on both
        // success and timeout, after the closure released its mutex guard.
        if wait {
            let mut state = lock.lock().expect("refresh state");
            let count = state.pages.waiters.get_mut(key).expect("pinned check");
            *count -= 1;
            if *count == 0 {
                state.pages.waiters.remove(key);
            }
        }
        result
    }
}

impl Drop for BackgroundPages {
    fn drop(&mut self) {
        let (lock, changed) = &*self.shared;
        lock.lock().expect("refresh state").stopping = true;
        changed.notify_all();
    }
}

fn refresh_project(
    page: &mut Page,
    root: &Path,
    config: Option<&Path>,
    depth: u32,
    selected: Option<&str>,
) -> Result<(), String> {
    let topology = archguard_engine::analyze_services(root).map_err(|error| error.to_string())?;
    if selected == Some(WORKSPACE_KEY) {
        let workspace = archguard_engine::analyze_workspace_with_topology(root, &topology)
            .map_err(|e| e.to_string())?;
        let fingerprint = archguard_engine::workspace_sources_fingerprint(root, &workspace)
            .map_err(|e| e.to_string())?;
        let mut hash = DefaultHasher::new();
        fingerprint.hash(&mut hash);
        serde_json::to_vec(&topology)
            .expect("topology serializes")
            .hash(&mut hash);
        archguard_engine::api_contracts_fingerprint(root).hash(&mut hash);
        let key = hash.finish();
        if page.inputs == Some(key) && page.rendered.is_some() {
            return Ok(());
        }
        let sources = archguard_engine::analyze_workspace_sources(root, &workspace)
            .map_err(|e| e.to_string())?;
        let html = crate::workspace_cmd::html(root, &workspace, Some(&sources), None)
            .map_err(|e| e.to_string())?;
        let mut warnings = Vec::new();
        if let Err(error) = crate::project_files::write_report(root, "workspace.html", &html) {
            warnings.push(format!("could not save workspace report: {error}"));
        }
        page.publish(root, selected, html, warnings);
        page.inputs = Some(key);
        page.rendered = Some((key, Instant::now()));
        return Ok(());
    }
    let scope = resolve_scope(root, selected, &topology)
        .ok_or_else(|| "Project is not available in this repository".to_owned())?;
    let services = if scope.project.is_some() && scope.root != root {
        archguard_engine::analyze_services(&scope.root).map_err(|error| error.to_string())?
    } else {
        topology
    };
    page.refresh(
        &scope,
        if selected.is_some() { None } else { config },
        depth,
        &services,
    );
    Ok(())
}

/// Run until interrupted (Ctrl-C). Loopback is deliberate, not configurable.
pub fn run(root: &Path, config: Option<&Path>, port: u16, depth: u32) -> io::Result<()> {
    if !root.is_dir() {
        return Err(io::Error::other("serve requires a project directory"));
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    let address = listener.local_addr()?;
    let allowed_hosts = [address.to_string(), format!("localhost:{}", address.port())];
    println!("http://{address}/");
    io::stdout().flush()?;
    eprintln!(
        "Serving {} · live updates while visible, adaptive polling · Ctrl-C to stop",
        root.display()
    );
    let pages = BackgroundPages::new(root.to_path_buf(), config.map(Path::to_path_buf), depth)?;
    let (send, receive) = mpsc::sync_channel::<TcpStream>(HTTP_WORKERS * 2);
    let receive = Mutex::new(receive);
    thread::scope(|threads| {
        for _ in 0..HTTP_WORKERS {
            let receive = &receive;
            let pages = &pages;
            let allowed_hosts = &allowed_hosts;
            threads.spawn(move || {
                loop {
                    let next = receive.lock().expect("request queue").recv();
                    let Ok(mut stream) = next else { break };
                    let _ = handle(&mut stream, allowed_hosts, pages, root, config);
                }
            });
        }
        for connection in listener.incoming() {
            let stream = connection?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            stream.set_write_timeout(Some(Duration::from_secs(10)))?;
            if let Err(mpsc::TrySendError::Full(mut stream)) = send.try_send(stream) {
                let _ = respond(
                    &mut stream,
                    503,
                    "text/plain",
                    "The local viewer is busy; retry shortly",
                );
            }
        }
        drop(send);
        Ok(())
    })
}

fn handle(
    stream: &mut TcpStream,
    allowed_hosts: &[String; 2],
    pages: &BackgroundPages,
    root: &Path,
    config: Option<&Path>,
) -> io::Result<()> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > MAX_HEADER_BYTES {
            return respond(stream, 431, "text/plain", "Request headers too large");
        }
    }
    // The read may already carry a body cut inside a multi-byte character.
    let header_end = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(bytes.len());
    let Ok(headers) = std::str::from_utf8(&bytes[..header_end]) else {
        return respond(stream, 400, "text/plain", "Invalid request");
    };
    let mut lines = headers.split("\r\n");
    let request: Vec<_> = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    if request.len() != 3 || !matches!(request[2], "HTTP/1.0" | "HTTP/1.1") {
        return respond(stream, 400, "text/plain", "Invalid request");
    }
    let hosts: Vec<_> = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect();
    if hosts.len() != 1
        || !allowed_hosts
            .iter()
            .any(|host| host.eq_ignore_ascii_case(hosts[0]))
    {
        return respond(
            stream,
            403,
            "text/plain",
            "Use the local viewer URL printed by archguard",
        );
    }
    let (path, query) = request[1]
        .split_once('?')
        .map_or((request[1], None), |(path, query)| (path, Some(query)));
    // R41: the dashboard endpoints. POST reaches only the decisions route;
    // everything else keeps the old GET-only contract.
    if request[0] == "POST" {
        if path == "/api/decisions" && query.is_none() {
            let body_prefix = bytes.get(header_end + 4..).unwrap_or_default();
            return handle_decision_post(stream, headers, hosts[0], body_prefix, root, config);
        }
        return respond(
            stream,
            405,
            "text/plain",
            "Only GET and POST /api/decisions are supported",
        );
    }
    if request[0] != "GET" {
        return respond(stream, 405, "text/plain", "Only GET is supported");
    }
    if matches!(
        path,
        "/__archguard/compare" | "/__archguard/refs" | "/__archguard/diff"
    ) {
        return handle_comparison(stream, path, query, root, config);
    }
    if path == "/api/review-dashboard" && query.is_none() {
        return handle_review_dashboard(stream, root, config);
    }
    if !matches!(path, "/" | "/__archguard/version" | "/__archguard/check") {
        return respond(stream, 404, "text/plain", "Not found");
    }
    let selected = match parse_project_query(query) {
        Ok(selected) => selected,
        Err(()) => return respond(stream, 400, "text/plain", "Invalid project selection"),
    };
    if selected.is_some() && selected.as_deref() != Some(WORKSPACE_KEY) {
        let topology = match archguard_engine::analyze_services(root) {
            Ok(topology) => topology,
            Err(error) => {
                return respond(
                    stream,
                    500,
                    "text/plain",
                    &format!("Project discovery failed: {error}"),
                );
            }
        };
        if resolve_scope(root, selected.as_deref(), &topology).is_none() {
            return respond(
                stream,
                404,
                "text/plain",
                "Project is not available in this repository",
            );
        }
    }
    let wait = path == "/__archguard/check";
    let page = match pages.snapshot(selected.as_deref(), wait) {
        Ok(page) => page,
        Err(error) => return respond(stream, 503, "text/plain", &error.to_string()),
    };
    if path == "/" {
        let loading = page.revision.is_empty();
        let html = if loading { LOADING_HTML } else { &page.html };
        let script = format!("<script>{}</script>", page.live_script(selected.as_deref()));
        let (prefix, suffix) = html.split_once("</body>").unwrap_or((html, ""));
        let closing = if html.contains("</body>") {
            "</body>"
        } else {
            ""
        };
        respond_parts(
            stream,
            if loading { 202 } else { 200 },
            "text/html; charset=utf-8",
            &[prefix, &script, closing, suffix],
        )
    } else {
        let body = if wait {
            serde_json::json!({"version":page.revision})
        } else {
            serde_json::json!({"version":page.revision,"refreshing":page.scheduled.is_some(),
                "status":if page.revision.is_empty() {"loading"} else if page.scheduled.is_some() {"refreshing"} else {"ready"}})
        };
        respond(stream, 200, "application/json", &body.to_string())
    }
}

/// R41: `GET /api/review-dashboard` — the live dashboard payload the
/// viewer's `loadDashboardPayload` fetches: the KPI block of a fresh
/// module-level run plus the persisted decision register. A corrupt
/// register is a 500, never a silently empty register.
fn handle_review_dashboard(
    stream: &mut TcpStream,
    root: &Path,
    config: Option<&Path>,
) -> io::Result<()> {
    let loaded = match load_config(config, root) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("serve: review dashboard config failed: {error}");
            return respond(
                stream,
                500,
                "application/json",
                &json_error(format!("config: {error}")),
            );
        }
    };
    let run = match analyze_run_quiet(root, &loaded, Level::Module) {
        Ok(run) => run,
        Err(error) => {
            eprintln!("serve: review dashboard analysis failed: {error}");
            return respond(
                stream,
                500,
                "application/json",
                &json_error(format!("analysis: {error}")),
            );
        }
    };
    let review_kpis = run
        .report
        .review_kpis
        .clone()
        .or_else(|| run.graph.review_kpis.clone());
    let register = match archguard_engine::load_register(root) {
        Ok(register) => Some(register),
        Err(error) => {
            eprintln!("serve: review dashboard register failed: {error}");
            return respond(
                stream,
                500,
                "application/json",
                &json_error(format!("register: {error}")),
            );
        }
    };
    let body = serde_json::json!({"review_kpis": review_kpis, "register": register});
    respond(stream, 200, "application/json", &body.to_string())
}

fn json_error(message: String) -> String {
    serde_json::json!({ "error": message }).to_string()
}

/// The Host check stops DNS rebinding, not a page on another origin posting to
/// loopback. A foreign `Origin` is refused; requiring JSON forces browsers into
/// a preflight this server never answers, also for clients that omit `Origin`.
fn cross_site_rejection(headers: &str, host: &str) -> Option<(u16, &'static str)> {
    let values = |wanted: &'static str| {
        headers
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .filter(move |(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value.trim())
    };
    let same_origin = format!("http://{host}");
    if values("origin").any(|origin| !origin.eq_ignore_ascii_case(&same_origin)) {
        return Some((403, "Cross-origin requests are not accepted"));
    }
    let mut types = values("content-type");
    let json = types.next().is_some_and(|value| {
        value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("application/json")
    });
    if !json || types.next().is_some() {
        return Some((415, "Decision body must be application/json"));
    }
    None
}

/// R41: `POST /api/decisions` — upsert one decision through the engine
/// (atomic tmp+rename, R19). Any failure goes to stderr as a 500 and the
/// register on disk stays byte-identical: a corrupt register is never
/// overwritten, a rejected body never reaches the writer.
fn handle_decision_post(
    stream: &mut TcpStream,
    headers: &str,
    host: &str,
    body_prefix: &[u8],
    root: &Path,
    config: Option<&Path>,
) -> io::Result<()> {
    if let Some((status, message)) = cross_site_rejection(headers, host) {
        return respond(stream, status, "text/plain", message);
    }
    let content_length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>())
        .collect::<Result<Vec<_>, _>>();
    let content_length = match content_length {
        Ok(lengths) if lengths.len() == 1 => lengths[0],
        _ => {
            return respond(
                stream,
                400,
                "text/plain",
                "Invalid or duplicate Content-Length",
            );
        }
    };
    if content_length > MAX_BODY_BYTES {
        return respond(stream, 413, "text/plain", "Decision body too large");
    }
    let mut body = body_prefix.to_vec();
    let mut buffer = [0; 4096];
    while body.len() < content_length {
        let count = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        body.extend_from_slice(&buffer[..count]);
    }
    if body.len() != content_length {
        return respond(
            stream,
            400,
            "text/plain",
            "Request body shorter than Content-Length",
        );
    }
    let Ok(text) = std::str::from_utf8(&body) else {
        return respond(
            stream,
            400,
            "text/plain",
            "Decision body is not valid UTF-8",
        );
    };
    let decision = match serde_json::from_str::<archguard_engine::core::ReviewDecision>(text) {
        Ok(decision) => decision,
        Err(error) => {
            return respond(
                stream,
                400,
                "text/plain",
                &format!("Invalid decision record: {error}"),
            );
        }
    };
    if decision.id.trim().is_empty()
        || decision.finding_ref.trim().is_empty()
        || decision.note.trim().is_empty()
        || decision.author.trim().is_empty()
    {
        return respond(
            stream,
            400,
            "text/plain",
            "Decision needs a non-empty id, finding_ref, note and author",
        );
    }
    let decision = if decision.ts.trim().is_empty() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        archguard_engine::core::ReviewDecision {
            ts: crate::review_session_cmd::iso_from_unix(now),
            ..decision
        }
    } else {
        decision
    };
    // A second write path must not bypass the ADR gate the CLI (R31) and
    // MCP (R34) enforce: accept-risk without an ADR is a 400 while the
    // config requires one.
    if matches!(
        decision.decision,
        archguard_engine::core::DecisionKind::AcceptRisk
    ) && decision.adr_link.as_deref().is_none_or(str::is_empty)
    {
        match load_config(config, root) {
            Ok(loaded) if loaded.config.review.adr_required_on_accept_risk => {
                return respond(
                    stream,
                    400,
                    "text/plain",
                    "accept_risk requires an adr_link ([review] adr_required_on_accept_risk is set)",
                );
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("serve: decision gate config failed: {error}");
                return respond(stream, 500, "text/plain", &format!("Config error: {error}"));
            }
        }
    }
    match archguard_engine::upsert_decision(root, decision) {
        Ok(register) => {
            let body = serde_json::json!({"status": "ok", "register": register});
            respond(stream, 200, "application/json", &body.to_string())
        }
        Err(error) => {
            eprintln!("serve: decision not saved: {error}");
            respond(
                stream,
                500,
                "application/json",
                &json_error(format!("decision not saved: {error}")),
            )
        }
    }
}

fn handle_comparison(
    stream: &mut TcpStream,
    path: &str,
    query: Option<&str>,
    root: &Path,
    config: Option<&Path>,
) -> io::Result<()> {
    let mut values = std::collections::BTreeMap::new();
    if let Some(query) = query {
        for part in query.split('&') {
            let Some((key, value)) = part.split_once('=') else {
                return respond(stream, 400, "text/plain", "Invalid comparison query");
            };
            let Ok(value) = decode_query_value(value) else {
                return respond(stream, 400, "text/plain", "Invalid comparison query");
            };
            if !matches!(key, "from" | "to" | "project")
                || value.is_empty()
                || value.len() > 1024
                || values.insert(key, value).is_some()
            {
                return respond(stream, 400, "text/plain", "Invalid comparison query");
            }
        }
    }
    let from = values.get("from");
    let to = values.get("to");
    if from.is_some() != to.is_some() || (path != "/__archguard/diff" && from.is_some()) {
        return respond(stream, 400, "text/plain", "Choose both branches");
    }
    let selected = values.get("project");
    if selected.is_some_and(|p| !valid_project_path(p)) {
        return respond(stream, 400, "text/plain", "Invalid project selection");
    }
    let scoped;
    let (root, config) = if let Some(selected) = selected {
        let topology = match archguard_engine::analyze_services(root) {
            Ok(topology) => topology,
            Err(error) => {
                return respond(stream, 422, "text/plain; charset=utf-8", &error.to_string());
            }
        };
        let Some(scope) = resolve_scope(root, Some(selected), &topology) else {
            return respond(
                stream,
                404,
                "text/plain",
                "Project is not available in this repository",
            );
        };
        scoped = scope.root;
        (scoped.as_path(), None)
    } else {
        (root, config)
    };
    if path == "/__archguard/compare" {
        let html = include_str!("viewer_git_compare.html")
            .replace("/*__APP__*/", include_str!("viewer_git_compare.js"));
        return respond(stream, 200, "text/html; charset=utf-8", &html);
    }
    if path == "/__archguard/refs" {
        return match archguard_engine::list_git_branches(root) {
            Ok(refs) => respond(
                stream,
                200,
                "application/json",
                &serde_json::to_string(&refs).expect("refs serialize"),
            ),
            Err(error) => respond(stream, 422, "text/plain; charset=utf-8", &error.to_string()),
        };
    }
    let result = if let (Some(from), Some(to)) = (from, to) {
        archguard_engine::compare_branches(root, from, to, config)
            .map(|d| archguard_engine::report::branch_diff_html(&d))
    } else {
        archguard_engine::compare_staged(root, "HEAD", config)
            .map(|d| archguard_engine::report::staged_diff_html(&d))
    };
    match result {
        Ok(html) => respond(stream, 200, "text/html; charset=utf-8", &html),
        Err(error) => respond(stream, 422, "text/plain; charset=utf-8", &error.to_string()),
    }
}

/// A request is restricted to a discovered manifest-defined project.
struct Scope {
    root: PathBuf,
    project: Option<String>,
    repository: Option<serde_json::Value>,
    overview: bool,
    exclude_dirs: Vec<String>,
}

fn resolve_scope(
    root: &Path,
    selected: Option<&str>,
    topology: &archguard_engine::core::ServiceTopology,
) -> Option<Scope> {
    let projects = topology
        .projects
        .iter()
        .filter(|project| project.kind != "workspace")
        .map(|project| project.path.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let overview = selected.is_none() && projects.len() >= 2;
    let analysis_root = if let Some(selected) = selected {
        if !topology
            .projects
            .iter()
            .any(|p| p.path == selected && p.kind != "workspace")
        {
            return None;
        }
        let mut target = root.to_path_buf();
        if selected != "." {
            for component in selected.split('/') {
                target.push(component);
                let metadata = fs::symlink_metadata(&target).ok()?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return None;
                }
            }
        }
        let canonical = target.canonicalize().ok()?;
        if !canonical.starts_with(root) {
            return None;
        }
        canonical
    } else {
        root.to_path_buf()
    };
    let mut exclude_dirs = Vec::new();
    if let Some(selected) = selected {
        for project in &topology.projects {
            if project.path == selected {
                continue;
            }
            let relative = if selected == "." {
                Some(project.path.as_str())
            } else {
                project.path.strip_prefix(&format!("{selected}/"))
            };
            if let Some(relative) = relative.filter(|path| !path.is_empty() && *path != ".") {
                exclude_dirs.push(relative.to_owned());
            }
        }
        exclude_dirs.sort();
        exclude_dirs.dedup();
    }
    let repository = (overview || selected.is_some()).then(|| serde_json::json!({
        "mode":if overview {"overview"} else {"project"}, "root":root,
        "active_project":selected, "projects":topology.projects, "excluded_projects":exclude_dirs,
    }));
    Some(Scope {
        root: analysis_root,
        project: selected.map(str::to_owned),
        repository,
        overview,
        exclude_dirs,
    })
}

fn parse_project_query(query: Option<&str>) -> Result<Option<String>, ()> {
    let Some(query) = query else {
        return Ok(None);
    };
    if query == "workspace=sources" {
        return Ok(Some(WORKSPACE_KEY.into()));
    }
    let Some(value) = query.strip_prefix("project=") else {
        return Err(());
    };
    if value.is_empty() || value.contains('&') {
        return Err(());
    }
    let value = decode_query_value(value)?;
    if !valid_project_path(&value) {
        return Err(());
    }
    Ok(Some(value))
}
fn decode_query_value(value: &str) -> Result<String, ()> {
    let mut decoded = Vec::new();
    let mut bytes = value.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        decoded.push(match byte {
            b'%' => {
                let high = (bytes.next().ok_or(())? as char).to_digit(16).ok_or(())?;
                let low = (bytes.next().ok_or(())? as char).to_digit(16).ok_or(())?;
                (high * 16 + low) as u8
            }
            b'+' => b' ',
            other => other,
        });
    }
    let value = String::from_utf8(decoded).map_err(|_| ())?;
    if value.chars().any(|c| c.is_control() || c == '\\') {
        return Err(());
    }
    Ok(value)
}
fn valid_project_path(value: &str) -> bool {
    !value.starts_with('/')
        && value.as_bytes().get(1) != Some(&b':')
        && (value == "." || !value.split('/').any(|part| matches!(part, "" | "." | "..")))
}

#[cfg(test)]
mod workspace_route_tests {
    use super::*;
    #[test]
    fn explicit_workspace_sources_has_separate_route_and_reload_scope() {
        assert_eq!(
            parse_project_query(Some("workspace=sources")),
            Ok(Some("\0workspace".into()))
        );
        for query in [
            "workspace=all",
            "workspace=sources&project=app",
            "project=%00workspace",
        ] {
            assert!(parse_project_query(Some(query)).is_err());
        }
        assert!(
            Page::default()
                .live_script(Some("\0workspace"))
                .contains("/__archguard/version?workspace=sources")
        );
    }
    #[test]
    fn workspace_cache_refreshes_deployment_relations_without_source_changes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"app","version":"1"}"#,
        )
        .unwrap();
        let file = dir.path().join("compose.yaml");
        fs::write(
            &file,
            "services:\n  one:\n    image: one\n  two:\n    image: two\n",
        )
        .unwrap();
        let mut page = Page::default();
        refresh_project(&mut page, dir.path(), None, 0, Some(WORKSPACE_KEY)).unwrap();
        let initial = page.revision.clone();
        page.html = "cached marker".into();
        refresh_project(&mut page, dir.path(), None, 0, Some(WORKSPACE_KEY)).unwrap();
        assert_eq!(page.html.as_ref(), "cached marker");
        fs::write(&file,"services:\n  one:\n    image: one\n    environment:\n      TWO_URL: http://two:8080\n  two:\n    image: two\n").unwrap();
        refresh_project(&mut page, dir.path(), None, 0, Some(WORKSPACE_KEY)).unwrap();
        assert_ne!(
            page.revision, initial,
            "deployment relation changes must invalidate the workspace HTML"
        );
        assert!(page.html.contains("configured_http"));
    }
    #[test]
    fn libraries_get_project_boundaries_and_manifest_edge_changes_invalidate_overview() {
        let dir = tempfile::tempdir().unwrap();
        for path in ["app", "lib"] {
            fs::create_dir(dir.path().join(path)).unwrap();
        }
        fs::write(
            dir.path().join("app/package.json"),
            r#"{"name":"app","version":"1.0.0"}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("lib/package.json"),
            r#"{"name":"lib","version":"1.0.0"}"#,
        )
        .unwrap();
        let topology = archguard_engine::analyze_services(dir.path()).unwrap();
        let scope = resolve_scope(dir.path(), None, &topology).unwrap();
        assert!(
            scope.overview,
            "multiple libraries must not share an unscoped source graph"
        );
        let mut page = Page::default();
        page.refresh(&scope, None, 0, &topology);
        assert!(page.html.contains("archguard-data"));
        page.html = "unchanged marker".into();
        page.refresh(&scope, None, 0, &topology);
        assert_eq!(page.html.as_ref(), "unchanged marker");
        fs::write(
            dir.path().join("app/package.json"),
            r#"{"name":"app","version":"1.0.0","dependencies":{"lib":"file:../lib"}}"#,
        )
        .unwrap();
        let topology = archguard_engine::analyze_services(dir.path()).unwrap();
        page.refresh(&scope, None, 0, &topology);
        assert_ne!(page.html.as_ref(), "unchanged marker");
        assert!(page.html.contains("npm_file"));
    }
}
fn encode_project(project: &str) -> String {
    let mut encoded = String::new();
    for byte in project.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write;
            write!(encoded, "%{byte:02X}").expect("String writing");
        }
    }
    encoded
}

#[derive(Default)]
struct PageCache {
    pages: BTreeMap<String, (Page, u64)>,
    waiters: BTreeMap<String, usize>,
    clock: u64,
}
impl PageCache {
    fn available(&self, project: &str) -> bool {
        self.pages.contains_key(project)
            || self.pages.len() < MAX_CACHED_PROJECTS
            || self
                .pages
                .iter()
                .any(|(key, (page, _))| page.scheduled.is_none() && !self.waiters.contains_key(key))
    }

    fn get(&mut self, project: &str) -> &mut Page {
        assert!(
            self.available(project),
            "busy cache entries cannot be evicted"
        );
        self.clock = self.clock.wrapping_add(1);
        if !self.pages.contains_key(project)
            && self.pages.len() >= MAX_CACHED_PROJECTS
            && let Some(oldest) = self
                .pages
                .iter()
                .filter(|(key, (page, _))| {
                    page.scheduled.is_none() && !self.waiters.contains_key(*key)
                })
                .min_by_key(|(_, (_, at))| at)
                .map(|(key, _)| key.clone())
        {
            self.pages.remove(&oldest);
        }
        let (page, at) = self.pages.entry(project.to_owned()).or_default();
        *at = self.clock;
        page
    }
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) -> io::Result<()> {
    respond_parts(stream, status, content_type, &[body])
}

fn respond_parts(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    parts: &[&str],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        parts.iter().map(|part| part.len()).sum::<usize>()
    )?;
    for part in parts {
        stream.write_all(part.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_status_probes_respect_the_background_analysis_cooldown() {
        let shared = Arc::new((Mutex::new(RefreshState::default()), Condvar::new()));
        {
            let mut state = shared.0.lock().unwrap();
            let page = state.pages.get("");
            page.revision = "completed".into();
            page.checked = Some(Instant::now() - Duration::from_secs(2));
            page.check_interval = Duration::from_secs(6);
        }
        let pages = BackgroundPages { shared };
        for _ in 0..3 {
            assert!(
                pages.snapshot(None, false).unwrap().scheduled.is_none(),
                "cheap HTTP polls must not restart an expensive analysis during its cooldown"
            );
        }
        {
            let mut state = pages.shared.0.lock().unwrap();
            assert!(state.jobs.is_empty());
            state.pages.get("").checked = Some(Instant::now() - Duration::from_secs(7));
        }
        let first = pages.snapshot(None, false).unwrap().scheduled;
        assert!(first.is_some());
        assert_eq!(first, pages.snapshot(None, false).unwrap().scheduled);
        assert_eq!(pages.shared.0.lock().unwrap().jobs.len(), 1);
    }

    #[test]
    fn source_preflight_tracks_derived_project_hints_even_in_excluded_directories() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("own")).unwrap();
        fs::write(root.path().join("own/A.ts"), "export const a = 1;").unwrap();
        let mut loaded = load_config(None, root.path()).unwrap();
        loaded.config.project.exclude.push("nested/**".into());
        let services = archguard_engine::analyze_services(root.path()).unwrap();
        let scope = resolve_scope(root.path(), None, &services).unwrap();
        let before_hints = archguard_engine::derive_project_hints(root.path()).unwrap();
        let before = source_inputs(&scope, 0, &loaded, &services).unwrap();
        fs::create_dir(root.path().join("nested")).unwrap();
        for index in 0..10 {
            fs::write(
                root.path().join(format!("nested/file{index}.java")),
                format!("class File{index} {{}}"),
            )
            .unwrap();
        }
        assert_ne!(
            before_hints,
            archguard_engine::derive_project_hints(root.path()).unwrap(),
            "the existing effective-config derivation sees these files"
        );
        assert_ne!(
            before,
            source_inputs(&scope, 0, &loaded, &services).unwrap(),
            "changes affecting effective roots/languages must invalidate the preflight"
        );
    }

    #[test]
    fn unchanged_sources_bypass_the_analysis_pipeline_but_body_edits_invalidate() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("A.ts");
        fs::write(&source, "export const a = 1;\n").unwrap();
        let topology = archguard_engine::analyze_services(root.path()).unwrap();
        let scope = resolve_scope(root.path(), None, &topology).unwrap();
        let mut page = Page::default();
        page.refresh(&scope, None, 0, &topology);
        assert!(page.html.contains("archguard-data"));
        page.html = "cached page marker".into();
        let cache = root.path().join(".archguard/cache.json");
        assert!(cache.exists());
        fs::write(&cache, "not read during an unchanged source probe").unwrap();
        page.refresh(&scope, None, 0, &topology);
        assert!(
            page.html.as_ref() == "cached page marker",
            "unchanged probes must skip extraction, metrics and compiler graph projection"
        );
        assert_eq!(
            fs::read_to_string(&cache).unwrap(),
            "not read during an unchanged source probe"
        );
        let times = fs::metadata(&source).unwrap();
        fs::write(&source, "export const a = 2;\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(times.modified().unwrap()))
            .unwrap();
        page.refresh(&scope, None, 0, &topology);
        assert!(
            page.html.contains("archguard-data"),
            "same-size edits with restored mtime must run the pipeline"
        );
        assert!(serde_json::from_slice::<serde_json::Value>(&fs::read(cache).unwrap()).is_ok());
    }

    #[test]
    fn unchanged_pages_expire_without_sleeping_for_the_relative_git_window() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("A.ts"), "export const a = 1;\n").unwrap();
        let mut page = Page {
            html: "".into(),
            revision: String::new(),
            warnings: Vec::new(),
            rendered: None,
            ..Default::default()
        };
        let topology = archguard_engine::analyze_services(root.path()).unwrap();
        let scope = resolve_scope(root.path(), None, &topology).unwrap();
        page.refresh(&scope, None, 0, &topology);
        let original = page.html.clone();
        assert!(original.contains("archguard-data"));
        // A marker makes re-rendering observable without production counters.
        page.html = "cached page marker".into();
        let topology = archguard_engine::analyze_services(root.path()).unwrap();
        let scope = resolve_scope(root.path(), None, &topology).unwrap();
        page.refresh(&scope, None, 0, &topology);
        assert_eq!(page.html.as_ref(), "cached page marker");
        page.rendered.as_mut().unwrap().1 = Instant::now() - MAX_PAGE_AGE;
        let topology = archguard_engine::analyze_services(root.path()).unwrap();
        let scope = resolve_scope(root.path(), None, &topology).unwrap();
        page.refresh(&scope, None, 0, &topology);
        assert_eq!(
            page.html, original,
            "relative Git windows refresh at least once per minute"
        );
    }

    #[test]
    fn importing_test_coverage_refreshes_page_without_source_edits() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("A.ts"), "export const a=1;\n").unwrap();
        let topology = archguard_engine::analyze_services(root.path()).unwrap();
        let scope = resolve_scope(root.path(), None, &topology).unwrap();
        let mut page = Page::default();
        page.refresh(&scope, None, 0, &topology);
        let old_revision = page.revision.clone();
        assert!(page.html.contains("not_imported"));
        let loaded = archguard_engine::load_config(None, root.path()).unwrap();
        let run =
            archguard_engine::analyze_run(root.path(), &loaded, archguard_engine::Level::File)
                .unwrap();
        fs::write(root.path().join("coverage.xml"),r#"<coverage><packages><package><classes><class filename="A.ts"><lines><line number="1" hits="0"/></lines></class></classes></package></packages></coverage>"#).unwrap();
        archguard_engine::import_test_coverage(
            root.path(),
            &run.config,
            &run.graph,
            Path::new("coverage.xml"),
            None,
        )
        .unwrap();
        page.refresh(&scope, None, 0, &topology);
        assert_ne!(
            page.revision, old_revision,
            "coverage snapshot must invalidate the live page even when its sources are unchanged"
        );
        let json = page
            .html
            .split("<script id=\"archguard-data\" type=\"application/json\">")
            .nth(1)
            .unwrap()
            .split("</script>")
            .next()
            .unwrap();
        let data: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(
            data["insights"]["test_coverage"]["files"][0]["lines"]["covered"],
            0
        );
        assert_eq!(
            data["insights"]["coverage_review"]["files"][0]["score"],
            1.0
        );
    }
    #[test]
    fn importing_sarif_refreshes_page_without_source_edits() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("A.ts"), "export const a=1;\n").unwrap();
        let topology = archguard_engine::analyze_services(root.path()).unwrap();
        let scope = resolve_scope(root.path(), None, &topology).unwrap();
        let mut page = Page::default();
        page.refresh(&scope, None, 0, &topology);
        let old_revision = page.revision.clone();
        let loaded = archguard_engine::load_config(None, root.path()).unwrap();
        let run =
            archguard_engine::analyze_run(root.path(), &loaded, archguard_engine::Level::File)
                .unwrap();
        fs::write(root.path().join("quality.sarif"), r#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"fixture-checker"}},"results":[{"ruleId":"fixture-rule","level":"warning","message":{"text":"review fixture source"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"A.ts"},"region":{"startLine":1}}}]}]}]}"#).unwrap();
        archguard_engine::import_quality_findings(
            root.path(),
            &run.config,
            &run.graph,
            Path::new("quality.sarif"),
            None,
        )
        .unwrap();
        page.refresh(&scope, None, 0, &topology);
        assert_ne!(
            page.revision, old_revision,
            "SARIF import must refresh without changed source files"
        );
        let json = page
            .html
            .split("<script id=\"archguard-data\" type=\"application/json\">")
            .nth(1)
            .unwrap()
            .split("</script>")
            .next()
            .unwrap();
        let data: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(
            data["insights"]["quality"]["findings"][0]["rule_id"],
            "fixture-rule"
        );
        assert_eq!(data["insights"]["quality"]["findings"][0]["node"], "A.ts");
        assert_eq!(
            data["insights"]["quality"]["findings"][0]["status"],
            "unchanged_since_import"
        );
    }
    #[test]
    fn review_record_creation_and_source_edits_refresh_the_review_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.py"), "value = 1\n").unwrap();
        let topology = archguard_engine::analyze_services(root).unwrap();
        let scope = resolve_scope(root, None, &topology).unwrap();
        let mut page = Page::default();
        page.refresh(&scope, None, 0, &topology);
        let before = page.revision.clone();
        let (report, _) =
            archguard_engine::generate_review(root, None, None, 1_793_232_000).unwrap();
        archguard_engine::save_review(root, &report).unwrap();
        page.refresh(&scope, None, 0, &topology);
        assert_ne!(
            page.revision, before,
            "recording a review must refresh a warm live page"
        );
        let payload = |html: &str| -> serde_json::Value {
            serde_json::from_str(
                html.split("<script id=\"archguard-data\" type=\"application/json\">")
                    .nth(1)
                    .unwrap()
                    .split("</script>")
                    .next()
                    .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(
            payload(&page.html)["insights"]["review_session"]["status"],
            "current"
        );
        fs::write(root.join("a.py"), "value = 2\n").unwrap();
        page.refresh(&scope, None, 0, &topology);
        assert_eq!(
            payload(&page.html)["insights"]["review_session"]["status"],
            "stale"
        );
    }
    #[test]
    fn asyncapi_edits_refresh_patterns_on_a_warm_live_page() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.py"), "value=1\n").unwrap();
        let topology = archguard_engine::analyze_services(root).unwrap();
        let scope = resolve_scope(root, None, &topology).unwrap();
        let mut page = Page::default();
        page.refresh(&scope, None, 0, &topology);
        let before = page.revision.clone();
        fs::write(root.join("asyncapi.yaml"), "asyncapi: 3.0.0\nchannels:\n  orders:\n    address: orders.created\noperations:\n  emit:\n    action: send\n    channel:\n      $ref: '#/channels/orders'\n").unwrap();
        page.refresh(&scope, None, 0, &topology);
        assert_ne!(page.revision, before);
        let json = page
            .html
            .split("<script id=\"archguard-data\" type=\"application/json\">")
            .nth(1)
            .unwrap()
            .split("</script>")
            .next()
            .unwrap();
        let data: serde_json::Value = serde_json::from_str(json).unwrap();
        let event = data["insights"]["patterns"]["assessments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["pattern"] == "event_driven")
            .unwrap();
        assert_eq!(event["status"], "candidate");
        assert_eq!(event["metrics"]["send_operations"], 1);
    }

    #[test]
    fn asyncapi_edits_refresh_repository_overview_without_source_scans() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in ["one", "two"] {
            fs::create_dir(root.join(name)).unwrap();
            fs::write(
                root.join(name).join("package.json"),
                format!(r#"{{"name":"{name}"}}"#),
            )
            .unwrap();
        }
        let topology = archguard_engine::analyze_services(root).unwrap();
        let scope = resolve_scope(root, None, &topology).unwrap();
        assert!(scope.overview);
        let mut page = Page::default();
        page.refresh(&scope, None, 0, &topology);
        let before = page.revision.clone();
        fs::write(root.join("asyncapi.yaml"), "asyncapi: 2.6.0\nchannels:\n  orders:\n    publish:\n      operationId: receiveOrders\n").unwrap();
        page.refresh(&scope, None, 0, &topology);
        assert_ne!(page.revision, before);
        assert!(
            page.html.contains("receiveOrders") || page.html.contains("/channels/orders/publish")
        );
    }

    #[test]
    fn engineering_imports_refresh_without_source_edits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = "export const a = 1;\n";
        fs::write(root.join("A.ts"), source).unwrap();
        let topology = archguard_engine::analyze_services(root).unwrap();
        let scope = resolve_scope(root, None, &topology).unwrap();
        let mut page = Page::default();
        page.refresh(&scope, None, 0, &topology);
        let mut previous = page.revision.clone();
        let loaded = archguard_engine::load_config(None, root).unwrap();
        let run =
            archguard_engine::analyze_run(root, &loaded, archguard_engine::Level::File).unwrap();
        let mut updated = |field: &str| {
            page.refresh(&scope, None, 0, &topology);
            assert_ne!(
                page.revision, previous,
                "{field} import must invalidate live page"
            );
            previous = page.revision.clone();
            let payload = page
                .html
                .split("<script id=\"archguard-data\" type=\"application/json\">")
                .nth(1)
                .unwrap()
                .split("</script>")
                .next()
                .unwrap();
            let data: serde_json::Value = serde_json::from_str(payload).unwrap();
            data["insights"][field].clone()
        };
        fs::write(
            root.join("junit.xml"),
            "<testsuite name=\"example\"><testcase name=\"one\" time=\"0.5\"/></testsuite>",
        )
        .unwrap();
        fs::write(root.join("mapping.json"),r#"{"schema_version":1,"tests":[{"name":"one","sources":[{"path":"A.ts","lines":[1]}]}]}"#).unwrap();
        archguard_engine::import_test_evidence(
            root,
            &run.config,
            &run.graph,
            Path::new("junit.xml"),
            Some(Path::new("mapping.json")),
            None,
        )
        .unwrap();
        assert_eq!(
            updated("test_evidence")["tests"][0]["duration_seconds"],
            0.5
        );
        fs::write(root.join("mutation-report.json"),serde_json::json!({"schemaVersion":"2","thresholds":{"high":80,"low":60},"files":{"A.ts":{"language":"typescript","source":source,"mutants":[{"id":"1","mutatorName":"NumberLiteral","status":"Survived","location":{"start":{"line":1,"column":18},"end":{"line":1,"column":19}},"replacement":"2"}]}}}).to_string()).unwrap();
        archguard_engine::import_mutations(
            root,
            &run.config,
            &run.graph,
            Path::new("mutation-report.json"),
            None,
        )
        .unwrap();
        assert_eq!(updated("mutations")["summary"]["survived"], 1);
        let calls = archguard_engine::analyze_calls(root, &run.config);
        let coverage = archguard_engine::read_test_coverage(root, &run.config, &run.graph);
        let quality = archguard_engine::read_quality_findings(root, &run.config, &run.graph);
        let snapshot = archguard_engine::build_quality_snapshot(
            root,
            &run.config,
            &run.graph,
            &calls,
            &coverage,
            &quality,
        )
        .unwrap();
        archguard_engine::record_quality_snapshot(root, &snapshot, Some("test")).unwrap();
        assert_eq!(
            updated("quality_history")["records"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        fs::write(root.join("api.json"),r#"{"openapi":"3.1.0","info":{"title":"Test","version":"1"},"paths":{"/health":{"get":{"responses":{"200":{"description":"OK"}}}}}}"#).unwrap();
        archguard_engine::import_api_contract(
            root,
            &topology,
            Path::new("api.json"),
            "unmapped-api",
            None,
        )
        .unwrap();
        assert_eq!(
            updated("contracts")["contracts"][0]["operations"][0]["id"],
            "GET /health"
        );
        assert_eq!(fs::read_to_string(root.join("A.ts")).unwrap(), source);
    }

    #[test]
    fn importing_duplicate_groups_refreshes_the_quality_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.py"), "value=1\n").unwrap();
        fs::write(root.join("b.py"), "value=1\n").unwrap();
        let topology = archguard_engine::analyze_services(root).unwrap();
        let scope = resolve_scope(root, None, &topology).unwrap();
        let mut page = Page::default();
        page.refresh(&scope, None, 0, &topology);
        let before = page.revision.clone();
        let loaded = archguard_engine::load_config(None, root).unwrap();
        let run =
            archguard_engine::analyze_run(root, &loaded, archguard_engine::Level::Module).unwrap();
        fs::create_dir_all(root.join(".archguard")).unwrap();
        fs::write(root.join(".archguard/cpd.xml"),"<pmd-cpd><duplication lines='1' tokens='3'><file path='a.py' line='1'/><file path='b.py' line='1'/><codefragment>value=1</codefragment></duplication></pmd-cpd>").unwrap();
        archguard_engine::import_duplicates(
            root,
            &run.config,
            &run.graph,
            Path::new(".archguard/cpd.xml"),
            None,
        )
        .unwrap();
        page.refresh(&scope, None, 0, &topology);
        assert_ne!(
            page.revision, before,
            "CPD import must invalidate a warm live page"
        );
        let payload: serde_json::Value = serde_json::from_str(
            page.html
                .split("<script id=\"archguard-data\" type=\"application/json\">")
                .nth(1)
                .unwrap()
                .split("</script>")
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["insights"]["duplicates"]["groups"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn project_pages_are_bounded_and_recent_pages_survive_eviction() {
        let mut cache = PageCache::default();
        for index in 0..MAX_CACHED_PROJECTS {
            cache.get(&format!("project-{index}")).html = index.to_string().into();
        }
        assert_eq!(cache.get("project-0").html.as_ref(), "0");
        cache.get("new-project");
        assert_eq!(cache.pages.len(), MAX_CACHED_PROJECTS);
        assert!(cache.pages.contains_key("project-0"));
        assert!(!cache.pages.contains_key("project-1"));
    }

    #[test]
    fn a_completed_snapshot_stays_pinned_until_all_check_waiters_copy_it() {
        let mut cache = PageCache::default();
        for index in 0..MAX_CACHED_PROJECTS {
            let page = cache.get(&format!("project-{index}"));
            page.scheduled = Some(index as u64 + 1);
        }
        // The worker has published a completed result, but its check handlers
        // have not reacquired the mutex to copy it yet.
        cache.pages.get_mut("project-0").unwrap().0.scheduled = None;
        cache.waiters.insert("project-0".into(), 2);
        assert!(cache.available("project-0"));
        assert!(
            !cache.available("new-project"),
            "completed results still belong to their waiting HTTP checks"
        );
        *cache.waiters.get_mut("project-0").unwrap() -= 1;
        assert!(
            !cache.available("new-project"),
            "one remaining waiter still pins the result"
        );
        cache.waiters.remove("project-0");
        assert!(cache.available("new-project"));
        cache.get("new-project");
        assert!(!cache.pages.contains_key("project-0"));
        assert_eq!(cache.pages.len(), MAX_CACHED_PROJECTS);
    }

    #[test]
    fn project_eviction_skips_completed_results_still_awaited_by_a_check() {
        let mut cache = PageCache::default();
        for index in 0..MAX_CACHED_PROJECTS {
            cache.get(&format!("project-{index}"));
        }
        cache.waiters.insert("project-0".into(), 1);
        cache.get("new-project");
        assert!(
            cache.pages.contains_key("project-0"),
            "the oldest page is pinned until copied"
        );
        assert!(!cache.pages.contains_key("project-1"));
        assert_eq!(cache.pages.len(), MAX_CACHED_PROJECTS);
    }

    #[test]
    fn synchronous_check_releases_its_pin_after_copying_the_completed_page() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("A.ts"), "export const a = 1;\n").unwrap();
        let pages = BackgroundPages::new(root.path().to_path_buf(), None, 0).unwrap();
        let page = pages.snapshot(None, true).unwrap();
        assert!(!page.revision.is_empty());
        assert!(page.completed > 0);
        assert!(pages.shared.0.lock().unwrap().pages.waiters.is_empty());
    }

    #[test]
    fn synchronous_check_timeout_releases_only_its_own_pin() {
        let shared = Arc::new((Mutex::new(RefreshState::default()), Condvar::new()));
        {
            let mut state = shared.0.lock().unwrap();
            state.pages.get("project").scheduled = Some(1);
            state.pages.waiters.insert("project".into(), 1);
        }
        let pages = BackgroundPages { shared };
        let error = pages
            .snapshot_until(Some("project"), true, Instant::now())
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        let state = pages.shared.0.lock().unwrap();
        assert_eq!(
            state.pages.waiters.get("project"),
            Some(&1),
            "another waiter retains its pin"
        );
        assert_eq!(
            state.pages.pages["project"].0.scheduled,
            Some(1),
            "a timeout must not cancel a shared analysis"
        );
    }

    #[test]
    fn synchronous_checks_reserve_http_workers_for_nonblocking_browser_requests() {
        let shared = Arc::new((Mutex::new(RefreshState::default()), Condvar::new()));
        {
            let mut state = shared.0.lock().unwrap();
            for project in ["first", "second"] {
                state.pages.get(project).scheduled = Some(1);
                state.pages.waiters.insert(project.into(), 1);
            }
        }
        let pages = BackgroundPages { shared };
        let error = pages
            .snapshot_until(None, true, Instant::now())
            .err()
            .unwrap();
        assert_eq!(
            error.kind(),
            io::ErrorKind::WouldBlock,
            "excess synchronous checks fail immediately instead of taking every HTTP worker"
        );
        let state = pages.shared.0.lock().unwrap();
        assert!(
            !state.pages.pages.contains_key(""),
            "rejected checks cannot allocate a page or enqueue work"
        );
        assert_eq!(state.pages.waiters.values().sum::<usize>(), 2);
        assert!(state.jobs.is_empty());
        drop(state);
        assert!(
            pages.snapshot(Some("first"), false).is_ok(),
            "browser status requests do not consume a waiting-check slot"
        );
    }
}
