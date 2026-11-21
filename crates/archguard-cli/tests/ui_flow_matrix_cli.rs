//! `archguard ui-flow matrix` (ADR 0021): the table, JSON and CSV of a fixture.
use std::{fs, process::Command};

fn run(args: &[&str]) -> (i32, String, String) {
    let project = tempfile::tempdir().unwrap();
    for (path, text) in [
        ("Web/Web.csproj", "<Project />"),
        ("Web/Pages/Home.razor", "@page \"/\"\n"),
        (
            "Web/Pages/Orders.razor",
            "@page \"/orders\"\n@attribute [Authorize]\n",
        ),
    ] {
        let path = project.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["ui-flow", "matrix"])
        .arg(project.path())
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn table_lists_screens_by_role() {
    let (code, out, _) = run(&["--kind", "screens"]);
    assert_eq!(code, 0);
    let rows: Vec<Vec<&str>> = (out.lines())
        .map(|l| l.split_whitespace().collect())
        .collect();
    assert_eq!(
        rows[0],
        ["kind", "row", "where", "anonymous", "signed", "in"]
    );
    let row = |route: &str| rows.iter().find(|r| r[1] == route).unwrap().clone();
    // a screen with no declared access is "undeclared", never allowed
    assert_eq!(row("/")[3..], ["undeclared", "undeclared"]);
    assert_eq!(row("/orders")[3..], ["denied", "allowed"]);
}

#[test]
fn json_and_csv_carry_the_same_cells() {
    let (_, json, _) = run(&["--format", "json", "--kind", "screens"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["roles"][0], "anonymous");
    let orders = (v["rows"].as_array().unwrap())
        .iter()
        .find(|r| r["label"] == "/orders")
        .unwrap();
    assert_eq!(orders["cells"], serde_json::json!(["denied", "allowed"]));
    let (_, csv, _) = run(&["--format", "csv", "--kind", "screens"]);
    assert_eq!(
        csv.lines().next().unwrap(),
        "kind,row,file,line,requires,anonymous,signed in"
    );
    assert!(
        csv.lines()
            .any(|l| l.starts_with("screen,/orders,") && l.ends_with(",denied,allowed"))
    );
}

#[test]
fn foreign_format_is_a_usage_error() {
    let (code, _, err) = run(&["--format", "sarif"]);
    assert_ne!(code, 0);
    assert!(err.contains("matrix"), "{err}");
}
