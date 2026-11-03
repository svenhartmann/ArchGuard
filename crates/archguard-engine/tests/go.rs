use archguard_core::Config;
use archguard_engine::{analyze_calls, code_metrics};
#[test]
fn go_structs_functions_and_measured_complexity_reach_the_shared_model() {
    if std::env::var_os("ARCHGUARD_GO").is_none() {
        eprintln!("Go SDK integration test: set ARCHGUARD_GO");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("store")).unwrap();
    std::fs::write(
        d.path().join("go.mod"),
        "module example.org/shop\ngo 1.22\n",
    )
    .unwrap();
    std::fs::write(d.path().join("store/store.go"),"package store\ntype Store struct { Count int }\nfunc (s *Store) Add(n int) int { if n > 0 { s.Count += n }; return s.Count }\n").unwrap();
    std::fs::write(d.path().join("api.go"),"package shop\nimport \"example.org/shop/store\"\nfunc Run() int { s := store.Store{}; return s.Add(2) }\n").unwrap();
    let calls = analyze_calls(d.path(), &Config::default());
    assert!(!calls.classes.is_empty(), "{:?}", calls.diagnostics);
    let class = calls.classes.iter().find(|c| c.name == "Store").unwrap();
    assert_eq!(class.language.as_deref(), Some("go"));
    assert_eq!(class.graph_node, "store/store.go");
    assert!(
        calls
            .calls
            .iter()
            .any(|c| c.to_class.as_ref() == Some(&class.id) && c.resolution != "unresolved")
    );
    let (graph, _) =
        archguard_engine::analyze(d.path(), &Config::default(), archguard_engine::Level::File)
            .unwrap();
    let coverage = archguard_engine::read_test_coverage(d.path(), &Config::default(), &graph);
    let world = archguard_engine::architecture_world(&graph, &calls, &coverage);
    assert!(
        world
            .entities
            .iter()
            .any(|e| e.class_id.as_ref() == Some(&class.id) && e.max_complexity == Some(2)),
        "nodes={:?}, world={:?}, class={:?}",
        graph.nodes,
        world,
        class
    );
    let metrics = code_metrics(&calls);
    let measured = metrics
        .classes
        .iter()
        .find(|c| c.class_id == class.id)
        .unwrap();
    assert_eq!(measured.wmc.value, Some(2));
    assert_eq!(measured.dit.status, "not_applicable");
    assert_eq!(measured.noc.status, "not_applicable");
    assert!(
        metrics
            .methods
            .iter()
            .any(|m| m.cognitive.is_some() && m.source_lines.is_some())
    );
}
