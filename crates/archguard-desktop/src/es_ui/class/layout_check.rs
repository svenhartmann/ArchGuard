//! Tests for the layout, ticket 08.

use archguard_engine::{SearchGraph, core::ServiceTopology, search_graph};
use serde_json::{Value, json};

use super::boxes::{BoxContent, content};
use super::boxes::{Compartment, Line, Title};
use super::diagram::{BoxKind, DBox, DEdge, Diagram, EdgeStyle, diagram};
use super::layout::{Scene, field_rows, layout};
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
        dim: false,
    }
}

fn dedge(a: usize, b: usize, style: EdgeStyle) -> DEdge {
    DEdge {
        from: Key::Node(a),
        to: Key::Node(b),
        style,
        label: String::new(),
        dim: false,
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
    let d = Diagram {
        boxes: vec![dbox(0, BoxKind::Alias), dbox(1, BoxKind::Index)],
        edges: vec![dedge(0, 1, EdgeStyle::Alias)],
    };
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

#[test]
fn field_rows_give_the_top_of_every_field_line_relative_to_the_box() {
    let field = |p: &str| Line {
        path: p.into(),
        text: p.into(),
        ..Default::default()
    };
    let setting = Line {
        text: "a = 1".into(),
        ..Default::default()
    };
    let content = BoxContent {
        compartments: vec![
            Compartment {
                title: Title::Fields,
                lines: vec![field("a"), field("a.b")],
                chips: Vec::new(),
            },
            Compartment {
                title: Title::Settings,
                lines: vec![setting],
                chips: Vec::new(),
            },
        ],
        ..Default::default()
    };
    let mut d = Diagram::default();
    d.boxes.push(dbox(0, BoxKind::Index));
    let scene = layout(&d, std::slice::from_ref(&content));
    let p = &scene.boxes[0];
    let rows = field_rows(&content, p);
    // The header is 48 px; the compartment title takes a row, then one row per line.
    assert_eq!(
        rows,
        [
            (p.comps[0] + 16.0, "a".to_string()),
            (p.comps[0] + 32.0, "a.b".to_string())
        ]
    );
    assert_eq!(p.comps[0], 48.0);
}

// ---- ticket 16: edges and labels never over a foreign box ----

use super::layout::{PlacedEdge, label_size};
use archguard_engine::analyze_services;

/// Index selections and the full cluster of fixture 12, laid out.
fn fixture_scenes() -> Vec<(Diagram, Scene)> {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/12");
    let t = analyze_services(&root).expect("fixture");
    let g = search_graph(&t);
    let sw = Switches::default();
    let all: Vec<Key> = (0..g.nodes.len()).map(Key::Node).collect();
    let mut out = Vec::new();
    for i in 0..g.nodes.len() {
        for cluster in [None, Some(all.as_slice())] {
            let d = diagram(&t, &g, &Key::Node(i), cluster, &sw);
            let cs: Vec<_> = d
                .boxes
                .iter()
                .map(|b| content(&t, &g, &b.key, &sw))
                .collect();
            let s = layout(&d, &cs);
            out.push((d, s));
        }
    }
    out
}

/// Points along the path commands (`M`, `L`, `C` sampled, `Z`), as separate sub-paths.
fn polylines(path: &str) -> Vec<Vec<(f32, f32)>> {
    let tok: Vec<&str> = path.split_whitespace().collect();
    let num = |i: usize| {
        (
            tok[i].parse::<f32>().unwrap(),
            tok[i + 1].parse::<f32>().unwrap(),
        )
    };
    let (mut out, mut i): (Vec<Vec<(f32, f32)>>, usize) = (Vec::new(), 0);
    while i < tok.len() {
        match tok[i] {
            "M" => {
                out.push(vec![num(i + 1)]);
                i += 3;
            }
            "L" => {
                out.last_mut().unwrap().push(num(i + 1));
                i += 3;
            }
            "C" => {
                let cur = out.last().unwrap().last().copied().unwrap();
                let (c1, c2, e) = (num(i + 1), num(i + 3), num(i + 5));
                for s in 1..=20 {
                    let (t, u) = (s as f32 / 20.0, 1.0 - s as f32 / 20.0);
                    let f = |a: f32, b: f32, c: f32, d: f32| {
                        u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
                    };
                    out.last_mut()
                        .unwrap()
                        .push((f(cur.0, c1.0, c2.0, e.0), f(cur.1, c1.1, c2.1, e.1)));
                }
                i += 7;
            }
            _ => i += 1,
        }
    }
    out
}

fn inside(p: (f32, f32), b: &super::layout::Placed) -> bool {
    p.0 > b.x + 1.0 && p.0 < b.x + b.w - 1.0 && p.1 > b.y + 1.0 && p.1 < b.y + b.h - 1.0
}

fn crosses(line: &str, b: &super::layout::Placed) -> bool {
    polylines(line).iter().any(|pl| {
        pl.windows(2).any(|w| {
            let n = ((w[1].0 - w[0].0).hypot(w[1].1 - w[0].1) / 2.0)
                .ceil()
                .max(1.0) as usize;
            (0..=n).any(|k| {
                let t = k as f32 / n as f32;
                inside(
                    (
                        w[0].0 + (w[1].0 - w[0].0) * t,
                        w[0].1 + (w[1].1 - w[0].1) * t,
                    ),
                    b,
                )
            })
        })
    })
}

fn ends_of(d: &Diagram, e: usize) -> (usize, usize) {
    let at = |k: &Key| d.boxes.iter().position(|b| &b.key == k).unwrap();
    (at(&d.edges[e].from), at(&d.edges[e].to))
}

#[test]
fn no_edge_runs_through_a_foreign_box_in_fixture_12() {
    for (d, s) in fixture_scenes() {
        assert_eq!(s.edges.len(), d.edges.len());
        for (n, e) in s.edges.iter().enumerate() {
            let (a, b) = ends_of(&d, n);
            for (i, p) in s
                .boxes
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != a && i != b)
            {
                assert!(
                    !crosses(&e.line, p),
                    "{:?}: {:?} -> {:?} through {:?}",
                    e.style,
                    d.edges[n].from,
                    d.edges[n].to,
                    d.boxes[i].key
                );
            }
        }
    }
}

/// A service reaches an analyzer two columns away while an index stands between them.
#[test]
fn an_edge_across_a_column_goes_around_the_boxes_between() {
    let kinds = [BoxKind::Service, BoxKind::Index, BoxKind::Analyzer];
    let mut d = Diagram::default();
    for (i, k) in kinds.iter().enumerate() {
        d.boxes.push(dbox(i, *k));
    }
    d.edges.push(dedge(0, 2, EdgeStyle::Uses));
    let line = Line {
        text: "f".into(),
        ..Default::default()
    };
    let tall = BoxContent {
        compartments: vec![Compartment {
            title: Title::Fields,
            lines: vec![line; 12],
            chips: Vec::new(),
        }],
        ..Default::default()
    };
    let cs = [BoxContent::default(), tall, BoxContent::default()];
    let s = layout(&d, &cs);
    assert!(
        s.boxes[1].y <= s.boxes[0].y + 24.0,
        "the index is in the way"
    );
    assert!(
        !crosses(&s.edges[0].line, &s.boxes[1]),
        "{}",
        s.edges[0].line
    );
}

fn label_rect(e: &PlacedEdge) -> (f32, f32, f32, f32) {
    let (w, h) = label_size(&e.label);
    (e.lx, e.ly, w, h)
}

#[test]
fn no_edge_label_lies_over_a_box_in_fixture_12() {
    let (mut named, mut kept) = (0, 0);
    for (d, s) in fixture_scenes() {
        for (n, e) in s.edges.iter().enumerate() {
            named += usize::from(!d.edges[n].label.is_empty());
            kept += usize::from(!e.label.is_empty());
            if e.label.is_empty() {
                continue;
            }
            let (x, y, w, h) = label_rect(e);
            for (i, p) in s.boxes.iter().enumerate() {
                let apart = x + w <= p.x || p.x + p.w <= x || y + h <= p.y || p.y + p.h <= y;
                assert!(apart, "label {:?} over {:?}", e.label, d.boxes[i].key);
            }
        }
    }
    assert!(
        named > 0 && kept > 0,
        "labels exist and some are kept: {kept}/{named}"
    );
}

#[test]
fn a_single_index_starts_at_the_left_margin() {
    let d = Diagram {
        boxes: vec![dbox(0, BoxKind::Index)],
        edges: Vec::new(),
    };
    let s = layout(&d, &[BoxContent::default()]);
    assert_eq!((s.boxes[0].x, s.width), (20.0, 340.0));
}
