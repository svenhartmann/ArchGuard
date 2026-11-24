//! Findings of the Elasticsearch map (ADR 0023, items 8 and 9).
use archguard_engine::{analyze_services, core::ServiceTopology};
use std::{fs, path::Path};

fn put(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn fixture() -> ServiceTopology {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/elasticsearch-services/findings");
    analyze_services(&root).unwrap()
}

/// (kind, subject, services) of all findings of one kind, in reported order.
fn of(r: &ServiceTopology, kind: &str) -> Vec<(String, Vec<String>)> {
    r.elasticsearch
        .findings
        .iter()
        .filter(|f| f.kind == kind)
        .map(|f| (f.subject.clone(), f.services.clone()))
        .collect()
}

fn subjects(r: &ServiceTopology, kind: &str) -> Vec<String> {
    of(r, kind).into_iter().map(|(s, _)| s).collect()
}

#[test]
fn undeclared_pipeline_has_one_hit() {
    assert_eq!(
        subjects(&fixture(), "undeclared_pipeline"),
        ["missing-pipeline"]
    );
}

#[test]
fn undeclared_index_hits_only_unmatched_literals() {
    // logs-app is covered by the template pattern, orders by the alias, audit-log by the index.
    assert_eq!(
        subjects(&fixture(), "undeclared_index"),
        ["also-undeclared", "undeclared-ix"]
    );
}

#[test]
fn shared_write_index_resolves_alias_and_ignores_reads() {
    // writer-a writes orders-v1, writer-b writes the alias orders -> orders-v1.
    // audit-log: writer-a writes, writer-b only reads.
    let got = of(&fixture(), "shared_write_index");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "orders-v1");
    assert_eq!(got[0].1.len(), 2);
}

#[test]
fn data_view_patterns_are_checked_exactly() {
    // logs-app-* is covered by logs-*, orders by the alias, -nothing-* is dropped;
    // ghost-* is not covered.
    assert_eq!(subjects(&fixture(), "uncovered_pattern"), ["ghost-*"]);
}

#[test]
fn alert_indices_are_checked_exactly() {
    // m*trics-x covers m-metrics-x (star in the middle); phantom is declared nowhere.
    assert_eq!(subjects(&fixture(), "alert_undeclared_index"), ["phantom"]);
}

#[test]
fn findings_are_sorted_and_complete_fixture_has_no_incomplete_note() {
    let r = fixture();
    let keys: Vec<_> = r
        .elasticsearch
        .findings
        .iter()
        .map(|f| (f.kind.clone(), f.subject.clone()))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    assert!(r.elasticsearch.findings.iter().all(|f| {
        f.evidence
            .iter()
            .all(|e| !e.description.contains("incomplete"))
    }));
}

#[test]
fn without_elasticstack_declarations_no_index_finding() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:\n  api:\n    build: ./api\n",
    );
    put(
        root,
        "api/W.java",
        "import org.elasticsearch.client.Client;\nclass W { var r = new IndexRequest(\"x\"); }\n",
    );
    let r = analyze_services(root).unwrap();
    assert_eq!(r.elasticsearch.usages.len(), 1);
    assert!(r.elasticsearch.findings.is_empty());
}

#[test]
fn unknown_and_read_never_share_and_gaps_never_find() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:\n  a:\n    build: ./a\n  b:\n    build: ./b\n",
    );
    put(
        root,
        "infra/main.tf",
        "resource \"elasticstack_elasticsearch_index\" \"i\" {\n  name = \"shared\"\n}\nresource \"elasticstack_elasticsearch_index\" \"g\" {\n  name = var.n\n}\nresource \"elasticstack_kibana_data_view\" \"v\" {\n  data_view = {\n    name = \"v\"\n    title = var.t\n  }\n}\nresource \"elasticstack_kibana_alerting_rule\" \"r\" {\n  name = \"r\"\n  rule_type_id = \".es-query\"\n  params = jsonencode({ index = [var.i] })\n}\n",
    );
    put(
        root,
        "a/W.java",
        "import org.elasticsearch.client.Client;\nclass W { var r = new IndexRequest(\"shared\"); }\n",
    );
    put(
        root,
        "b/W.java",
        "import org.elasticsearch.client.Client;\nclass W { var r = new SearchRequest.Builder().index(\"shared\"); var o = ops.index(q, IndexCoordinates.of(\"shared\")); var n = new SearchRequest.Builder().index(Names.X); }\n",
    );
    let r = analyze_services(root).unwrap();
    // Only one writer; the other reads/unknown; the gap names yield nothing, but the
    // unresolved name in the declarations is noted on any finding.
    assert!(
        r.elasticsearch.findings.is_empty(),
        "{:?}",
        r.elasticsearch.findings
    );
}

#[test]
fn incomplete_declarations_are_noted_on_a_finding() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:\n  api:\n    build: ./api\n",
    );
    put(
        root,
        "infra/main.tf",
        "resource \"elasticstack_elasticsearch_index\" \"g\" {\n  name = var.n\n}\n",
    );
    put(
        root,
        "api/W.java",
        "import org.elasticsearch.client.Client;\nclass W { var r = new IndexRequest(\"lost\"); }\n",
    );
    let r = analyze_services(root).unwrap();
    let f = &r.elasticsearch.findings;
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].kind, "undeclared_index");
    assert!(
        f[0].evidence
            .iter()
            .any(|e| e.description.contains("incomplete"))
    );
}

#[test]
fn comma_separated_and_excluded_usage_names() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:\n  api:\n    build: ./api\n",
    );
    put(
        root,
        "infra/main.tf",
        "resource \"elasticstack_elasticsearch_index\" \"a\" {\n  name = \"a\"\n}\nresource \"elasticstack_elasticsearch_index\" \"b\" {\n  name = \"b\"\n}\n",
    );
    put(
        root,
        "api/W.java",
        "import org.elasticsearch.client.Client;\nclass W { var r = new SearchRequest.Builder().index(\"a,b,-c\"); var s = new SearchRequest.Builder().index(\"a,z\"); }\n",
    );
    let r = analyze_services(root).unwrap();
    assert_eq!(subjects(&r, "undeclared_index"), ["a,z"]);
}

#[test]
fn alias_of_an_index_with_unknown_name_is_declared_and_keeps_its_own_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:\n  a:\n    build: ./a\n  b:\n    build: ./b\n",
    );
    put(
        root,
        "infra/main.tf",
        "resource \"elasticstack_elasticsearch_index\" \"i\" {\n  name = local.n\n  alias {\n    name = \"orders\"\n  }\n}\n",
    );
    let java = |access: &str| {
        format!(
            "import org.elasticsearch.client.Client;\nclass W {{ var r = new {access}(\"orders\"); }}\n"
        )
    };
    put(root, "a/W.java", &java("IndexRequest"));
    put(root, "b/W.java", &java("IndexRequest"));
    let r = analyze_services(root).unwrap();
    assert!(subjects(&r, "undeclared_index").is_empty());
    assert_eq!(subjects(&r, "shared_write_index"), ["orders"]);
}

#[test]
fn alias_declared_on_a_known_and_an_unknown_index_keeps_its_own_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:\n  a:\n    build: ./a\n  b:\n    build: ./b\n",
    );
    put(
        root,
        "infra/main.tf",
        "resource \"elasticstack_elasticsearch_index\" \"i\" {\n  name = local.n\n  alias {\n    name = \"orders\"\n  }\n}\nresource \"elasticstack_elasticsearch_index\" \"j\" {\n  name = \"o-v1\"\n  alias {\n    name = \"orders\"\n  }\n}\n",
    );
    let java = |name: &str| {
        format!(
            "import org.elasticsearch.client.Client;\nclass W {{ var r = new IndexRequest(\"{name}\"); }}\n"
        )
    };
    put(root, "a/W.java", &java("orders"));
    put(root, "b/W.java", &java("o-v1"));
    let r = analyze_services(root).unwrap();
    assert!(subjects(&r, "shared_write_index").is_empty());
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.message.contains("not a literal"))
    );
}
