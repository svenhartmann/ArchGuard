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
fn inference_endpoint_lists_its_settings() {
    let detail = json!({"detail": {"settings": {"task": "text_embedding", "model_id": "m"}}});
    let (t, g) = small(vec![art("inference_endpoint", "ep", detail)], json!([]));
    let b = box_of(&t, &g, "inference_endpoint");
    let texts: Vec<String> = rows(&b, Title::Inference)
        .into_iter()
        .map(|r| r.2)
        .collect();
    assert_eq!(texts, ["model_id = m", "task = text_embedding"]);
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

#[test]
fn field_lines_carry_their_full_path_other_lines_none() {
    let b = docs(&Switches::default());
    let fields = b
        .compartments
        .iter()
        .find(|c| c.title == Title::Fields)
        .expect("fields");
    let raw = fields
        .lines
        .iter()
        .find(|l| l.text == "raw")
        .expect("title.raw");
    assert_eq!(raw.path, "title.raw");
    assert!(
        fields.lines.iter().all(|l| !l.path.is_empty()),
        "every line is a field"
    );
    let other = b
        .compartments
        .iter()
        .find(|c| c.title == Title::Analysis)
        .expect("analysis");
    assert!(other.lines.iter().all(|l| l.path.is_empty()));
}

fn note_topology() -> (ServiceTopology, SearchGraph) {
    let std_def = r#"{"type":"custom","tokenizer":"standard"}"#;
    let detail = json!({
        "fields": [
            {"path": "t", "kind": "text"},
            {"path": "k", "kind": "keyword", "normalizer": "n"},
            {"path": "s", "kind": "keyword", "search_analyzer": "default_search"}],
        "analysis": [
            {"kind": "analyzer", "name": "default", "definition": std_def},
            {"kind": "analyzer", "name": "default_search", "definition": std_def},
            {"kind": "normalizer", "name": "n", "definition": r#"{"type":"custom","filter":["lowercase"]}"#}]});
    let es = json!({"artifacts": [{"kind": "index", "name": "ix", "address": "index.ix",
        "scope": "terraform:infra", "patterns": [], "references": [], "evidence": [], "detail": detail}],
        "usages": [], "findings": []});
    let t = ServiceTopology {
        elasticsearch: serde_json::from_value(es).unwrap(),
        ..Default::default()
    };
    let g = search_graph(&t);
    (t, g)
}

#[test]
fn the_box_note_and_the_field_details_name_the_same_analyzers() {
    use super::details::field_details;
    let (t, g) = note_topology();
    let i = g.nodes.iter().position(|n| n.kind == "index").unwrap();
    let b = content(&t, &g, &Key::Node(i), &Switches::default());
    let fields = b
        .compartments
        .iter()
        .find(|c| c.title == Title::Fields)
        .unwrap();
    let note = |p: &str| {
        fields
            .lines
            .iter()
            .find(|l| l.text == p)
            .unwrap()
            .note
            .clone()
    };
    let text = field_details(&t, &g, i, "t").unwrap();
    let (a, s) = (text.index.unwrap().name, text.search.unwrap().name);
    assert_eq!(note("t"), format!("{{{a} / \u{2315} {s}}}"));
    // `default` applies to text fields only: a keyword field shows its normalizer alone.
    let k = field_details(&t, &g, i, "k").unwrap();
    assert!(k.index.is_none() && k.search.is_none());
    assert_eq!(note("k"), "{norm n}");
    // A keyword field naming a search analyzer is analyzed: `default` indexes it.
    let s = field_details(&t, &g, i, "s").unwrap();
    let (a, b) = (s.index.unwrap().name, s.search.unwrap().name);
    assert_eq!(note("s"), format!("{{{a} / \u{2315} {b}}}"));
}

fn opensearch_topology() -> ServiceTopology {
    let art = |kind: &str, name: &str| {
        serde_json::json!({"kind": kind, "name": name, "address": format!("res.{name}"),
            "scope": "terraform:infra/main", "flavor": "opensearch",
            "patterns": [], "references": [], "evidence": []})
    };
    let es = serde_json::json!({"findings": [], "usages": [],
        "artifacts": [art("index", "os-idx"), art("ism_policy", "os-ism")]});
    ServiceTopology {
        elasticsearch: serde_json::from_value(es).unwrap(),
        ..Default::default()
    }
}

#[test]
fn opensearch_artifact_names_the_product_in_the_root_line() {
    let t = opensearch_topology();
    let g = search_graph(&t);
    let i = g.nodes.iter().position(|n| n.kind == "index").unwrap();
    let b = content(&t, &g, &Key::Node(i), &Switches::default());
    assert_eq!(b.root, "OpenSearch \u{b7} infra/main");
}
