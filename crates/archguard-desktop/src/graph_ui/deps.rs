//! The Dependencies display: one card per shown dependency and, for a
//! package dependency that also runs the other way, the explanation of that
//! package cycle (`explain_package_cycle`, the web viewer's
//! `renderPackageCycleExplanation`). Labels live in `ui/deps_cycle.slint`.

use std::collections::HashSet;

use archguard_engine::analysis::{CycleWitness, PackageCycleExplanation, explain_package_cycle};
use slint::ModelRc;

use crate::model_sync::sync_rows;
use crate::state::GraphState;
use crate::tab_support::model;
use crate::ui::{DepCard, DepCycle, DepScopeRow, DepStep};

/// Evidence lines per example edge, as the web viewer shows them.
const EVIDENCE_LINES: usize = 3;

/// The "dep-explain" action: `arg` is `from\nto`; the same card again
/// closes the explanation.
pub fn toggle(g: &mut GraphState, arg: &str) {
    let pair = arg
        .split_once('\n')
        .map(|(a, b)| (a.to_string(), b.to_string()));
    g.dep = if g.dep == pair { None } else { pair };
}

/// The cards of the shown dependencies; only the open explanation is
/// computed.
pub fn cards(
    g: &GraphState,
    selected: Option<usize>,
    old: &ModelRc<DepCard>,
    shown: bool,
) -> ModelRc<DepCard> {
    let v = &g.view;
    let shown = shown.then(|| v.shown(selected, g.mode));
    let both: HashSet<(usize, usize)> = v.links.iter().map(|l| (l.from, l.to)).collect();
    sync_rows(
        old,
        shown.into_iter().flatten().map(|(i, lane)| {
            let l = &v.links[i];
            let node = |row: usize| &g.tree.nodes[v.rows[row].node];
            let (from, to) = (node(l.from).name.clone(), node(l.to).name.clone());
            let pair = (!node(l.from).file || !node(l.to).file) && both.contains(&(l.to, l.from));
            let explained = pair && g.dep.as_ref().is_some_and(|(a, b)| *a == from && *b == to);
            DepCard {
                cycle: if explained {
                    cycle(g, &from, &to)
                } else {
                    DepCycle::default()
                },
                from: from.into(),
                to: to.into(),
                weight: l.weight.min(i32::MAX as u64) as i32,
                kind: v.link_kind(i, lane, selected, g.mode) as i32,
                refs: l
                    .refs
                    .iter()
                    .map(|r| format!("{}:{}  {}", r.file, r.line, r.raw.trim()))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into(),
                ref_count: l.refs.len() as i32,
                pair,
                explained,
            }
        }),
    )
}

/// The explanation of packages `from` → `to` in the Graph's (possibly
/// simulated) analysis graph.
pub fn cycle(g: &GraphState, from: &str, to: &str) -> DepCycle {
    explain_package_cycle(&g.tree.full, from, to).map_or_else(DepCycle::default, |e| view(&e))
}

/// `names` without the leading path segments they all share; each keeps
/// its last segment (the web viewer's `detailLabels`).
fn relative<'a>(names: &[&'a str]) -> Vec<&'a str> {
    let parts: Vec<Vec<&str>> = names.iter().map(|n| n.split('/').collect()).collect();
    let mut shared = 0;
    while !parts.is_empty()
        && parts
            .iter()
            .all(|p| p.len() > shared + 1 && p[shared] == parts[0][shared])
    {
        shared += 1;
    }
    names
        .iter()
        .map(|n| n.splitn(shared + 1, '/').last().unwrap_or(n))
        .collect()
}

fn view(e: &PackageCycleExplanation) -> DepCycle {
    let [a, b] = &e.directions;
    let packages = relative(&[a.from.as_str(), a.to.as_str()]);
    let label = |p: &str| {
        if p == a.from {
            packages[0]
        } else {
            packages[1]
        }
    };
    let w = &e.witness;
    DepCycle {
        has: !w.edges.is_empty(),
        verdict: match (w.connected, w.closed) {
            (true, true) => 0,
            (true, false) => 1,
            _ => 2,
        },
        typed: w.files.iter().all(|f| f.ends_with(".java")),
        start: label(&w.start).into(),
        middle: label(&w.middle).into(),
        steps: model(steps(w)),
        rows: model(e.directions.iter().map(|d| DepScopeRow {
            label: format!("{} → {}", label(&d.from), label(&d.to)).into(),
            production: d.counts.production as i32,
            test: d.counts.test as i32,
            mixed: d.counts.mixed as i32,
            unknown: d.counts.unknown as i32,
        })),
        mixed: a.counts.mixed + b.counts.mixed > 0,
        unknown: a.counts.unknown + b.counts.unknown > 0,
        file_cycles: e.file_cycles as i32,
    }
}

/// The example as file boxes; a connected example is one chain, separate
/// examples are two pairs.
fn steps(w: &CycleWitness) -> Vec<DepStep> {
    let files: Vec<&str> = w.files.iter().map(String::as_str).collect();
    let labels = relative(&files);
    let label = |f: &str| {
        files
            .iter()
            .position(|x| *x == f)
            .map_or(f, |i| labels[i])
            .into()
    };
    let mut out = Vec::new();
    for (i, e) in w.edges.iter().enumerate() {
        out.push(DepStep {
            file: label(&e.from),
            evidence: e
                .refs
                .iter()
                .take(EVIDENCE_LINES)
                .map(|r| format!("{}:{} {}", r.file, r.line, r.raw.trim()))
                .collect::<Vec<_>>()
                .join("\n")
                .into(),
            arrow: true,
            back: false,
        });
        if !w.connected || i + 1 == w.edges.len() {
            out.push(DepStep {
                file: label(&e.to),
                back: w.closed,
                ..Default::default()
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use slint::Model;

    use super::*;
    use crate::tree::Tree;

    /// The Graph of a temporary copy of `fixtures/py-layers`, whose
    /// `domain/ledger.py` imports `app.service` while `app` uses `domain`.
    fn py_layers() -> (tempfile::TempDir, GraphState) {
        fn copy(from: &Path, to: &Path) {
            std::fs::create_dir_all(to).unwrap();
            for e in std::fs::read_dir(from).unwrap().flatten() {
                let name = e.file_name();
                if name == ".archguard" {
                    continue;
                }
                if e.path().is_dir() {
                    copy(&e.path(), &to.join(&name));
                } else {
                    std::fs::copy(e.path(), to.join(&name)).unwrap();
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        copy(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/py-layers"),
            dir.path(),
        );
        let root = dir.path().canonicalize().unwrap();
        let loaded = archguard_engine::load_config(None, &root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        (dir, GraphState::new(Tree::from_graph(&run.graph)))
    }

    fn package(g: &GraphState, label: &str) -> String {
        let n = g
            .tree
            .nodes
            .iter()
            .find(|n| !n.file && n.label == label)
            .expect("package");
        n.name.clone()
    }

    /// app → domain: report.py → ledger.py → service.py returns to another
    /// file of `app`, so the package cycle is no file cycle.
    #[test]
    fn the_py_layers_package_cycle_reaches_a_different_file() {
        let (_dir, g) = py_layers();
        let (app, domain) = (package(&g, "app"), package(&g, "domain"));
        let c = cycle(&g, &app, &domain);
        assert!(c.has);
        assert_eq!(c.verdict, 1, "a different file is reached");
        assert_eq!((c.start.as_str(), c.middle.as_str()), ("app", "domain"));
        let files: Vec<String> = c.steps.iter().map(|s| s.file.to_string()).collect();
        assert_eq!(
            files,
            ["app/report.py", "domain/ledger.py", "app/service.py"]
        );
        let evidence = c.steps.row_data(1).unwrap().evidence;
        assert!(
            evidence.contains("ledger.py:2 from app import service"),
            "{evidence}"
        );
        let rows: Vec<_> = c
            .rows
            .iter()
            .map(|r| (r.label.to_string(), r.production, r.test, r.unknown))
            .collect();
        assert_eq!(
            rows,
            [
                ("app → domain".to_string(), 0, 0, 3),
                ("domain → app".to_string(), 0, 0, 1)
            ]
        );
        assert!(c.unknown && !c.mixed);
        assert_eq!(c.file_cycles, 0);
        assert!(!cycle(&g, &domain, "missing").has);
    }

    /// Only a package dependency that runs both ways offers the
    /// explanation; it is computed for the card that opened it.
    #[test]
    fn a_card_explains_its_package_cycle_on_request() {
        let (_dir, mut g) = py_layers();
        let (app, domain) = (package(&g, "app"), package(&g, "domain"));
        let card = |g: &GraphState, from: &str, to: &str| {
            let all = cards(g, None, &ModelRc::default(), true);
            all.iter()
                .find(|c| c.from == from && c.to == to)
                .expect("card")
        };
        let api = package(&g, "api");
        assert!(!card(&g, &api, &app).pair, "api → app has no reverse");
        let c = card(&g, &app, &domain);
        assert!(c.pair && !c.explained && !c.cycle.has);
        toggle(&mut g, &format!("{app}\n{domain}"));
        let c = card(&g, &app, &domain);
        assert!(c.explained && c.cycle.has);
        assert!(
            !card(&g, &domain, &app).explained,
            "only the opened direction"
        );
        toggle(&mut g, &format!("{app}\n{domain}"));
        assert!(
            !card(&g, &app, &domain).explained,
            "the same card closes it"
        );
    }

    #[test]
    fn labels_drop_the_shared_path() {
        assert_eq!(relative(&["src/app", "src/domain"]), ["app", "domain"]);
        assert_eq!(
            relative(&["a/x.py", "a/y.py", "a/x.py"]),
            ["x.py", "y.py", "x.py"]
        );
        assert_eq!(relative(&["a/b", "a"]), ["a/b", "a"]);
    }
}
