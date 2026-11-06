//! Services snapshot: the `services` branch of `exportSnapshot()` with
//! `serviceMatches` and `serviceEdgeInCycle` of `viewer_services.js`.

use archguard_engine::core::{ServiceDependency, ServiceDependencyKind, ServiceTopology};
use serde_json::{Value, json};

use super::cells::{Lines, cell, location, row, text, vcell};
use super::snapshot::{Snapshot, add_filters, base, extend};

/// The Services tab's settings (`serviceUi`).
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceFilters {
    /// The configuration kind shown.
    pub kind: ServiceDependencyKind,
    /// "all", "incoming", "outgoing" or "both".
    pub direction: String,
    /// Id of the selected service.
    pub reference: Option<String>,
    /// Id of the selected project.
    pub selected_project: Option<String>,
    pub cycles_only: bool,
    /// Members of the focused cycle.
    pub focused_cycle: Option<Vec<String>>,
}

impl Default for ServiceFilters {
    /// The web viewer's start state.
    fn default() -> Self {
        ServiceFilters {
            kind: ServiceDependencyKind::ConfiguredHttp,
            direction: "all".into(),
            reference: None,
            selected_project: None,
            cycles_only: false,
            focused_cycle: None,
        }
    }
}

const COVERAGE: &str = "Visible service connections of the selected configuration kind; full repository project inventory independent of the visible inventory page.";
const NOTES: [&str; 2] = [
    "Project identities include manifest paths; matching package names or technologies do not establish service connections. Libraries and test tools are inventory, not additional runtime services.",
    "Configured HTTP endpoints are not evidence of executed runtime calls. Startup dependencies describe depends_on order. Each edge kind has its own cycles.",
];

fn in_cycle(t: &ServiceTopology, f: &ServiceFilters, e: &ServiceDependency) -> bool {
    t.cycles
        .iter()
        .filter(|c| c.kind == f.kind)
        .any(|c| c.nodes.contains(&e.from) && c.nodes.contains(&e.to))
}

/// `serviceMatches`.
fn matches(t: &ServiceTopology, f: &ServiceFilters, e: &ServiceDependency) -> bool {
    if f.cycles_only && !in_cycle(t, f, e) {
        return false;
    }
    if let Some(c) = &f.focused_cycle
        && (!c.contains(&e.from) || !c.contains(&e.to))
    {
        return false;
    }
    let Some(selected) = f.reference.as_deref().filter(|_| f.direction != "all") else {
        return true;
    };
    let (incoming, outgoing) = (e.to == selected, e.from == selected);
    match f.direction.as_str() {
        "incoming" => incoming,
        "outgoing" => outgoing,
        _ => incoming || outgoing,
    }
}

/// The snapshot of the Services tab.
pub fn services_snapshot(project: &str, t: &ServiceTopology, f: &ServiceFilters) -> Snapshot {
    let all: Vec<_> = t.dependencies.iter().filter(|e| e.kind == f.kind).collect();
    let shown: Vec<_> = all.iter().filter(|e| matches(t, f, e)).collect();
    let cycles = t.cycles.iter().filter(|c| c.kind == f.kind).count();
    let mut map = base(project, "services");
    extend(
        &mut map,
        json!({
            "kind": "services",
            "status": if t.complete { "complete" } else { "partial" },
            "coverage": COVERAGE,
            "monorepo": t.monorepo,
            "monorepo_reason": t.monorepo_reason,
            "compose_files": t.compose_files,
            "projects": t.projects,
        }),
    );
    add_filters(
        &mut map,
        json!({
            "kind": f.kind, "direction": f.direction, "reference": f.reference,
            "selected_project": f.selected_project, "cycles_only": f.cycles_only, "focused_cycle": f.focused_cycle,
        }),
    );
    extend(
        &mut map,
        json!({
            "metrics": {
                "scope_nodes": t.services.len(), "scope_dependencies": all.len(),
                "shown_nodes": t.services.len(), "shown_dependencies": shown.len(), "cyclic_groups": cycles,
            },
            "nodes": t.services,
            "edges": shown.iter().map(|e| json!({
                "from": e.from, "to": e.to, "kind": e.kind, "count": e.evidence.len(), "count_exact": true,
                "cyclic_current_scope": in_cycle(t, f, e), "evidence": e.evidence,
            })).collect::<Vec<_>>(),
            "count_unit": "configuration evidence records",
            "diagnostics": t.diagnostics,
            "notes": NOTES,
        }),
    );
    Snapshot(Value::Object(map))
}

/// The "Project inventory" section of `exportMarkdown`.
pub(super) fn append_projects(lines: &mut Lines, s: &Value) {
    let projects = s["projects"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if projects.is_empty() {
        return;
    }
    lines.extend(
        [
            "",
            "## Project inventory",
            "",
            &vcell(&s["monorepo_reason"]),
            "",
            "| Project | Path | Kind | Ecosystem | Runtime | Manifest |",
            "| --- | --- | --- | --- | --- | --- |",
        ]
        .map(String::from),
    );
    for p in projects {
        let cells =
            ["name", "path", "kind", "ecosystem", "runtime", "manifest"].map(|k| vcell(&p[k]));
        lines.push(row(&cells));
    }
    lines.extend(
        [
            "",
            "### Project evidence",
            "",
            "| Project | Location | Evidence |",
            "| --- | --- | --- |",
        ]
        .map(String::from),
    );
    for p in projects {
        for ev in p["evidence"].as_array().into_iter().flatten() {
            lines.push(row(&[
                vcell(&p["id"]),
                cell(&location(ev)),
                cell(&text(&ev["description"])),
            ]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn shop() -> ServiceTopology {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/service-shop");
        archguard_engine::analyze_services(&root).expect("topology")
    }

    #[test]
    fn services_json_counts_the_shown_connections() {
        let t = shop();
        let f = ServiceFilters {
            kind: ServiceDependencyKind::Startup,
            ..Default::default()
        };
        let startup = t.dependencies.iter().filter(|e| e.kind == f.kind).count();
        assert!(startup > 0, "service-shop declares depends_on");
        let v = services_snapshot("service-shop", &t, &f).0;
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["kind"], "services");
        assert_eq!(v["filters"]["kind"], "startup");
        assert_eq!(v["metrics"]["scope_dependencies"], startup);
        assert_eq!(v["metrics"]["shown_dependencies"], startup);
        assert_eq!(v["nodes"].as_array().unwrap().len(), t.services.len());
        let first = &v["edges"][0];
        assert_eq!(first["count"], first["evidence"].as_array().unwrap().len());
    }

    #[test]
    fn direction_filter_keeps_the_selected_service_s_edges() {
        let t = shop();
        let any = t
            .dependencies
            .iter()
            .find(|e| e.kind == ServiceDependencyKind::Startup)
            .expect("edge");
        let f = ServiceFilters {
            kind: ServiceDependencyKind::Startup,
            direction: "outgoing".into(),
            reference: Some(any.from.clone()),
            ..Default::default()
        };
        let v = services_snapshot("service-shop", &t, &f).0;
        let edges = v["edges"].as_array().unwrap();
        assert!(!edges.is_empty());
        assert!(edges.iter().all(|e| e["from"] == any.from.as_str()));
    }

    #[test]
    fn services_markdown_lists_projects_and_mermaid() {
        let f = ServiceFilters {
            kind: ServiceDependencyKind::Startup,
            ..Default::default()
        };
        let md = services_snapshot("service-shop", &shop(), &f).markdown();
        assert!(md.contains("```mermaid\nflowchart LR\n"), "{md}");
        assert!(md.contains("-->|\"startup\"|"), "{md}");
        assert!(md.contains("## Project inventory"), "{md}");
        assert!(md.contains("| kind | startup |"), "{md}");
    }
}
