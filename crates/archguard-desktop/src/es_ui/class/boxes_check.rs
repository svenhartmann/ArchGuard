//! Tests for boxes, ticket 05.

use std::path::Path;

use archguard_engine::core::ServiceTopology;
use archguard_engine::{SearchGraph, analyze_services, search_graph};
use serde_json::json;

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
        row(
            1,
            Prefix::Multi,
            "shingle",
            "text",
            "{shingle_a / ⌕ default_search}"
        )
    );
    assert_eq!(
        f[3],
        row(
            0,
            Prefix::Field,
            "body",
            "text",
            "{default / ⌕ default_search}"
        )
    );
    assert_eq!(f[5], row(0, Prefix::Field, "author", "object", ""));
    assert_eq!(
        f[6],
        row(
            1,
            Prefix::None,
            "name",
            "text",
            "{idx_a / ⌕ default_search}"
        )
    );
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

fn art(kind: &str, name: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut a = json!({
        "kind": kind, "name": name, "address": format!("res.{name}"), "scope": "terraform:infra",
        "patterns": [], "references": [],
        "evidence": [{"file": "main.tf", "pointer": "", "line": 3, "description": ""}],
    });
    a.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    a
}

fn small(
    arts: Vec<serde_json::Value>,
    usages: serde_json::Value,
) -> (ServiceTopology, SearchGraph) {
    let mut t = ServiceTopology::default();
    t.elasticsearch.artifacts = serde_json::from_value(json!(arts)).unwrap();
    t.elasticsearch.usages = serde_json::from_value(usages).unwrap();
    let g = search_graph(&t);
    (t, g)
}

fn box_of(t: &ServiceTopology, g: &SearchGraph, kind: &str) -> BoxContent {
    let i = g.nodes.iter().position(|n| n.kind == kind).expect(kind);
    content(t, g, &Key::Node(i), &Switches::default())
}

#[test]
fn template_with_many_settings_and_patterns() {
    let settings: serde_json::Map<String, serde_json::Value> = (1..=8)
        .map(|i| (format!("s{i}"), json!(i.to_string())))
        .collect();
    let detail = json!({"detail": {"settings": settings}, "patterns": ["logs-*"]});
    let (t, g) = small(vec![art("index_template", "tpl", detail)], json!([]));
    let b = box_of(&t, &g, "index_template");
    assert_eq!(b.stereotype, "index_template");
    assert_eq!(b.root, "infra · main.tf:3");
    let s = rows(&b, Title::Settings);
    assert_eq!(s.len(), 7);
    assert_eq!(s[0].2, "s1 = 1");
    let last = &b.compartments[0].lines[6];
    assert_eq!(last.summary, Some(-2));
    assert_eq!(rows(&b, Title::Patterns)[0].2, "logs-*");
}

#[test]
fn alias_points_to_its_index() {
    let alias = art(
        "alias",
        "al",
        json!({"references": [{"kind": "index", "name": "docs"}]}),
    );
    let (t, g) = small(vec![alias, art("index", "docs", json!({}))], json!([]));
    let b = box_of(&t, &g, "alias");
    assert_eq!(rows(&b, Title::PointsTo)[0].2, "docs");
}

#[test]
fn service_shows_writes_and_reads() {
    let ev = json!([{"file": "a.cs", "pointer": "", "line": 1, "description": ""}]);
    let usages = json!([
        {"service": "svc", "index": "docs", "access": "write", "evidence": ev},
        {"service": "svc", "index": "docs", "access": "read", "evidence": ev},
    ]);
    let (t, g) = small(vec![art("index", "docs", json!({}))], usages);
    let b = box_of(&t, &g, "service");
    let texts: Vec<String> = rows(&b, Title::Access).into_iter().map(|r| r.2).collect();
    assert_eq!(texts, ["✎ docs", "↓ docs"]);
}

#[test]
fn undetailed_index_gets_a_hint() {
    let (mut t, _) = small(vec![art("index", "docs", json!({}))], json!([]));
    let hint = |t: &ServiceTopology| {
        let g = search_graph(t);
        rows(&box_of(t, &g, "index"), Title::Fields)[0].2.clone()
    };
    assert_eq!(hint(&t), "not_declared");
    t.diagnostics.push(archguard_engine::core::ServiceDiagnostic {
        file: "main.tf".into(),
        service: None,
        message: "res.docs: index mapping, settings or analysis are not literal (file(), local., var. and references are not evaluated); not read: mappings.".into(),
    });
    assert_eq!(hint(&t), "not_read");
}

#[test]
fn filter_box_lists_type_first_and_users() {
    let (t, g) = topology();
    let owner = g.nodes.iter().position(|n| n.label == "docs").unwrap();
    let key = Key::Part {
        owner,
        kind: "filter".into(),
        name: "syn".into(),
    };
    let b = content(&t, &g, &key, &Switches::default());
    assert_eq!(b.stereotype, "filter · synonym");
    assert_eq!(b.name, "syn");
    assert_eq!(b.root, "docs");
    let p = rows(&b, Title::Parameters);
    assert_eq!(p[0].2, "type = synonym");
    assert!(p[1].2.starts_with("synonyms = "));
    assert_eq!(rows(&b, Title::UsedBy)[0].2, "analyzer:qry_a");
}

#[test]
fn declared_default_applies_to_text_fields_without_analyzer() {
    let def = r#"{"type":"custom","tokenizer":"standard"}"#;
    let detail = json!({"detail": {
        "fields": [{"path": "t", "kind": "text"}, {"path": "k", "kind": "keyword"}],
        "analysis": [{"kind": "analyzer", "name": "default", "definition": def}],
    }});
    let (t, g) = small(vec![art("index", "docs", detail)], json!([]));
    let f = rows(&box_of(&t, &g, "index"), Title::Fields);
    assert_eq!(f[0].4, "{default}");
    assert_eq!(f[1].4, "");
}
