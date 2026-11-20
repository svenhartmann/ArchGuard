//! The target architecture page ([`super::target`]) as one `ArcTarget`.

use std::collections::HashSet;

use archguard_engine::core::{ArchitectureDependency, Graph, MappedArchitectureComponent, Ref};

use super::target::{Row, TargetView, rows};
use crate::tab_support::model as list;
use crate::ui::{ArcTarget, ArcTargetComponent, ArcTargetRef, ArcTargetRelation};

// shortcut: long lists stop at a cap with the rest counted; add paging
// when a project needs to browse past it here instead of in the graph.
/// Evidence records and private accesses of the selected direction.
const EVIDENCE: usize = 50;
const ACCESSES: usize = 20;
/// Records shown per private access.
const ACCESS_REFS: usize = 5;
/// Files of an open component, unassigned and ambiguous files.
const FILES: usize = 200;

fn int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

fn limit(n: Option<u32>) -> i32 {
    n.map_or(-1, |n| i32::try_from(n).unwrap_or(i32::MAX))
}

fn relation(v: &TargetView, row: &Row<'_>) -> ArcTargetRelation {
    ArcTargetRelation {
        from: row.from.into(),
        to: row.to.into(),
        state: row.state as i32,
        refs: row.dep.map_or("0".into(), |d| {
            let bound = if d.count_exact { "" } else { "+" };
            format!("{}{bound}", d.import_references).into()
        }),
        selected: v
            .selected
            .as_ref()
            .is_some_and(|(f, t)| f == row.from && t == row.to),
    }
}

/// The page's window model; `graph` decides which lines open in the graph.
pub(super) fn model(v: &TargetView, graph: &Graph) -> ArcTarget {
    let r = match &v.report {
        Ok(None) => return ArcTarget::default(),
        Err(e) => {
            return ArcTarget {
                state: 2,
                error: e.as_str().into(),
                ..Default::default()
            };
        }
        Ok(Some(r)) => r,
    };
    let nodes: HashSet<&str> = graph.nodes.iter().map(|n| n.name.as_str()).collect();
    let file = |name: &str| ArcTargetRef {
        text: name.into(),
        node: if nodes.contains(name) {
            name.into()
        } else {
            "".into()
        },
        head: false,
    };
    let all = rows(v, r);
    let mut m = ArcTarget {
        state: 1,
        name: r.name.as_str().into(),
        mapped: int(r.mapped_nodes),
        total: int(r.source_nodes),
        forbidden: int(r.violating_dependencies),
        interfaces: int(r.violating_interfaces),
        coupling: int(r.coupling_violations),
        complete: r.status == "complete",
        only_violations: v.only_violations,
        query: v.query.as_str().into(),
        matching: int(all.len()),
        more: all.len() > v.limit,
        relations: list(all.iter().take(v.limit).map(|row| relation(v, row))),
        components: list(r.components.iter().map(|c| component(v, c, &file))),
        unmapped: list(r.unmapped.iter().take(FILES).map(|n| file(n))),
        unmapped_count: int(r.unmapped_nodes),
        ambiguous: list(
            r.ambiguous
                .iter()
                .take(FILES)
                .map(|a| format!("{} → {}", a.node, a.components.join(", ")).into()),
        ),
        ambiguous_count: int(r.ambiguous_nodes),
        unclassified: int(r.unclassified_edges),
        empty: r.empty_components.join(", ").into(),
        limitations: r.limitations.join("\n").into(),
        ..Default::default()
    };
    let selected = all.iter().find(|row| {
        v.selected
            .as_ref()
            .is_some_and(|(f, t)| f == row.from && t == row.to)
    });
    if let Some(row) = selected {
        m.selection = format!("{} → {}", row.from, row.to).into();
        m.sel_state = row.state as i32;
        if let Some(d) = row.dep {
            evidence(&mut m, d, &nodes);
        }
    }
    m
}

fn reference(r: &Ref, nodes: &HashSet<&str>) -> ArcTargetRef {
    ArcTargetRef {
        text: format!("{}:{}  {}", r.file, r.line, r.raw.trim()).into(),
        node: if nodes.contains(r.file.as_str()) {
            r.file.as_str().into()
        } else {
            "".into()
        },
        head: false,
    }
}

/// The selected direction's sampled evidence and private accesses.
fn evidence(m: &mut ArcTarget, d: &ArchitectureDependency, nodes: &HashSet<&str>) {
    m.sel_edges = int(d.source_edges);
    m.evidence_total = int(d.evidence_total);
    m.sampled = d.evidence_total > d.evidence.len()
        || d.import_references > d.evidence_total as u64
        || !d.count_exact;
    m.evidence = list(
        d.evidence
            .iter()
            .take(EVIDENCE)
            .map(|r| reference(r, nodes)),
    );
    m.private_total = int(d.private_accesses.len());
    let mut private = vec![];
    for access in d.private_accesses.iter().take(ACCESSES) {
        private.push(ArcTargetRef {
            text: format!("{} → {}", access.from, access.to).into(),
            node: if nodes.contains(access.to.as_str()) {
                access.to.as_str().into()
            } else {
                "".into()
            },
            head: true,
        });
        private.extend(
            access
                .evidence
                .iter()
                .take(ACCESS_REFS)
                .map(|r| reference(r, nodes)),
        );
    }
    m.private = list(private);
}

fn component(
    v: &TargetView,
    c: &MappedArchitectureComponent,
    file: &dyn Fn(&str) -> ArcTargetRef,
) -> ArcTargetComponent {
    let open = v.open.contains(&c.name);
    let listed = if open { c.nodes.len().min(FILES) } else { 0 };
    ArcTargetComponent {
        name: c.name.as_str().into(),
        files: int(c.node_count),
        include: c.include.join(", ").into(),
        has_public: c.public.is_some(),
        public: c.public.as_deref().unwrap_or_default().join(", ").into(),
        incoming: int(c.incoming_components.len()),
        outgoing: int(c.outgoing_components.len()),
        max_in: limit(c.max_incoming_components),
        max_out: limit(c.max_outgoing_components),
        open,
        nodes: list(c.nodes.iter().take(listed).map(|n| file(n))),
        more_files: if open { int(c.nodes.len() - listed) } else { 0 },
    }
}

#[cfg(test)]
mod tests {
    use super::super::target::{State, TargetView, build, select, toggle_component};
    use super::*;
    use archguard_engine::core::Config;
    use slint::Model as _;
    use std::path::Path;

    /// py-layers with a declared target model: Domain → App is forbidden
    /// (ledger imports app.service), App reaches past Domain's public
    /// money.py into order.py, Api → Legacy is allowed but unused,
    /// Legacy is empty and app/report.py unassigned.
    const MODEL: &str = r#"
[architecture.model]
name = "Order service"
[[architecture.model.components]]
name = "Api"
include = ["api/**"]
max_outgoing_components = 0
[[architecture.model.components]]
name = "App"
include = ["app/service.py", "app/usecase.py"]
[[architecture.model.components]]
name = "Domain"
include = ["domain/**"]
public = ["domain/money.py"]
[[architecture.model.components]]
name = "Legacy"
include = ["legacy/**"]
[[architecture.model.allowed_dependencies]]
from = "Api"
to = "App"
[[architecture.model.allowed_dependencies]]
from = "App"
to = "Domain"
[[architecture.model.allowed_dependencies]]
from = "Api"
to = "Legacy"
"#;

    /// A temp copy of py-layers with `extra` appended to its archguard.toml,
    /// analyzed: its folder (kept alive), config and graph.
    fn fixture(extra: &str) -> (tempfile::TempDir, Config, Graph) {
        let dir = tempfile::tempdir().expect("temp dir");
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/py-layers");
        let toml = std::fs::read_to_string(from.join("archguard.toml")).expect("toml");
        std::fs::write(dir.path().join("archguard.toml"), toml + extra).expect("write");
        for package in std::fs::read_dir(from.join("src")).expect("src").flatten() {
            let to = dir.path().join("src").join(package.file_name());
            std::fs::create_dir_all(&to).expect("package dir");
            for file in std::fs::read_dir(package.path())
                .expect("package")
                .flatten()
            {
                std::fs::copy(file.path(), to.join(file.file_name())).expect("copy");
            }
        }
        let loaded = archguard_engine::load_config(None, dir.path()).expect("config");
        let run = archguard_engine::analyze_run_quiet(
            dir.path(),
            &loaded,
            archguard_engine::Level::Module,
        )
        .expect("analysis");
        (dir, run.config, run.graph)
    }

    fn shown(v: &TargetView, g: &Graph) -> Vec<(String, i32)> {
        model(v, g)
            .relations
            .iter()
            .map(|r| (format!("{}>{}", r.from, r.to), r.state))
            .collect()
    }

    #[test]
    fn without_a_model_the_page_says_how_to_declare_one() {
        let (_dir, config, graph) = fixture("");
        let v = build(&graph, &config);
        assert!(matches!(v.report, Ok(None)));
        assert_eq!(model(&v, &graph).state, 0);
    }

    /// Violations first, then by name; the unused allowed direction is
    /// listed too; the filter and "Only violations" narrow the list.
    #[test]
    fn directions_list_violations_first_and_filter() {
        let (_dir, config, graph) = fixture(MODEL);
        let mut v = build(&graph, &config);
        let m = model(&v, &graph);
        assert_eq!(m.state, 1);
        assert_eq!(
            (m.name.as_str(), m.forbidden, m.complete),
            ("Order service", 1, false)
        );
        assert_eq!(
            (m.coupling, m.interfaces),
            (1, 2),
            "Api's limit; service.py and usecase.py → order.py"
        );
        let row = |s: &str, st: State| (s.to_string(), st as i32);
        assert_eq!(
            shown(&v, &graph),
            [
                row("App>Domain", State::Private),
                row("Domain>App", State::Forbidden),
                row("Api>App", State::Allowed),
                row("Api>Legacy", State::NotObserved),
            ]
        );
        v.only_violations = true;
        assert_eq!(shown(&v, &graph).len(), 2);
        v.only_violations = false;
        v.query = "legacy".into();
        assert_eq!(shown(&v, &graph), [row("Api>Legacy", State::NotObserved)]);
    }

    /// A page holds `limit` directions; `more` says there is a next one.
    #[test]
    fn directions_come_in_pages() {
        let (_dir, config, graph) = fixture(MODEL);
        let mut v = build(&graph, &config);
        assert_eq!(v.limit, 100);
        v.limit = 2;
        let m = model(&v, &graph);
        assert_eq!((m.relations.row_count(), m.matching, m.more), (2, 4, true));
    }

    /// Selecting a direction shows its file:line evidence and private
    /// accesses, each opening its file in the graph; selecting it again
    /// closes it.
    #[test]
    fn a_selected_direction_shows_evidence_and_private_access() {
        let (_dir, config, graph) = fixture(MODEL);
        let mut v = build(&graph, &config);
        select(&mut v, 1);
        let m = model(&v, &graph);
        assert_eq!(m.selection, "Domain → App");
        let e = m.evidence.row_data(0).expect("evidence");
        assert!(e.text.starts_with("domain/ledger.py:2"), "{}", e.text);
        assert_eq!(e.node, "domain/ledger.py");
        select(&mut v, 0);
        let m = model(&v, &graph);
        assert_eq!(m.selection, "App → Domain");
        assert_eq!(m.private_total, 2);
        let head = m.private.row_data(0).expect("access");
        assert!(
            head.head && head.text.ends_with("→ domain/order.py"),
            "{}",
            head.text
        );
        assert_eq!(head.node, "domain/order.py");
        select(&mut v, 0);
        assert_eq!(v.selected, None);
    }

    /// Per component: include patterns, public interface, coupling counts
    /// against limits and the file list only once opened; coverage.
    #[test]
    fn components_and_mapping_coverage() {
        let (_dir, config, graph) = fixture(MODEL);
        let mut v = build(&graph, &config);
        let c = |v: &TargetView, i| model(v, &graph).components.row_data(i).expect("component");
        let api = c(&v, 0);
        assert_eq!(
            (api.name.as_str(), api.outgoing, api.max_out, api.max_in),
            ("Api", 1, 0, -1)
        );
        assert_eq!(api.include, "api/**");
        assert_eq!(api.nodes.row_count(), 0, "files on demand");
        let domain = c(&v, 2);
        assert!(domain.has_public && domain.public == "domain/money.py");
        toggle_component(&mut v, 0);
        let api = c(&v, 0);
        assert!(api.open);
        assert_eq!(api.nodes.row_count(), 2);
        assert_eq!(api.nodes.row_data(0).unwrap().node, "api/http.py");
        let m = model(&v, &graph);
        assert_eq!(m.unmapped_count, 1);
        assert_eq!(m.unmapped.row_data(0).unwrap().text, "app/report.py");
        assert_eq!(m.empty, "Legacy");
    }
}
