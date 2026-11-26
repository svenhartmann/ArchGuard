//! Tests for the layout, ticket 08.

use archguard_engine::{SearchGraph, core::ServiceTopology, search_graph};
use serde_json::{Value, json};

use super::boxes::{BoxContent, content};
use super::boxes::{Compartment, Line, Title};
use super::diagram::{BoxKind, DBox, DEdge, Diagram, EdgeStyle, diagram};
use super::layout::{Scene, layout};
use super::types::{Key, Switches};
use crate::diagram::HeadShape;

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

/// A part's content carries its owner's label as name, so parts are found by key.
fn label<'a>(k: &'a Key, c: &'a BoxContent) -> &'a str {
    match k {
        Key::Part { name, .. } => name,
        Key::Node(_) => &c.name,
    }
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
            .find(|&i| d.boxes[i].kind == kind && label(&d.boxes[i].key, &cs[i]) == name)
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
    assert!(
        a1.y < at(BoxKind::Analyzer, "name_fold").1.y,
        "title_fold is used first"
    );
    assert_eq!(t1.y, a1.y, "leaf on the height of its first analyzer");
    let (ip, products) = at(BoxKind::Index, "products");
    assert_eq!(products.h, expected_h(&cs[ip]));
    assert_eq!(a1.h, expected_h(&cs[ia]));
}

fn dbox(i: usize, kind: BoxKind) -> DBox {
    DBox {
        key: Key::Node(i),
        kind,
        ghost: false,
        warn: false,
        selected: i == 0,
    }
}

fn dedge(a: usize, b: usize, style: EdgeStyle) -> DEdge {
    DEdge {
        from: Key::Node(a),
        to: Key::Node(b),
        style,
        label: String::new(),
    }
}

/// `n` groups of service, alias, index, analyzer and tokenizer, wired in a chain.
fn synthetic(n: usize) -> (Diagram, Vec<BoxContent>) {
    let kinds = [
        BoxKind::Service,
        BoxKind::Alias,
        BoxKind::Index,
        BoxKind::Analyzer,
        BoxKind::Tokenizer,
    ];
    let mut d = Diagram::default();
    for i in 0..n * 5 {
        d.boxes.push(dbox(i, kinds[i % 5]));
    }
    let styles = [
        EdgeStyle::Write,
        EdgeStyle::Alias,
        EdgeStyle::Uses,
        EdgeStyle::Uses,
    ];
    for g in 0..n {
        for (k, &s) in styles.iter().enumerate() {
            d.edges.push(dedge(g * 5 + k, g * 5 + k + 1, s));
        }
        if g + 1 < n {
            d.edges
                .push(dedge(g * 5 + 3, (g + 1) * 5 + 3, EdgeStyle::Uses));
        }
    }
    let line = Line {
        text: "field".into(),
        ..Default::default()
    };
    let comp = Compartment {
        title: Title::Fields,
        lines: vec![line; 4],
        chips: Vec::new(),
    };
    let cs = (0..n * 5)
        .map(|_| BoxContent {
            compartments: vec![comp.clone()],
            ..Default::default()
        })
        .collect();
    (d, cs)
}

#[test]
fn edge_ends_follow_the_style() {
    let mut d = Diagram::default();
    let kinds = [
        BoxKind::Service,
        BoxKind::Template,
        BoxKind::Index,
        BoxKind::Alias,
        BoxKind::Analyzer,
    ];
    for (i, k) in kinds.iter().enumerate() {
        d.boxes.push(dbox(i, *k));
    }
    let wanted = [
        (EdgeStyle::Write, HeadShape::Filled, false),
        (EdgeStyle::Read, HeadShape::Filled, false),
        (EdgeStyle::Covers, HeadShape::Hollow, true),
        (EdgeStyle::Alias, HeadShape::Diamond, false),
        (EdgeStyle::Uses, HeadShape::Open, true),
        (EdgeStyle::Pipeline, HeadShape::Open, false),
    ];
    for (k, (s, _, _)) in wanted.iter().enumerate() {
        d.edges.push(dedge(k % 2, 2 + k % 3, *s));
    }
    let cs = vec![BoxContent::default(); 5];
    let scene = layout(&d, &cs);
    for (e, (style, shape, dashed)) in scene.edges.iter().zip(wanted) {
        assert_eq!((e.style, e.shape, e.dashed), (style, shape, dashed));
        let closed = e.head.contains('Z');
        assert_eq!(closed, shape != HeadShape::Open, "{style:?}: {}", e.head);
        assert!(!e.line.is_empty() && !e.line.contains('Z'));
    }
}

#[test]
fn layout_time_grows_about_linearly() {
    let best = |n: usize| {
        let (d, cs) = synthetic(n);
        (0..3)
            .map(|_| {
                let t = std::time::Instant::now();
                std::hint::black_box(layout(&d, &cs));
                t.elapsed().as_secs_f64()
            })
            .fold(f64::MAX, f64::min)
    };
    let (small, large) = (best(200), best(800));
    assert!(large / small < 8.0, "{small} -> {large}");
}

/// The alias diamond lies outside the alias box, along the edge toward the target.
#[test]
fn the_alias_diamond_sits_outside_its_box() {
    let mut d = Diagram::default();
    d.boxes = vec![dbox(0, BoxKind::Alias), dbox(1, BoxKind::Index)];
    d.edges = vec![dedge(0, 1, EdgeStyle::Alias)];
    let cs = vec![BoxContent::default(); 2];
    let s = layout(&d, &cs);
    let diamond = s.edges[0].head.split('Z').next().unwrap();
    let nums: Vec<f32> = diamond
        .split_whitespace()
        .filter_map(|w| w.parse().ok())
        .collect();
    let b = &s.boxes[0];
    assert!(nums.len() >= 8, "{diamond}");
    for xy in nums.chunks(2) {
        let inside = xy[0] > b.x && xy[0] < b.x + b.w && xy[1] > b.y && xy[1] < b.y + b.h;
        assert!(!inside, "{xy:?} inside {b:?}: {diamond}");
    }
}
