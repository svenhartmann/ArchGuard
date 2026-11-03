use archguard_engine::{
    core::{Config, GraphBuilder},
    knowledge_metrics,
};
use std::{fs, path::Path, process::Command};
fn git(root: &Path, args: &[&str], author: &str) {
    assert!(
        Command::new("git")
            .current_dir(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", author)
            .env("GIT_AUTHOR_EMAIL", format!("{author}@example.test"))
            .env("GIT_COMMITTER_NAME", author)
            .env("GIT_COMMITTER_EMAIL", format!("{author}@example.test"))
            .output()
            .unwrap()
            .status
            .success()
    );
}
#[test]
fn knowledge_concentration_counts_file_touches_and_handles_missing_history() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("pkg")).unwrap();
    fs::write(root.join("pkg/a.ts"), "export const a=1;").unwrap();
    let mut b = GraphBuilder::new();
    b.ensure_path("pkg/a.ts", Some("ts"));
    b.ensure_path("pkg/new.ts", Some("ts"));
    let graph = b.build(root.to_string_lossy().into_owned());
    let config = Config::default();
    assert_eq!(
        knowledge_metrics(root, &config, &graph).unwrap().status,
        "unavailable"
    );
    git(root, &["init", "-q"], "A");
    git(root, &["add", "pkg/a.ts"], "A");
    git(root, &["commit", "-qm", "one"], "A");
    for (i, author) in [(2, "A"), (3, "B")] {
        fs::write(root.join("pkg/a.ts"), format!("export const a={i};")).unwrap();
        git(root, &["add", "pkg/a.ts"], author);
        git(root, &["commit", "-qm", "next"], author);
    }
    fs::write(root.join("pkg/new.ts"), "export const x=1;").unwrap();
    let value = serde_json::to_value(knowledge_metrics(root, &config, &graph).unwrap()).unwrap();
    let file = &value["files"][0];
    assert_eq!(file["touches"], 3);
    assert_eq!(file["contributors"], 2);
    assert_eq!(file["bus_factor_50"], 1);
    assert!((file["top_share"].as_f64().unwrap() - 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(
        value["files"][1]["bus_factor_50"],
        serde_json::Value::Null,
        "uncommitted files have no ownership evidence"
    );
    assert_eq!(value["packages"][0]["bus_factor_50"], 1);
}
