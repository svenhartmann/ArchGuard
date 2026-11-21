//! Details → pattern candidates and API contracts on temp copies of the
//! repo fixtures, built as the tab builds them (`build_from`), without a
//! window.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use slint::Model;

use super::state::{ServicesState, build_from};
use crate::state::marks::Marks;
use crate::tree::Tree;

/// A copy of the tracked files of `fixtures/<name>`.
fn fixture(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    let listed = std::process::Command::new("git")
        .arg("-C")
        .arg(&src)
        .args(["ls-files", "-z", "."])
        .output()
        .unwrap();
    assert!(listed.status.success());
    for file in listed.stdout.split(|&b| b == 0).filter(|f| !f.is_empty()) {
        let file = Path::new(std::str::from_utf8(file).unwrap());
        let to = dir.path().join(file);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::copy(src.join(file), &to).unwrap();
    }
    dir
}

/// The tab state of `root` after an analysis (`sections::read`, as
/// `build_from` does), and the analysed graph.
fn state(root: &Path) -> (ServicesState, archguard_engine::core::Graph) {
    let loaded = archguard_engine::load_config(None, root).unwrap();
    let run = archguard_engine::analyze_run_quiet(root, &loaded, archguard_engine::Level::Module)
        .unwrap();
    let calls = archguard_engine::analyze_calls(root, &run.config);
    let topology = Arc::new(archguard_engine::analyze_services(root).map_err(|e| e.to_string()));
    let mut s = build_from(topology);
    super::sections::read(&mut s, root, &run, &calls);
    (s, run.graph)
}

/// py-layers: the layered candidate with its counter-evidence comes
/// first, the dependency levels after it as a structural observation; an
/// open candidate lists its evidence and marks it on the Graph's rows.
#[test]
fn pattern_candidates_list_evidence_and_mark_the_graph() {
    let dir = fixture("py-layers");
    let root = dir.path().canonicalize().unwrap();
    let (mut s, graph) = state(&root);
    let m = s.patterns.model();
    assert_eq!(m.state, 1);
    let rows: Vec<_> = m.rows.iter().collect();
    assert_eq!(
        (rows[0].kind, rows[0].status),
        (0, 1),
        "layered, with counter-evidence"
    );
    assert_eq!(
        (rows[1].kind, rows[1].status),
        (1, 0),
        "dependency levels, candidate"
    );
    assert_eq!((m.candidates, m.structures), (1, 1));
    assert_eq!(m.total as usize, rows.len());
    assert!(rows.iter().all(|r| !r.open));

    s.patterns.choose(0);
    let d = s.patterns.model().detail;
    assert_eq!((d.evidence_total, d.counter_total), (10, 2));
    let counter: Vec<_> = d.counter.iter().collect();
    assert_eq!(counter[0].title, "domain/ledger.py → app/service.py");
    assert!(counter[0].place.contains("domain/ledger.py:"));
    assert!(!d.more_evidence);
    assert!(d.nodes > 0 && d.metrics.contains("backward_dependencies"));

    let (files, pairs) = s.patterns.highlight().unwrap();
    assert!(files.contains(&"domain/ledger.py".to_string()));
    let tree = Tree::from_graph(&graph);
    let marks = Marks::new(&tree, &files, &pairs);
    let open: BTreeSet<usize> = (0..tree.nodes.len()).collect();
    let view = tree.view(&open);
    let name = |r: usize| tree.nodes[view.rows[r].node].name.as_str();
    let links: Vec<_> = marks
        .links(&tree, &view)
        .into_iter()
        .map(|(a, b)| (name(a), name(b)))
        .collect();
    assert!(
        links.contains(&("domain/ledger.py", "app/service.py")),
        "{links:?}"
    );

    s.patterns.choose(0);
    assert!(s.patterns.highlight().is_none(), "closed again");
    assert!(!s.patterns.model().rows.row_data(0).unwrap().open);
}

fn service(root: &Path, name: &str) -> String {
    let t = archguard_engine::analyze_services(root).unwrap();
    t.services
        .iter()
        .find(|s| s.name == name)
        .unwrap()
        .id
        .clone()
}

/// An OpenAPI document of `n` operations; with `token`, the first one
/// requires a query parameter.
fn openapi(n: usize, token: bool) -> String {
    let mut paths = serde_json::Map::new();
    for i in 0..n {
        let mut get = json!({"operationId": format!("item{i}"), "responses": {"200": {"description": "OK",
            "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Item"}}}}}});
        if token && i == 0 {
            get["parameters"] = json!([{"name": "token", "in": "query", "required": true, "schema": {"type": "string"}}]);
        }
        paths.insert(format!("/items/{i}"), json!({"get": get}));
    }
    json!({"openapi": "3.1.0", "info": {"title": "Orders", "version": "1"}, "paths": paths,
        "components": {"schemas": {"Item": {"type": "object"}}}})
    .to_string()
}

/// compose-services: without an import the section says so; after two
/// imports it lists the provider's operations with schemas and declared
/// consumers 20 at a time, and the changes between the imports.
#[test]
fn api_contracts_list_operations_consumers_and_changes() {
    let dir = fixture("compose-services");
    let root = dir.path().canonicalize().unwrap();
    let (s, _) = state(&root);
    let m = s.contracts.model();
    assert_eq!(
        (m.state, m.total, m.status.as_str()),
        (1, 0, "not_imported")
    );

    let topology = archguard_engine::analyze_services(&root).unwrap();
    let (orders, shipping) = (service(&root, "orders"), service(&root, "shipping"));
    let usage = json!({"schema_version": 1, "usages": [{"consumer": shipping, "operation": "GET /items/0"}]});
    fs::write(root.join("usage.json"), usage.to_string()).unwrap();
    for token in [false, true] {
        fs::write(root.join("api.json"), openapi(25, token)).unwrap();
        archguard_engine::import_api_contract(
            &root,
            &topology,
            Path::new("api.json"),
            &orders,
            Some(Path::new("usage.json")),
        )
        .unwrap();
    }

    let (mut s, _) = state(&root);
    let m = s.contracts.model();
    assert_eq!(m.total, 1);
    let c = m.contracts.row_data(0).unwrap();
    assert_eq!(
        (c.provider.as_str(), c.operations, c.open),
        (orders.as_str(), 25, false)
    );
    assert!(c.mapped);

    s.contracts.choose(0);
    let d = s.contracts.model().detail;
    assert_eq!(d.operations.row_count(), 20);
    assert!(d.more);
    let first = d
        .operations
        .iter()
        .find(|o| o.title == "GET /items/0 · item0")
        .unwrap();
    assert_eq!(first.schemas, "#/components/schemas/Item");
    assert_eq!(first.consumers, shipping.as_str());
    assert!(first.pointer.starts_with("api.json#"));
    let change = d
        .changes
        .row_data(0)
        .expect("a change against the previous import");
    assert!(
        change.title.starts_with("GET /items/0 · "),
        "{}",
        change.title
    );
    assert_eq!(change.consumers, shipping.as_str());

    s.contracts.more_details();
    let d = s.contracts.model().detail;
    assert_eq!((d.operations.row_count(), d.more), (25, false));
}
