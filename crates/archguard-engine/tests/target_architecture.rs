use archguard_engine::core::Status;
use archguard_engine::{Level, analyze_run, architecture_context, load_config};
use serde_json::Value;
use std::fs;

fn model() -> String {
    r#"derive_layers=false
[project]
source_roots=['src']
languages=['ts']
exclude=[]
[rules]
deny_cycles='off'
[architecture.model]
name='Business boundaries'
severity='error'
require_assignment=true
[[architecture.model.components]]
name='Experience'
include=['ui/**']
[[architecture.model.components]]
name='Domain'
include=['core/**']
[[architecture.model.components]]
name='Storage'
include=['storage/**']
[[architecture.model.allowed_dependencies]]
from='Experience'
to='Domain'
[[architecture.model.allowed_dependencies]]
from='Storage'
to='Domain'
"#
    .into()
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, contents) in [
        (
            "src/ui/A.ts",
            "import {x} from '../core/B';\nimport {y} from '../storage/C';\nexport const a=x+y;",
        ),
        ("src/core/B.ts", "export const x=1;"),
        (
            "src/storage/C.ts",
            "import {x} from '../core/B';\nexport const y=x;",
        ),
    ] {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    dir
}
fn run(dir: &tempfile::TempDir, config: &str) -> (archguard_engine::core::Report, Value) {
    fs::write(dir.path().join("archguard.toml"), config).unwrap();
    let loaded = load_config(None, dir.path()).expect("modeled target architecture loads");
    let run = analyze_run(dir.path(), &loaded, Level::Module).unwrap();
    let context = architecture_context(dir.path(), &run.config, &run.graph, &run.report);
    (
        run.report,
        serde_json::to_value(context).unwrap()["observed"]["target_model"].clone(),
    )
}
#[test]
fn logical_boundaries_gate_original_edges_and_keep_allowed_observations_at_every_depth() {
    let dir = fixture();
    let mut identity = None;
    for depth in [0, 1, 20] {
        let (report, target) = run(&dir, &format!("{}\n[modules]\ndepth={depth}\n", model()));
        assert_eq!(report.summary.status, Status::Fail);
        let findings: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.id.starts_with("target:"))
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence[0].file, "ui/A.ts");
        assert_eq!(findings[0].evidence[0].line, 2);
        assert_eq!(findings[0].nodes, vec!["storage/C.ts", "ui/A.ts"]);
        if let Some(id) = &identity {
            assert_eq!(id, &findings[0].id);
        } else {
            identity = Some(findings[0].id.clone());
        }
        assert_eq!(target["status"], "complete");
        assert_eq!(target["mapped_nodes"], 3);
        let edges = target["dependencies"].as_array().unwrap();
        assert_eq!(edges.len(), 3);
        assert_eq!(edges.iter().filter(|e| e["allowed"] == false).count(), 1);
        assert!(
            edges
                .iter()
                .any(|e| e["from"] == "Experience" && e["to"] == "Domain" && e["allowed"] == true)
        );
    }
    let (warning, _) = run(
        &dir,
        &model().replace("severity='error'", "severity='warning'"),
    );
    assert_eq!(warning.summary.status, Status::Pass);
}
#[test]
fn ambiguous_unmapped_and_empty_components_are_explicit_without_guessing_dependencies() {
    let dir = fixture();
    fs::write(dir.path().join("src/loose.ts"), "export const loose=1;").unwrap();
    let config = format!(
        "{}\n[[architecture.model.components]]\nname='Duplicate'\ninclude=['ui/**']\n[[architecture.model.components]]\nname='Missing'\ninclude=['missing/**']\n",
        model()
    );
    let (report, target) = run(&dir, &config);
    assert_eq!(target["status"], "partial");
    assert_eq!(target["source_nodes"], 4);
    assert_eq!(target["mapped_nodes"], 2);
    assert_eq!(target["unmapped"], serde_json::json!(["loose.ts"]));
    assert_eq!(target["ambiguous"][0]["node"], "ui/A.ts");
    assert!(
        target["empty_components"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("Missing"))
    );
    assert_eq!(target["unclassified_edges"], 2);
    assert_eq!(
        target["dependencies"].as_array().unwrap().len(),
        1,
        "ambiguous origin cannot invent logical edges"
    );
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|f| f.id.starts_with("target:"))
            .count(),
        2
    );
    let (optional, _) = run(
        &dir,
        &config.replace("require_assignment=true", "require_assignment=false"),
    );
    assert_eq!(
        optional
            .findings
            .iter()
            .filter(|f| f.id.starts_with("target:"))
            .count(),
        1
    );
}
#[test]
fn invalid_models_fail_before_analysis_and_model_is_optional() {
    let dir = fixture();
    run(&dir, &model());
    for invalid in [
        model().replace("name='Business boundaries'", "name=''"),
        model().replace("include=['ui/**']", "include=['[']"),
        model().replace("include=['ui/**']", "include=[]"),
        model().replace("name='Storage'", "name='Domain'"),
        model().replace("from='Storage'", "from='Unknown'"),
        model().replace("to='Domain'", "to='Experience'"),
        format!(
            "{}\n[[architecture.model.allowed_dependencies]]\nfrom='Experience'\nto='Domain'",
            model()
        ),
    ] {
        fs::write(dir.path().join("archguard.toml"), invalid).unwrap();
        assert!(load_config(None, dir.path()).is_err());
    }
    let (report, target) = run(&dir, "derive_layers=false\n[rules]\ndeny_cycles='off'\n");
    assert!(target.is_null());
    assert!(report.findings.is_empty());
}

#[test]
fn symbolic_targets_limit_coverage_without_becoming_mapped_source_files() {
    use archguard_engine::core::{Edge, Node, NodeId, NodeKind};
    let dir = fixture();
    fs::write(dir.path().join("archguard.toml"), model()).unwrap();
    let loaded = load_config(None, dir.path()).unwrap();
    let run = analyze_run(dir.path(), &loaded, Level::File).unwrap();
    let mut graph = run.graph;
    let from = graph.nodes.iter().find(|n| n.name == "ui/A.ts").unwrap().id;
    graph.nodes.push(Node {
        id: NodeId(9000),
        name: "core/*".into(),
        kind: NodeKind::File,
        parent: None,
        lang: None,
    });
    graph.edges.push(Edge {
        from,
        to: NodeId(9000),
        count: Some(2),
        refs: vec![],
    });
    let report = archguard_engine::analyze_target_architecture(&graph, &run.config)
        .unwrap()
        .unwrap();
    assert_eq!(report.source_nodes, 3);
    assert_eq!(report.mapped_nodes, 3);
    assert_eq!(report.unclassified_edges, 1);
    assert_eq!(report.total_edges, 4);
    assert_eq!(report.status, "partial");
    assert!(report.unmapped.is_empty());
    assert!(
        report
            .components
            .iter()
            .all(|c| !c.nodes.iter().any(|n| n.ends_with("/*")))
    );
}

#[test]
fn model_export_preserves_all_stored_evidence_for_paginated_inspection() {
    let dir = fixture();
    fs::write(dir.path().join("archguard.toml"), model()).unwrap();
    let loaded = load_config(None, dir.path()).unwrap();
    let run = analyze_run(dir.path(), &loaded, Level::File).unwrap();
    let mut graph = run.graph;
    let names: std::collections::BTreeMap<_, _> =
        graph.nodes.iter().map(|n| (n.id, n.name.clone())).collect();
    let edge = graph
        .edges
        .iter_mut()
        .find(|edge| names[&edge.from] == "ui/A.ts" && names[&edge.to] == "storage/C.ts")
        .unwrap();
    edge.refs = (1..=11)
        .map(|line| archguard_engine::core::Ref {
            file: "ui/A.ts".into(),
            line,
            raw: format!("import {{value{line}}} from '../storage/C';"),
        })
        .collect();
    edge.count = Some(11);
    let target = archguard_engine::analyze_target_architecture(&graph, &run.config)
        .unwrap()
        .unwrap();
    let dependency = target.dependencies.iter().find(|d| !d.allowed).unwrap();
    assert_eq!(dependency.evidence_total, 11);
    assert_eq!(
        dependency.evidence.len(),
        11,
        "model export does not add a second silent source-evidence cap"
    );
}
