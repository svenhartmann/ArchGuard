//! Architecture snapshot (`architectureExport` in
//! `viewer_architecture.js`) and the "Target model" section of
//! `exportMarkdown`.

use archguard_engine::core::TargetArchitectureReport;
use serde_json::{Value, json};

use super::cells::{Lines, cell, row, text, vcell};
use super::snapshot::{Snapshot, add_filters, base, extend};

const NOTE: &str = "Allowed but unobserved directions are policy permissions, not missing dependencies. Mapping coverage does not establish policy conformance. Original target assessment is separate from dependency-graph simulation.";

fn violates(edge: &Value) -> bool {
    edge["allowed"] != true
        || edge["private_accesses"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
}

/// `architectureRelations`: allowed directions, overwritten by observed
/// dependencies, filtered and violations first.
fn relations(model: &Value, only_violations: bool, query: &str) -> Vec<Value> {
    let key = |e: &Value| (text(&e["from"]), text(&e["to"]));
    let mut rows: Vec<Value> = Vec::new();
    let allowed = model["allowed_dependencies"].as_array().into_iter().flatten().map(|e| {
        json!({"from": e["from"], "to": e["to"], "allowed": true, "source_edges": 0, "import_references": 0, "count_exact": true, "evidence": [], "evidence_total": 0})
    });
    let observed = model["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .cloned();
    for edge in allowed.chain(observed) {
        match rows.iter_mut().find(|r| key(r) == key(&edge)) {
            Some(slot) => *slot = edge,
            None => rows.push(edge),
        }
    }
    let query = query.trim().to_lowercase();
    rows.retain(|e| {
        (!only_violations || violates(e))
            && format!("{} {}", text(&e["from"]), text(&e["to"]))
                .to_lowercase()
                .contains(&query)
    });
    rows.sort_by(|a, b| {
        violates(b)
            .cmp(&violates(a))
            .then_with(|| key(a).cmp(&key(b)))
    });
    rows
}

fn kind(edge: &Value) -> &'static str {
    match () {
        _ if edge["allowed"] != true => "forbidden",
        _ if violates(edge) => "private_access",
        _ => "allowed",
    }
}

/// The snapshot of the target architecture; `model` is `None` without
/// `[architecture.model]` (status "unavailable").
pub fn architecture_snapshot(
    project: &str,
    model: Option<&TargetArchitectureReport>,
    only_violations: bool,
    query: &str,
) -> Snapshot {
    let m = json!(model);
    let edges: Vec<Value> = relations(&m, only_violations, query)
        .into_iter()
        .map(|mut e| {
            e["kind"] = json!(kind(&e));
            e["count"] = e["import_references"].clone();
            e
        })
        .collect();
    let mut map = base(project, "architecture");
    extend(
        &mut map,
        json!({
            "kind": "architecture",
            "status": model.map_or("unavailable", |m| m.status.as_str()),
            "coverage": "Declared target architecture and original full-source dependency observations; graph simulations do not update this assessment.",
        }),
    );
    add_filters(
        &mut map,
        json!({"only_violations": only_violations, "query": query}),
    );
    let count = |k: &str| m.get(k).cloned().unwrap_or(json!(0));
    let mut notes = vec![json!(NOTE)];
    notes.extend(m["limitations"].as_array().cloned().unwrap_or_default());
    extend(
        &mut map,
        json!({
            "model": m,
            "metrics": {
                "source_files": count("source_nodes"), "mapped_files": count("mapped_nodes"),
                "forbidden_directions": count("violating_dependencies"), "interface_violations": count("violating_interfaces"),
                "coupling_violations": count("coupling_violations"), "unclassified_connections": count("unclassified_edges"),
            },
            "nodes": m["components"].as_array().into_iter().flatten().map(|c| json!({"id": c["name"], "name": c["name"]})).collect::<Vec<_>>(),
            "edges": edges,
            "count_unit": "source import references",
            "notes": notes,
            "diagnostics": [],
        }),
    );
    Snapshot(Value::Object(map))
}

fn list(v: &Value, sep: &str) -> String {
    v.as_array()
        .into_iter()
        .flatten()
        .map(vcell)
        .collect::<Vec<_>>()
        .join(sep)
}

fn or_dash(v: &Value) -> String {
    if v.is_null() { "—".into() } else { text(v) }
}

/// The "Target model" section; nothing without a model.
pub(super) fn append_model(lines: &mut Lines, model: &Value) {
    if model.is_null() {
        return;
    }
    lines.extend(["", "## Target model", "", "The complete model and assignment coverage below ignore relationship filters. The directed graph, relationship table and source evidence use the exported filters.", ""].map(String::from));
    lines.push(format!(
        "Model: {}. Gate severity: {}. Require assignment: {}.",
        vcell(&model["name"]),
        vcell(&model["severity"]),
        vcell(&model["require_assignment"])
    ));
    lines.extend(
        [
            "",
            "| Component | Include patterns | Assigned files | Source files |",
            "| --- | --- | ---: | --- |",
        ]
        .map(String::from),
    );
    let components = model["components"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    for c in components {
        lines.push(row(&[
            vcell(&c["name"]),
            list(&c["include"], "<br>"),
            vcell(&c["node_count"]),
            list(&c["nodes"], "<br>"),
        ]));
    }
    lines.extend(["", "### Public interfaces and coupling limits", "", "| Component | Public interface | Incoming components | Incoming limit | Outgoing components | Outgoing limit |", "| --- | --- | --- | ---: | --- | ---: |"].map(String::from));
    for c in components {
        let public = match c["public"].as_array() {
            None => "Unrestricted".to_string(),
            Some(p) if p.is_empty() => "Closed".into(),
            Some(p) => p.iter().map(text).collect::<Vec<_>>().join(", "),
        };
        let joined = |k: &str| {
            c[k].as_array()
                .into_iter()
                .flatten()
                .map(text)
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(row(&[
            vcell(&c["name"]),
            cell(&public),
            cell(&joined("incoming_components")),
            cell(&or_dash(&c["max_incoming_components"])),
            cell(&joined("outgoing_components")),
            cell(&or_dash(&c["max_outgoing_components"])),
        ]));
    }
    lines.extend(["", "### Private source access (complete model)", ""].map(String::from));
    for edge in model["dependencies"].as_array().into_iter().flatten() {
        for access in edge["private_accesses"].as_array().into_iter().flatten() {
            let path = [&edge["from"], &edge["to"], &access["from"], &access["to"]].map(vcell);
            lines.push(format!("- {}", path.join(" → ")));
            for r in access["evidence"].as_array().into_iter().flatten() {
                lines.push(format!(
                    "  - {}",
                    cell(&format!(
                        "{}:{} {}",
                        text(&r["file"]),
                        text(&r["line"]),
                        text(&r["raw"])
                    ))
                ));
            }
        }
    }
    append_directions(lines, model);
}

fn append_directions(lines: &mut Lines, model: &Value) {
    lines.extend(
        [
            "",
            "### Declared allowed directions",
            "",
            "| Source | Target |",
            "| --- | --- |",
        ]
        .map(String::from),
    );
    let allowed = model["allowed_dependencies"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    lines.extend(
        allowed
            .iter()
            .map(|e| row(&[vcell(&e["from"]), vcell(&e["to"])])),
    );
    if allowed.is_empty() {
        lines.extend([
            String::new(),
            "No cross-component directions are allowed.".into(),
        ]);
    }
    append_assignment(lines, model);
}

fn append_assignment(lines: &mut Lines, model: &Value) {
    let v = |k: &str| vcell(&model[k]);
    lines.extend(["", "### Assignment coverage", ""].map(String::from));
    lines.push(format!(
        "{} / {} source files assigned. {} unassigned; {} ambiguous.",
        v("mapped_nodes"),
        v("source_nodes"),
        v("unmapped_nodes"),
        v("ambiguous_nodes")
    ));
    lines.push(String::new());
    lines.push(format!("{} / {} source connections classified; {} unclassified. Classified connections include internal component dependencies.", v("evaluated_edges"), v("total_edges"), v("unclassified_edges")));
    lines.extend(["", "| Unassigned source file |", "| --- |"].map(String::from));
    let unmapped = model["unmapped"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    lines.extend(unmapped.iter().map(|n| row(&[vcell(n)])));
    if unmapped.is_empty() {
        lines.extend([String::new(), "No unassigned source files.".into()]);
    }
    lines.extend(
        [
            "",
            "| Ambiguous source file | Candidate components |",
            "| --- | --- |",
        ]
        .map(String::from),
    );
    let ambiguous = model["ambiguous"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    lines.extend(
        ambiguous
            .iter()
            .map(|a| row(&[vcell(&a["node"]), list(&a["components"], "<br>")])),
    );
    if ambiguous.is_empty() {
        lines.extend([String::new(), "No ambiguous assignments.".into()]);
    }
    let empty = list(&model["empty_components"], ", ");
    lines.extend([
        String::new(),
        format!(
            "Components without assigned files: {}.",
            if empty.is_empty() {
                "none".into()
            } else {
                empty
            }
        ),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> TargetArchitectureReport {
        let component = |name: &str, nodes: &[&str]| json!({"name": name, "include": [format!("src/{name}/**")], "node_count": nodes.len(), "nodes": nodes, "public": null});
        serde_json::from_value(json!({
            "schema_version": 1, "name": "layers", "status": "complete", "severity": "error", "require_assignment": false,
            "source_nodes": 3, "mapped_nodes": 2, "unmapped_nodes": 1, "ambiguous_nodes": 0,
            "total_edges": 2, "evaluated_edges": 2, "unclassified_edges": 0, "violating_dependencies": 1,
            "components": [component("ui", &["src/ui/a.ts"]), component("core", &["src/core/b.ts"])],
            "allowed_dependencies": [{"from": "ui", "to": "core"}],
            "dependencies": [{"from": "core", "to": "ui", "allowed": false, "source_edges": 1, "import_references": 2, "count_exact": true, "evidence_total": 1,
                "evidence": [{"file": "src/core/b.ts", "line": 3, "raw": "import '../ui/a'"}]}],
            "unmapped": ["src/main.ts"], "ambiguous": [], "empty_components": [], "limitations": ["Static imports only."],
        })).expect("model")
    }

    #[test]
    fn architecture_json_lists_violations_first() {
        let v = architecture_snapshot("demo", Some(&model()), false, "").0;
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["kind"], "architecture");
        assert_eq!(v["status"], "complete");
        assert_eq!(v["metrics"]["forbidden_directions"], 1);
        assert_eq!(v["edges"][0]["kind"], "forbidden");
        assert_eq!(v["edges"][0]["count"], 2);
        assert_eq!(v["edges"][1]["kind"], "allowed");
        assert_eq!(v["notes"][1], "Static imports only.");
        let only = architecture_snapshot("demo", Some(&model()), true, "").0;
        assert_eq!(only["edges"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn architecture_markdown_has_the_model_and_mermaid() {
        let md = architecture_snapshot("demo", Some(&model()), false, "").markdown();
        assert!(md.contains("## Target model"), "{md}");
        assert!(md.contains("| src/main.ts |"), "{md}");
        assert!(md.contains("n1 -->|\"forbidden\"| n0"), "{md}");
        assert!(md.contains("| ui | Unrestricted |"), "{md}");
    }

    #[test]
    fn without_a_model_the_status_is_unavailable() {
        let s = architecture_snapshot("demo", None, false, "");
        assert_eq!(s.0["status"], "unavailable");
        assert_eq!(s.0["metrics"]["source_files"], 0);
        assert!(!s.markdown().contains("## Target model"));
    }
}
