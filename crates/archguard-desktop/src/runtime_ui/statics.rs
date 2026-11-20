//! Runtime tab → "Static indicators": the structural report behind
//! `archguard performance` (`analyze_performance`), as the web viewer's
//! "Source & build indicators" (`renderPerformancePanel`) and, for the
//! chosen node, its "Source cost evidence" (`renderPerformanceDetails`).
//! Static: nothing here was measured at runtime. Numbers are formatted
//! here, labels live in `ui/runtime_static.slint`.

use std::path::PathBuf;

use archguard_engine::core::{
    AbstractionMetric, Config, ContractCandidate, Graph, PerformanceReport, ReviewSources,
    SourceCostFinding,
};
use slint::ComponentHandle;

use crate::context::Loaded;
use crate::format::Locale;
use crate::tab_support::{Calls, TabState, Texts, model};
use crate::ui::{
    AppWindow, RtContract, RtCost, RtDistance, RtEntry, RtGroup, RtNode, RtStatic, RuntimeData,
};

/// Rows per list, as the web viewer shows them.
const ENTRIES: usize = 20;
const GROUPS: usize = 10;
const DISTANCES: usize = 10;
const COSTS: usize = 20;
const CONTRACTS: usize = 10;
const DIAGNOSTICS: usize = 10;

/// The tab's static part for one document; built from the analysis.
pub struct StaticState {
    report: Result<PerformanceReport, String>,
    /// The node whose evidence is shown; `None`: the first listed one.
    selected: Option<String>,
}

impl TabState for StaticState {
    fn push(&mut self, ui: &AppWindow) {
        let l = Texts::of(ui).locale();
        ui.global::<RuntimeData>().set_statics(build(self, l));
    }

    fn pending(ui: &AppWindow, error: Option<&str>) {
        ui.global::<RuntimeData>().set_statics(RtStatic {
            state: if error.is_some() { 2 } else { 0 },
            error: error.unwrap_or_default().into(),
            ..Default::default()
        });
    }
}

/// What the report needs, copied from the document on the UI thread.
pub struct Inputs {
    calls: Calls,
    root: PathBuf,
    config: Config,
    graph: Graph,
    view: Graph,
    sources: ReviewSources,
}

impl Inputs {
    pub fn of(data: &Loaded) -> Self {
        Inputs {
            calls: Calls::of(data),
            root: data.root.to_path_buf(),
            config: data.run.config.clone(),
            graph: data.run.graph.clone(),
            view: data.run.view.clone(),
            sources: data.sources.clone(),
        }
    }

    /// The same engine calls as the CLI's `archguard performance`. Slow;
    /// runs on a worker thread.
    pub fn build(self) -> StaticState {
        let calls = self.calls.get();
        let report = archguard_engine::analysis::match_review_entry_points(
            &self.graph,
            &self.sources,
            &self.config.review.entry_points,
        )
        .and_then(|ids| {
            archguard_engine::analyze_performance(
                &self.root,
                &self.config,
                &self.graph,
                &self.view,
                &self.sources,
                &calls,
                &ids,
            )
        })
        .map_err(|e| e.to_string());
        let mut s = StaticState {
            report,
            selected: None,
        };
        snapshot_overrides(&mut s);
        s
    }
}

/// Show the evidence of node `name`.
pub fn select(s: &mut StaticState, name: String) {
    s.selected = Some(name);
}

/// `name` is `node` or lies below it.
fn under(name: &str, node: &str) -> bool {
    name.strip_prefix(node)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// `file::Class` → `Class`; a module-level pseudo class (`file::(module)`)
/// reads as its file.
fn class_label(id: &str) -> &str {
    match id.rsplit_once("::") {
        Some((file, name)) if name.starts_with('(') => file,
        Some((_, name)) => name,
        None => id,
    }
}

fn lines(l: Locale, n: Option<u64>) -> slint::SharedString {
    n.map(|n| l.int(i64::try_from(n).unwrap_or(i64::MAX)))
        .unwrap_or_default()
        .into()
}

/// The measured node metrics, worst D′ first.
fn distances(r: &PerformanceReport) -> Vec<&AbstractionMetric> {
    let mut rows: Vec<_> = r
        .abstractions
        .iter()
        .filter(|m| m.coverage == "complete" && m.distance.is_some())
        .collect();
    rows.sort_by(|a, b| {
        b.distance
            .partial_cmp(&a.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.name.cmp(&b.name))
    });
    rows
}

fn cost(f: &SourceCostFinding, l: Locale) -> RtCost {
    RtCost {
        kind: match f.kind.as_str() {
            "heavy_import" => 0,
            "barrel" => 1,
            _ => 2,
        },
        other: f.kind.replace('_', " ").into(),
        node: f.name.as_str().into(),
        place: format!("{}:{}", f.file, f.line).into(),
        message: f.message.as_str().into(),
        raw: f.raw.trim().into(),
        library: f.library.clone().unwrap_or_default().into(),
        reexports: l.int(f.reexports.into()).into(),
        code_lines: l.int(f.code_lines.into()).into(),
    }
}

fn contract(c: &ContractCandidate) -> RtContract {
    RtContract {
        from: class_label(&c.from_class).into(),
        to: class_label(&c.to_class).into(),
        message: c.message.as_str().into(),
        interfaces: c
            .interfaces
            .iter()
            .map(|i| class_label(i))
            .collect::<Vec<_>>()
            .join(", ")
            .into(),
        evidence: c
            .evidence
            .iter()
            .map(|e| format!("{}:{} {}", e.file, e.line, e.raw.trim()))
            .collect::<Vec<_>>()
            .join("\n")
            .into(),
    }
}

/// The node whose evidence shows: the chosen one, else the first listed.
fn chosen(s: &StaticState, r: &PerformanceReport) -> Option<String> {
    s.selected.clone().or_else(|| {
        let first = distances(r).first().map(|m| m.name.clone());
        first
            .or_else(|| r.source_findings.first().map(|f| f.name.clone()))
            .or_else(|| r.entries.first().map(|e| e.name.clone()))
    })
}

/// A / I / D′ and the source-cost findings of `name` and the nodes below it.
fn node(r: &PerformanceReport, name: &str, l: Locale) -> RtNode {
    let metric = r
        .abstractions
        .iter()
        .find(|m| m.scope == "node" && m.name == name);
    let measured = metric
        .filter(|m| m.coverage == "complete")
        .and_then(|m| Some((m.abstractness?, m.distance?)));
    RtNode {
        name: name.into(),
        has_metric: metric.is_some(),
        measured: measured.is_some(),
        a: measured
            .map(|(a, _)| l.dec(a, 2))
            .unwrap_or_default()
            .into(),
        i: metric
            .map(|m| l.dec(m.instability, 2))
            .unwrap_or_default()
            .into(),
        d: measured
            .map(|(_, d)| l.dec(d, 2))
            .unwrap_or_default()
            .into(),
        abstract_types: metric
            .map(|m| l.int(m.abstract_types.into()))
            .unwrap_or_default()
            .into(),
        types: metric
            .map(|m| l.int(m.types.into()))
            .unwrap_or_default()
            .into(),
        reason: metric
            .and_then(|m| m.reason.clone())
            .unwrap_or_default()
            .into(),
        costs: model(
            r.source_findings
                .iter()
                .filter(|f| under(&f.name, name))
                .take(COSTS)
                .map(|f| cost(f, l)),
        ),
        contracts: model(
            r.contract_candidates
                .iter()
                .filter(|c| under(&c.from_node, name) || under(&c.to_node, name))
                .take(CONTRACTS)
                .map(contract),
        ),
    }
}

fn report_rows(r: &PerformanceReport, l: Locale) -> RtStatic {
    let kinds = |k: &str| r.source_findings.iter().filter(|f| f.kind == k).count() as i32;
    RtStatic {
        entries: model(r.entries.iter().take(ENTRIES).map(|e| RtEntry {
            name: e.name.as_str().into(),
            files: l.int(e.reachable_files.into()).into(),
            path: l.int(e.longest_component_path.into()).into(),
            cyclic: l.int(e.cyclic_components.into()).into(),
            lines: lines(l, e.lines),
        })),
        entries_total: r.entries.len() as i32,
        groups: model(r.cyclic_groups.iter().take(GROUPS).map(|g| RtGroup {
            size: l.count(g.nodes.len()).into(),
            files: l.int(g.source_files.into()).into(),
            lines: lines(l, g.lines),
            nodes: g.nodes.join(", ").into(),
        })),
        distances: model(distances(r).into_iter().take(DISTANCES).map(|m| {
            RtDistance {
                name: m.name.as_str().into(),
                group: m.scope == "scc",
                a: m.abstractness
                    .map(|a| l.dec(a, 2))
                    .unwrap_or_default()
                    .into(),
                i: l.dec(m.instability, 2).into(),
                d: m.distance.map(|d| l.dec(d, 2)).unwrap_or_default().into(),
            }
        })),
        heavy: kinds("heavy_import"),
        barrels: kinds("barrel"),
        contracts: r.contract_candidates.len() as i32,
        contracts_limited: r.contract_candidates_truncated,
        costs: model(r.source_findings.iter().take(COSTS).map(|f| cost(f, l))),
        diagnostics: model(
            r.diagnostics
                .iter()
                .take(DIAGNOSTICS)
                .map(|d| d.as_str().into()),
        ),
        ..Default::default()
    }
}

pub fn build(s: &StaticState, l: Locale) -> RtStatic {
    match &s.report {
        Err(e) => RtStatic {
            state: 2,
            error: e.as_str().into(),
            ..Default::default()
        },
        Ok(r) => {
            let selected = chosen(s, r);
            RtStatic {
                state: 1,
                node: selected.map(|n| node(r, &n, l)).unwrap_or_default(),
                ..report_rows(r, l)
            }
        }
    }
}

/// `ARCHGUARD_DESKTOP_RUNTIME_NODE=<name>` shows that node's evidence in
/// snapshot runs.
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut StaticState) {
    s.selected = std::env::var("ARCHGUARD_DESKTOP_RUNTIME_NODE")
        .ok()
        .or(s.selected.take());
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut StaticState) {}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use slint::Model;

    use super::*;

    /// A temporary copy of `fixtures/py-layers` (app ↔ domain cycle) with
    /// one eager `pandas` import added, its API files as entry points.
    fn py_layers() -> (tempfile::TempDir, StaticState) {
        let dir = tempfile::tempdir().unwrap();
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/py-layers");
        for rel in [
            "archguard.toml",
            "src/app/service.py",
            "src/app/usecase.py",
            "src/app/report.py",
            "src/api/rest.py",
            "src/api/http.py",
            "src/domain/order.py",
            "src/domain/ledger.py",
            "src/domain/money.py",
        ] {
            let to = dir.path().join(rel);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(src.join(rel), to).unwrap();
        }
        std::fs::write(dir.path().join("src/app/plot.py"), "import pandas\n").unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut loaded = archguard_engine::load_config(None, &root).expect("config");
        loaded.config.review.entry_points = vec!["**/api/*.py".into()];
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        // Packages as nodes, so app ↔ domain is a cyclic build group.
        use archguard_engine::core::{Cut, aggregate};
        let view = aggregate(&run.graph, &Cut::at_depth(&run.graph, 0));
        let inputs = Inputs {
            calls: Calls::unshared(&root, &run.config),
            config: run.config.clone(),
            graph: run.graph.clone(),
            view,
            sources: ReviewSources::default(),
            root,
        };
        (dir, inputs.build())
    }

    /// Entry points by reach, the app ↔ domain build cycle and the eager
    /// import, which is evidence of `app` and of its file.
    #[test]
    fn the_report_lists_entries_cycles_and_source_costs() {
        let (_dir, mut s) = py_layers();
        let m = build(&s, Locale::En);
        assert_eq!(m.state, 1, "{}", m.error);
        let entries: Vec<_> = m.entries.iter().map(|e| e.name.to_string()).collect();
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert!(entries.iter().all(|n| n.contains("api/")), "{entries:?}");
        assert!(
            m.entries
                .iter()
                .all(|e| e.files.parse::<u32>().unwrap() > 1)
        );
        let groups: Vec<_> = m.groups.iter().map(|g| g.nodes.to_string()).collect();
        assert!(
            groups
                .iter()
                .any(|g| g.contains("app") && g.contains("domain")),
            "{groups:?}"
        );
        assert_eq!((m.heavy, m.barrels), (1, 0));
        let cost = m.costs.row_data(0).unwrap();
        assert_eq!(cost.kind, 0);
        assert!(cost.place.ends_with("app/plot.py:1"), "{}", cost.place);
        let file = cost.node.to_string();
        let package = file
            .rsplit_once('/')
            .map_or(file.clone(), |(p, _)| p.to_string());
        select(&mut s, package.clone());
        let n = build(&s, Locale::En).node;
        assert_eq!(n.name.as_str(), package);
        assert_eq!(n.costs.row_count(), 1, "findings below the node count");
        assert_eq!(n.costs.row_data(0).unwrap().library.as_str(), "pandas");
        select(&mut s, file.replace("app/plot.py", "api"));
        assert_eq!(build(&s, Locale::En).node.costs.row_count(), 0);
    }

    #[test]
    fn names_below_a_node_belong_to_it() {
        assert!(under("src/app", "src/app") && under("src/app/x.py", "src/app"));
        assert!(!under("src/application", "src/app"));
        assert_eq!(class_label("src/a.ts::Order"), "Order");
        assert_eq!(class_label("api/http.py::(module)"), "api/http.py");
    }

    #[test]
    fn a_failed_report_shows_its_error() {
        let s = StaticState {
            report: Err("boom".into()),
            selected: None,
        };
        let m = build(&s, Locale::En);
        assert_eq!((m.state, m.error.as_str()), (2, "boom"));
    }
}
