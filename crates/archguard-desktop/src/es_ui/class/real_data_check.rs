//! Tests for the layout with real data, ticket 17.

use std::sync::Arc;

use archguard_engine::{SearchGraph, core::ServiceTopology, search_graph};
use serde_json::{Value, json};

use super::boxes::{BoxContent, ChainClass, Chip, Compartment, Stage, Title, content};
use super::diagram::{BoxKind, DBox, Diagram, EdgeStyle, diagram};
use super::layout::{Placed, layout};
use super::layout_check::{crosses, dedge, synthetic};
use super::types::{Key, Switches};
use super::view::fit;
use super::wrap::{fit_box, fit_text};
use crate::es_ui::state::build_from;

const CHIP_H: f32 = 14.0;

fn chip(text: &str) -> Chip {
    Chip {
        text: text.into(),
        stage: Stage::Filter,
        class: ChainClass::Declared,
    }
}

fn chain_box(names: &[&str]) -> (Placed, BoxContent) {
    let mut d = Diagram::default();
    d.boxes.push(DBox {
        key: Key::Node(0),
        kind: BoxKind::Analyzer,
        ghost: false,
        warn: false,
        selected: true,
        dim: false,
    });
    let c = BoxContent {
        compartments: vec![
            Compartment {
                title: Title::Chain,
                lines: Vec::new(),
                chips: names.iter().map(|n| chip(n)).collect(),
            },
            Compartment {
                title: Title::UsedBy,
                lines: Vec::new(),
                chips: Vec::new(),
            },
        ],
        ..Default::default()
    };
    let p = layout(&d, std::slice::from_ref(&c)).boxes.remove(0);
    (p, c)
}

fn rects(p: &Placed) -> Vec<(f32, f32, f32, f32)> {
    let top = p.comps[0];
    p.chips
        .iter()
        .map(|&(x, y, w)| (x, top + y, w, CHIP_H))
        .collect()
}

#[test]
fn seven_chips_do_not_overlap_each_other_or_the_compartment_below() {
    let names = [
        "standard",
        "lowercase",
        "asciifolding",
        "ui_stemmer",
        "ui_edge_ngram",
        "ui_shingle",
        "trim",
    ];
    let (p, _) = chain_box(&names);
    let r = rects(&p);
    for (i, a) in r.iter().enumerate() {
        assert!(
            a.0 >= 0.0 && a.0 + a.2 <= p.w,
            "chip {i} leaves the box: {a:?}"
        );
        for b in &r[i + 1..] {
            let apart =
                a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1;
            assert!(apart, "{a:?} overlaps {b:?}");
        }
    }
    let bottom = r.iter().map(|a| a.1 + a.3).fold(0.0, f32::max);
    assert!(
        bottom <= p.comps[1],
        "chips reach into the next compartment"
    );
    let (few, _) = chain_box(&names[..2]);
    assert!(p.h > few.h, "the box grows with the chip rows");
}

#[test]
fn a_chip_wider_than_the_box_is_cut_to_the_box() {
    let long = "x".repeat(60);
    let (p, _) = chain_box(&[&long, "trim"]);
    for a in rects(&p) {
        assert!(
            a.0 >= 0.0 && a.0 + a.2 <= p.w,
            "{a:?} in a box {} wide",
            p.w
        );
    }
}

fn parts_topology(same_name: bool) -> (ServiceTopology, SearchGraph) {
    let detail = json!({"fields": [], "settings": {},
        "analysis": [{"kind": "analyzer", "name": "an", "definition": r#"{"type":"custom","tokenizer":"standard"}"#}]});
    let art = |scope: &str| {
        json!({"kind": "index", "name": "docs", "address": format!("{scope}.docs"), "scope": scope,
        "patterns": [], "references": [], "evidence": [], "detail": detail})
    };
    let arts = if same_name {
        vec![art("terraform:a"), art("terraform:b")]
    } else {
        vec![art("terraform:a")]
    };
    let elasticsearch =
        serde_json::from_value(json!({"artifacts": arts, "usages": [], "findings": []})).unwrap();
    let t = ServiceTopology {
        elasticsearch,
        ..Default::default()
    };
    let g = search_graph(&t);
    (t, g)
}

fn part_root(t: &ServiceTopology, g: &SearchGraph, owner: usize) -> String {
    let key = Key::Part {
        owner,
        kind: "analyzer".into(),
        name: "an".into(),
    };
    content(t, g, &key, &Switches::default()).root
}

#[test]
fn the_owner_line_of_a_building_block_is_the_short_label_and_a_root_only_when_it_tells_apart() {
    let (t, g) = parts_topology(false);
    assert_eq!(part_root(&t, &g, 0), g.nodes[0].short_label);
    let (t, g) = parts_topology(true);
    let (a, b) = (part_root(&t, &g, 0), part_root(&t, &g, 1));
    assert_ne!(a, b, "two owners with one name need their root");
}

fn unread_topology() -> (ServiceTopology, SearchGraph) {
    let detail = json!({"fields": [], "settings": {"number_of_shards": "1"}, "analysis": []});
    let art = json!({"kind": "index", "name": "docs", "address": "r.docs", "scope": "terraform:a",
        "patterns": [], "references": [], "evidence": [], "detail": detail});
    let elasticsearch =
        serde_json::from_value(json!({"artifacts": [art], "usages": [], "findings": []})).unwrap();
    let diag: Value = json!({"file": "a.tf", "service": null, "message":
        "r.docs: index mapping, settings or analysis are not literal (file(), local., var. and references are not evaluated); not read: mappings."});
    let t = ServiceTopology {
        elasticsearch,
        diagnostics: vec![serde_json::from_value(diag).unwrap()],
        ..Default::default()
    };
    let g = search_graph(&t);
    (t, g)
}

#[test]
fn an_index_whose_mapping_was_not_read_says_so_instead_of_none() {
    let (t, g) = unread_topology();
    let b = content(&t, &g, &Key::Node(0), &Switches::default());
    let fields = b.compartments.iter().find(|c| c.title == Title::Fields);
    let text = fields
        .and_then(|c| c.lines.first())
        .map(|l| l.text.as_str());
    assert_eq!(text, Some("mapping_not_read"));
}

#[test]
fn a_cluster_of_180_boxes_wraps_into_a_scene_near_the_canvas_shape() {
    let (d, cs) = synthetic(36);
    let s = layout(&d, &cs);
    let aspect = s.width / s.height;
    assert!((0.5..=3.0).contains(&aspect), "{} x {}", s.width, s.height);
}

#[test]
fn fit_never_goes_below_forty_percent() {
    let mut s = build_from(Arc::new(Ok(ServiceTopology::default())));
    s.canvas = (9000.0, 12000.0);
    fit(&mut s, 1000.0, 800.0);
    assert!((s.zoom - 0.4).abs() < 1e-4, "{}", s.zoom);
}

/// Three boxes in one column and an edge from the first to the third passes the second.
#[test]
fn an_edge_within_a_column_goes_around_the_box_between() {
    let mut d = Diagram::default();
    for i in 0..3 {
        let mut b = DBox {
            key: Key::Node(i),
            kind: BoxKind::Index,
            ghost: false,
            warn: false,
            selected: i == 0,
            dim: false,
        };
        b.selected = i == 0;
        d.boxes.push(b);
    }
    d.edges.push(dedge(0, 2, EdgeStyle::Uses));
    let s = layout(
        &d,
        &[
            BoxContent::default(),
            BoxContent::default(),
            BoxContent::default(),
        ],
    );
    let ys: Vec<f32> = s.boxes.iter().map(|b| b.y).collect();
    assert!(ys[0] < ys[1] && ys[1] < ys[2], "{ys:?}");
    assert!(
        !crosses(&s.edges[0].line, &s.boxes[1]),
        "{}",
        s.edges[0].line
    );
}

/// A declared `default_search` is used by the index like a declared `default`.
#[test]
fn a_declared_default_search_gets_the_same_uses_edge_as_default() {
    let detail = json!({"fields": [{"path": "k", "kind": "keyword"}], "settings": {},
        "analysis": [
            {"kind": "analyzer", "name": "default", "definition": r#"{"tokenizer":"standard"}"#},
            {"kind": "analyzer", "name": "default_search", "definition": r#"{"tokenizer":"standard"}"#}]});
    let art = json!({"kind": "index", "name": "docs", "address": "r.docs", "scope": "terraform:a",
        "patterns": [], "references": [], "evidence": [], "detail": detail});
    let elasticsearch =
        serde_json::from_value(json!({"artifacts": [art], "usages": [], "findings": []})).unwrap();
    let t = ServiceTopology {
        elasticsearch,
        ..Default::default()
    };
    let g = search_graph(&t);
    let part = |name: &str| Key::Part {
        owner: 0,
        kind: "analyzer".into(),
        name: name.into(),
    };
    let d = diagram(&t, &g, &Key::Node(0), None, &Switches::default());
    let uses = |to: Key| d.edges.iter().any(|e| e.from == Key::Node(0) && e.to == to);
    assert!(uses(part("default")), "default edge");
    assert!(uses(part("default_search")), "default_search edge");
}

#[test]
fn long_texts_are_cut_per_row_with_an_ellipsis_and_hints_stay_whole() {
    assert_eq!(fit_text("short", 100.0, 7.0), "short");
    let cut = fit_text(&"n".repeat(80), 70.0, 7.0);
    assert_eq!(cut.chars().count(), 10);
    assert!(cut.ends_with('…'));
    let line = |t: &str| super::boxes::Line {
        text: t.into(),
        ..Default::default()
    };
    let mut c = BoxContent {
        name: "n".repeat(80),
        root: "r".repeat(80),
        compartments: vec![Compartment {
            title: Title::UsedBy,
            lines: vec![line(&"u".repeat(80)), line("mapping_not_read")],
            chips: Vec::new(),
        }],
        ..Default::default()
    };
    fit_box(&mut c, 190.0);
    assert!(c.name.ends_with('…') && c.name.chars().count() < 25);
    assert!(c.root.ends_with('…'));
    assert!(c.compartments[0].lines[0].text.ends_with('…'));
    assert_eq!(c.compartments[0].lines[1].text, "mapping_not_read");
}
