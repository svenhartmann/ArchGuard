//! Tests for boxes, ticket 05.

use std::path::Path;

use archguard_engine::core::ServiceTopology;
use archguard_engine::{SearchGraph, analyze_services, search_graph};

use super::boxes::{BoxContent, Prefix, Title, content};
use super::types::{Key, Switches};

fn topology() -> (ServiceTopology, SearchGraph) {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/02/terraform");
    let t = analyze_services(&root).unwrap();
    let g = search_graph(&t);
    (t, g)
}

fn docs(sw: &Switches) -> BoxContent {
    let (t, g) = topology();
    let i = g
        .nodes
        .iter()
        .position(|n| n.kind == "index" && n.label == "docs")
        .expect("docs node");
    content(&t, &g, &Key::Node(i), sw)
}

fn rows(b: &BoxContent, title: Title) -> Vec<(usize, Prefix, String, String, String)> {
    b.compartments
        .iter()
        .find(|c| c.title == title)
        .map(|c| {
            c.lines
                .iter()
                .map(|l| {
                    (
                        l.depth,
                        l.prefix,
                        l.text.clone(),
                        l.kind_text.clone(),
                        l.note.clone(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn index_box_like_the_wireframe() {
    let b = docs(&Switches::default());
    assert_eq!(b.stereotype, "index");
    assert_eq!(b.name, "docs");
    let titles: Vec<Title> = b.compartments.iter().map(|c| c.title).collect();
    assert_eq!(titles, [Title::Fields, Title::Analysis]);
    let f = rows(&b, Title::Fields);
    let row = |d, p, t: &str, k: &str, n: &str| (d, p, t.to_string(), k.to_string(), n.to_string());
    assert_eq!(
        f[0],
        row(0, Prefix::Field, "title", "text", "{idx_a / ⌕ qry_a}")
    );
    assert_eq!(
        f[1],
        row(1, Prefix::Multi, "raw", "keyword", "{norm fold_n}")
    );
    assert_eq!(
        f[2],
        row(1, Prefix::Multi, "shingle", "text", "{shingle_a}")
    );
    assert_eq!(f[5], row(0, Prefix::Field, "author", "object", ""));
    assert_eq!(f[6], row(1, Prefix::None, "name", "text", "{idx_a}"));
}

#[test]
fn switches_and_part_boxes() {
    let off = Switches {
        multi_fields: false,
        ..Switches::default()
    };
    assert!(
        rows(&docs(&off), Title::Fields)
            .iter()
            .all(|r| r.1 != Prefix::Multi)
    );
    let on = Switches {
        collapse_fields: true,
        ..Switches::default()
    };
    let b = docs(&on);
    assert_eq!(b.compartments[0].lines.len(), 1);
    assert_eq!(b.compartments[0].lines[0].summary, Some(9));

    let (t, g) = topology();
    let owner = g.nodes.iter().position(|n| n.label == "docs").unwrap();
    let key = Key::Part {
        owner,
        kind: "analyzer".into(),
        name: "qry_a".into(),
    };
    let b = content(&t, &g, &key, &Switches::default());
    assert_eq!(b.stereotype, "analyzer · custom");
    let chips: Vec<&str> = b.compartments[0]
        .chips
        .iter()
        .map(|c| c.text.as_str())
        .collect();
    assert_eq!(chips, ["whitespace", "lowercase", "syn"]);
    let used = rows(&b, Title::UsedBy);
    assert_eq!(used[0].2, "title");
    assert_eq!(used[0].4, "(search)");
}
