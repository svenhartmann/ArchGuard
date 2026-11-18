use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn contracts(root: &Path, args: &[&str]) -> Value {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .arg("contracts")
        .arg(root)
        .args(["--format", "json"])
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn graphql_baseline_is_saved_explicitly_and_later_removals_are_breaking() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("compose.yaml"),
        "services:\n  api:\n    build: ./api\n  web:\n    build: ./web\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("api")).unwrap();
    fs::create_dir_all(root.join("web")).unwrap();
    let schema = root.join("api/schema.graphqls");
    fs::write(
        &schema,
        "type Query { user(id: ID!): User }\ntype User { id: ID! name: String }\n",
    )
    .unwrap();
    fs::write(
        root.join("web/get.graphql"),
        "query Get { user(id: 1) { name } }\n",
    )
    .unwrap();

    let first = contracts(root, &[]);
    assert_eq!(first["graphql"]["comparison_status"], "no_baseline");
    assert_eq!(first["graphql"]["links"][0]["operation"], "query Get");
    assert!(
        !root.join(".archguard/graphql-contracts.json").exists(),
        "reading never writes"
    );

    let saved = contracts(root, &["--graphql-baseline"]);
    assert_eq!(saved["graphql"]["comparison_status"], "compared");
    assert_eq!(saved["graphql"]["changes"], serde_json::json!([]));

    fs::write(
        &schema,
        "type Query { user: User }\ntype User { id: ID! }\n",
    )
    .unwrap();
    let changed = contracts(root, &[]);
    let kinds: Vec<(&str, &str)> = changed["graphql"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["kind"].as_str().unwrap(),
                c["operation"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(
        kinds.contains(&("argument_removed", "Query.user(id:)")),
        "{kinds:?}"
    );
    assert!(kinds.contains(&("field_removed", "User.name")), "{kinds:?}");
}
