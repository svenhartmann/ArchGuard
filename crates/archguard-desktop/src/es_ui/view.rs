//! The model of the tab: the search graph as rows and arcs of the Services tree.

use std::collections::HashMap;

use super::state::EsState;
use crate::services_ui::layout::{self, Link, Node};
use crate::services_ui::{paths, search, view::evidence, view::shorten};
use crate::tab_support::model;
use crate::ui::{EsKind, EsModel, SvcEdge, SvcRow, SvcSearchDetail};
use archguard_engine::core::SearchArtifact;

/// The laid-out graph: layout nodes (groups first, one per kind with shown nodes) and
/// what each stands for.
struct Scene {
    nodes: Vec<Node>,
    /// Row text per layout node.
    names: Vec<String>,
    /// Graph node per layout node; `None` for a group.
    graph: Vec<Option<usize>>,
    links: Vec<Link>,
    /// Graph edge per link.
    edge_of: Vec<usize>,
    /// Shown nodes per kind (the group rows).
    shown: HashMap<String, usize>,
    /// Shown declared nodes per kind: what presses a filter key, ghosts do not.
    real: HashMap<String, usize>,
}

/// Shown nodes: the kind filter (ghosts always), or the focused node with its neighbours.
fn visible(s: &EsState) -> Vec<bool> {
    let nodes = &s.graph.nodes;
    let Some(f) = s.focus else {
        return nodes
            .iter()
            .map(|n| !n.declared || s.kind_shown(&n.kind))
            .collect();
    };
    let mut out = vec![false; nodes.len()];
    out[f] = true;
    for e in &s.graph.edges {
        if e.from == f {
            out[e.to] = true;
        } else if e.to == f {
            out[e.from] = true;
        }
    }
    out
}

fn scene(s: &EsState) -> Scene {
    let shown = visible(s);
    let nodes = &s.graph.nodes;
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut real: HashMap<String, usize> = HashMap::new();
    for (n, _) in nodes.iter().zip(&shown).filter(|(_, v)| **v) {
        *counts.entry(n.kind.clone()).or_default() += 1;
        if n.declared {
            *real.entry(n.kind.clone()).or_default() += 1;
        }
    }
    let mut out = Scene {
        nodes: Vec::new(),
        names: Vec::new(),
        graph: Vec::new(),
        links: Vec::new(),
        edge_of: Vec::new(),
        shown: counts,
        real,
    };
    let mut group = HashMap::new();
    // Groups in the kinds' order; the layout sorts siblings by name, so the name is the rank.
    let kinds = s.kinds();
    for (rank, kind) in kinds.iter().enumerate() {
        let Some(&n) = out.shown.get(kind) else {
            continue;
        };
        group.insert(kind.as_str(), out.nodes.len());
        out.nodes.push(Node {
            id: format!("g{rank}"),
            name: format!("{rank:03}"),
            parent: None,
        });
        out.names.push(format!("{kind} ({n})"));
        out.graph.push(None);
    }
    let mut at = vec![None; nodes.len()];
    for (i, n) in nodes.iter().enumerate().filter(|(i, _)| shown[*i]) {
        let name = format!("{}{}", if n.warn { "⚠ " } else { "" }, n.label);
        at[i] = Some(out.nodes.len());
        out.nodes.push(Node {
            id: format!("n{i}"),
            name: name.clone(),
            parent: group.get(n.kind.as_str()).copied(),
        });
        out.names.push(name);
        out.graph.push(Some(i));
    }
    for (i, e) in s.graph.edges.iter().enumerate() {
        if let (Some(from), Some(to)) = (at[e.from], at[e.to]) {
            let key = format!("{i:09}");
            out.links.push(Link {
                from,
                to,
                key,
                side: None,
            });
            out.edge_of.push(i);
        }
    }
    out
}

/// The focused node's detail, from the Services section's builder; a service has none, an
/// undeclared name is an artifact with nothing but its name.
fn detail(s: &EsState) -> Option<SvcSearchDetail> {
    let n = &s.graph.nodes[s.focus?];
    let t = (*s.topology).as_ref().ok()?;
    if n.kind == "service" {
        return None;
    }
    let ghost;
    let a = match n.artifact {
        Some(i) => t.elasticsearch.artifacts.get(i)?,
        None => {
            ghost = SearchArtifact {
                kind: n.kind.clone(),
                name: Some(n.title.clone()),
                address: n.title.clone(),
                scope: String::new(),
                patterns: Vec::new(),
                references: Vec::new(),
                evidence: Vec::new(),
            };
            &ghost
        }
    };
    Some(search::detail(a, t, evidence))
}

pub fn build(s: &EsState) -> EsModel {
    if (*s.topology).is_err() {
        return EsModel {
            state: 2,
            ..Default::default()
        };
    }
    let sc = scene(s);
    let lay = layout::layout_fixed_roots(&sc.nodes, &sc.links, s.viewport);
    let total = |kind: &str, declared: bool| {
        s.graph
            .nodes
            .iter()
            .filter(|n| n.kind == kind && n.declared == declared)
            .count() as i32
    };
    let detail = detail(s);
    EsModel {
        state: 1,
        rows: model(rows(s, &sc, &lay)),
        edges: model(arcs(s, &sc, &lay)),
        hierarchy: lay
            .hierarchy
            .iter()
            .map(|p| paths::rounded(p, 6.0))
            .collect::<Vec<_>>()
            .join(" ")
            .into(),
        width: lay.width,
        height: lay.height,
        zoom: s.zoom,
        kinds: model(s.kinds().iter().map(|k| EsKind {
            kind: k.as_str().into(),
            count: total(k, true),
            shown: sc.real.get(k).copied().unwrap_or(0) as i32,
            ghosts: total(k, false),
        })),
        focus: s
            .focus
            .map(|f| s.graph.nodes[f].title.as_str().into())
            .unwrap_or_default(),
        total: s.graph.nodes.len() as i32,
        has_detail: detail.is_some(),
        detail: detail.unwrap_or_default(),
    }
}

fn rows(s: &EsState, sc: &Scene, lay: &layout::Layout) -> Vec<SvcRow> {
    let (mut incoming, mut outgoing) = (vec![0; sc.nodes.len()], vec![0; sc.nodes.len()]);
    for l in &sc.links {
        outgoing[l.from] += 1;
        incoming[l.to] += 1;
    }
    lay.rows
        .iter()
        .map(|r| {
            let graph = sc.graph[r.node];
            // Icon and padding, plus the In/Out column (bare counts when compact).
            let counts = match (graph, lay.compact) {
                (None, _) => 36.0,
                (_, true) => 90.0,
                _ => 145.0,
            };
            let label_width = lay.w - r.indent - counts;
            SvcRow {
                name: shorten(&sc.names[r.node], label_width).into(),
                context: graph.is_none(),
                service: graph.map_or(-1, |g| g as i32),
                left: r.x - lay.w / 2.0,
                y: r.y,
                w: lay.w,
                h: lay.h,
                indent: r.indent,
                label_width,
                relation_x: lay.relation_x,
                compact: lay.compact,
                selected: graph.is_some() && graph == s.focus,
                in_cycle: false,
                incoming: incoming[r.node],
                outgoing: outgoing[r.node],
            }
        })
        .collect()
}

/// write = outgoing (2), read = incoming (1), the rest neutral; unknown is dashed.
fn arcs(s: &EsState, sc: &Scene, lay: &layout::Layout) -> Vec<SvcEdge> {
    lay.routes
        .iter()
        .map(|route| {
            let i = sc.edge_of[route.link];
            let e = &s.graph.edges[i];
            let p = &route.points;
            let line = if e.relation == "unknown" {
                paths::dashed(p, 6.0, 3.0)
            } else {
                paths::curve(p)
            };
            SvcEdge {
                line: line.into(),
                head: paths::head(p[5], p[4]).into(),
                color: match e.relation {
                    "write" => 2,
                    "read" => 1,
                    _ => 0,
                },
                selected: false,
                edge: i as i32,
                tip_x: p[5].x,
                tip_y: p[5].y,
                top: p[0].y.min(p[5].y),
                bottom: p[0].y.max(p[5].y),
            }
        })
        .collect()
}

/// `fitServices` for a viewport of `w` x `h`.
pub fn fit(s: &mut EsState, w: f32, h: f32) {
    let sc = scene(s);
    let lay = layout::layout_fixed_roots(&sc.nodes, &sc.links, s.viewport);
    if lay.width > 0.0 && lay.height > 0.0 {
        let z = ((w - 24.0).max(1.0) / lay.width).min((h - 24.0).max(1.0) / lay.height);
        s.zoom = z.clamp(0.01, 3.0);
    }
}

#[cfg(test)]
mod tests {
    use archguard_engine::core::{
        SearchArtifact, SearchFinding, SearchIndexUsage, SearchReference, ServiceTopology,
    };
    use slint::Model;

    use super::*;
    use crate::es_ui::state::build_from;

    fn artifact(kind: &str, name: &str, references: &[(&str, &str)]) -> SearchArtifact {
        SearchArtifact {
            kind: kind.into(),
            name: Some(name.into()),
            address: format!("res.{name}"),
            scope: String::new(),
            patterns: Vec::new(),
            references: references
                .iter()
                .map(|(kind, name)| SearchReference {
                    kind: (*kind).into(),
                    name: (*name).into(),
                })
                .collect(),
            evidence: Vec::new(),
        }
    }

    fn usage(service: &str, index: &str, access: &str) -> SearchIndexUsage {
        SearchIndexUsage {
            service: service.into(),
            index: index.into(),
            access: access.into(),
            evidence: Vec::new(),
        }
    }

    /// svc-a writes idx-a and idx-b (unknown), svc-b reads idx-a and writes the
    /// undeclared `missing`; idx-a uses pipe, idx-b has a finding.
    fn state() -> EsState {
        let mut t = ServiceTopology::default();
        let es = &mut t.elasticsearch;
        es.artifacts = vec![
            artifact("index", "idx-a", &[("ingest_pipeline", "pipe")]),
            artifact("index", "idx-b", &[]),
            artifact("alias", "al", &[]),
            artifact("ingest_pipeline", "pipe", &[]),
        ];
        es.usages = vec![
            usage("svc-a", "idx-a", "write"),
            usage("svc-a", "idx-b", "unknown"),
            usage("svc-b", "idx-a", "read"),
            usage("svc-b", "missing", "write"),
        ];
        es.findings = vec![SearchFinding {
            kind: "undeclared_index".into(),
            subject: "idx-b".into(),
            services: Vec::new(),
            evidence: Vec::new(),
        }];
        build_from(std::sync::Arc::new(Ok(t)))
    }

    fn names(m: &EsModel) -> Vec<String> {
        m.rows.iter().map(|r| r.name.to_string()).collect()
    }

    fn row(m: &EsModel, name: &str) -> crate::ui::SvcRow {
        m.rows
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("no row {name}: {:?}", names(m)))
    }

    fn kind_at(s: &EsState, kind: &str) -> usize {
        s.kinds().iter().position(|k| k == kind).unwrap()
    }

    #[test]
    fn the_default_shows_services_indices_and_ghosts_not_the_pipeline() {
        let mut s = state();
        let m = build(&s);
        assert_eq!(m.state, 1);
        assert_eq!(m.total, 7);
        let n = names(&m);
        assert!(
            n.contains(&"svc-a".into()) && n.contains(&"svc-b".into()),
            "{n:?}"
        );
        assert!(
            n.contains(&"⚠ missing".into()) && n.contains(&"al".into()),
            "{n:?}"
        );
        assert!(!n.iter().any(|x| x == "pipe"), "{n:?}");
        // Hidden nodes are counted, not lost.
        let k = m
            .kinds
            .iter()
            .find(|k| k.kind == "ingest_pipeline")
            .unwrap();
        assert_eq!((k.count, k.shown), (1, 0));
        let i = kind_at(&s, "ingest_pipeline");
        s.toggle_kind(i);
        let m = build(&s);
        assert!(names(&m).contains(&"pipe".into()));
        let k = m
            .kinds
            .iter()
            .find(|k| k.kind == "ingest_pipeline")
            .unwrap();
        assert_eq!((k.count, k.shown), (1, 1));
    }

    #[test]
    fn a_kind_key_is_pressed_by_real_nodes_only_ghosts_are_counted_apart() {
        let mut t = ServiceTopology::default();
        t.elasticsearch.artifacts = vec![artifact("ingest_pipeline", "pipe", &[])];
        t.elasticsearch.usages = vec![usage("svc-a", "missing", "write")];
        let mut s = build_from(std::sync::Arc::new(Ok(t)));
        let k = |s: &EsState, kind: &str| build(s).kinds.iter().find(|k| k.kind == kind).unwrap();
        // Only a ghost is behind `index`: the key is not pressed, the ghost has its own count.
        let ix = k(&s, "index");
        assert_eq!((ix.count, ix.shown, ix.ghosts), (0, 0, 1));
        assert!(names(&build(&s)).contains(&"⚠ missing".into()));
        s.toggle_kind(kind_at(&s, "ingest_pipeline"));
        let p = k(&s, "ingest_pipeline");
        assert_eq!((p.count, p.shown, p.ghosts), (1, 1, 0));
    }

    #[test]
    fn a_group_row_per_kind_comes_before_its_nodes() {
        let m = build(&state());
        let n = names(&m);
        assert_eq!(
            n,
            [
                "service (2)",
                "svc-a",
                "svc-b",
                "index (3)",
                "idx-a",
                "⚠ idx-b",
                "⚠ missing",
                "alias (1)",
                "al"
            ]
        );
        let groups: Vec<bool> = m.rows.iter().map(|r| r.context).collect();
        assert_eq!(
            groups,
            [true, false, false, true, false, false, false, true, false]
        );
        assert!(m.rows.iter().all(|r| r.context == (r.service < 0)));
    }

    #[test]
    fn focus_shows_the_node_its_neighbours_and_their_groups_only() {
        let mut s = state();
        let a = row(&build(&s), "⚠ idx-b").service as usize;
        s.focus_on(a);
        let m = build(&s);
        assert_eq!(names(&m), ["service (1)", "svc-a", "index (1)", "⚠ idx-b"]);
        assert_eq!(m.focus, "idx-b");
        assert!(row(&m, "⚠ idx-b").selected);
        // A group row has no node, so nothing changes.
        s.focus = None;
        s.focus_on(usize::MAX);
        assert_eq!(build(&s).rows.row_count(), 9);
        s.focus_on(a);
        s.focus = None;
        assert_eq!(build(&s).focus, "");
    }

    #[test]
    fn rows_carry_the_warning_and_the_visible_edge_counts() {
        let m = build(&state());
        let b = row(&m, "⚠ idx-b");
        assert!(!b.in_cycle && !b.context);
        assert_eq!((b.incoming, b.outgoing), (1, 0));
        let a = row(&m, "idx-a");
        assert_eq!((a.incoming, a.outgoing), (1, 1));
        let svc = row(&m, "svc-a");
        // idx-a (write) and idx-b (unknown).
        assert_eq!((svc.incoming, svc.outgoing), (0, 2));
    }

    #[test]
    fn edges_are_coloured_per_relation_and_unknown_is_dashed() {
        let mut s = state();
        let i = kind_at(&s, "ingest_pipeline");
        s.toggle_kind(i);
        let m = build(&s);
        let by_color = |c: i32| m.edges.iter().filter(|e| e.color == c).count();
        // write: svc-a→idx-a, svc-b→missing; read: idx-a→svc-b; reference idx-a→pipe;
        // unknown svc-a→idx-b.
        assert_eq!(by_color(2), 2);
        assert_eq!(by_color(1), 1);
        assert_eq!(by_color(0), 2);
        let dashes = |e: &crate::ui::SvcEdge| e.line.matches('M').count();
        let solid = m.edges.iter().find(|e| e.color == 2).unwrap();
        let unknown = m
            .edges
            .iter()
            .find(|e| e.color == 0 && dashes(e) > 1)
            .unwrap();
        assert_eq!(dashes(&solid), 1);
        assert_eq!(m.edges.iter().filter(|e| dashes(e) > 1).count(), 1);
        assert!(unknown.tip_y > 0.0);
    }

    #[test]
    fn edges_only_join_shown_nodes() {
        let m = build(&state());
        // The reference to the hidden pipeline is not drawn.
        assert_eq!(m.edges.row_count(), 4);
    }

    #[test]
    fn covers_is_neutral() {
        let mut s = state();
        s.graph.edges.push(archguard_engine::SearchEdge {
            from: 0,
            to: 2,
            relation: "covers",
        });
        let m = build(&s);
        assert_eq!(m.edges.iter().filter(|e| e.color == 0).count(), 2);
    }

    #[test]
    fn without_elasticsearch_data_the_tab_is_ready_and_empty() {
        let m = build(&build_from(std::sync::Arc::new(Ok(
            ServiceTopology::default(),
        ))));
        assert_eq!((m.state, m.total), (1, 0));
        assert_eq!((m.rows.row_count(), m.edges.row_count()), (0, 0));
    }

    #[test]
    fn a_large_graph_lays_out_in_linear_work() {
        let mut t = ServiceTopology::default();
        t.elasticsearch.artifacts = (0..3000)
            .map(|i| artifact("index", &format!("idx-{i}"), &[]))
            .collect();
        t.elasticsearch.usages = (0..3000)
            .map(|i| usage(&format!("svc-{}", i % 50), &format!("idx-{i}"), "write"))
            .collect();
        let began = std::time::Instant::now();
        let m = build(&build_from(std::sync::Arc::new(Ok(t))));
        assert_eq!(m.rows.row_count(), 3000 + 50 + 2);
        assert!(began.elapsed().as_secs() < 20, "{:?}", began.elapsed());
    }

    #[test]
    fn fit_scales_the_canvas_into_the_viewport() {
        let mut s = state();
        s.viewport = 1480.0;
        let m = build(&s);
        fit(&mut s, 600.0, 200.0);
        let z = ((600.0f32 - 24.0) / m.width).min((200.0 - 24.0) / m.height);
        assert!((s.zoom - z).abs() < 1e-4 && s.zoom < 1.0, "{}", s.zoom);
    }
    #[test]
    fn read_only_services_keep_the_group_order_of_the_kinds() {
        let mut t = ServiceTopology::default();
        t.elasticsearch.artifacts = vec![artifact("index", "i", &[]), artifact("alias", "al", &[])];
        t.elasticsearch.usages = vec![usage("svc", "i", "read")];
        let m = build(&build_from(std::sync::Arc::new(Ok(t))));
        let groups: Vec<String> = names(&m).into_iter().filter(|n| n.contains(" (")).collect();
        assert_eq!(groups, ["service (1)", "index (1)", "alias (1)"]);
    }

    #[test]
    fn focus_keeps_the_group_order_of_the_kinds() {
        let mut t = ServiceTopology::default();
        t.elasticsearch.artifacts = vec![
            artifact("alerting_rule", "r", &[("index", "i")]),
            artifact("index", "i", &[]),
        ];
        t.elasticsearch.usages = vec![usage("svc", "i", "write")];
        let mut s = build_from(std::sync::Arc::new(Ok(t)));
        let at = row(&build(&s), "i").service as usize;
        s.focus_on(at);
        let groups: Vec<String> = names(&build(&s))
            .into_iter()
            .filter(|n| n.contains(" ("))
            .collect();
        assert_eq!(groups, ["service (1)", "index (1)", "alerting_rule (1)"]);
    }

    /// The detail bar of a focused artifact is the Services section's detail of that artifact.
    #[test]
    fn focus_on_an_artifact_fills_the_detail_from_the_services_section() {
        let mut s = state();
        let m = build(&s);
        let at = |name: &str| row(&m, name).service as usize;
        let (a, b, svc) = (at("idx-a"), at("⚠ idx-b"), at("svc-a"));
        s.focus_on(a);
        let m = build(&s);
        assert!(m.has_detail);
        let t = (*s.topology).as_ref().unwrap();
        let same = crate::services_ui::search::detail(
            &t.elasticsearch.artifacts[0],
            t,
            crate::services_ui::view::evidence,
        );
        assert_eq!(
            (m.detail.writers.as_str(), m.detail.readers.as_str()),
            ("svc-a", "svc-b")
        );
        assert_eq!(m.detail.writers, same.writers);
        assert_eq!(m.detail.references.row_count(), same.references.row_count());
        s.focus_on(b);
        assert_eq!(build(&s).detail.findings.row_count(), 1);
        s.focus_on(svc);
        assert!(!build(&s).has_detail);
        s.focus = None;
        assert!(!build(&s).has_detail);
    }

    /// An undeclared name has no declaration: what is known is its users and findings.
    #[test]
    fn a_ghost_has_users_but_no_declaration() {
        let mut s = state();
        let g = row(&build(&s), "⚠ missing").service as usize;
        s.focus_on(g);
        let m = build(&s);
        assert!(m.has_detail);
        assert_eq!(m.detail.writers, "svc-b");
        assert_eq!(m.detail.declaration.row_count(), 0);
        assert_eq!(m.detail.scope, "");
    }
}
