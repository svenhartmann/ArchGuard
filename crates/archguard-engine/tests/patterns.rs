use archguard_engine::core::{
    CallAnalysis, Config, Graph, GraphBuilder, Ref, ServiceDependencyKind,
};
use archguard_engine::{
    Level, analyze_architecture_patterns, analyze_run, analyze_services, architecture_patterns,
    generate_review, load_config, read_saved_review, save_review,
};
use serde_json::{Value, json};
use std::fs;
fn graph(edges: &[(&str, &str)]) -> Graph {
    let mut b = GraphBuilder::new();
    for (a, z) in edges {
        let from = b.ensure_path(a, None);
        let to = b.ensure_path(z, None);
        b.edge(
            from,
            to,
            Ref {
                file: (*a).into(),
                line: 7,
                raw: format!("{a} -> {z}"),
            },
        )
        .unwrap();
    }
    b.build("/unused".into())
}
fn row<'a>(report: &'a Value, pattern: &str) -> &'a Value {
    report["assessments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pattern"] == pattern)
        .unwrap()
}

#[test]
fn pattern_recall_finds_anonymous_dependency_levels_without_claiming_named_layers() {
    let root = tempfile::tempdir().unwrap();
    let g = graph(&[
        ("alpha/One.ts", "beta/Two.ts"),
        ("beta/Two.ts", "gamma/Three.ts"),
        ("alpha/One.ts", "gamma/Three.ts"),
        ("other/X.ts", "other/Y.ts"),
        ("other/Y.ts", "other/X.ts"),
    ]);
    let report =
        serde_json::to_value(architecture_patterns(root.path(), &Config::default(), &g).unwrap())
            .unwrap();
    let levels = row(&report, "dependency_layers");
    assert_eq!(levels["status"], "candidate");
    assert_eq!(levels["metrics"]["dependency_levels"], 3);
    assert_eq!(levels["metrics"]["source_files"], 3);
    assert_eq!(
        levels["counter_evidence_total"], 0,
        "an unrelated cycle must not poison this component"
    );
    assert_ne!(
        row(&report, "layered")["status"],
        "candidate",
        "a DAG is not proof of architectural responsibilities"
    );
    let witness: Vec<_> = levels["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "dependency_level")
        .map(|e| (e["from"].as_str().unwrap(), e["to"].as_str().unwrap()))
        .collect();
    assert_eq!(
        witness,
        vec![
            ("alpha/One.ts", "beta/Two.ts"),
            ("beta/Two.ts", "gamma/Three.ts")
        ]
    );
    assert!(
        levels["missing_evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().contains("architectural intent"))
    );
}

#[test]
fn pattern_recall_finds_local_chains_beside_branching_code_with_distinct_ids() {
    let root = tempfile::tempdir().unwrap();
    let g = graph(&[
        ("first/A.ts", "first/B.ts"),
        ("first/B.ts", "first/C.ts"),
        ("second/A.ts", "second/B.ts"),
        ("second/B.ts", "second/C.ts"),
        ("branch/A.ts", "branch/B.ts"),
        ("branch/A.ts", "branch/C.ts"),
    ]);
    let report =
        serde_json::to_value(architecture_patterns(root.path(), &Config::default(), &g).unwrap())
            .unwrap();
    let chains: Vec<_> = report["assessments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["pattern"] == "pipeline" && p["status"] == "candidate")
        .collect();
    assert_eq!(
        chains.len(),
        2,
        "unrelated branching code must not hide local chains"
    );
    assert_ne!(chains[0]["id"], chains[1]["id"]);
    assert!(
        chains
            .iter()
            .all(|p| p["scope"] != "." && p["nodes_total"] == 3)
    );
}

#[test]
fn pattern_recall_keeps_unrelated_cycles_out_of_role_layer_counter_evidence() {
    let root = tempfile::tempdir().unwrap();
    let g = graph(&[
        ("web/A.ts", "application/B.ts"),
        ("application/B.ts", "data/C.ts"),
        ("utility/X.ts", "utility/Y.ts"),
        ("utility/Y.ts", "utility/X.ts"),
    ]);
    let report =
        serde_json::to_value(architecture_patterns(root.path(), &Config::default(), &g).unwrap())
            .unwrap();
    assert_eq!(row(&report, "layered")["status"], "candidate");
    assert_eq!(row(&report, "layered")["counter_evidence_total"], 0);
}

#[test]
fn pattern_recall_recognizes_inversion_without_core_or_adapter_names() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("billing/logic")).unwrap();
    fs::write(
        root.path().join("billing/logic/Checkout.java"),
        "package billing.logic;\nclass Checkout {\n Contract contract;\n}\n",
    )
    .unwrap();
    let services = analyze_services(root.path()).unwrap();
    let mut calls: CallAnalysis = serde_json::from_value(json!({"status":"partial","files":3,"diagnostics":[],"methods":[],"classes":[
        {"id":"Contract","name":"Contract","package":"billing.contracts","file":"billing/contracts/Contract.java","graph_node":"billing/contracts/Contract.java","line":1,"kind":"interface"},
        {"id":"Checkout","name":"Checkout","package":"billing.logic","file":"billing/logic/Checkout.java","graph_node":"billing/logic/Checkout.java","line":1,"kind":"class"},
        {"id":"Jdbc","name":"Jdbc","package":"postgres.driver","file":"postgres/driver/Jdbc.java","graph_node":"postgres/driver/Jdbc.java","line":1,"kind":"class","supertypes":["Contract"]}
    ],"calls":[{"from_class":"Checkout","to_class":"Contract","target_label":"Contract.save","dispatch":"interface","resolution":"project","file":"billing/logic/Checkout.java","line":5,"column":1,"code":"contract.save()"}]})).unwrap();
    let pairs = [
        (
            "billing/logic/Checkout.java",
            "billing/contracts/Contract.java",
        ),
        (
            "postgres/driver/Jdbc.java",
            "billing/contracts/Contract.java",
        ),
    ];
    let g = graph(&pairs);
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &g,
        &calls,
        &services,
    ))
    .unwrap();
    let inversion = row(&report, "dependency_inversion");
    assert_eq!(inversion["status"], "candidate");
    assert!(
        inversion["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "port_call")
    );
    assert_eq!(
        row(&report, "ports_and_adapters")["status"],
        "insufficient_evidence"
    );
    let leaking = graph(&[
        pairs[0],
        pairs[1],
        ("billing/logic/Checkout.java", "postgres/driver/Jdbc.java"),
    ]);
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &leaking,
        &calls,
        &services,
    ))
    .unwrap();
    assert_eq!(row(&report, "dependency_inversion")["status"], "mixed");
    calls.calls.clear();
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &g,
        &calls,
        &services,
    ))
    .unwrap();
    assert!(
        !report["assessments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["pattern"] == "dependency_inversion" && p["status"] == "candidate"),
        "interface declarations alone are insufficient"
    );
    calls.details=Some(serde_json::from_value(json!({"definition":"source-types-state/v1","class_states":[],"type_uses":[{"from_class":"Checkout","from_method":null,"to_class":"Contract","target_label":"Contract","role":"field","resolution":"project","file":"billing/logic/Checkout.java","line":3,"code":"Contract contract"}]})).unwrap());
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &g,
        &calls,
        &services,
    ))
    .unwrap();
    let inversion = row(&report, "dependency_inversion");
    assert_eq!(
        inversion["status"], "candidate",
        "typed dependencies establish indirection even without a method call"
    );
    assert!(
        inversion["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "port_type"
                && e["line"] == 3
                && e["file"] == "billing/logic/Checkout.java")
    );
}

#[test]
fn pattern_recall_cycle_condensation_never_invents_a_connected_file_path() {
    let root = tempfile::tempdir().unwrap();
    let g = graph(&[
        ("A.ts", "B.ts"),
        ("B.ts", "C.ts"),
        ("C.ts", "B.ts"),
        ("C.ts", "D.ts"),
        ("D.ts", "E.ts"),
    ]);
    let report =
        serde_json::to_value(architecture_patterns(root.path(), &Config::default(), &g).unwrap())
            .unwrap();
    let levels = row(&report, "dependency_layers");
    assert_eq!(levels["status"], "mixed");
    assert_eq!(levels["metrics"]["dependency_levels"], 4);
    assert_eq!(levels["metrics"]["cyclic_files"], 2);
    assert_eq!(levels["metrics"]["witness_connected"], false);
    let original: std::collections::BTreeSet<_> = g
        .edges
        .iter()
        .map(|e| {
            (
                g.nodes[e.from.0 as usize].name.as_str(),
                g.nodes[e.to.0 as usize].name.as_str(),
            )
        })
        .collect();
    for e in levels["evidence"].as_array().unwrap() {
        assert!(original.contains(&(e["from"].as_str().unwrap(), e["to"].as_str().unwrap())));
    }
    for pairs in [
        vec![("A.ts", "B.ts"), ("B.ts", "C.ts"), ("C.ts", "A.ts")],
        vec![("A.ts", "B.ts"), ("A.ts", "C.ts")],
    ] {
        let report = serde_json::to_value(
            architecture_patterns(root.path(), &Config::default(), &graph(&pairs)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            row(&report, "dependency_layers")["status"],
            "insufficient_evidence"
        );
        assert_ne!(row(&report, "pipeline")["status"], "candidate");
    }
}
#[test]
fn typed_ports_need_both_a_core_caller_and_outer_implementation_and_expose_leaks() {
    let root = tempfile::tempdir().unwrap();
    for name in ["core/Port.java", "core/Use.java", "infra/Db.java"] {
        let file = root.path().join(name);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, "// synthetic compiler input\n".repeat(10)).unwrap();
    }
    let services = analyze_services(root.path()).unwrap();
    let calls:CallAnalysis=serde_json::from_value(json!({"status":"partial","files":3,"diagnostics":[],"classes":[
        {"id":"Port","name":"Port","package":"core","file":"core/Port.java","graph_node":"core/Port.java","line":2,"kind":"interface"},
        {"id":"Use","name":"Use","package":"core","file":"core/Use.java","graph_node":"core/Use.java","line":1,"kind":"class"},
        {"id":"Db","name":"Db","package":"infra","file":"infra/Db.java","graph_node":"infra/Db.java","line":3,"kind":"class","supertypes":["Port"]}
    ],"methods":[],"calls":[{"from_class":"Use","to_class":"Port","target_label":"Port.save","dispatch":"interface","resolution":"project","file":"core/Use.java","line":9,"column":1,"code":"port.save()"}]})).unwrap();
    let clean = graph(&[
        ("core/Use.java", "core/Port.java"),
        ("infra/Db.java", "core/Port.java"),
    ]);
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &clean,
        &calls,
        &services,
    ))
    .unwrap();
    let p = row(&report, "ports_and_adapters");
    assert_eq!(p["status"], "candidate");
    assert!(
        p["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "port_call" && e["line"] == 9)
    );
    assert!(
        p["missing_evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m.as_str().unwrap().contains("partial"))
    );
    let leaking = graph(&[
        ("core/Use.java", "core/Port.java"),
        ("infra/Db.java", "core/Port.java"),
        ("core/Use.java", "infra/Db.java"),
    ]);
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &leaking,
        &calls,
        &services,
    ))
    .unwrap();
    assert_eq!(row(&report, "ports_and_adapters")["status"], "mixed");
    let mut names_only = calls.clone();
    names_only.calls.clear();
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &clean,
        &names_only,
        &services,
    ))
    .unwrap();
    assert_eq!(
        row(&report, "ports_and_adapters")["status"],
        "insufficient_evidence"
    );
    let mut ambiguous = calls;
    ambiguous.classes.push(ambiguous.classes[0].clone());
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &clean,
        &ambiguous,
        &services,
    ))
    .unwrap();
    assert_eq!(
        row(&report, "ports_and_adapters")["status"],
        "insufficient_evidence"
    );
}
#[test]
fn modular_structure_is_not_a_deployment_claim_and_http_is_not_startup() {
    let root = tempfile::tempdir().unwrap();
    let g = graph(&[("sales/a.ts", "sales/b.ts"), ("stock/a.ts", "stock/b.ts")]);
    let report =
        serde_json::to_value(architecture_patterns(root.path(), &Config::default(), &g).unwrap())
            .unwrap();
    assert_eq!(row(&report, "modular_monolith")["status"], "candidate");
    fs::create_dir(root.path().join("api")).unwrap();
    fs::create_dir(root.path().join("worker")).unwrap();
    fs::write(root.path().join("compose.yaml"),"services:\n  api:\n    build: ./api\n    environment:\n      OTHER_URL: http://worker:8080\n  worker:\n    build: ./worker\n").unwrap();
    let services = analyze_services(root.path()).unwrap();
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &g,
        &CallAnalysis::default(),
        &services,
    ))
    .unwrap();
    assert_eq!(row(&report, "microservices")["status"], "candidate");
    assert_eq!(row(&report, "modular_monolith")["status"], "mixed");
    let mut startup = services.clone();
    for d in &mut startup.dependencies {
        d.kind = ServiceDependencyKind::Startup;
    }
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &g,
        &CallAnalysis::default(),
        &startup,
    ))
    .unwrap();
    assert_eq!(
        row(&report, "microservices")["status"],
        "insufficient_evidence"
    );
    let mut replicas = services;
    for s in &mut replicas.services {
        s.project = None;
        s.build_context = Some("same".into());
    }
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &g,
        &CallAnalysis::default(),
        &replicas,
    ))
    .unwrap();
    assert_eq!(
        row(&report, "microservices")["status"],
        "insufficient_evidence"
    );
}
#[test]
fn asyncapi_v2_uses_application_perspective_and_review_inputs_track_contract_edits() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.ts"), "export const a=1;\n").unwrap();
    let api = "asyncapi: 2.6.0\nchannels:\n  orders:\n    publish:\n      operationId: consume\n    subscribe:\n      operationId: produce\n";
    fs::write(root.path().join("asyncapi.yaml"), api).unwrap();
    let loaded = load_config(None, root.path()).unwrap();
    let run = analyze_run(root.path(), &loaded, Level::Module).unwrap();
    let report =
        serde_json::to_value(architecture_patterns(root.path(), &run.config, &run.graph).unwrap())
            .unwrap();
    let events = row(&report, "event_driven")["evidence"].as_array().unwrap();
    assert!(
        events
            .iter()
            .any(|e| e["pointer"] == "/channels/orders/publish"
                && e["message"].as_str().unwrap().contains("receive"))
    );
    assert!(
        events
            .iter()
            .any(|e| e["pointer"] == "/channels/orders/subscribe"
                && e["message"].as_str().unwrap().contains("send"))
    );
    let (review, _) = generate_review(root.path(), None, None, 1793232000).unwrap();
    assert_eq!(
        row(&review.architecture["patterns"], "event_driven")["status"],
        "candidate"
    );
    save_review(root.path(), &review).unwrap();
    fs::write(
        root.path().join("asyncapi.yaml"),
        api.replace("orders", "invoices"),
    )
    .unwrap();
    assert_eq!(
        read_saved_review(root.path(), &loaded, 1793232000).status,
        "stale"
    );
}
#[cfg(unix)]
#[test]
fn asyncapi_paths_and_symlinks_cannot_read_outside_the_project() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.yaml"), "asyncapi: 3.0.0\n").unwrap();
    symlink(
        outside.path().join("secret.yaml"),
        root.path().join("asyncapi.yaml"),
    )
    .unwrap();
    let mut config = Config::default();
    config.architecture.asyncapi = vec!["../secret.yaml".into()];
    let report =
        serde_json::to_value(architecture_patterns(root.path(), &config, &graph(&[])).unwrap())
            .unwrap();
    assert_eq!(report["coverage"]["asyncapi_status"], "partial");
    assert_eq!(row(&report, "event_driven")["metrics"]["documents"], 0);
    assert!(report["diagnostics"].as_array().unwrap().len() >= 2);
}

#[test]
fn disconnected_role_edges_do_not_combine_into_a_layered_candidate() {
    let root = tempfile::tempdir().unwrap();
    let g = graph(&[
        ("web/a.ts", "application/b.ts"),
        ("application/other.ts", "data/c.ts"),
    ]);
    let report =
        serde_json::to_value(architecture_patterns(root.path(), &Config::default(), &g).unwrap())
            .unwrap();
    assert_eq!(row(&report, "layered")["status"], "insufficient_evidence");
}

#[test]
fn service_units_must_have_an_http_connection_to_each_other() {
    let root = tempfile::tempdir().unwrap();
    for name in ["api", "worker"] {
        fs::create_dir(root.path().join(name)).unwrap();
    }
    fs::write(root.path().join("compose.yaml"), "services:\n  api:\n    build: ./api\n    environment:\n      CACHE_URL: http://cache:80\n  worker:\n    build: ./worker\n    environment:\n      CACHE_URL: http://cache:80\n  cache:\n    image: cache\n").unwrap();
    let report = serde_json::to_value(
        architecture_patterns(root.path(), &Config::default(), &graph(&[])).unwrap(),
    )
    .unwrap();
    assert_eq!(
        row(&report, "microservices")["status"],
        "insufficient_evidence"
    );
}

#[test]
fn unmapped_graph_references_are_not_presented_as_physical_file_locations() {
    let root = tempfile::tempdir().unwrap();
    let report = serde_json::to_value(
        architecture_patterns(
            root.path(),
            &Config::default(),
            &graph(&[
                ("web/a.ts", "application/b.ts"),
                ("application/b.ts", "data/c.ts"),
            ]),
        )
        .unwrap(),
    )
    .unwrap();
    for e in row(&report, "layered")["evidence"].as_array().unwrap() {
        assert!(
            e["file"].is_null(),
            "A logical graph path without a unique physical source mapping must not be a source location"
        );
        assert!(e["message"].as_str().unwrap().contains("unmapped"));
    }
}

#[test]
fn resolved_framework_annotations_supply_layer_roles_without_directory_names() {
    let root = tempfile::tempdir().unwrap();
    let services = analyze_services(root.path()).unwrap();
    let g = graph(&[("x/A.java", "y/B.java"), ("y/B.java", "z/C.java")]);
    let classes=serde_json::from_value(json!([
        {"id":"A","name":"A","package":"x","file":"x/A.java","graph_node":"x/A.java","line":1,"kind":"class"},
        {"id":"B","name":"B","package":"y","file":"y/B.java","graph_node":"y/B.java","line":1,"kind":"class"},
        {"id":"C","name":"C","package":"z","file":"z/C.java","graph_node":"z/C.java","line":1,"kind":"class"}
    ])).unwrap();
    let annotations = [
        (
            "A",
            "x/A.java",
            "org.springframework.web.bind.annotation.RestController",
        ),
        ("B", "y/B.java", "org.springframework.stereotype.Service"),
        ("C", "z/C.java", "org.springframework.stereotype.Repository"),
    ];
    let uses:Vec<_>=annotations.iter().map(|(owner,file,target)|json!({"from_class":owner,"from_method":null,"to_class":target,"target_label":target,"role":"class_annotation","resolution":"external","file":file,"line":1,"code":format!("@{target}")})).collect();
    let details = Some(
        serde_json::from_value(
            json!({"definition":"source-types-state/v1","class_states":[],"type_uses":uses}),
        )
        .unwrap(),
    );
    let mut calls = CallAnalysis {
        status: "ready".into(),
        classes,
        details,
        ..Default::default()
    };
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &g,
        &calls,
        &services,
    ))
    .unwrap();
    assert_eq!(row(&report, "layered")["status"], "candidate");
    assert!(
        row(&report, "layered")["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "framework_role")
    );
    for used in &mut calls.details.as_mut().unwrap().type_uses {
        used.resolution = "unresolved".into();
    }
    let report = serde_json::to_value(analyze_architecture_patterns(
        root.path(),
        &Config::default(),
        &g,
        &calls,
        &services,
    ))
    .unwrap();
    assert_ne!(
        row(&report, "layered")["status"],
        "candidate",
        "unresolved annotations cannot assign architectural roles"
    );
}
