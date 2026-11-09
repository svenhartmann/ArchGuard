//! Self-contained HTML viewer for [`Graph`] (T34, v2 in T39, Kap. 8.3).
//!
//! [`to_html`] produces ONE standalone page: the FULL hierarchical graph
//! (every node with `parent`/`kind`, every edge with its sampled `refs`
//! and its F12 total `count` — `null` for legacy graphs without one)
//! plus the analysis findings travel as an inline
//! `<script type="application/json">` payload; the client computes a fixed
//! SVG containment tree. Only choosing the optional Network view loads
//! `force-graph` asynchronously from the **pinned** jsDelivr URL
//! ([`FORCE_GRAPH_URL`]) and requires an internet connection. Tree and
//! dependency exploration are self-contained (inline CSS, vanilla JS, no
//! framework). The skeleton and client live in [`crate::html_app`].
//!
//! The client shows the start cut (`focus`/`depth` like
//! [`crate::to_mermaid`] via [`crate::render::focus_subset`], otherwise
//! [`archguard_core::view::Cut::at_depth`] at `depth`) and mirrors the
//! server-side semantics so drill-down matches the CLI exactly:
//! double-click expands a node into its children (breadcrumb bar
//! collapses back), a cycles panel isolates each finding's subgraph, a
//! DSM tab renders the current view as a matrix, the sidebar shows
//! per-node fan-in/out/instability; the metric strip shows the scope ACD.
//! Labeled package cards use stable source identities. Edge widths
//! encode full import counts, colors encode cycles. Search reveals nodes;
//! Light/Dark/System use a shared DOM/canvas palette and optional storage.
//!
//! Browse is the default; clicks inspect, and node positions stay fixed.
//! Cycle repair comparison searches bounded dependency-change combinations,
//! ranks their importing-file/reference scope, and recomputes selected-scope
//! and full-source cycle metrics. Plans carry evidence and source assumptions;
//! they are not executable source refactorings. Multi-op previews undo together.
//! Explicit Simulation mode enables confirmed contextual cut/move actions
//! (undo/clear in the changes tray). Ops
//! apply client-side (mirrors of `apply_ops` + `aggregate`) and the
//! recomputed view re-runs Tarjan + ACD with static cycle-difference colors.
//! "Export changes" downloads the
//! recorded ops in the frozen T37 wire format of
//! `archguard-analysis`'s `SimOp` enum. The browser NEVER edits code —
//! applying ops to the real graph stays `archguard simulate` / MCP.

use std::collections::{BTreeSet, HashMap, HashSet};

use archguard_core::{CallAnalysis, Finding, Graph, NodeId, PerformanceReport, ReviewSources};
use serde_json::json;

use crate::html_app::{APP_JS, TEMPLATE};
use crate::render::focus_subset;

/// HTML-only string pooling keeps repeated assembly-qualified compiler IDs
/// from dominating large reports. The viewer restores the ordinary call model
/// once; CLI/MCP JSON and user exports retain their original schema.
fn html_calls(calls: Option<&CallAnalysis>) -> serde_json::Value {
    let mut value = serde_json::to_value(calls).expect("call model serializes");
    if calls.is_none_or(|model| model.calls.len() < 1000) {
        return value;
    }
    let mut index = HashMap::<String, usize>::new();
    let mut strings = Vec::<String>::new();
    let mut fields = serde_json::Map::new();
    for table in ["classes", "methods", "calls"] {
        let mut pooled = BTreeSet::new();
        for row in value[table].as_array_mut().expect("call table").iter_mut() {
            for (field, item) in row.as_object_mut().expect("call row").iter_mut() {
                let values = match item {
                    serde_json::Value::String(_) => std::slice::from_mut(item),
                    serde_json::Value::Array(items)
                        if !items.is_empty() && items.iter().all(|v| v.is_string()) =>
                    {
                        items.as_mut_slice()
                    }
                    _ => continue,
                };
                pooled.insert(field.clone());
                for value in values {
                    let serde_json::Value::String(text) = value.take() else {
                        unreachable!("string field")
                    };
                    let id = if let Some(id) = index.get(&text) {
                        *id
                    } else {
                        let id = strings.len();
                        index.insert(text.clone(), id);
                        strings.push(text);
                        id
                    };
                    *value = json!(id);
                }
            }
        }
        fields.insert(table.into(), json!(pooled));
    }
    value["compact"] = json!({"encoding":"string-pool-v1","strings":strings,"fields":fields});
    value
}

/// The single external resource of the page: `force-graph` pinned to an
/// exact version on the jsDelivr CDN. Pinning is deliberate — a floating
/// `@1` would let a library update silently change the viewer.
const FORCE_GRAPH_URL: &str =
    "https://cdn.jsdelivr.net/npm/force-graph@1.52.0/dist/force-graph.min.js";

/// Optional source evidence supplied by the engine; renderers perform no IO.
#[derive(Default)]
pub struct HtmlContext<'a> {
    /// Original source IDs of an explicitly expanded initial view. The complete
    /// hierarchy stays embedded for evidence and subsequent navigation.
    pub initial_cut: Option<&'a archguard_core::Cut>,
    pub calls: Option<&'a CallAnalysis>,
    pub review_sources: Option<&'a ReviewSources>,
    pub entry_point_ids: &'a [NodeId],
    pub entry_points: &'a [String],
    pub performance: Option<&'a PerformanceReport>,
    pub insights: Option<&'a serde_json::Value>,
    /// Independent deployment configuration evidence; not source import edges.
    pub services: Option<&'a archguard_core::ServiceTopology>,
    /// Snapshot history rows for the health sparkline (R09/R39) as plain
    /// JSON; `None` when the caller did not read them.
    pub review_history: Option<&'a serde_json::Value>,
    /// KPI block completed with evidence the cached graph lacks (zones
    /// from compiler abstractness); `None` embeds the graph's own block.
    pub review_kpis: Option<&'a archguard_core::ReviewKpis>,
}

/// Render `g` as one self-contained HTML page (see the module docs).
///
/// `focus`/`depth` select the start cut as in [`crate::to_mermaid`]:
/// with `focus`, the payload pins the [`crate::render::focus_subset`]
/// ids as the initial view (drill-down into children still works because
/// the FULL graph is embedded); without, the client starts at
/// `Cut::at_depth(depth)` itself. `cyclic_edges` (the `(from, to)` pairs
/// inside SCCs, computed on the analysis cut) are embedded verbatim and
/// re-projected onto whatever view the client shows; `findings` feed the
/// cycles panel; repair plans are computed from the current graph. `initial_tab` (T40) names the
/// tab the page opens on (`"graph"`, `"cycles"` or `"dsm"`; `None` keeps
/// the default graph tab — `cycles --format html` passes `"cycles"`).
/// The output ends with a
/// trailing newline and is deterministic for equal inputs.
pub fn to_html(
    g: &Graph,
    focus: Option<&str>,
    depth: u32,
    cyclic_edges: &HashSet<(NodeId, NodeId)>,
    findings: &[Finding],
    initial_tab: Option<&str>,
) -> String {
    to_html_with_context(
        g,
        focus,
        depth,
        cyclic_edges,
        findings,
        initial_tab,
        &HtmlContext::default(),
    )
}

/// Render the dependency viewer with a detailed, source-backed call model.
/// Graph metrics and simulation remain based on `g`; the call model records
/// original source evidence and does not simulate rewritten code.
pub fn to_html_with_calls(
    g: &Graph,
    focus: Option<&str>,
    depth: u32,
    cyclic_edges: &HashSet<(NodeId, NodeId)>,
    findings: &[Finding],
    initial_tab: Option<&str>,
    calls: &CallAnalysis,
) -> String {
    to_html_with_context(
        g,
        focus,
        depth,
        cyclic_edges,
        findings,
        initial_tab,
        &HtmlContext {
            calls: Some(calls),
            ..HtmlContext::default()
        },
    )
}

pub fn to_html_with_context(
    g: &Graph,
    focus: Option<&str>,
    depth: u32,
    cyclic_edges: &HashSet<(NodeId, NodeId)>,
    findings: &[Finding],
    initial_tab: Option<&str>,
    context: &HtmlContext<'_>,
) -> String {
    page_html(&payload_value(
        g,
        focus,
        depth,
        cyclic_edges,
        findings,
        initial_tab,
        context,
    ))
}

/// Payload sections `archguard serve` can deliver on request instead of
/// inline: compiler calls and class/method metrics dominate large reports.
const DEFERRED_SECTIONS: [&str; 2] = ["calls", "code_metrics"];

/// A viewer page whose largest payload sections may be fetched on demand.
pub struct HtmlPage {
    /// The page to serve; deferred sections hold `{"deferred": name}`.
    pub html: String,
    /// `(name, JSON)` of every deferred section, in [`DEFERRED_SECTIONS`] order.
    pub deferred: Vec<(&'static str, String)>,
    /// The self-contained page for saved reports, when anything was deferred.
    pub complete: Option<String>,
}

/// Like [`to_html_with_context`], but sections whose JSON exceeds
/// `defer_above` bytes move out of the page (`None` keeps everything inline).
/// Exported reports stay single files: `complete` embeds every section.
#[expect(
    clippy::too_many_arguments,
    reason = "the renderer inputs plus the deferral threshold"
)]
#[tracing::instrument(skip_all)]
pub fn to_html_page(
    g: &Graph,
    focus: Option<&str>,
    depth: u32,
    cyclic_edges: &HashSet<(NodeId, NodeId)>,
    findings: &[Finding],
    initial_tab: Option<&str>,
    context: &HtmlContext<'_>,
    defer_above: Option<usize>,
) -> HtmlPage {
    let mut payload = payload_value(
        g,
        focus,
        depth,
        cyclic_edges,
        findings,
        initial_tab,
        context,
    );
    let Some(limit) = defer_above else {
        return HtmlPage {
            html: page_html(&payload),
            deferred: Vec::new(),
            complete: None,
        };
    };
    let mut deferred = Vec::new();
    for name in DEFERRED_SECTIONS {
        let slot = if name == "calls" {
            payload.get_mut("calls")
        } else {
            payload
                .get_mut("insights")
                .and_then(|insights| insights.get_mut(name))
        };
        let Some(slot) = slot.filter(|value| !value.is_null()) else {
            continue;
        };
        let json = serde_json::to_string(slot).expect("payload sections serialize");
        if json.len() > limit {
            deferred.push((name, json));
        }
    }
    if deferred.is_empty() {
        return HtmlPage {
            html: page_html(&payload),
            deferred,
            complete: None,
        };
    }
    let complete = page_html(&payload);
    for (name, _) in &deferred {
        let placeholder = json!({ "deferred": name });
        if *name == "calls" {
            payload["calls"] = placeholder;
        } else {
            payload["insights"][*name] = placeholder;
        }
    }
    HtmlPage {
        html: page_html(&payload),
        deferred,
        complete: Some(complete),
    }
}

fn payload_value(
    g: &Graph,
    focus: Option<&str>,
    depth: u32,
    cyclic_edges: &HashSet<(NodeId, NodeId)>,
    findings: &[Finding],
    initial_tab: Option<&str>,
    context: &HtmlContext<'_>,
) -> serde_json::Value {
    let keep = if let Some(cut) = context.initial_cut {
        let view = archguard_core::aggregate(g, cut);
        let focused = focus_subset(&view, focus, depth);
        let names: HashSet<&str> = view
            .nodes
            .iter()
            .filter(|node| focused.as_ref().is_none_or(|ids| ids.contains(&node.id)))
            .map(|node| node.name.as_str())
            .collect();
        Some(
            g.nodes
                .iter()
                .filter(|node| names.contains(node.name.as_str()))
                .map(|node| node.id)
                .collect(),
        )
    } else {
        focus_subset(g, focus, depth)
    };
    let mut cyclic: Vec<[u32; 2]> = cyclic_edges
        .iter()
        .map(|(from, to)| [from.0, to.0])
        .collect();
    cyclic.sort_unstable();

    json!({
        "root": g.root,
        "semantic": g.semantic,
        "nodes": g.nodes.iter().map(|n| json!({
            "id": n.id.0,
            "name": n.name,
            "kind": n.kind,
            "parent": n.parent.map(|p| p.0),
            "lang": n.lang,
        })).collect::<Vec<_>>(),
        "edges": g.edges.iter().map(|e| json!({
            "from": e.from.0,
            "to": e.to.0,
            "refs": e.refs.iter().map(|r| json!({
                "file": r.file,
                "line": r.line,
                "raw": r.raw,
            })).collect::<Vec<_>>(),
            // F12: the exact total (null for legacy graphs without one —
            // the client then falls back to the sample length).
            "count": e.count,
        })).collect::<Vec<_>>(),
        "findings": findings,
        // The KPI block the pipeline attached to this graph (Start panels);
        // null means NOT MEASURED and never renders invented zeros.
        "reviewKpis": context.review_kpis.or(g.review_kpis.as_ref()),
        // Health sparkline rows (R39); null stays "not provided", never empty.
        "reviewHistory": context.review_history,
        "cyclicEdges": cyclic,
        "initialTab": initial_tab,
        "developmentPaths": serde_json::from_str::<serde_json::Value>(archguard_core::DEVELOPMENT_PATHS).expect("development routes"),
        "calls": html_calls(context.calls),
        "performance": context.performance,
        "insights": context.insights,
        "services": context.services,
        "reviewContext": context.review_sources.map(|sources| json!({
            "sources": sources,
            "entry_point_ids": context.entry_point_ids,
            "entry_points": context.entry_points,
        })),
        "view": {
            "focus": focus,
            "depth": depth,
            "visible": keep.map(|set| {
                let mut ids: Vec<u32> = set.iter().map(|id| id.0).collect();
                ids.sort_unstable();
                ids
            }),
        },
    })
}

fn page_html(payload: &serde_json::Value) -> String {
    let payload = serde_json::to_string(payload)
        .expect("the payload is plain serde_json data and always serializable");
    // A literal `</` inside the JSON (e.g. in a node name) would end the
    // `<script>` element early; `<\/` is the equivalent JSON escape and
    // `</` can only ever occur inside a string value, so the replacement
    // is loss-free for parsers. `<!--` followed by `<script` would make the
    // parser ignore the real closing tag and swallow the application script.
    let payload = payload.replace("</", "<\\/").replace("<!--", "<\\u0021--");
    // Substitution order: JS first, then the CDN pin, payload LAST — the
    // payload (graph data) can therefore never be re-substituted.
    TEMPLATE
        .replace("__APP_JS__", APP_JS)
        .replace("__CDN__", FORCE_GRAPH_URL)
        .replace("__PAYLOAD__", &payload)
}

#[cfg(test)]
mod tests {
    use super::{FORCE_GRAPH_URL, to_html};
    use archguard_core::{
        Edge, Finding, FindingKind, Graph, Node, NodeId, NodeKind, Ref, Severity, SuggestedCut,
    };
    use std::collections::HashSet;

    fn module(id: u32, name: &str) -> Node {
        Node {
            id: NodeId(id),
            name: name.to_string(),
            kind: NodeKind::Module,
            parent: None,
            lang: None,
        }
    }

    fn file(id: u32, name: &str, parent: Option<u32>) -> Node {
        Node {
            id: NodeId(id),
            name: name.to_string(),
            kind: NodeKind::File,
            parent: parent.map(NodeId),
            lang: Some("ts".to_string()),
        }
    }

    fn edge_with(from: u32, to: u32, n: usize) -> Edge {
        Edge {
            from: NodeId(from),
            to: NodeId(to),
            refs: (0..n)
                .map(|k| Ref {
                    file: format!("{from}.ts"),
                    line: k as u32 + 1,
                    raw: "import …".to_string(),
                })
                .collect(),
            // Builder-shaped: the total equals the (complete) sample.
            count: Some(n as u64),
        }
    }

    fn edge(from: u32, to: u32) -> Edge {
        edge_with(from, to, 1)
    }

    /// The shared fixture: module `m` (id 5) holds files `a` (0) and `b`
    /// (1); files `c` (2), `d` (3), `e` (4) are root-level. One 2-cycle
    /// `a ↔ b` plus tails `a → c`, `c → d`, `b → e`, `d → e`.
    fn sample_graph() -> Graph {
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![
                file(0, "a", Some(5)),
                file(1, "b", Some(5)),
                file(2, "c", None),
                file(3, "d", None),
                file(4, "e", None),
                module(5, "m"),
            ],
            edges: vec![
                edge_with(0, 1, 2),
                edge(1, 0),
                edge(0, 2),
                edge_with(2, 3, 3),
                edge(1, 4),
                edge(3, 4),
            ],
        }
    }

    #[test]
    fn served_page_defers_large_sections_and_keeps_the_complete_report() {
        let calls = archguard_core::CallAnalysis {
            status: "partial".into(),
            files: 1,
            ..Default::default()
        };
        let insights = serde_json::json!({"code_metrics":{"classes":[{"class_id":"A"}]},"cohesion":{"kept":true}});
        let context = super::HtmlContext {
            calls: Some(&calls),
            insights: Some(&insights),
            ..Default::default()
        };
        let render = |defer_above| {
            super::to_html_page(
                &sample_graph(),
                None,
                0,
                &cyclic_edges(),
                &findings(),
                None,
                &context,
                defer_above,
            )
        };
        let complete = super::to_html_with_context(
            &sample_graph(),
            None,
            0,
            &cyclic_edges(),
            &findings(),
            None,
            &context,
        );
        let page = render(Some(0));
        let data = payload(&page.html);
        assert_eq!(data["calls"], serde_json::json!({"deferred":"calls"}));
        assert_eq!(
            data["insights"]["code_metrics"],
            serde_json::json!({"deferred":"code_metrics"})
        );
        assert_eq!(data["insights"]["cohesion"], insights["cohesion"]);
        let sections: std::collections::BTreeMap<_, _> = page.deferred.iter().cloned().collect();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&sections["calls"]).unwrap(),
            payload(&complete)["calls"]
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&sections["code_metrics"]).unwrap(),
            insights["code_metrics"]
        );
        assert_eq!(page.complete.as_deref(), Some(complete.as_str()));
        // Below the threshold nothing changes: one complete page, no requests.
        let small = render(None);
        assert_eq!(small.html, complete);
        assert!(small.deferred.is_empty() && small.complete.is_none());
        let large_only = render(Some(1 << 20));
        assert_eq!(large_only.html, complete);
    }

    #[test]
    fn html_preserves_semantic_graph_coverage_without_separate_call_payload() {
        let mut graph = sample_graph();
        graph.semantic = Some(archguard_core::SemanticDependencyCoverage {
            status: "partial".into(),
            projected_calls: 12,
            skipped_project_calls: 1,
            ..Default::default()
        });
        let page = to_html(&graph, None, 0, &cyclic_edges(), &findings(), None);
        let data = payload(&page);
        assert_eq!(
            data["semantic"],
            serde_json::to_value(&graph.semantic).unwrap()
        );
        assert!(data["calls"].is_null());
    }

    fn cyclic_edges() -> HashSet<(NodeId, NodeId)> {
        [(NodeId(0), NodeId(1)), (NodeId(1), NodeId(0))]
            .into_iter()
            .collect()
    }

    fn cycle_finding() -> Finding {
        Finding {
            message: None,
            evidence: Vec::new(),
            kind: FindingKind::Cycle,
            id: "scc-1".to_string(),
            severity: Severity::Error,
            nodes: vec!["a".to_string(), "b".to_string()],
            edge_count: 2,
            cycles_sampled: 1,
            sampling: None,
            suggested_cuts: vec![SuggestedCut {
                from: "a".to_string(),
                to: "b".to_string(),
                score: 0.5,
                reason: "breaks the only cycle".to_string(),
            }],
        }
    }

    fn findings() -> Vec<Finding> {
        vec![cycle_finding()]
    }

    /// Extract the inline JSON payload from the page.
    fn payload(html: &str) -> serde_json::Value {
        let marker = "<script id=\"archguard-data\" type=\"application/json\">";
        let start = html.find(marker).expect("payload marker") + marker.len();
        let end = start + html[start..].find("</script>").expect("payload end");
        serde_json::from_str(&html[start..end]).expect("payload is valid JSON")
    }

    #[test]
    fn html_embeds_service_topology_separately_and_escapes_evidence() {
        let services: archguard_core::ServiceTopology = serde_json::from_value(serde_json::json!({
            "schema_version": 1, "root": "/repo", "monorepo": false,
            "monorepo_reason": "one local app", "compose_files": ["compose.yaml"],
            "services": [{"id": "compose.yaml::api", "name": "api", "compose_file": "compose.yaml", "build_context": "apps/api", "networks": ["default"], "profiles": []}],
            "dependencies": [], "cycles": [], "complete": false,
            "diagnostics": [{"file": "compose.yaml", "service": "api", "message": "</script><img src=x onerror=alert(1)>"}]
        })).unwrap();
        let original = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        let page = super::to_html_with_context(
            &sample_graph(),
            None,
            0,
            &cyclic_edges(),
            &findings(),
            Some("services"),
            &super::HtmlContext {
                services: Some(&services),
                ..Default::default()
            },
        );
        let value = payload(&page);
        assert_eq!(value["services"], serde_json::to_value(&services).unwrap());
        assert_eq!(value["nodes"], payload(&original)["nodes"]);
        assert_eq!(value["edges"], payload(&original)["edges"]);
        assert_eq!(value["initialTab"], "services");
        assert_eq!(payload(&original)["services"], serde_json::Value::Null);
        assert!(page.contains("<\\/script><img"));
        assert!(!page.contains("</script><img"));
    }

    #[test]
    fn html_embeds_call_evidence_separately_and_escapes_untrusted_source() {
        let calls = archguard_core::CallAnalysis {
            status: "partial".into(),
            files: 1,
            calls: vec![archguard_core::MethodCall {
                from_class: "p.A".into(),
                from_method: Some("p.A#run()".into()),
                to_class: Some("p.B".into()),
                to_method: Some("p.B#save()".into()),
                target_label: "p.B#save()".into(),
                dispatch: "instance".into(),
                resolution: "project".into(),
                file: "src/p/A.java".into(),
                line: 7,
                column: 12,
                code: "b.save(\"</script><img src=x onerror=alert(1)>\")".into(),
            }],
            ..Default::default()
        };
        let original = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        let page = super::to_html_with_calls(
            &sample_graph(),
            None,
            0,
            &cyclic_edges(),
            &findings(),
            None,
            &calls,
        );
        let data = payload(&page);
        assert_eq!(data["calls"], serde_json::to_value(&calls).unwrap());
        assert_eq!(data["nodes"], payload(&original)["nodes"]);
        assert_eq!(data["edges"], payload(&original)["edges"]);
        assert!(page.contains("<\\/script><img"));
        assert!(!page.contains("</script><img"));
    }

    #[test]
    fn large_call_payload_pools_symbols_and_actual_viewer_decodes_losslessly() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let name = format!(
            "csharp:external:Assembly, Version=8.0.0.0, PublicKeyToken={}::Namespace.Type",
            "abc".repeat(70)
        );
        let site = archguard_core::MethodCall {
            from_class: name.clone(),
            from_method: Some(format!("{name}#Run()")),
            to_class: Some(name.clone()),
            to_method: None,
            target_label: format!("{name}.Save()"),
            dispatch: "static".into(),
            resolution: "external".into(),
            file: "src/ä.cs".into(),
            line: 7,
            column: 12,
            code: "Save(\"</script><img src=x>\")".into(),
        };
        let mut calls = archguard_core::CallAnalysis { status: "partial".into(), files: 2,
            classes: vec![serde_json::from_value(serde_json::json!({"id":name,"name":"Type","package":"Namespace","file":"src/ä.cs","graph_node":"Namespace/ä.cs","line":1,"kind":"class","supertypes":["External.Base"]})).unwrap()],
            methods: vec![serde_json::from_value(serde_json::json!({"id":format!("{name}#Run()"),"class_id":name,"name":"Run","signature":"Run()","file":"src/ä.cs","line":2,"is_static":false})).unwrap()],
            calls: (0..1200).map(|line| { let mut call=site.clone(); call.line=line; call }).collect(),
            diagnostics: vec!["Compiler diagnostic stays literal".into()], ..Default::default() };
        calls.methods[0].complexity = Some(archguard_core::MethodComplexity {
            cyclomatic: 7,
            max_nesting: 3,
            definition: "ast-decisions-v1".into(),
        });
        calls.classes[0].supertype_sites = Some(vec![archguard_core::CallSupertypeSite {
            target: "External.Base".into(),
            file: "src/ä.cs".into(),
            line: 1,
        }]);
        calls.classes[0].declaration_files = Some(vec!["src/ä.cs".into()]);
        calls.classes.push(
            serde_json::from_value(serde_json::json!({
                "id":"Other","name":"Other","package":"Namespace","file":"Other.cs",
                "graph_node":"Namespace/Other.cs","line":1,"kind":"class",
                "supertype_sites":[],"declaration_files":["Other.cs"]
            }))
            .unwrap(),
        );
        let expected = serde_json::to_value(&calls).unwrap();
        let page = super::to_html_with_calls(
            &sample_graph(),
            None,
            0,
            &cyclic_edges(),
            &findings(),
            None,
            &calls,
        );
        let encoded = payload(&page)["calls"].clone();
        assert_eq!(encoded["compact"]["encoding"], "string-pool-v1");
        assert!(
            serde_json::to_vec(&encoded).unwrap().len()
                < serde_json::to_vec(&expected).unwrap().len() / 2,
            "repeated compiler identities must not dominate the report transport"
        );
        assert!(!page.contains("</script><img"));
        let viewer = include_str!("viewer.js");
        let start = viewer.find("function decodeCallPayload(").unwrap();
        let end = start + viewer[start..].find("\n}").unwrap() + 2;
        let script = format!(
            "const fs=require('node:fs');{};process.stdout.write(JSON.stringify(decodeCallPayload(JSON.parse(fs.readFileSync(0,'utf8')))));",
            &viewer[start..end]
        );
        let mut child = Command::new("node")
            .args(["-e", &script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Node for actual viewer decoder");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&encoded).unwrap())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            expected
        );
    }

    #[test]
    fn to_html_loads_the_pinned_network_library_only_on_demand() {
        let page = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        assert!(page.starts_with("<!DOCTYPE html>"));
        assert!(page.contains(FORCE_GRAPH_URL), "pinned CDN URL missing");
        assert!(
            page.contains(&format!("const GRAPH_LIBRARY_URL='{FORCE_GRAPH_URL}';")),
            "lazy network loader must use the pinned URL"
        );
        assert!(page.contains("script.async=true;script.src=GRAPH_LIBRARY_URL;"));
        assert!(page.contains("prefers-color-scheme"), "no dark/light CSS");
        assert!(page.contains("<input id=\"filter\""), "no filter input");
        assert!(
            page.contains("REQUIRES AN INTERNET CONNECTION"),
            "no online-requirement comment"
        );
        // No parser-blocking external resource. Only choosing Network creates
        // the optional script, so default SVG views work entirely offline.
        assert_eq!(page.matches("<script src=").count(), 0);
        assert!(!page.contains("<link"));
    }

    #[test]
    fn to_html_payload_carries_full_graph_findings_and_cyclic_edges() {
        let page = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        assert!(page.contains(r#"id="archguard-data" type="application/json""#));
        let value = payload(&page);
        assert_eq!(value["root"], "/abs/projekt");
        // The FULL hierarchical graph is embedded, not just the start cut.
        let nodes = value["nodes"].as_array().expect("nodes array");
        assert_eq!(nodes.len(), 6, "all nodes (incl. hidden parents) embedded");
        let by_name = |want: &str| {
            nodes
                .iter()
                .find(|n| n["name"] == want)
                .unwrap_or_else(|| panic!("node {want} missing"))
                .clone()
        };
        assert_eq!(by_name("m")["kind"], "module");
        assert_eq!(by_name("m")["parent"], serde_json::Value::Null);
        assert_eq!(by_name("a")["kind"], "file");
        assert_eq!(by_name("a")["parent"], 5, "a hangs under m");
        assert_eq!(by_name("a")["lang"], "ts");
        // Edges carry their sampled refs (file:line:raw) for hover.
        let edges = value["edges"].as_array().expect("edges array");
        assert_eq!(edges.len(), 6);
        assert_eq!(edges[0]["from"], 0);
        assert_eq!(edges[0]["to"], 1);
        let refs = edges[0]["refs"].as_array().expect("refs array");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0]["file"], "0.ts");
        assert_eq!(refs[0]["line"], 1);
        assert_eq!(refs[0]["raw"], "import …");
        // F12: the exact total travels with every edge (builder-shaped:
        // a → b carries 2); a legacy edge would be `null` here.
        assert_eq!(edges[0]["count"], serde_json::json!(2));
        assert_eq!(edges[1]["count"], serde_json::json!(1));
        // Cyclic pairs and findings are embedded for the client.
        assert_eq!(
            value["cyclicEdges"],
            serde_json::json!([[0, 1], [1, 0]]),
            "cyclic edges must be embedded sorted"
        );
        let embedded_findings = value["findings"].as_array().expect("findings array");
        assert_eq!(embedded_findings.len(), 1);
        assert_eq!(embedded_findings[0]["id"], "scc-1");
        assert_eq!(embedded_findings[0]["kind"], "cycle");
        assert_eq!(embedded_findings[0]["nodes"], serde_json::json!(["a", "b"]));
        assert_eq!(embedded_findings[0]["suggested_cuts"][0]["from"], "a");
    }

    #[test]
    fn to_html_focus_pins_the_start_view_but_keeps_the_full_graph() {
        let page = to_html(
            &sample_graph(),
            Some("a"),
            1,
            &cyclic_edges(),
            &findings(),
            None,
        );
        let value = payload(&page);
        // The full graph stays embedded so drill-down can reveal more.
        assert_eq!(value["nodes"].as_array().expect("nodes").len(), 6);
        assert_eq!(value["edges"].as_array().expect("edges").len(), 6);
        // The start view is the focus subset (a, b, c) — v1 semantics.
        assert_eq!(value["view"]["focus"], "a");
        assert_eq!(value["view"]["depth"], 1);
        assert_eq!(value["view"]["visible"], serde_json::json!([0, 1, 2]));
        // Without focus, visible is null and the client applies depth.
        let page = to_html(&sample_graph(), None, 2, &cyclic_edges(), &findings(), None);
        let value = payload(&page);
        assert_eq!(value["view"]["focus"], serde_json::Value::Null);
        assert_eq!(value["view"]["depth"], 2);
        assert_eq!(value["view"]["visible"], serde_json::Value::Null);
        assert_eq!(
            page,
            to_html(&sample_graph(), None, 2, &cyclic_edges(), &findings(), None)
        );
        assert!(page.ends_with('\n'));
    }

    #[test]
    fn page_contains_the_v2_panel_dom_markers() {
        let page = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        for marker in [
            "id=\"tabs\"",
            "id=\"breadcrumbs\"",
            "id=\"sidebar\"",
            "id=\"tab-graph\"",
            "id=\"tab-cycles\"",
            "id=\"tab-dsm\"",
            "id=\"cycles-panel\"",
            "id=\"dsm-tab\"",
            "id=\"ops-strip\"",
            "id=\"undo-ops\"",
            "id=\"clear-ops\"",
            "id=\"export-ops\"",
            "id=\"graph\"",
            "Export SimOps",
        ] {
            assert!(page.contains(marker), "DOM marker missing: {marker}");
        }
    }

    #[test]
    fn embedded_script_mirrors_the_server_rust_semantics() {
        let page = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        for marker in [
            "mirrors Cut::at_depth",
            "mirrors Cut::expand",
            "aggregate mirror",
            "nearest_visible — mirrors",
            "mirrors apply_ops",
            "mirror of scc.rs internal_edges",
            "lakos mirror",
            "function findSccs",
            "function viewAcd",
            "function replayOps",
            "function recordOp",
            "dblclick",
            "screen2GraphCoords",
            "function containmentLayout",
            "onRenderFramePre",
            "enableNodeDrag(false)",
            "onLinkClick",
            "linkState",
            "instability",
            "prefers-color-scheme",
        ] {
            assert!(
                page.contains(marker),
                "client mirror marker missing: {marker}"
            );
        }
    }

    #[test]
    fn embedded_script_uses_only_accessors_the_pinned_force_graph_has() {
        // force-graph 1.52.0 (the pinned CDN build) exposes NO linkOpacity
        // and NO onNodeDoubleClick accessor, and fg.refresh() does not
        // exist. Fades go through rgba colours, double-click is wired on
        // the canvas via screen2GraphCoords, and the render loop already
        // redraws every animation frame. Regression guard: these three
        // hallucinated APIs must never come back into the template.
        let page = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        for banned in [".linkOpacity(", ".onNodeDoubleClick(", "fg.refresh("] {
            assert!(
                !page.contains(banned),
                "not part of the force-graph 1.52 API: {banned}"
            );
        }
    }

    #[test]
    fn edge_geometry_follows_the_f17_scale_and_directed_pair_routing() {
        // F17 regression guard: directed pairs stay separate curved arrows
        // (constant linkCurvature — the pinned 1.52 build mirrors the arc
        // when endpoints swap, so A→B and B→A never overlap), the stroke
        // width is the bounded log scale widthFor() of the F12 total, the
        // arrowhead is clipped against the painted box, and per-direction
        // selection paints the exact count beside the selected path.
        let page = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        for marker in [
            // the documented width scale, cap included
            "function widthFor(count)",
            "Math.min(8, 1.5 + Math.log2(Math.max(1, count)))",
            // directed pair routing: curvature + enabled, boundary-clear arrowheads
            "const CURVATURE = 0.25",
            ".linkCurvature(() => CURVATURE)",
            "function arrowGeometry(l, gap=ARROW_GAP)",
            "function arrowFits(l, arrow, size, margin)",
            ".linkDirectionalArrowLength(0)",
            ".linkDirectionalArrowRelPos(1)",
            ".nodeRelSize(4)",
            ".nodeVal(n => Math.pow((radiusOf(n) + ARROW_GAP) / 4, 2))",
            // width carries the F12 total, never the sample
            ".linkWidth(l => widthFor(edgeWeight(l)))",
            // per-direction selection: count beside the path, background clears it
            ".onBackgroundClick(",
            ".linkCanvasObjectMode(",
            ".linkCanvasObject(",
            "selectedLinkKey",
            // static direction/width legend (F01 rule: createElement/textContent)
            "id=\"edge-legend\"",
            "function buildLegend()",
            "graphEvidenceUnit()+t(' per direction',' je Richtung')",
            // sidebar shows the selected edge with its exact count
            "weightLabel(edge)+' '+graphEvidenceUnit()",
        ] {
            assert!(page.contains(marker), "F17 marker missing: {marker}");
        }
    }

    #[test]
    fn op_encoder_emits_the_frozen_t37_simops_shape() {
        // Local pin of the frozen wire format from
        // crates/archguard-analysis/src/simulate.rs (externally tagged,
        // snake_case; the report crate must NOT depend on
        // archguard-analysis, so the shape is pinned structurally with
        // serde_json::Value instead of the SimOp enum itself).
        const GOLDEN: &str = r#"[{"cut_edge":{"from":"app/billing","to":"domain/money"}},{"move_node":{"node":"domain/x","new_parent":"app"}}]"#;
        let ops: Vec<serde_json::Value> = serde_json::from_str(GOLDEN).expect("golden parses");
        assert_eq!(ops.len(), 2);
        let one_key = |v: &serde_json::Value| -> Vec<String> {
            v.as_object()
                .expect("op is an object")
                .keys()
                .cloned()
                .collect()
        };
        assert_eq!(one_key(&ops[0]), ["cut_edge"], "one externally tagged key");
        assert_eq!(one_key(&ops[1]), ["move_node"]);
        let keys_of = |v: &serde_json::Value| {
            let mut k: Vec<String> = v
                .as_object()
                .expect("op payload is an object")
                .keys()
                .cloned()
                .collect();
            k.sort();
            k
        };
        assert_eq!(keys_of(&ops[0]["cut_edge"]), ["from", "to"]);
        assert_eq!(keys_of(&ops[1]["move_node"]), ["new_parent", "node"]);
        // serde_json's default Map sorts keys, so a byte-exact re-check
        // against GOLDEN is impossible without the (forbidden) serde
        // derive/feature: pin VALUE equality against the contract and a
        // stable serialize→parse round trip instead. The byte-exact
        // string lives in archguard-analysis's own contract test.
        assert_eq!(
            ops[0],
            serde_json::json!({"cut_edge": {"from": "app/billing", "to": "domain/money"}})
        );
        assert_eq!(
            ops[1],
            serde_json::json!({"move_node": {"node": "domain/x", "new_parent": "app"}})
        );
        let round_trip: Vec<serde_json::Value> =
            serde_json::from_str(&serde_json::to_string(&ops).expect("serialize"))
                .expect("round trip");
        assert_eq!(round_trip, ops, "serialization is value-stable");
        // The viewer's encoder functions build exactly these shapes.
        let page = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        assert!(
            page.contains("return { cut_edge: { from: from, to: to } };"),
            "JS cut encoder deviates from the frozen shape"
        );
        assert!(
            page.contains("return { move_node: { node: node, new_parent: newParent } };"),
            "JS move encoder deviates from the frozen shape"
        );
        assert!(
            page.contains("JSON.stringify(ops)"),
            "export uses the ops array"
        );
    }

    #[test]
    fn payload_escapes_script_terminating_sequences() {
        let mut g = sample_graph();
        g.nodes[2].name = "</script><b>x</b>".to_string();
        let page = to_html(&g, None, 0, &cyclic_edges(), &findings(), None);
        assert!(
            page.contains("<\\/script"),
            "`</` must be escaped to `<\\/`"
        );
        // Still exactly one closing script tag for the payload element,
        // and the payload parses back with the escaped name intact.
        let value = payload(&page);
        assert_eq!(value["nodes"][2]["name"], "</script><b>x</b>");
    }

    #[test]
    fn payload_cannot_switch_the_script_element_into_double_escaped_state() {
        let mut g = sample_graph();
        g.nodes[2].name = "<!--<script>x".to_string();
        let page = to_html(&g, None, 0, &cyclic_edges(), &findings(), None);
        // `<!--<script` would make the HTML parser ignore the real `</script>`
        // and swallow the application script behind the payload.
        assert!(!page.contains("<!--<script>x"));
        assert_eq!(payload(&page)["nodes"][2]["name"], "<!--<script>x");
    }

    #[test]
    fn graph_tooltip_styles_target_the_library_tooltip_not_the_canvas_container() {
        let page = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        // force-graph 1.52 nests .float-tooltip-kap inside its container.
        // Styling #graph > div paints the container, not the floating label.
        assert!(
            page.contains("#graph .float-tooltip-kap"),
            "the generated report must style the tooltip class used by force-graph"
        );
        assert!(!page.contains("#graph > div {"));
        assert!(!page.contains("#graph .graph-tooltip"));
    }

    #[test]
    fn tooltip_labels_render_untrusted_values_as_text() {
        // F01: tooltip values come from the source code itself (node names,
        // paths, raw imports) and may legally contain HTML. The label
        // accessors must hand force-graph text-only elements, never HTML
        // strings — the library inserts strings via innerHTML.
        let mut g = sample_graph();
        g.nodes[2].name = "src/<img src=x onerror=alert(1)>.ts".to_string();
        g.edges[0].refs[0].raw =
            "import evil from \"./evil\"; // <img src=x onerror=alert(1)> </script>".to_string();
        let page = to_html(&g, None, 0, &cyclic_edges(), &findings(), None);
        // The payload keeps the literal text (displayable as-is) …
        let value = payload(&page);
        assert_eq!(
            value["nodes"][2]["name"],
            "src/<img src=x onerror=alert(1)>.ts"
        );
        assert!(
            value["edges"][0]["refs"][0]["raw"]
                .as_str()
                .expect("raw string")
                .contains("<img src=x onerror=alert(1)>"),
            "raw import text must survive verbatim"
        );
        // … while the script-breaking sequence stays escaped and the page
        // still has exactly the two real script closers (payload/app)
        // — no injected closer, no breakout.
        assert!(page.contains("<\\/script"));
        assert_eq!(page.matches("</script>").count(), 2);
        // Both label accessors go through the text-only element builder,
        // whose lines exclusively become text nodes.
        assert!(
            page.contains("function tooltipEl("),
            "tooltip helper missing"
        );
        assert!(
            page.contains(".nodeLabel(n => tooltipEl("),
            "nodeLabel does not build a text-only element"
        );
        assert!(
            page.contains(".linkLabel(l => tooltipEl("),
            "linkLabel does not build a text-only element"
        );
        assert!(
            page.contains("document.createTextNode(String(line))"),
            "tooltip lines must go through text nodes"
        );
    }

    #[test]
    fn initial_tab_is_embedded_and_mirrored_by_the_client_hook() {
        // T40: `cycles --format html` opens the cycles panel through the
        // payload's initialTab field.
        let page = to_html(
            &sample_graph(),
            None,
            0,
            &cyclic_edges(),
            &findings(),
            Some("cycles"),
        );
        assert_eq!(payload(&page)["initialTab"], "cycles");
        assert!(
            page.contains("data.initialTab === 'cycles'"),
            "client hook for the initial tab missing"
        );
        // None (the `graph` call sites) stays null — default graph tab.
        let page = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        assert_eq!(payload(&page)["initialTab"], serde_json::Value::Null);
    }

    #[test]
    fn review_kpi_payload_is_embedded_without_a_dashboard_tab() {
        // The graph's review_kpis block reaches the client for the Start
        // panels; the former Review Dashboard tab and its payload are gone.
        let mut graph = sample_graph();
        graph.review_kpis = Some(archguard_core::ReviewKpis {
            tangledness: Some(archguard_core::Tangledness {
                tangled_nodes: 2,
                total: 5,
                quote: 0.4,
            }),
            ..Default::default()
        });
        let page = to_html(&graph, None, 0, &cyclic_edges(), &findings(), None);
        for removed in [
            "tab-review-dashboard",
            "data-tab=\"review-dashboard\"",
            "renderReviewDashboard",
            "renderDashDecisions",
            "renderDashPr",
            "/api/review-dashboard",
            "/api/decisions",
        ] {
            assert!(!page.contains(removed), "{removed} must not ship");
        }
        // The measured tangledness quote reaches the client verbatim.
        assert_eq!(payload(&page)["reviewKpis"]["tangledness"]["quote"], 0.4);
        // R39 input: what the caller provides reaches the client.
        let history = serde_json::json!([{"tag": "snap-1"}]);
        let page = super::to_html_with_context(
            &graph,
            None,
            0,
            &cyclic_edges(),
            &findings(),
            None,
            &super::HtmlContext {
                review_history: Some(&history),
                ..Default::default()
            },
        );
        assert_eq!(payload(&page)["reviewHistory"][0]["tag"], "snap-1");
        for key in ["reviewRegister", "prComment"] {
            assert!(payload(&page).get(key).is_none(), "{key} must be gone");
        }
        // A graph without the KPI pass — and a context without history —
        // embeds both as null, not as zeros or empty lists.
        let bare = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        assert!(
            payload(&bare)["reviewKpis"].is_null(),
            "not measured stays null"
        );
        assert!(
            payload(&bare)["reviewHistory"].is_null(),
            "reviewHistory must stay null when not provided"
        );
        // Zones need compiler evidence the cached graph KPIs lack; the
        // caller's completed KPI block replaces the graph's.
        let completed = archguard_core::ReviewKpis {
            zones: Some(archguard_core::ZoneStats {
                zone_of_pain: 1,
                ..Default::default()
            }),
            ..Default::default()
        };
        let page = super::to_html_with_context(
            &sample_graph(),
            None,
            0,
            &cyclic_edges(),
            &findings(),
            None,
            &super::HtmlContext {
                review_kpis: Some(&completed),
                ..Default::default()
            },
        );
        assert_eq!(payload(&page)["reviewKpis"]["zones"]["zone_of_pain"], 1);
    }

    #[test]
    fn dash_kpi_and_zone_panels_embed_kpis_and_abstractions() {
        // R37/R38: the KPI card grid (viewer_dash_kpis.js) and the A/I zone
        // scatter (viewer_dash_zones.js) ship in the page; the review KPIs
        // and the abstraction points they render reach the client verbatim.
        let mut graph = sample_graph();
        graph.review_kpis = Some(archguard_core::ReviewKpis {
            sdp_violations: vec![archguard_core::SdpViolation {
                from: "m".to_string(),
                to: "c".to_string(),
                i_from: Some(0.2),
                i_to: Some(0.8),
                evidence: vec!["a.ts:1".to_string()],
            }],
            tangledness: Some(archguard_core::Tangledness {
                tangled_nodes: 2,
                total: 5,
                quote: 0.4,
            }),
            health: Some(archguard_core::HealthIndex {
                score: Some(72.5),
                level: Some(archguard_core::HealthLevel::Yellow),
                trend: Some(-3.5),
                parts: Default::default(),
            }),
            ..Default::default()
        });
        // AbstractionMetric carries no Default: every field is explicit.
        let performance = archguard_core::PerformanceReport {
            abstractions: vec![archguard_core::AbstractionMetric {
                scope: "node".to_string(),
                name: "m".to_string(),
                nodes: vec!["m".to_string()],
                types: 4,
                abstract_types: 2,
                fan_in: 3,
                fan_out: 1,
                instability: 0.25,
                abstractness: Some(0.5),
                distance: None,
                coverage: "complete".to_string(),
                reason: None,
            }],
            ..Default::default()
        };
        let page = super::to_html_with_context(
            &graph,
            None,
            0,
            &cyclic_edges(),
            &findings(),
            None,
            &super::HtmlContext {
                performance: Some(&performance),
                ..Default::default()
            },
        );
        assert!(
            page.contains("function renderDashKpis"),
            "KPI card panel not embedded"
        );
        assert!(
            page.contains("function renderDashZones"),
            "zone scatter panel not embedded"
        );
        let value = payload(&page);
        assert_eq!(value["reviewKpis"]["tangledness"]["quote"], 0.4);
        assert_eq!(value["reviewKpis"]["sdp_violations"][0]["from"], "m");
        assert_eq!(value["reviewKpis"]["health"]["trend"], -3.5);
        // The zone scatter plots this abstraction point by name.
        assert_eq!(value["performance"]["abstractions"][0]["name"], "m");
        assert_eq!(value["performance"]["abstractions"][0]["abstractness"], 0.5);
        // Honest-null path: without the KPI pass and without performance
        // context the page is still produced and both blocks stay null —
        // the panels then render their "not measured" states, never zeros.
        let bare = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        assert!(
            bare.contains("function renderDashKpis") && bare.contains("function renderDashZones"),
            "panels must ship even without measured data"
        );
        assert!(
            payload(&bare)["reviewKpis"].is_null(),
            "not measured stays null"
        );
        assert!(
            payload(&bare)["performance"].is_null(),
            "absent performance context stays null, never an empty report"
        );
    }

    #[test]
    fn dash_health_panel_embeds_history() {
        // R39: the health panel (viewer_dash_health.js) ships in the page;
        // the health index and the snapshot health series reach the client
        // verbatim — and stay null when absent, so the panel renders its
        // honest "not measured" state, never zeros.
        let mut parts = std::collections::BTreeMap::new();
        parts.insert("coverage".to_string(), 0.8);
        parts.insert("tangledness".to_string(), 0.4);
        let mut graph = sample_graph();
        graph.review_kpis = Some(archguard_core::ReviewKpis {
            health: Some(archguard_core::HealthIndex {
                score: Some(71.5),
                level: Some(archguard_core::HealthLevel::Green),
                trend: Some(2.0),
                parts,
            }),
            ..Default::default()
        });
        let history = serde_json::json!([
            {"timestamp": 1, "tag": null, "score": null},
            {"timestamp": 2, "tag": "snap", "score": 60.0},
            {"timestamp": 3, "tag": null, "score": 71.5}
        ]);
        let page = super::to_html_with_context(
            &graph,
            None,
            0,
            &cyclic_edges(),
            &findings(),
            None,
            &super::HtmlContext {
                review_history: Some(&history),
                ..Default::default()
            },
        );
        assert!(
            page.contains("function renderDashHealth"),
            "health panel not embedded"
        );
        let value = payload(&page);
        assert_eq!(value["reviewKpis"]["health"]["score"], 71.5);
        assert_eq!(value["reviewKpis"]["health"]["level"], "green");
        assert_eq!(value["reviewKpis"]["health"]["trend"], 2.0);
        assert_eq!(value["reviewKpis"]["health"]["parts"]["coverage"], 0.8);
        // The honest null story inside the series: a pre-R28 snapshot
        // without KPIs keeps score null — never plotted as a zero.
        assert!(value["reviewHistory"][0]["score"].is_null());
        assert_eq!(value["reviewHistory"][1]["tag"], "snap");
        assert_eq!(value["reviewHistory"][2]["score"], 71.5);
        // Honest-null path without any context: the panel still ships and
        // the key stays null, never an empty list.
        let bare = to_html(&sample_graph(), None, 0, &cyclic_edges(), &findings(), None);
        assert!(
            bare.contains("function renderDashHealth"),
            "panel must ship even without measured data"
        );
        assert!(
            payload(&bare)["reviewHistory"].is_null(),
            "absent history stays null"
        );
    }
}
