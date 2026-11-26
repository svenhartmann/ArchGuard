//! Index names through `local.*` and `jsondecode(file())`, chain depth 5, file trust boundary
//! (ticket 30, decision 2026-11-26). Fixtures under `fixtures/es-classdiagram/30`.
use archguard_core::{SearchArtifact, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::{Path, PathBuf};

fn fixture(dir: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/es-classdiagram/30")
        .join(dir)
}

fn analyze(root: &Path) -> ServiceTopology {
    let mut report = ServiceTopology::default();
    let discovery = Discovery::new(root, &mut report).expect("discovery");
    let read = |_: &Path, p: &Path| std::fs::read_to_string(p).ok();
    archguard_search::analyze(root, &discovery, &read, &mut report);
    report
}

fn indices(report: &ServiceTopology) -> Vec<&SearchArtifact> {
    let mut v: Vec<_> = report
        .elasticsearch
        .artifacts
        .iter()
        .filter(|a| a.kind == "index")
        .collect();
    v.sort_by_key(|a| a.address.clone());
    v
}

fn name_gaps(report: &ServiceTopology) -> usize {
    report
        .diagnostics
        .iter()
        .filter(|d| archguard_search::is_elasticsearch_gap(d) && d.message.contains("name is not"))
        .count()
}

/// Copy of the fixture directory in a temp dir, so tests can add a symlink or a big file.
fn copy(dir: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let from = fixture(dir);
    let mut stack = vec![from.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let to = tmp.path().join(e.path().strip_prefix(&from).unwrap());
            if e.path().is_dir() {
                std::fs::create_dir_all(&to).unwrap();
                stack.push(e.path());
            } else {
                std::fs::copy(e.path(), to).unwrap();
            }
        }
    }
    tmp
}

#[test]
fn names_and_mappings_resolve_through_locals_and_a_json_file() {
    let report = analyze(&fixture("terraform"));
    let found: Vec<_> = indices(&report);
    let names: Vec<_> = found.iter().map(|a| a.name.as_deref()).collect();
    assert_eq!(
        names,
        [Some("orders-2026-01-01"), Some("orders-2025-06-01")]
    );
    for a in &found {
        let fields: Vec<_> = a
            .detail
            .as_ref()
            .expect("detail")
            .fields
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert_eq!(
            fields,
            ["body", "body.keyword", "id", "title", "title.keyword"]
        );
    }
    let aliases: Vec<_> = report
        .elasticsearch
        .artifacts
        .iter()
        .filter(|a| a.kind == "alias" && a.name.as_deref() == Some("orders"))
        .flat_map(|a| a.references.iter().map(|r| r.name.as_str()))
        .collect();
    assert_eq!(aliases, ["orders-2026-01-01", "orders-2025-06-01"]);
    assert_eq!(name_gaps(&report), 0);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("mappings"))
    );
}

#[test]
fn a_file_outside_the_root_is_not_read() {
    let tmp = copy("outside_parent");
    let root = tmp.path().join("proj");
    #[cfg(unix)]
    {
        std::fs::write(tmp.path().join("secret.json"), r#"["leaked"]"#).unwrap();
        std::os::unix::fs::symlink("../secret.json", root.join("link.json")).unwrap();
    }
    let report = analyze(&root);
    assert!(indices(&report).iter().all(|a| a.name.is_none()));
    assert_eq!(name_gaps(&report), 2);
}

#[test]
fn a_big_or_broken_file_is_a_gap() {
    for (dir, file, text) in [
        ("big", "big.json", format!("[\"{}\"]", "x".repeat(1 << 20))),
        ("broken", "broken.json", r#"["orders","#.to_string()),
    ] {
        let tmp = copy(dir);
        std::fs::write(tmp.path().join(file), text).unwrap();
        let report = analyze(tmp.path());
        assert!(indices(&report)[0].name.is_none(), "{dir}");
        assert_eq!(name_gaps(&report), 1, "{dir}");
    }
}

#[test]
fn chains_of_depth_5_resolve_depth_6_and_cycles_are_gaps() {
    let report = analyze(&fixture("depth6"));
    let by = |n: &str| {
        indices(&report)
            .into_iter()
            .find(|a| a.address.ends_with(n))
            .unwrap()
            .name
            .clone()
    };
    assert_eq!(by(".five").as_deref(), Some("five"));
    assert_eq!(by(".six"), None);
    let report = analyze(&fixture("cycle"));
    assert_eq!(indices(&report)[0].name, None);
    assert_eq!(name_gaps(&report), 1);
}
