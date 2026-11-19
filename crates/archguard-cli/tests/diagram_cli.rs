//! `archguard diagram`: the arc42 diagram catalogue and its diagrams in
//! the terminal.

use std::path::Path;
use std::{fs, process::Command};

/// Python packages `app` (with `web` and `data`) and `lib`; `app/data`
/// and `lib` import each other, so the overview has a cycle; `view` calls
/// `b` calls `load` calls `helper`, so chapter 6 has a scenario.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let write = |path: &str, text: &str| {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    for init in ["app", "app/web", "app/data", "lib"] {
        write(&format!("{init}/__init__.py"), "");
    }
    write(
        "app/web/views.py",
        "from lib.b import b\n\ndef view():\n    return b()\n",
    );
    write(
        "app/data/repo.py",
        "from lib.util import helper\n\ndef load():\n    return helper()\n",
    );
    write("lib/util.py", "def helper():\n    return 1\n");
    write(
        "lib/b.py",
        "from app.data.repo import load\n\ndef b():\n    return load()\n",
    );
    dir
}

fn run(root: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .arg("diagram")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = String::from_utf8(output.stdout).unwrap();
    assert!(!out.contains('\x1b'), "no colour in a pipe: {out}");
    out
}

fn catalogue(root: &Path) -> Vec<serde_json::Value> {
    let json: serde_json::Value = serde_json::from_str(&run(root, &["--format", "json"])).unwrap();
    json.as_array().unwrap().clone()
}

#[test]
fn the_list_names_every_diagram_with_chapter_kind_and_where_it_is() {
    let dir = project();
    let list = run(dir.path(), &[]);
    let header = list.lines().next().unwrap();
    for column in ["ID", "CHAPTER", "KIND", "IN DOC", "IN TEXT", "TITLE"] {
        assert!(header.contains(column), "{list}");
    }
    let blocks = list
        .lines()
        .find(|l| l.starts_with("building-blocks "))
        .unwrap();
    let cells: Vec<&str> = blocks.split_whitespace().collect();
    assert_eq!(cells[1..5], ["5", "packages", "yes", "yes"], "{list}");
    let ids: Vec<String> = catalogue(dir.path())
        .iter()
        .map(|d| d["id"].as_str().unwrap().to_string())
        .collect();
    for id in &ids {
        assert!(
            list.lines().any(|l| l.starts_with(id.as_str())),
            "{id}: {list}"
        );
    }
}

#[test]
fn a_building_block_graph_is_drawn_with_its_cycle_marked() {
    let dir = project();
    let drawn = run(dir.path(), &["building-blocks", "--ascii"]);
    assert!(drawn.contains("| ! app/data"), "{drawn}");
    assert!(drawn.contains("| ! lib"), "{drawn}");
    assert!(drawn.contains('^') && drawn.contains("!1"), "{drawn}");
    assert!(drawn.is_ascii(), "{drawn}");
    let unicode = run(dir.path(), &["building-blocks"]);
    assert!(unicode.contains('┌') && unicode.contains('▲'), "{unicode}");
    let json: serde_json::Value =
        serde_json::from_str(&run(dir.path(), &["building-blocks", "--format", "json"])).unwrap();
    assert_eq!(json["id"], "building-blocks");
    assert!(
        json["relations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["cycle"] == true)
    );
}

#[test]
fn a_package_level_is_drawn_without_being_chosen() {
    let dir = project();
    let drawn = run(dir.path(), &["--level", "app", "--ascii"]);
    assert!(drawn.contains("| web"), "{drawn}");
    assert!(drawn.contains("| ! data"), "{drawn}");
    let json: serde_json::Value =
        serde_json::from_str(&run(dir.path(), &["--level", "app", "--format", "json"])).unwrap();
    assert_eq!(json["id"], "building-blocks@app");
    assert_eq!(json["chosen"], false);
    assert_eq!(json["shown"], false);
    let by_id = run(dir.path(), &["cycles@app", "--ascii"]);
    assert!(by_id.contains("cycles@app"), "{by_id}");
}

#[test]
fn a_runtime_scenario_is_drawn_as_a_sequence() {
    let dir = project();
    let scenario = catalogue(dir.path())
        .into_iter()
        .find(|d| d["kind"] == "sequence")
        .expect("a scenario in chapter 6");
    let id = scenario["id"].as_str().unwrap();
    let drawn = run(dir.path(), &[id, "--ascii"]);
    assert!(drawn.contains("1. "), "{drawn}");
    assert!(drawn.contains("->|"), "a call: {drawn}");
    assert!(drawn.contains("|<."), "a dashed return: {drawn}");
}

#[test]
fn an_unknown_id_is_a_usage_error_that_points_to_the_list() {
    let dir = project();
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["diagram"])
        .arg(dir.path())
        .arg("nope")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("archguard diagram"), "{err}");
    let level = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["diagram"])
        .arg(dir.path())
        .args(["--level", "nowhere"])
        .output()
        .unwrap();
    assert_eq!(level.status.code(), Some(2));
    let err = String::from_utf8_lossy(&level.stderr);
    assert!(err.contains("app"), "names the packages with levels: {err}");
}
