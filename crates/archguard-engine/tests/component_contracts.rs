use archguard_engine::{Level, analyze_run, analyze_target_architecture, load_config};
use serde_json::Value;
use std::fs;

fn project(policy: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, code) in [
        (
            "src/ui/page.ts",
            "import '../core/api';\nimport '../core/internal';\nimport '../core/api';\n",
        ),
        (
            "src/core/api.ts",
            "import './internal';\nimport '../storage/db';\n",
        ),
        ("src/core/internal.ts", "export const value=1;\n"),
        ("src/storage/db.ts", "export const db=1;\n"),
    ] {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, code).unwrap();
    }
    fs::write(dir.path().join("archguard.toml"), format!("derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['ts']\nexclude=[]\n[rules]\ndeny_cycles='off'\n[architecture.model]\nname='Product'\nrequire_assignment=true\n[[architecture.model.components]]\nname='Web'\ninclude=['ui/**']\n[[architecture.model.components]]\nname='Domain'\ninclude=['core/**']\n{policy}\n[[architecture.model.components]]\nname='Storage'\ninclude=['storage/**']\n[[architecture.model.allowed_dependencies]]\nfrom='Web'\nto='Domain'\n[[architecture.model.allowed_dependencies]]\nfrom='Domain'\nto='Storage'\n")).unwrap();
    dir
}
fn analyze(dir: &tempfile::TempDir) -> (archguard_engine::AnalysisRun, Value) {
    let loaded = load_config(None, dir.path()).expect("component contracts load");
    let run = analyze_run(dir.path(), &loaded, Level::Module).unwrap();
    let model = analyze_target_architecture(&run.graph, &run.config)
        .unwrap()
        .unwrap();
    (run, serde_json::to_value(model).unwrap())
}
#[test]
fn public_interfaces_and_neighbor_budgets_gate_original_sources_at_every_depth() {
    let dir =
        project("public=['core/api.ts']\nmax_incoming_components=1\nmax_outgoing_components=0");
    let mut ids = None;
    for depth in [0, 1, 20] {
        let mut loaded = load_config(None, dir.path()).expect("component contracts load");
        loaded.config.modules.depth = depth;
        let run = analyze_run(dir.path(), &loaded, Level::Module).unwrap();
        let model = serde_json::to_value(
            analyze_target_architecture(&run.graph, &run.config)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            model["violating_dependencies"], 0,
            "component direction is allowed"
        );
        assert_eq!(model["violating_interfaces"], 1);
        assert_eq!(model["coupling_violations"], 1);
        let domain = model["components"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "Domain")
            .unwrap();
        assert_eq!(domain["incoming_components"], serde_json::json!(["Web"]));
        assert_eq!(
            domain["outgoing_components"],
            serde_json::json!(["Storage"])
        );
        assert_eq!(domain["public_nodes"], serde_json::json!(["core/api.ts"]));
        let findings: Vec<_> = run
            .report
            .findings
            .iter()
            .filter(|f| f.id.starts_with("target:"))
            .collect();
        assert_eq!(
            findings.len(),
            2,
            "internal calls and repeated imports do not create extra violations"
        );
        let access = findings
            .iter()
            .find(|f| f.message.as_ref().unwrap().contains("private"))
            .unwrap();
        assert_eq!(access.nodes, ["core/internal.ts", "ui/page.ts"]);
        assert_eq!(access.evidence[0].line, 2);
        let current: Vec<_> = findings.iter().map(|f| f.id.clone()).collect();
        if let Some(ids) = &ids {
            assert_eq!(&current, ids);
        } else {
            ids = Some(current);
        }
    }
}
#[test]
fn omitted_interfaces_remain_open_and_empty_interfaces_are_explicitly_closed() {
    let (legacy, _) = analyze(&project(""));
    assert!(legacy.report.findings.is_empty());
    let (closed, model) = analyze(&project(
        "public=[]\nmax_incoming_components=1\nmax_outgoing_components=1",
    ));
    assert_eq!(model["violating_interfaces"], 2);
    assert_eq!(
        model["coupling_violations"], 0,
        "limits count distinct component neighbors, not imports"
    );
    assert_eq!(closed.report.findings.len(), 2);
    let (_, inbound) = analyze(&project("max_incoming_components=0"));
    assert_eq!(inbound["coupling_violations"], 1);
}
#[test]
fn invalid_interface_patterns_are_rejected_and_unassigned_sources_are_not_guessed() {
    for policy in ["public=['[']", "public=['']", "max_incoming_components=-1"] {
        assert!(load_config(None, project(policy).path()).is_err());
    }
    let dir = project("public=['core/api.ts']");
    fs::write(
        dir.path().join("src/unassigned.ts"),
        "import './core/internal';\n",
    )
    .unwrap();
    let (_, model) = analyze(&dir);
    assert_eq!(model["status"], "partial");
    assert_eq!(
        model["violating_interfaces"], 1,
        "unclassified callers remain an explicit coverage gap"
    );
    assert_eq!(model["unclassified_edges"], 1);
}
