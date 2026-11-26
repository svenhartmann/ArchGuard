//! Tests for the diagram graph, ticket 04.

use archguard_engine::{SearchGraph, core::ServiceTopology, search_graph};
use serde_json::{Value, json};

use super::diagram::{BoxKind, DEdge, Diagram, EdgeStyle, diagram, diagram_field};
use super::types::{Key, Switches};

fn art(kind: &str, name: &str, refs: Value, detail: Value) -> Value {
    json!({"kind": kind, "name": name, "address": format!("{kind}.{name}"), "scope": "terraform:infra",
        "patterns": [], "references": refs, "evidence": [], "detail": detail})
}

fn analysis(k: &str, n: &str, d: &str) -> Value {
    json!({"kind": k, "name": n, "definition": d})
}

fn topo() -> (ServiceTopology, SearchGraph) {
    let products = json!({
        "fields": [{"path": "title", "kind": "text", "analyzer": "title_fold"},
                   {"path": "tag", "kind": "keyword", "normalizer": "lower_n"}],
        "settings": {"default_pipeline": "pipe"},
        "analysis": [
            analysis("analyzer", "title_fold", r#"{"type":"custom","tokenizer":"tok","filter":["lowercase","my_stop","icu_x"]}"#),
            analysis("analyzer", "default", r#"{"tokenizer":"standard"}"#),
            analysis("tokenizer", "tok", r#"{"type":"pattern"}"#),
            analysis("filter", "my_stop", r#"{"type":"stop"}"#),
            analysis("normalizer", "lower_n", r#"{"type":"custom","filter":["my_stop"]}"#)]});
    let other = json!({"fields": [{"path": "name", "kind": "text", "analyzer": "title_fold"}], "analysis": [
        analysis("analyzer", "title_fold", r#"{"type":"custom","tokenizer":"tok","filter":["lowercase","my_stop","icu_x"]}"#),
        analysis("analyzer", "x", r#"{"tokenizer":"standard"}"#)]});
    let elasticsearch = serde_json::from_value(json!({
        "artifacts": [
            art("index", "products", json!([{"kind": "ilm_policy", "name": "ilm1"}, {"kind": "ingest_pipeline", "name": "pipe"}]), products),
            art("alias", "prod-alias", json!([{"kind": "index", "name": "products"}]), Value::Null),
            art("ingest_pipeline", "pipe", json!([]), Value::Null),
            art("ilm_policy", "ilm1", json!([]), Value::Null),
            art("index", "other", json!([]), other),
            art("watch", "w1", json!([{"kind": "ingest_pipeline", "name": "pipe"}]), Value::Null)],
        "usages": [
            {"service": "api", "index": "prod-alias", "access": "write", "evidence": []},
            {"service": "reader", "index": "products", "access": "read", "evidence": []},
            {"service": "ghosty", "index": "missing", "access": "write", "evidence": []}],
        "findings": [{"kind": "undeclared_index", "subject": "missing", "services": ["ghosty"], "evidence": []}]
    }))
    .unwrap();
    let t = ServiceTopology {
        elasticsearch,
        ..Default::default()
    };
    let g = search_graph(&t);
    (t, g)
}

fn node(g: &SearchGraph, kind: &str, label: &str) -> Key {
    Key::Node(
        g.nodes
            .iter()
            .position(|n| n.kind == kind && n.label == label)
            .unwrap_or_else(|| panic!("no {kind} {label}")),
    )
}

fn part(g: &SearchGraph, owner: &str, kind: &str, name: &str) -> Key {
    let Key::Node(owner) = node(g, "index", owner) else {
        unreachable!()
    };
    Key::Part {
        owner,
        kind: kind.into(),
        name: name.into(),
    }
}

fn run(sel: &Key, cluster: Option<&[Key]>, sw: &Switches) -> (Diagram, SearchGraph) {
    let (t, g) = topo();
    (diagram(&t, &g, sel, cluster, sw), g)
}

fn has(d: &Diagram, from: &Key, to: &Key, style: EdgeStyle, label: &str) -> bool {
    d.edges.contains(&DEdge {
        from: from.clone(),
        to: to.clone(),
        style,
        label: label.into(),
        dim: false,
    })
}

fn count(d: &Diagram, kind: BoxKind) -> usize {
    d.boxes.iter().filter(|b| b.kind == kind).count()
}

fn keys(d: &Diagram) -> Vec<&Key> {
    d.boxes.iter().map(|b| &b.key).collect()
}

#[test]
fn index_selection_shows_neighbors_alias_services_and_declared_parts() {
    let (_, g) = topo();
    let sel = node(&g, "index", "products");
    let (d, g) = run(&sel, None, &Switches::default());
    assert_eq!(d.boxes[0].key, sel);
    assert!(d.boxes[0].selected);
    let (alias, api, reader) = (
        node(&g, "alias", "prod-alias"),
        node(&g, "service", "api"),
        node(&g, "service", "reader"),
    );
    let (pipe, ilm) = (
        node(&g, "ingest_pipeline", "pipe"),
        node(&g, "ilm_policy", "ilm1"),
    );
    for k in [&alias, &api, &reader, &pipe, &ilm] {
        assert!(keys(&d).contains(&k), "{k:?}");
    }
    assert!(!keys(&d).contains(&&node(&g, "index", "other")));
    assert!(has(&d, &alias, &sel, EdgeStyle::Alias, ""));
    assert!(has(&d, &api, &alias, EdgeStyle::Write, ""));
    assert!(has(&d, &reader, &sel, EdgeStyle::Read, ""));
    assert!(has(
        &d,
        &sel,
        &pipe,
        EdgeStyle::Pipeline,
        "default_pipeline"
    ));
    assert!(!has(&d, &sel, &pipe, EdgeStyle::Reference, ""));
    assert!(has(&d, &sel, &ilm, EdgeStyle::Lifecycle, "lifecycle"));
    let (tf, tok, stop) = (
        part(&g, "products", "analyzer", "title_fold"),
        part(&g, "products", "tokenizer", "tok"),
        part(&g, "products", "filter", "my_stop"),
    );
    assert!(has(&d, &sel, &tf, EdgeStyle::Uses, ""));
    assert!(has(
        &d,
        &sel,
        &part(&g, "products", "normalizer", "lower_n"),
        EdgeStyle::Uses,
        ""
    ));
    assert!(has(
        &d,
        &sel,
        &part(&g, "products", "analyzer", "default"),
        EdgeStyle::Uses,
        ""
    ));
    assert!(has(&d, &tf, &tok, EdgeStyle::Uses, ""));
    assert!(has(&d, &tf, &stop, EdgeStyle::Uses, ""));
    // `lowercase` is built in, `icu_x` undeclared: no boxes.
    assert_eq!(count(&d, BoxKind::Filter), 1);
    assert_eq!(count(&d, BoxKind::Analyzer), 2);
}

#[test]
fn service_selection_shows_only_its_index_side() {
    let (_, g) = topo();
    let api = node(&g, "service", "api");
    let (d, g) = run(&api, None, &Switches::default());
    let alias = node(&g, "alias", "prod-alias");
    assert_eq!(keys(&d), [&api, &alias]);
    assert_eq!(d.boxes[0].kind, BoxKind::Service);
    assert_eq!(d.boxes[1].kind, BoxKind::Alias);
    assert_eq!(d.edges.len(), 1);
    assert!(has(&d, &api, &alias, EdgeStyle::Write, ""));
}

#[test]
fn alias_selection_is_transparent_to_its_services() {
    let (_, g) = topo();
    let alias = node(&g, "alias", "prod-alias");
    let (d, g) = run(&alias, None, &Switches::default());
    let (api, products) = (node(&g, "service", "api"), node(&g, "index", "products"));
    assert!(keys(&d).contains(&&api) && keys(&d).contains(&&products));
    assert_eq!(count(&d, BoxKind::Analyzer), 0);
}

#[test]
fn index_next_to_an_alias_brings_the_alias_services_but_not_its_parts() {
    let (_, g) = topo();
    let pipe = node(&g, "ingest_pipeline", "pipe");
    let (d, g) = run(&pipe, None, &Switches::default());
    let products = node(&g, "index", "products");
    assert!(keys(&d).contains(&&products));
    assert_eq!(count(&d, BoxKind::Analyzer), 0);
    assert_eq!(
        d.boxes.iter().find(|b| b.key == pipe).unwrap().kind,
        BoxKind::Processing
    );
    assert!(has(
        &d,
        &products,
        &pipe,
        EdgeStyle::Pipeline,
        "default_pipeline"
    ));
}

#[test]
fn watch_selection_has_a_reference_edge_to_its_pipeline() {
    let (_, g) = topo();
    let w = node(&g, "watch", "w1");
    let (d, g) = run(&w, None, &Switches::default());
    let pipe = node(&g, "ingest_pipeline", "pipe");
    assert_eq!(keys(&d), [&w, &pipe]);
    assert!(has(&d, &w, &pipe, EdgeStyle::Reference, ""));
}

#[test]
fn part_selection_shows_owner_used_and_using_parts_of_that_owner() {
    let (_, g) = topo();
    let tok = part(&g, "products", "tokenizer", "tok");
    let (d, g) = run(&tok, None, &Switches::default());
    let tf = part(&g, "products", "analyzer", "title_fold");
    assert_eq!(keys(&d), [&tok, &node(&g, "index", "products"), &tf]);
    assert!(has(&d, &tf, &tok, EdgeStyle::Uses, ""));
    assert!(has(
        &d,
        &node(&g, "index", "products"),
        &tf,
        EdgeStyle::Uses,
        ""
    ));

    let (d, g) = run(&tf, None, &Switches::default());
    assert_eq!(count(&d, BoxKind::Tokenizer), 1);
    assert_eq!(count(&d, BoxKind::Filter), 1);
    assert_eq!(count(&d, BoxKind::Analyzer), 1);
    assert_eq!(count(&d, BoxKind::Index), 1);
    assert!(!keys(&d).contains(&&part(&g, "products", "normalizer", "lower_n")));
}

#[test]
fn ghost_index_is_a_ghost_and_carries_the_engine_warning() {
    let (_, g) = topo();
    let missing = node(&g, "index", "missing");
    let (d, g) = run(&missing, None, &Switches::default());
    let b = &d.boxes[0];
    assert!(b.ghost && b.warn && b.selected);
    assert_eq!(b.kind, BoxKind::Index);
    let ghosty = node(&g, "service", "ghosty");
    assert!(has(&d, &ghosty, &missing, EdgeStyle::Write, ""));
    let pipe = d
        .boxes
        .iter()
        .find(|b| b.key == node(&g, "ingest_pipeline", "pipe"));
    assert!(pipe.is_none());
}

#[test]
fn cluster_has_exactly_the_given_keys_their_parts_and_merges_equal_parts() {
    let (_, g) = topo();
    let (products, other) = (node(&g, "index", "products"), node(&g, "index", "other"));
    let cluster = [products.clone(), other.clone()];
    let (d, g) = run(&products, Some(&cluster), &Switches::default());
    let tf = part(&g, "products", "analyzer", "title_fold");
    assert!(keys(&d).contains(&&products) && keys(&d).contains(&&other));
    assert!(!keys(&d).contains(&&node(&g, "service", "api")));
    assert!(!keys(&d).contains(&&node(&g, "ingest_pipeline", "pipe")));
    // title_fold of both owners is one box, `x` stays separate.
    assert!(keys(&d).contains(&&tf));
    assert!(!keys(&d).contains(&&part(&g, "other", "analyzer", "title_fold")));
    assert_eq!(count(&d, BoxKind::Analyzer), 3);
    assert!(has(&d, &other, &tf, EdgeStyle::Uses, ""));
}

#[test]
fn switches_drop_services_and_analysis() {
    let (_, g) = topo();
    let sel = node(&g, "index", "products");
    let sw = Switches {
        services: false,
        ..Switches::default()
    };
    let (d, _) = run(&sel, None, &sw);
    assert_eq!(count(&d, BoxKind::Service), 0);
    assert!(d.edges.iter().all(|e| !matches!(
        e.style,
        EdgeStyle::Write | EdgeStyle::Read | EdgeStyle::Unknown
    )));
    assert!(count(&d, BoxKind::Analyzer) > 0);

    let sw = Switches {
        analysis: false,
        ..Switches::default()
    };
    let (d, _) = run(&sel, None, &sw);
    for k in [
        BoxKind::Analyzer,
        BoxKind::Normalizer,
        BoxKind::Tokenizer,
        BoxKind::Filter,
        BoxKind::CharFilter,
    ] {
        assert_eq!(count(&d, k), 0);
    }
    assert!(d.edges.iter().all(|e| e.style != EdgeStyle::Uses));
    assert_eq!(count(&d, BoxKind::Service), 2);
}

fn dim_names(d: &Diagram, dim: bool) -> Vec<String> {
    let mut v: Vec<String> = d
        .boxes
        .iter()
        .filter(|b| b.dim == dim)
        .map(|b| match &b.key {
            Key::Part { name, .. } => name.clone(),
            Key::Node(_) => format!("#{:?}", b.kind),
        })
        .collect();
    v.sort();
    v
}

#[test]
fn a_field_in_focus_keeps_its_analysis_chain_and_dims_the_rest() {
    let (t, g) = topo();
    let sel = node(&g, "index", "products");
    let d = diagram_field(&t, &g, &sel, None, &Switches::default(), Some("title"));
    // The index and the analyzer with the declared tokenizer and filter stay; icu_x is only a chip.
    assert_eq!(
        dim_names(&d, false),
        ["#Index", "my_stop", "title_fold", "tok"]
    );
    let dimmed = dim_names(&d, true);
    for name in ["default", "lower_n", "#Alias", "#Service", "#Processing"] {
        assert!(
            dimmed.iter().any(|n| n == name),
            "{name} is dim: {dimmed:?}"
        );
    }
    let edge = |from: &Key, to: &Key| {
        d.edges
            .iter()
            .find(|e| &e.from == from && &e.to == to)
            .unwrap_or_else(|| panic!("{from:?} {to:?}"))
    };
    let fold = part(&g, "products", "analyzer", "title_fold");
    assert!(!edge(&sel, &fold).dim);
    assert!(!edge(&fold, &part(&g, "products", "tokenizer", "tok")).dim);
    assert!(!edge(&fold, &part(&g, "products", "filter", "my_stop")).dim);
    assert!(edge(&sel, &part(&g, "products", "analyzer", "default")).dim);
    assert!(
        edge(
            &part(&g, "products", "normalizer", "lower_n"),
            &part(&g, "products", "filter", "my_stop")
        )
        .dim
    );
    assert!(
        d.edges
            .iter()
            .filter(|e| e.style != EdgeStyle::Uses)
            .all(|e| e.dim)
    );
}

#[test]
fn a_normalizer_field_keeps_the_normalizer_and_no_field_dims_nothing() {
    let (t, g) = topo();
    let sel = node(&g, "index", "products");
    let d = diagram_field(&t, &g, &sel, None, &Switches::default(), Some("tag"));
    assert_eq!(dim_names(&d, false), ["#Index", "lower_n", "my_stop"]);
    let d = diagram_field(&t, &g, &sel, None, &Switches::default(), None);
    assert!(d.boxes.iter().all(|b| !b.dim) && d.edges.iter().all(|e| !e.dim));
    let d = diagram_field(&t, &g, &sel, None, &Switches::default(), Some("nope"));
    assert!(
        d.boxes.iter().all(|b| !b.dim),
        "an unknown field dims nothing"
    );
}
