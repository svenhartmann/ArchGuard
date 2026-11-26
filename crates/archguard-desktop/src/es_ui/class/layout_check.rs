//! Tests for the layout, ticket 08.

use archguard_engine::{SearchGraph, core::ServiceTopology, search_graph};
use serde_json::{Value, json};

use super::boxes::{BoxContent, content};
use super::diagram::{BoxKind, Diagram, diagram};
use super::layout::{Scene, layout};
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
                   {"path": "name", "kind": "text", "analyzer": "name_fold"}],
        "settings": {"shards": "3"},
        "analysis": [
            analysis("analyzer", "title_fold", r#"{"type":"custom","tokenizer":"tok","filter":["lowercase","my_stop"]}"#),
            analysis("analyzer", "name_fold", r#"{"type":"custom","tokenizer":"tok2","filter":["my_stop"]}"#),
            analysis("tokenizer", "tok", r#"{"type":"pattern"}"#),
            analysis("tokenizer", "tok2", r#"{"type":"pattern"}"#),
            analysis("filter", "my_stop", r#"{"type":"stop"}"#)]});
    let elasticsearch = serde_json::from_value(json!({
        "artifacts": [
            art("index", "products", json!([]), products),
            art("index", "other", json!([]), json!({"fields": [{"path": "x", "kind": "keyword"}]})),
            art("alias", "prod-alias", json!([{"kind": "index", "name": "other"}]), Value::Null),
            art("ingest_pipeline", "pipe", json!([]), Value::Null)],
        "usages": [
            {"service": "api", "index": "prod-alias", "access": "write", "evidence": []},
            {"service": "reader", "index": "products", "access": "read", "evidence": []}],
        "findings": []
    }))
    .unwrap();
    let t = ServiceTopology {
        elasticsearch,
        ..Default::default()
    };
    let g = search_graph(&t);
    (t, g)
}

fn scene() -> (Diagram, Vec<BoxContent>, Scene) {
    let (t, g) = topo();
    let sw = Switches::default();
    let sel = Key::Node(g.nodes.iter().position(|n| n.label == "products").unwrap());
    let cluster: Vec<Key> = (0..g.nodes.len()).map(Key::Node).collect();
    let d = diagram(&t, &g, &sel, Some(&cluster), &sw);
    let cs: Vec<_> = d
        .boxes
        .iter()
        .map(|b| content(&t, &g, &b.key, &sw))
        .collect();
    let s = layout(&d, &cs);
    (d, cs, s)
}

fn col(k: BoxKind) -> usize {
    match k {
        BoxKind::Service | BoxKind::Processing => 0,
        BoxKind::Alias | BoxKind::Template | BoxKind::Index => 1,
        BoxKind::Analyzer | BoxKind::Normalizer => 2,
        _ => 3,
    }
}

/// Rows of the box's compartments: title and lines, plus one chip row if it has chips.
fn expected_h(c: &BoxContent) -> f32 {
    let rows: f32 = c
        .compartments
        .iter()
        .map(|k| 16.0 * (1 + k.lines.len()) as f32)
        .sum();
    let chips = c
        .compartments
        .iter()
        .filter(|k| !k.chips.is_empty())
        .count();
    48.0 + rows + 18.0 * chips as f32
}

#[test]
fn four_columns_no_overlap() {
    let (d, cs, s) = scene();
    assert_eq!(s.boxes.len(), d.boxes.len());
    assert!(d.boxes.iter().any(|b| b.kind == BoxKind::Tokenizer));
    let widths = [230.0, 300.0, 270.0, 190.0];
    for (b, p) in d.boxes.iter().zip(&s.boxes) {
        assert_eq!(p.w, widths[col(b.kind)], "{:?}", b.kind);
    }
    for (i, (a, pa)) in d.boxes.iter().zip(&s.boxes).enumerate() {
        for (b, pb) in d.boxes.iter().zip(&s.boxes).skip(i + 1) {
            let apart_x = pa.x + pa.w <= pb.x || pb.x + pb.w <= pa.x;
            let apart_y = pa.y + pa.h <= pb.y || pb.y + pb.h <= pa.y;
            assert!(apart_x || apart_y, "{:?} overlaps {:?}", a.key, b.key);
        }
    }
    let at = |kind, name: &str| {
        let i = (0..d.boxes.len())
            .find(|&i| d.boxes[i].kind == kind && cs[i].name == name)
            .unwrap_or_else(|| panic!("no {kind:?} {name}"));
        (i, s.boxes[i].clone())
    };
    let (_, alias) = at(BoxKind::Alias, "prod-alias");
    let (_, other) = at(BoxKind::Index, "other");
    assert!(
        alias.y + alias.h <= other.y && alias.x == other.x,
        "alias above its index"
    );
    let (ia, a1) = at(BoxKind::Analyzer, "title_fold");
    let (_, t1) = at(BoxKind::Tokenizer, "tok");
    assert_eq!(t1.y, a1.y, "leaf on the height of its first analyzer");
    let (ip, products) = at(BoxKind::Index, "products");
    assert_eq!(products.h, expected_h(&cs[ip]));
    assert_eq!(a1.h, expected_h(&cs[ia]));
}
