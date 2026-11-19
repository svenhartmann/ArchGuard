//! Golden cases for [`plan_cycle_repairs`], taken from the web viewer's
//! `planCycleRepairs` (cycle_planner.js at 46b5462) run on the same graphs.

use archguard_core::{ArchguardError, Edge, Graph, Node, NodeId, NodeKind, Ref};

use crate::repair_plans::{DEFAULT_MAX_EXAMINED, RepairPlans, RepairStats, plan_cycle_repairs};

/// `names` in id order; `dir/file` gets the listed node `dir` as parent.
/// Each edge `(from, to, refs, count)` carries `refs` import lines in `from`.
fn graph(names: &[&str], edges: &[(&str, &str, u32, Option<u64>)]) -> Graph {
    let id = |n: &str| NodeId(names.iter().position(|x| *x == n).expect("listed") as u32);
    let parent = |n: &str| n.rsplit_once('/').map(|(dir, _)| id(dir));
    Graph {
        schema_version: 1,
        root: "/abs".into(),
        nodes: names
            .iter()
            .map(|n| Node {
                id: id(n),
                name: (*n).into(),
                kind: NodeKind::File,
                parent: parent(n),
                lang: None,
            })
            .collect(),
        edges: edges
            .iter()
            .map(|&(from, to, refs, count)| Edge {
                from: id(from),
                to: id(to),
                refs: (1..=refs)
                    .map(|line| Ref {
                        file: from.into(),
                        line,
                        raw: format!("import {to}"),
                    })
                    .collect(),
                count,
            })
            .collect(),
        semantic: None,
        review_kpis: None,
    }
}

fn plan(g: &Graph, members: &[&str], limit: usize) -> RepairPlans {
    let members: Vec<String> = members.iter().map(|m| (*m).into()).collect();
    plan_cycle_repairs(g, &members, limit).expect("members exist")
}

fn stats(groups: usize, nodes: usize, edges: usize, largest: usize) -> RepairStats {
    RepairStats {
        groups,
        cyclic_nodes: nodes,
        cyclic_edges: edges,
        largest_scc: largest,
    }
}

/// Each plan as `key: from>to,…`.
fn cuts(r: &RepairPlans) -> Vec<String> {
    r.plans
        .iter()
        .map(|p| {
            let ops: Vec<String> = p
                .ops
                .iter()
                .map(|op| match op {
                    crate::simulate::SimOp::CutEdge { from, to } => format!("{from}>{to}"),
                    other => panic!("only cuts: {other:?}"),
                })
                .collect();
            format!("{}: {}", p.key, ops.join(","))
        })
        .collect()
}

const ONE: Option<u64> = Some(1);

/// Two cycles a→b→a and a→b→c→a share a→b: the single cut of that edge
/// ranks first, ahead of the two-cut alternatives.
#[test]
fn one_cut_of_a_shared_edge_fixes_both_cycles() {
    let g = graph(
        &["a", "b", "c"],
        &[
            ("a", "b", 1, ONE),
            ("b", "a", 3, Some(3)),
            ("b", "c", 1, ONE),
            ("c", "a", 1, ONE),
        ],
    );
    let r = plan(&g, &["c", "a", "b", "a"], DEFAULT_MAX_EXAMINED);
    assert_eq!(r.scope_names, ["a", "b", "c"], "deduplicated and sorted");
    assert_eq!(r.before, stats(1, 3, 4, 3));
    assert_eq!(r.source_before, stats(1, 3, 4, 3));
    assert!(r.search.complete);
    assert_eq!((r.search.examined, r.search.limit), (7, 2048));
    assert!(!r.ranking_lower_bound);
    assert_eq!(cuts(&r), ["0: a>b", "1,2: b>a,b>c", "1,3: b>a,c>a"]);
    let best = &r.plans[0];
    assert!(best.complete);
    assert_eq!(
        (best.after, best.source_after),
        (stats(0, 0, 0, 0), stats(0, 0, 0, 0))
    );
    assert_eq!(best.affected_files, ["a"]);
    assert_eq!((best.reference_count, best.removed_edges), (1, 1));
    assert_eq!(r.plans[1].affected_files, ["b"]);
    assert_eq!(r.plans[1].reference_count, 4);
    assert_eq!(r.plans[1].op_references, [3, 1]);
    assert_eq!(r.plans[1].evidence.len(), 4);
    assert_eq!(r.plans[2].affected_files, ["b", "c"]);
}

/// One SCC {a, b, c} of two 2-cycles needs two cuts. Members are files
/// below module `m`; x/y outside the scope keeps a source cycle with m/a.
/// The unknown total of m/b→m/c makes ranking and its plans lower bounds.
#[test]
fn a_cycle_needing_two_cuts() {
    let g = graph(
        &["m", "m/a", "m/b", "m/c", "x", "x/y"],
        &[
            ("m/a", "m/b", 2, Some(2)),
            ("m/b", "m/a", 1, ONE),
            ("m/b", "m/c", 1, None),
            ("m/c", "m/b", 4, Some(4)),
            ("x/y", "m/a", 1, ONE),
            ("m/a", "x/y", 1, ONE),
        ],
    );
    let r = plan(&g, &["m/a", "m/b", "m/c"], DEFAULT_MAX_EXAMINED);
    assert_eq!(r.before, stats(1, 3, 4, 3));
    assert_eq!(r.source_before, stats(1, 4, 6, 4));
    assert!(r.search.complete);
    assert_eq!(r.search.examined, 8);
    assert!(r.ranking_lower_bound);
    assert_eq!(
        cuts(&r),
        [
            "1,2: m/b>m/a,m/b>m/c",
            "0,2: m/a>m/b,m/b>m/c",
            "1,3: m/b>m/a,m/c>m/b"
        ]
    );
    let best = &r.plans[0];
    assert_eq!(best.source_after, stats(1, 2, 2, 2));
    assert_eq!(best.affected_files, ["m/b"]);
    assert_eq!(best.reference_count, 2);
    assert!(best.reference_lower_bound && best.evidence_truncated);
    assert_eq!(best.evidence.len(), 2);
    let last = &r.plans[2];
    assert_eq!(last.reference_count, 5);
    assert!(!last.reference_lower_bound && !last.evidence_truncated);
}

/// A ring of four 2-cycles needs four cuts; three examined states only
/// reach partial improvements.
fn ring() -> Graph {
    let mut edges = Vec::new();
    for (x, y) in [("a", "b"), ("b", "c"), ("c", "d"), ("d", "a")] {
        edges.push((x, y, 1, ONE));
        edges.push((y, x, 1, ONE));
    }
    graph(&["a", "b", "c", "d"], &edges)
}

#[test]
fn reaching_the_search_limit_leaves_the_search_incomplete() {
    let r = plan(&ring(), &["a", "b", "c", "d"], 3);
    assert!(!r.search.complete);
    assert_eq!((r.search.examined, r.search.limit), (3, 3));
    assert_eq!(cuts(&r), ["0,1: a>b,a>d", "0: a>b"]);
    assert!(!r.plans[0].complete);
    assert_eq!(r.plans[0].after, stats(1, 3, 4, 3));
    assert_eq!(r.plans[1].after, stats(1, 4, 7, 4));
}

#[test]
fn the_full_search_finds_four_cut_plans() {
    let r = plan(&ring(), &["a", "b", "c", "d"], DEFAULT_MAX_EXAMINED);
    assert!(r.search.complete);
    assert_eq!(r.search.examined, 57);
    assert_eq!(
        cuts(&r),
        [
            "0,1,4,5: a>b,a>d,c>b,c>d",
            "2,3,6,7: b>a,b>c,d>a,d>c",
            "0,1,3,5: a>b,a>d,b>c,c>d"
        ]
    );
    assert_eq!(r.plans[2].affected_files, ["a", "b", "c"]);
}

#[test]
fn limits_are_clamped() {
    assert_eq!(plan(&ring(), &["a", "b"], 0).search.limit, 1);
    assert_eq!(plan(&ring(), &["a", "b"], 1 << 20).search.limit, 8192);
}

#[test]
fn an_unknown_member_is_an_error() {
    let g = graph(&["a", "b"], &[("a", "b", 1, ONE), ("b", "a", 1, ONE)]);
    let err = plan_cycle_repairs(&g, &["a".into(), "zz".into()], 10).unwrap_err();
    assert_eq!(
        err.to_string(),
        ArchguardError::Analysis("Cycle member not found: zz".into()).to_string()
    );
}

#[test]
fn overlapping_members_are_an_error() {
    let g = graph(&["m", "m/a"], &[]);
    let err = plan_cycle_repairs(&g, &["m".into(), "m/a".into()], 10).unwrap_err();
    assert!(
        err.to_string().contains("Cycle member scopes overlap: m/a"),
        "{err}"
    );
}

#[test]
fn an_acyclic_scope_needs_no_search() {
    let g = graph(&["a", "b"], &[("a", "b", 1, ONE)]);
    let r = plan(&g, &["a", "b"], 10);
    assert!(r.search.complete);
    assert_eq!(r.search.examined, 0);
    assert!(r.plans.is_empty());
    assert_eq!(r.before, RepairStats::default());
}

/// The JSON names are those of the CLI/MCP output; ops use the frozen
/// SimOp shape so a plan replays through `archguard simulate --ops`.
#[test]
fn plans_serialize_with_simop_cuts() {
    let g = graph(&["a", "b"], &[("a", "b", 1, ONE), ("b", "a", 1, ONE)]);
    let json = serde_json::to_value(plan(&g, &["a", "b"], 10)).expect("serializes");
    assert_eq!(json["plans"][0]["ops"][0]["cut_edge"]["from"], "a");
    assert_eq!(json["search"]["complete"], true);
    assert_eq!(json["before"]["cyclic_nodes"], 2);
    assert_eq!(json["plans"][0]["evidence"][0]["line"], 1);
}
