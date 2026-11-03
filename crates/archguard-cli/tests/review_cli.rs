use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn hotspots_exposes_ranked_evidence_without_claiming_missing_git_is_zero() {
    let root = tempdir().unwrap();
    for (path, source) in [
        ("a.ts", "import './b';\n"),
        ("b.ts", "import './a';\n"),
        ("unused.ts", "export const unused = 1;\n"),
    ] {
        fs::write(root.path().join(path), source).unwrap();
    }
    fs::write(
        root.path().join("archguard.toml"),
        "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["hotspots", "--format", "json"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(value["nodes"][0]["name"], "a.ts");
    assert!(value["nodes"][0]["score"].as_u64().unwrap() > 0);
    assert!(value["nodes"][0]["changes"].is_null());
    assert_eq!(value["nodes"][2]["name"], "unused.ts");
    assert_eq!(value["nodes"][2]["orphan"], "isolated");
}

#[test]
fn html_contains_source_review_evidence_and_configured_entry_points() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("entry.ts"), "export const entry = 1;\n").unwrap();
    fs::write(root.path().join("archguard.toml"),"derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[review]\nentry_points=['entry.ts']\ngit_since='180.days'\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["graph", "--format", "html"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let page = String::from_utf8(output.stdout).unwrap();
    let payload = page
        .split("<script id=\"archguard-data\" type=\"application/json\">")
        .nth(1)
        .unwrap()
        .split("</script>")
        .next()
        .unwrap();
    let value: Value = serde_json::from_str(payload).unwrap();
    assert_eq!(
        value["reviewContext"]["entry_point_ids"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        value["reviewContext"]["sources"]["files"][0]["path"],
        "entry.ts"
    );
    assert_eq!(value["reviewContext"]["sources"]["files"][0]["lines"], 1);
}

#[test]
fn expanded_html_preserves_full_sources_and_initial_cut_even_with_focus() {
    let root = tempdir().unwrap();
    for (path, source) in [
        ("app/main.ts", "import '../lib/service';\n"),
        ("lib/service.ts", "export const service = 1;\n"),
        ("other/unused.ts", "export const unused = 1;\n"),
    ] {
        let file = root.path().join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, source).unwrap();
    }
    fs::write(
        root.path().join("archguard.toml"),
        "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n",
    )
    .unwrap();
    for focused in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_archguard"));
        command.args(["graph", "--format", "html", "--expand", "app"]);
        if focused {
            command.args(["--focus", "app/main.ts", "--depth", "1"]);
        }
        let output = command.arg(root.path()).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let page = String::from_utf8(output.stdout).unwrap();
        let payload = page
            .split("<script id=\"archguard-data\" type=\"application/json\">")
            .nth(1)
            .unwrap()
            .split("</script>")
            .next()
            .unwrap();
        let value: Value = serde_json::from_str(payload).unwrap();
        let nodes = value["nodes"].as_array().unwrap();
        assert_eq!(
            nodes.len(),
            6,
            "full hierarchy is needed for review and further navigation"
        );
        assert_eq!(value["reviewContext"]["sources"]["mapped_files"], 3);
        let sources = value["reviewContext"]["sources"]["files"]
            .as_array()
            .unwrap();
        assert!(
            sources
                .iter()
                .any(|source| source["path"] == "lib/service.ts" && source["lines"] == 1)
        );
        let ids = value["view"]["visible"].as_array().unwrap();
        let names: Vec<_> = nodes
            .iter()
            .filter(|node| ids.contains(&node["id"]))
            .map(|node| node["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            if focused {
                vec!["app/main.ts", "lib"]
            } else {
                vec!["app/main.ts", "lib", "other"]
            }
        );
    }
}
