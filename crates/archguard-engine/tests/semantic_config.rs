use archguard_engine::{core::ConfigSource, load_config};

#[test]
fn semantic_project_options_roundtrip_and_keep_provenance() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("archguard.toml"),
        r#"
[semantic.java]
release = 21
classpath = ["lib/api.jar", "target/classes"]
[semantic.csharp]
project = "src/App.csproj"
configuration = "Release"
target_framework = "net8.0"
[semantic.typescript]
project = "apps/web/tsconfig.json"
compiler = "node_modules/typescript/lib/typescript.js"
"#,
    )
    .unwrap();
    let loaded = load_config(None, root.path()).unwrap();
    let value = serde_json::to_value(&loaded.config).unwrap();
    assert_eq!(value["semantic"]["java"]["release"], 21);
    assert_eq!(value["semantic"]["csharp"]["configuration"], "Release");
    assert_eq!(
        value["semantic"]["typescript"]["project"],
        "apps/web/tsconfig.json"
    );
    assert_eq!(loaded.sources["semantic"], ConfigSource::Declared);
    let encoded = toml::to_string(&loaded.config).unwrap();
    let decoded: archguard_engine::core::Config = toml::from_str(&encoded).unwrap();
    assert_eq!(loaded.config, decoded);
}

#[test]
fn invalid_semantic_options_are_rejected_before_starting_a_compiler() {
    for text in [
        "[semantic.java]\nrelease = 0",
        "[semantic.java]\nclasspath = ['']",
        "[semantic.csharp]\nconfiguration = ''",
        "[semantic.csharp]\nproject = ''",
        "[semantic.csharp]\ntarget_framework = ''",
        "[semantic.typescript]\nproject = ''",
        "[semantic.typescript]\ncompiler = ''",
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("archguard.toml"), text).unwrap();
        assert!(load_config(None, root.path()).is_err(), "accepted {text}");
    }
}
