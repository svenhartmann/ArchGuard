//! Tests for the list model, ticket 06.

use archguard_engine::core::ServiceTopology;
use archguard_engine::search_graph;
use serde_json::json;

use super::list::{GroupTitle::*, ListGroup, list, members};
use super::types::{Group, Key};

fn art(kind: &str, name: &str, scope: &str, detail: serde_json::Value) -> serde_json::Value {
    json!({"kind": kind, "name": name, "address": format!("res.{name}"), "scope": scope,
        "patterns": [], "references": [], "evidence": [], "detail": detail})
}

fn analysis(parts: &[(&str, &str, &str)]) -> serde_json::Value {
    let a: Vec<_> = parts
        .iter()
        .map(|(k, n, d)| json!({"kind": k, "name": n, "definition": d}))
        .collect();
    json!({"analysis": a})
}

fn usage(service: &str, index: &str, access: &str) -> serde_json::Value {
    json!({"service": service, "index": index, "access": access, "evidence": []})
}

fn topology() -> ServiceTopology {
    let std = r#"{"tokenizer":"standard"}"#;
    let mut pipe = art("index", "x-pipe", "terraform:infra/other", json!(null));
    pipe["references"] = json!([{"kind": "ingest_pipeline", "name": "missing"}]);
    let es = json!({"findings": [], "usages": [
        usage("svc", "xAby", "write"), usage("svc", "gone", "write"), usage("svc", "al", "read")],
        "artifacts": [
        art("index", "xAby", "terraform:infra/main", analysis(&[
            ("filter", "f", r#"{"type":"lowercase"}"#), ("analyzer", "std2", std)])),
        art("index", "a-b", "terraform:infra/main", json!(null)),
        pipe,
        art("alias", "al", "terraform:infra/main", json!(null)),
        art("index_template", "tpl", "terraform:infra/main", analysis(&[("analyzer", "std2", std)])),
        art("component_template", "ct", "terraform:infra/main",
            analysis(&[("analyzer", "std2", r#"{"tokenizer":"keyword"}"#)])),
        art("ilm_policy", "pol", "terraform:infra/main", json!(null))]});
    ServiceTopology {
        elasticsearch: serde_json::from_value(es).unwrap(),
        ..Default::default()
    }
}

fn rows(g: &[ListGroup], at: usize) -> Vec<(String, String, usize, usize, bool)> {
    g[at]
        .rows
        .iter()
        .map(|r| (r.label.clone(), r.side.clone(), r.writes, r.reads, r.warn))
        .collect()
}

fn row(l: &str, s: &str, w: usize, r: usize, warn: bool) -> (String, String, usize, usize, bool) {
    (l.into(), s.into(), w, r, warn)
}

#[test]
fn groups_counters_search_filter() {
    let t = topology();
    let g = search_graph(&t);
    let all = list(&t, &g, "", Group::All);
    let titles: Vec<_> = all.iter().map(|x| x.title.clone()).collect();
    let main = || Indices("infra/main".into());
    assert_eq!(
        titles,
        [
            Services,
            main(),
            Indices("infra/other".into()),
            IndicesUndeclared,
            Aliases,
            Templates,
            Analysis,
            Processing
        ]
    );
    assert_eq!(rows(&all, 0), [row("svc", "", 2, 1, false)]);
    assert_eq!(
        rows(&all, 1),
        [row("xAby", "", 1, 0, false), row("a-b", "", 0, 0, false)]
    );
    assert_eq!(rows(&all, 3), [row("gone", "", 1, 0, true)]);
    // Reading through the alias counts at the alias, not at an index.
    assert_eq!(rows(&all, 4), [row("al", "", 0, 1, false)]);
    assert_eq!(
        rows(&all, 5),
        [row("tpl", "", 0, 0, false), row("ct", "", 0, 0, false)]
    );
    // Same kind, name and definition merge; same name with another definition stays apart.
    assert_eq!(
        rows(&all, 6),
        [
            row("std2 · xAby", "analyzer", 0, 0, false),
            row("std2 · ct", "analyzer", 0, 0, false),
            row("f", "filter", 0, 0, false)
        ]
    );
    assert_eq!(
        rows(&all, 7),
        [
            row("pol", "ilm_policy", 0, 0, false),
            row("missing", "ingest_pipeline", 0, 0, true)
        ]
    );
    assert_eq!(members(&all).len(), 1 + 3 + 1 + 1 + 2 + 3 + 2);
    assert!(members(&all).contains(&Key::Node(0)));

    // Case-insensitive substring over the label: `ab` finds `xAby`, not `a-b`.
    let hit = list(&t, &g, "ab", Group::All);
    assert_eq!(hit.len(), 2, "{hit:?}");
    assert_eq!(rows(&hit, 0), [row("xAby", "", 1, 0, false)]);
    assert_eq!(rows(&hit, 1), [row("std2 · xAby", "analyzer", 0, 0, false)]);

    let only = list(&t, &g, "", Group::Aliases);
    assert_eq!(
        only.iter().map(|x| x.title.clone()).collect::<Vec<_>>(),
        [Aliases]
    );
    let ix = list(&t, &g, "", Group::Indices);
    assert_eq!(ix.len(), 4 - 1);
    assert!(list(&t, &g, "zzz", Group::All).is_empty());
}

fn set_topology(arts: &[serde_json::Value]) -> ServiceTopology {
    let es = json!({"findings": [], "usages": [], "projects": ["p", "q"], "artifacts": arts});
    ServiceTopology {
        elasticsearch: serde_json::from_value(es).unwrap(),
        ..Default::default()
    }
}

#[test]
fn foreign_indices_get_their_own_root_group() {
    let mut far = art("index", "far", "terraform:infra/main", json!(null));
    far["origins"] = json!(["q"]);
    let mut own = art("index", "own", "terraform:infra/main", json!(null));
    own["origins"] = json!(["p"]);
    let t = set_topology(&[own, far]);
    let all = list(&t, &search_graph(&t), "", Group::All);
    let titles: Vec<_> = all.iter().map(|x| x.title.clone()).collect();
    assert_eq!(
        titles,
        [
            Indices("infra/main".into()),
            Indices("[q] infra/main".into())
        ]
    );
    assert_eq!(rows(&all, 0), [row("own", "", 0, 0, false)]);
    assert_eq!(rows(&all, 1), [row("[q] far", "", 0, 0, false)]);
}

#[test]
fn a_root_group_drops_the_root_suffix_but_keeps_the_full_label_as_title() {
    let t = set_topology(&[
        art("index", "dup", "terraform:infra/main", json!(null)),
        art("index", "dup", "terraform:infra/other", json!(null)),
        art("index", "solo (x)", "terraform:infra/main", json!(null)),
    ]);
    let all = list(&t, &search_graph(&t), "", Group::All);
    let shown: Vec<_> = all
        .iter()
        .map(|x| {
            x.rows
                .iter()
                .map(|r| (r.label.as_str(), r.title.as_str()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        shown,
        [
            vec![("dup", "dup (main)"), ("solo (x)", "solo (x)")],
            vec![("dup", "dup (other)")]
        ]
    );
}

#[test]
fn a_name_with_parentheses_stays_whole() {
    let t = set_topology(&[art(
        "index",
        "logs (main)",
        "terraform:infra/main",
        json!(null),
    )]);
    let all = list(&t, &search_graph(&t), "", Group::All);
    assert_eq!(rows(&all, 0), [row("logs (main)", "", 0, 0, false)]);
}
