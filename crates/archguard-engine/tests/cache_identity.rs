use archguard_engine::{Level, analyze_run, load_config};
use std::fs;

#[test]
fn content_config_manifests_and_schema_invalidate_but_rules_do_not() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    let source = dir.path().join("src/a.ts");
    fs::write(&source, "import './b';\nimport 'external';").unwrap();
    fs::write(dir.path().join("src/b.ts"), "export const b = 1;").unwrap();
    fs::write(dir.path().join("src/c.ts"), "export const c = 1;").unwrap();
    let run = || {
        analyze_run(
            dir.path(),
            &load_config(None, dir.path()).unwrap(),
            Level::File,
        )
        .unwrap()
    };
    let first = run();
    let warm = run();
    assert!(!first.cache_hit && warm.cache_hit);
    assert_eq!(first.warnings, warm.warnings);
    assert!(!warm.warnings.is_empty());
    let modified = fs::metadata(&source).unwrap().modified().unwrap();
    fs::write(&source, "import './c';\nimport 'external';").unwrap();
    fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let changed = run();
    assert!(!changed.cache_hit);
    assert_ne!(first.graph.edges, changed.graph.edges);
    assert!(run().cache_hit);
    fs::write(
        dir.path().join("archguard.toml"),
        "[rules]\ndeny_cycles = 'off'",
    )
    .unwrap();
    assert!(run().cache_hit, "rules do not change extraction");
    fs::write(
        dir.path().join("archguard.toml"),
        "[project]\nsource_roots = ['other']\nlanguages = ['ts']",
    )
    .unwrap();
    assert!(!run().cache_hit);
    assert!(run().graph.nodes.iter().any(|node| node.name == "src/a.ts"));
    fs::write(
        dir.path().join("composer.json"),
        r#"{"autoload":{"psr-4":{"App\\":"src/"}}}"#,
    )
    .unwrap();
    assert!(!run().cache_hit);
    assert!(run().cache_hit);
    fs::write(
        dir.path().join("composer.json"),
        r#"{"autoload":{"psr-4":{"Other\\":"src/"}}}"#,
    )
    .unwrap();
    assert!(!run().cache_hit);
    let cache = dir.path().join(".archguard/cache.json");
    for field in ["version", "graph_version", "endpoint", "parent", "count"] {
        let mut entry: serde_json::Value =
            serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
        match field {
            "version" => entry["schema_version"] = 999.into(),
            "graph_version" => entry["graph"]["schema_version"] = 999.into(),
            "endpoint" => entry["graph"]["edges"][0]["to"] = 999.into(),
            "parent" => entry["graph"]["nodes"][0]["parent"] = 0.into(),
            _ => entry["graph"]["edges"][0]["count"] = serde_json::Value::Null,
        }
        fs::write(&cache, serde_json::to_vec(&entry).unwrap()).unwrap();
        assert!(!run().cache_hit, "reject {field}");
        assert!(run().cache_hit);
    }
}
