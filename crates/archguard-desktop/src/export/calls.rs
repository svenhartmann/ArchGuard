//! Calls snapshot: the `calls` branch of `exportSnapshot()` in
//! `viewer_export.js` — the selected call-cycle group with its witness,
//! edges and call sites in Cycles mode, else the matching connections;
//! Rankings mode adds the ranked rows.

use archguard_engine::core::CallAnalysis;
use serde_json::{Map, Value, json};

use super::cells::{Lines, cell, row};
use super::snapshot::{Snapshot, add_filters, base, extend};

/// One directed call connection with its call sites (indices into
/// `CallAnalysis::calls`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CallEdge {
    pub from: String,
    /// The target id, or the target label of an unresolved/external call.
    pub to: String,
    pub resolution: String,
    pub calls: Vec<usize>,
}

/// A cyclic call group (`callCycleUi.groups`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CallCycle {
    pub id: String,
    /// "class" or "method".
    pub kind: String,
    pub members: Vec<String>,
    /// One closed example path.
    pub witness: Vec<String>,
    pub edges: Vec<CallEdge>,
    pub call_sites: usize,
}

/// One row of "Most calls".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CallRank {
    pub name: String,
    pub incoming: usize,
    pub outgoing: usize,
    pub total: usize,
    pub cyclomatic: u32,
    pub max_nesting: u32,
}

/// What the Calls tab shows, as plain data.
#[derive(Debug, Clone, PartialEq)]
pub struct CallsInput<'a> {
    pub project: String,
    pub analysis: &'a CallAnalysis,
    /// The toolbar filters under the web's names (`language`, `package`,
    /// `query`, `resolution`, `include_internal`, `element`).
    pub filters: Value,
    /// Node id → shown name, in order.
    pub nodes: Vec<(String, String)>,
    /// Connections and Rankings mode: the matching connections.
    pub edges: Vec<CallEdge>,
    /// Cycles mode: the matching groups and the selected one's index.
    pub cycles: Option<(Vec<CallCycle>, Option<usize>)>,
    /// Rankings mode: level ("class"/"method"), sort and the ordered rows.
    pub rankings: Option<(String, String, Vec<CallRank>)>,
}

const COVERAGE: &str = "All matching class relations, including additional list pages. Original call-analysis snapshot; dependency simulations do not rewrite call sites.";
const CYCLE_COVERAGE: &str = "Complete selected call-cycle group and all its internal directed edges and source call sites, including additional list pages. The catalogue contains every matching cyclic group. Original call-analysis snapshot; dependency simulations do not rewrite call sites.";
const NOTE: &str = "Static call resolution is not a runtime trace. Partial or unavailable analysis does not establish the absence of other calls.";
const CYCLE_NOTES: [&str; 3] = [
    "Language, package and search select whole groups by their members. Return paths outside the selected package remain included. Element focus and connection target/internal-call preferences do not filter cycle groups.",
    "Type/module cycles aggregate calls between different owners and do not establish a continuous function/method-call cycle. Module owners group top-level functions, not declared classes. Method cycles retain exact declared function/method identities, including overloads, internal calls and direct recursion.",
    "Static call resolution is not a runtime trace or proof of executed recursion. Partial or unavailable analysis and missing method identities do not establish the absence of other cycles.",
];

/// The snapshot of the Calls tab.
pub fn calls_snapshot(input: &CallsInput) -> Snapshot {
    let a = input.analysis;
    let mut map = base(&input.project, "calls");
    let cycles = input.cycles.as_ref();
    let selected = cycles.and_then(|(groups, at)| at.map(|i| &groups[i]));
    let edges = selected.map_or(input.edges.as_slice(), |g| g.edges.as_slice());
    let sites: usize = edges.iter().map(|e| e.calls.len()).sum();
    let view = match (cycles, &input.rankings) {
        (Some(_), _) => "cycles",
        (None, Some(_)) => "rankings",
        (None, None) => "connections",
    };
    extend(
        &mut map,
        json!({"kind": "calls", "status": a.status,
            "coverage": if cycles.is_some() { CYCLE_COVERAGE } else { COVERAGE }}),
    );
    add_filters(&mut map, json!({"call_view": view}));
    add_filters(&mut map, input.filters.clone());
    if let Some((groups, _)) = cycles {
        add_cycles(&mut map, groups, selected);
    }
    let mut metrics = json!({"shown_nodes": input.nodes.len(), "shown_dependencies": edges.len(), "call_sites": sites});
    if let Some((groups, _)) = cycles {
        metrics["cyclic_groups"] = json!(groups.len());
    }
    let notes: Vec<&str> = if cycles.is_some() {
        CYCLE_NOTES.to_vec()
    } else {
        vec![NOTE]
    };
    extend(
        &mut map,
        json!({
            "analyzers": a.analyzers,
            "metrics": metrics,
            "nodes": input.nodes.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
            "edges": edges.iter().map(|e| edge(e, a, cycles.is_some())).collect::<Vec<_>>(),
            "count_unit": "static call sites",
            "diagnostics": a.diagnostics,
            "notes": notes,
        }),
    );
    if let Some((level, sort, rows)) = &input.rankings {
        extend(
            &mut map,
            json!({"rankings": {"level": level, "sort": sort, "rows": rows.iter().map(rank).collect::<Vec<_>>()}}),
        );
    }
    Snapshot(Value::Object(map))
}

/// The cycle filters, the selected group and the catalogue.
fn add_cycles(map: &mut Map<String, Value>, groups: &[CallCycle], selected: Option<&CallCycle>) {
    add_filters(
        map,
        json!({"cycle_kind": groups.first().map(|g| g.kind.as_str()), "selected_cycle": selected.map(|g| &g.id)}),
    );
    extend(
        map,
        json!({
            "selected_cycle": selected.map(|g| cycle(g, true)),
            "cycle_groups": groups.iter().map(|g| cycle(g, false)).collect::<Vec<_>>(),
        }),
    );
}

fn cycle(g: &CallCycle, detail: bool) -> Value {
    let mut v = json!({"id": g.id, "kind": g.kind, "members": g.members, "witness": g.witness});
    if detail {
        v["edges"] = g
            .edges
            .iter()
            .map(|e| json!({"from": e.from, "to": e.to, "call_sites": e.calls.len()}))
            .collect();
    } else {
        v["edges"] = json!(g.edges.len());
        v["call_sites"] = json!(g.call_sites);
    }
    v
}

fn edge(e: &CallEdge, a: &CallAnalysis, cyclic: bool) -> Value {
    let evidence: Vec<_> = e.calls.iter().filter_map(|&i| a.calls.get(i)).collect();
    let mut v = json!({"from": e.from, "to": e.to, "kind": "call", "resolution": e.resolution,
        "count": evidence.len(), "count_exact": true, "evidence": evidence});
    if cyclic {
        v["cyclic_current_scope"] = json!(true);
    }
    v
}

fn rank(r: &CallRank) -> Value {
    json!({"name": r.name, "incoming": r.incoming, "outgoing": r.outgoing, "total": r.total,
        "cyclomatic": r.cyclomatic, "max_nesting": r.max_nesting})
}

fn list(v: &Value, sep: &str) -> String {
    let items = v.as_array().into_iter().flatten();
    items
        .map(|m| cell(m.as_str().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join(sep)
}

/// The call-cycle catalogue, the selected group and the rankings, after
/// the notes (`exportMarkdown`'s `cycle_groups` branch).
pub(super) fn append(lines: &mut Lines, s: &Value) {
    if let Some(groups) = s["cycle_groups"].as_array() {
        lines.extend(
            [
                "",
                "## Call-cycle catalogue",
                "",
                "| Level | Members | Directed edges | Static call sites | Closed example path |",
                "| --- | --- | ---: | ---: | --- |",
            ]
            .map(String::from),
        );
        for g in groups {
            let cells = [&g["kind"], &g["edges"], &g["call_sites"]].map(super::cells::vcell);
            let (members, path) = (list(&g["members"], ", "), list(&g["witness"], " → "));
            lines.push(row(&[
                cells[0].clone(),
                members,
                cells[1].clone(),
                cells[2].clone(),
                path,
            ]));
        }
        if groups.is_empty() {
            lines.extend(["", "No recorded cyclic groups match these member filters. See the analysis status and coverage limits above."].map(String::from));
        }
    }
    let g = &s["selected_cycle"];
    if g.is_object() {
        lines.extend([
            String::new(),
            format!("## Selected {} call-cycle group", super::cells::vcell(&g["kind"])),
            String::new(),
            format!("Members: {}.", list(&g["members"], ", ")),
            String::new(),
            format!("One closed example path: {}.", list(&g["witness"], " → ")),
            String::new(),
            "The directed graph and source evidence below include every internal edge of this selected group.".into(),
        ]);
    }
    append_rankings(lines, &s["rankings"]);
}

fn append_rankings(lines: &mut Lines, r: &Value) {
    let Some(rows) = r["rows"].as_array() else {
        return;
    };
    let head = format!(
        "Level: {}. Sorted by: {}.",
        super::cells::vcell(&r["level"]),
        super::cells::vcell(&r["sort"])
    );
    lines.extend(
        [
            "",
            "## Call rankings",
            "",
            &head,
            "",
            "| Name | Incoming | Outgoing | Total | Cyclomatic | Max. nesting |",
            "| --- | ---: | ---: | ---: | ---: | ---: |",
        ]
        .map(String::from),
    );
    for r in rows {
        let keys = [
            "name",
            "incoming",
            "outgoing",
            "total",
            "cyclomatic",
            "max_nesting",
        ];
        lines.push(row(&keys.map(|k| super::cells::vcell(&r[k]))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::MethodCall;

    fn analysis() -> CallAnalysis {
        let call = |from: &str, to: &str, line| MethodCall {
            from_class: from.into(),
            from_method: None,
            to_class: Some(to.into()),
            to_method: None,
            target_label: to.into(),
            dispatch: "static".into(),
            resolution: "project".into(),
            file: format!("{from}.java"),
            line,
            column: 3,
            code: format!("{to}.run()"),
        };
        CallAnalysis {
            status: "ready".into(),
            calls: vec![
                call("a.A", "b.B", 4),
                call("b.B", "a.A", 7),
                call("a.A", "b.B", 9),
            ],
            ..Default::default()
        }
    }

    fn input(a: &CallAnalysis) -> CallsInput<'_> {
        let edge = |from: &str, to: &str, calls: Vec<usize>| CallEdge {
            from: from.into(),
            to: to.into(),
            resolution: "project".into(),
            calls,
        };
        CallsInput {
            project: "demo".into(),
            analysis: a,
            filters: json!({"package": "a", "query": ""}),
            nodes: vec![("a.A".into(), "A".into()), ("b.B".into(), "B".into())],
            edges: vec![edge("a.A", "b.B", vec![0, 2]), edge("b.B", "a.A", vec![1])],
            cycles: None,
            rankings: None,
        }
    }

    #[test]
    fn connections_carry_their_call_sites() {
        let a = analysis();
        let v = calls_snapshot(&input(&a)).0;
        assert_eq!(v["kind"], "calls");
        assert_eq!(v["filters"]["workspace"], "calls");
        assert_eq!(v["filters"]["call_view"], "connections");
        assert_eq!(v["filters"]["package"], "a");
        assert_eq!(v["metrics"]["call_sites"], 3);
        assert_eq!(v["edges"][0]["count"], 2);
        assert_eq!(v["edges"][0]["evidence"][1]["line"], 9);
        assert!(v.get("cycle_groups").is_none());
        let md = calls_snapshot(&input(&a)).markdown();
        assert!(md.contains("| Matching static call sites | 3 |"), "{md}");
        assert!(
            md.contains("| a.A → b.B | a.A.java:9:3 | b.B.run() |"),
            "{md}"
        );
        assert!(md.contains("  n0 -->|\"2\"| n1"), "{md}");
    }

    #[test]
    fn cycles_export_the_selected_group_and_the_catalogue() {
        let a = analysis();
        let mut i = input(&a);
        let group = CallCycle {
            id: "g1".into(),
            kind: "class".into(),
            members: vec!["a.A".into(), "b.B".into()],
            witness: vec!["a.A".into(), "b.B".into(), "a.A".into()],
            edges: i.edges.clone(),
            call_sites: 3,
        };
        i.edges.clear();
        i.cycles = Some((vec![group], Some(0)));
        let s = calls_snapshot(&i);
        let v = &s.0;
        assert_eq!(v["filters"]["call_view"], "cycles");
        assert_eq!(v["filters"]["selected_cycle"], "g1");
        assert_eq!(v["selected_cycle"]["edges"][0]["call_sites"], 2);
        assert_eq!(v["cycle_groups"][0]["edges"], 2);
        assert_eq!(v["metrics"]["cyclic_groups"], 1);
        assert_eq!(v["metrics"]["shown_dependencies"], 2);
        assert_eq!(v["edges"][1]["cyclic_current_scope"], true);
        let md = s.markdown();
        assert!(
            md.contains("| class | a.A, b.B | 2 | 3 | a.A → b.B → a.A |"),
            "{md}"
        );
        assert!(
            md.contains("One closed example path: a.A → b.B → a.A."),
            "{md}"
        );
        assert!(md.contains("## Source evidence"), "{md}");
    }

    #[test]
    fn rankings_list_their_rows() {
        let a = analysis();
        let mut i = input(&a);
        let row = CallRank {
            name: "B".into(),
            incoming: 2,
            outgoing: 1,
            total: 3,
            ..Default::default()
        };
        i.rankings = Some(("class".into(), "incoming".into(), vec![row]));
        let s = calls_snapshot(&i);
        assert_eq!(s.0["filters"]["call_view"], "rankings");
        assert_eq!(s.0["rankings"]["rows"][0]["total"], 3);
        let md = s.markdown();
        assert!(md.contains("| B | 2 | 1 | 3 | 0 | 0 |"), "{md}");
    }
}
