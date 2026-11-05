//! Security tab: the web viewer's dependency scan view (renderSecurity in
//! viewer_security.js). It reads the saved `.archguard/security.json`
//! through `archguard_engine::read_security`, as `archguard graph` and
//! `archguard serve` do; it never starts Trivy or network work.
//!
//! All tab state is one [`SecurityState`], built by [`build`] and shown by
//! [`push`] as one `SecurityModel` value.

mod snapshot;

use std::path::Path;

use archguard_engine::core::Config;
use archguard_engine::{SecurityFinding, SecurityReport};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::context::Loaded;
use crate::documents;
use crate::ui::{AppWindow, SecurityData, SecurityFindingItem, SecurityLine, SecurityModel};
use snapshot::{SEVERITIES, safe_url};

/// Findings per page (`securityUi.limit`).
const PAGE: usize = 100;

/// The saved scan, the scan command for this root and the list filters.
pub struct SecurityState {
    report: SecurityReport,
    command: String,
    query: String,
    /// 0 all, else 1 + position in [`SEVERITIES`].
    severity: usize,
    limit: usize,
}

/// Read the saved scan for an analysis. `data.run.graph.root` is the root
/// the web viewer reads and puts into its scan command.
pub fn build(data: &Loaded) -> SecurityState {
    build_from(Path::new(&data.run.graph.root), &data.run.config)
}

fn build_from(root: &Path, config: &Config) -> SecurityState {
    SecurityState {
        report: archguard_engine::read_security(root, config),
        command: snapshot::scan_command(&root.to_string_lossy()),
        query: String::new(),
        severity: 0,
        limit: PAGE,
    }
}

/// Register the tab's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    let d = ui.global::<SecurityData>();
    d.on_set_query(action(ui, |s, q: SharedString| {
        s.query = q.into();
        s.limit = PAGE;
    }));
    d.on_set_severity(action(ui, |s, i: i32| {
        s.severity = usize::try_from(i).unwrap_or(0).min(SEVERITIES.len());
        s.limit = PAGE;
    }));
    let more = action(ui, |s, ()| s.limit += PAGE);
    d.on_load_more(move || more(()));
    d.on_open_url(|url| {
        if let Some(url) = safe_url(Some(url.as_str())) {
            let url = url.to_string();
            std::thread::spawn(move || crate::web::open_in_browser(&url));
        }
    });
}

/// A callback that runs `f` on the tab state and refreshes the window.
fn action<A: 'static>(
    ui: &AppWindow,
    f: impl Fn(&mut SecurityState, A) + 'static,
) -> impl Fn(A) + 'static {
    let w = ui.as_weak();
    move |arg| {
        let Some(ui) = w.upgrade() else { return };
        documents::with_active_state(|s: &mut SecurityState| {
            f(s, arg);
            push(&ui, s);
        });
    }
}

/// Fill the tab after an analysis finished. The dependency files are hashed
/// on a worker thread; snapshot runs read in place so the PNG shows them.
pub fn show(ui: &AppWindow, data: &Loaded) {
    // A document keeps its state; switching back only shows it again.
    if documents::with_active_state(|s: &mut SecurityState| push(ui, s)).is_some() {
        return;
    }
    let Some(doc) = documents::active_id() else {
        return;
    };
    ui.global::<SecurityData>()
        .set_model(SecurityModel::default());
    if std::env::var_os("ARCHGUARD_DESKTOP_SNAPSHOT").is_some() {
        return install(ui, doc, build(data));
    }
    let (root, config) = (
        std::path::PathBuf::from(&data.run.graph.root),
        data.run.config.clone(),
    );
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let state = build_from(&root, &config);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                install(&ui, doc, state);
            }
        });
    });
}

/// Store the state for `doc`; show it only if `doc` is still active.
fn install(ui: &AppWindow, doc: documents::DocId, state: SecurityState) {
    if documents::set_state(doc, state) {
        documents::with_active_state(|s: &mut SecurityState| push(ui, s));
    }
}

/// Show the state in the window.
pub fn push(ui: &AppWindow, s: &SecurityState) {
    ui.global::<SecurityData>().set_model(model(s));
}

fn model(s: &SecurityState) -> SecurityModel {
    let r = &s.report;
    let counts = snapshot::counts(r);
    let available = snapshot::available(r, &counts);
    let shown_if = |ok: bool, n: usize| if ok { n.to_string() } else { "—".into() };
    let metrics = [
        shown_if(true, counts.packages_scanned),
        shown_if(available, counts.affected_packages),
        shown_if(available, counts.findings),
        shown_if(true, counts.targets),
    ];
    let findings = snapshot::filtered(r, &s.query, s.severity);
    let page: Vec<_> = findings.iter().take(s.limit).map(|f| finding(f)).collect();
    SecurityModel {
        loaded: true,
        status: r.status.as_str().into(),
        metrics: strings(metrics),
        severities: strings(counts.by_severity.map(|n| shown_if(available, n))),
        scanner: match &r.scanner_version {
            Some(v) => format!("{} {v}", r.scanner),
            None => r.scanner.clone(),
        }
        .into(),
        scanned_at: r.scanned_at.clone().unwrap_or_default().into(),
        database_updated_at: r.database_updated_at.clone().unwrap_or_default().into(),
        command: s.command.as_str().into(),
        targets: rows(
            r.targets
                .iter()
                .map(|t| line(format!("{} · {}", t.path, t.ecosystem), t.packages as i32)),
        ),
        diagnostics: rows(r.diagnostics.iter().map(|d| line(d.clone(), -1))),
        query: s.query.as_str().into(),
        severity: s.severity as i32,
        shown: page.len() as i32,
        matching: findings.len() as i32,
        total: counts.findings as i32,
        more: findings.len() > s.limit,
        empty: snapshot::empty(r, &counts, findings.len()) as i32,
        findings: rows(page),
    }
}

fn finding(f: &SecurityFinding) -> SecurityFindingItem {
    SecurityFindingItem {
        severity: SEVERITIES[snapshot::severity_rank(&f.severity)].into(),
        package: f.package.as_str().into(),
        id: f.id.as_str().into(),
        title: f.title.as_str().into(),
        ecosystem: f.ecosystem.as_str().into(),
        installed: f.installed_version.as_str().into(),
        fixed: f.fixed_version.clone().unwrap_or_default().into(),
        url: safe_url(f.url.as_deref()).unwrap_or_default().into(),
        targets: rows(f.targets.iter().map(|t| line(t.clone(), -1))),
    }
}

fn line(text: String, packages: i32) -> SecurityLine {
    SecurityLine {
        text: text.into(),
        packages,
    }
}

fn strings<const N: usize>(items: [String; N]) -> ModelRc<SharedString> {
    rows(items.into_iter().map(SharedString::from))
}

fn rows<T: Clone + 'static>(items: impl IntoIterator<Item = T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items.into_iter().collect::<Vec<_>>()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;
    use snapshot::tests::{finding as row, report};

    fn temp_project(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "archguard-desktop-security-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp project");
        std::fs::write(
            dir.join("Cargo.lock"),
            "version = 4\n[[package]]\nname = 'demo-dep'\nversion = '1.0.0'\n",
        )
        .expect("lockfile");
        dir
    }

    fn config(root: &Path) -> Config {
        archguard_engine::load_config(None, root)
            .expect("default config")
            .config
    }

    fn state(report: SecurityReport) -> SecurityState {
        SecurityState {
            report,
            command: snapshot::scan_command("/p"),
            query: String::new(),
            severity: 0,
            limit: PAGE,
        }
    }

    /// Without a saved scan the tab shows the web's empty state.
    #[test]
    fn missing_snapshot_is_not_scanned() {
        let root = temp_project("missing");
        let m = model(&build_from(&root, &config(&root)));
        assert_eq!(m.status, "not_scanned");
        let metrics: Vec<_> = m.metrics.iter().map(|s| s.to_string()).collect();
        assert_eq!(metrics, ["0", "—", "—", "0"]);
        assert!(m.severities.iter().all(|s| s == "—"));
        assert_eq!((m.shown, m.matching, m.total), (0, 0, 0));
        assert_eq!(m.empty, snapshot::Empty::NotScanned as i32);
        assert_eq!(m.scanner, "trivy");
        assert_eq!(m.diagnostics.row_count(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A saved scan whose fingerprint matches the dependency files is
    /// "ready" and shows its findings, read through the engine.
    #[test]
    fn saved_snapshot_is_read_through_the_engine() {
        let root = temp_project("saved");
        let config = config(&root);
        let fingerprint = archguard_engine::security_fingerprint(&root, &config).expect("hash");
        std::fs::create_dir_all(root.join(".archguard")).expect("dir");
        std::fs::write(
            root.join(".archguard/security.json"),
            format!(
                r#"{{"schema_version":1,"status":"ready","scanner":"trivy","scanner_version":"0.76.0","scanned_at":"2026-10-27T09:00:00Z","database_updated_at":null,"fingerprint":"{fingerprint}","packages_scanned":1,"targets":[{{"path":"Cargo.lock","ecosystem":"cargo","packages":1}}],"findings":[{{"id":"CVE-2026-11111","package":"demo-dep","installed_version":"1.0.0","fixed_version":"1.0.1","ecosystem":"cargo","severity":"high","title":"Synthetic test advisory","url":"https://example.invalid/a","targets":["Cargo.lock"]}}],"diagnostics":[]}}"#
            ),
        )
        .expect("snapshot");
        let s = build_from(&root, &config);
        let m = model(&s);
        assert_eq!(m.status, "ready");
        let metrics: Vec<_> = m.metrics.iter().map(|s| s.to_string()).collect();
        assert_eq!(metrics, ["1", "1", "1", "1"]);
        let sev: Vec<_> = m.severities.iter().map(|s| s.to_string()).collect();
        assert_eq!(sev, ["0", "1", "0", "0", "0"]);
        assert_eq!(m.scanner, "trivy 0.76.0");
        assert_eq!(m.database_updated_at, "");
        assert_eq!(
            m.targets.row_data(0).expect("target").text,
            "Cargo.lock · cargo"
        );
        let f = m.findings.row_data(0).expect("finding");
        assert_eq!(
            (f.id.as_str(), f.fixed.as_str()),
            ("CVE-2026-11111", "1.0.1")
        );
        assert_eq!(m.empty, 0);
        assert!(s.command.starts_with("archguard security -- '"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// 250 findings page by 100, as "Load more findings"; filters reset paging
    /// in `wire`, and a filter without matches says so.
    #[test]
    fn findings_page_and_filter() {
        let findings = (0..250)
            .map(|i| row(&format!("CVE-{i:03}"), "pkg", "1.0.0", "medium"))
            .collect();
        let mut s = state(report("stale", findings));
        let m = model(&s);
        assert_eq!(
            (m.shown, m.matching, m.total, m.more),
            (100, 250, 250, true)
        );
        s.limit += PAGE * 2;
        let m = model(&s);
        assert_eq!((m.shown, m.more), (250, false));
        s.severity = 2;
        let m = model(&s);
        assert_eq!((m.shown, m.matching, m.total), (0, 0, 250));
        assert_eq!(m.empty, snapshot::Empty::Filtered as i32);
        s.severity = 3;
        s.query = "cve-24".into();
        assert_eq!(model(&s).matching, 10);
    }
}
