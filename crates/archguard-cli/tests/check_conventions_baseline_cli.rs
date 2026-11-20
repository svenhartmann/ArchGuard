//! `[views] conventions` with a baseline: `baseline write` records the
//! current convention findings, `check --baseline` fails only on new ones
//! and notes baselined ones that no longer occur.
use std::{fs, path::Path, process::Command};

const CONFIG: &str = "[views]\nconventions = [\"navigation_literal\"]\n";
const HOME: &str =
    "@page \"/\"\n<a href=\"@ItemUrl()\">Item</a>\n<a href=\"@CartUrl()\">Cart</a>\n";

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn archguard(dir: &Path, args: &[&str]) -> (Option<i32>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code(), text)
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".archguard/archguard.toml", CONFIG);
    write(dir.path(), "Web/Web.csproj", "<Project />");
    write(dir.path(), "Web/Program.cs", "class Program { }\n");
    write(dir.path(), "Web/Pages/Home.razor", HOME);
    dir
}

#[test]
fn only_new_convention_findings_fail_and_fixed_ones_are_noted() {
    let dir = project();
    let (code, text) = archguard(dir.path(), &["check"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("2 convention violations"), "{text}");

    let (code, text) = archguard(dir.path(), &["baseline", "write"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("2 convention ids"), "{text}");
    let (code, text) = archguard(dir.path(), &["check", "--baseline"]);
    assert_eq!(code, Some(0), "{text}");

    let more = format!("{HOME}<a href=\"@OrderUrl()\">Order</a>\n");
    write(dir.path(), "Web/Pages/Home.razor", &more);
    let (code, text) = archguard(dir.path(), &["check", "--baseline"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("1 convention violation "), "{text}");
    assert!(text.contains("OrderUrl"), "{text}");
    assert!(
        !text.contains("ItemUrl"),
        "accepted ones stay quiet: {text}"
    );

    let (code, sarif) = archguard(dir.path(), &["check", "--baseline", "--format", "sarif"]);
    assert_eq!(code, Some(1), "{sarif}");
    assert!(
        sarif.contains("\"archguard/convention/navigation_literal\""),
        "{sarif}"
    );

    let fewer = HOME.replace("<a href=\"@CartUrl()\">Cart</a>\n", "");
    write(dir.path(), "Web/Pages/Home.razor", &fewer);
    let (code, text) = archguard(dir.path(), &["check", "--baseline"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("CartUrl"), "{text}");
    assert!(text.contains("remove it from the baseline"), "{text}");
    assert!(
        !text.contains("convention violation"),
        "a note is no violation: {text}"
    );
}

/// The ids and the parsed JSON of `check --format json`.
fn check_json(dir: &Path) -> (serde_json::Value, Vec<String>) {
    let (_, json) = archguard(dir, &["check", "--format", "json"]);
    let value: serde_json::Value = serde_json::from_str(&json).expect(&json);
    let ids = (value["findings"].as_array().unwrap().iter())
        .map(|f| f["id"].as_str().unwrap().to_string())
        .collect();
    (value, ids)
}

#[test]
fn notes_show_under_findings_and_intentional_ones_do_not_fail() {
    let dir = project();
    let (_, ids) = check_json(dir.path());
    let id = |part: &str| ids.iter().find(|i| i.contains(part)).expect(part).clone();
    let (item, cart) = (id("ItemUrl"), id("CartUrl"));
    let gap = "view-gap:Web/Pages/Home.razor:role not readable";
    let record = |finding: &str, decision: &str, note: &str| {
        let args = [
            "review-decision",
            "record",
            "--finding",
            finding,
            "--decision",
            decision,
            "--note",
            note,
            "--author",
            "sven",
        ];
        let (code, text) = archguard(dir.path(), &args);
        assert_eq!(code, Some(0), "{text}");
    };
    record(&item, "accept-risk", "helper URL is fine here");
    let (code, text) = archguard(dir.path(), &["check"]);
    assert_eq!(code, Some(1), "the other one still fails: {text}");
    assert!(
        text.contains("  note (intentional, by sven): helper URL is fine here"),
        "{text}"
    );
    assert!(text.contains("  accepted: intentional note"), "{text}");

    record(&cart, "fix", "use the route constant");
    record(gap, "defer", "later");
    let (code, text) = archguard(dir.path(), &["check"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(
        text.contains("  note (fix, by sven): use the route constant"),
        "{text}"
    );
    assert!(text.contains("Noted gaps:"), "{text}");
    let gap_line = format!("[{gap}] note (later, by sven): later");
    assert!(text.contains(&gap_line), "{text}");

    let (value, _) = check_json(dir.path());
    let finding = |id: &str| {
        (value["findings"].as_array().unwrap().iter())
            .find(|f| f["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} in {value}"))
    };
    assert_eq!(finding(&item)["note"]["decision"], "intentional");
    assert_eq!(finding(&item)["accepted_by_note"], true);
    assert_eq!(finding(&cart)["note"]["text"], "use the route constant");
    assert_eq!(finding(&cart)["note"]["author"], "sven");
    assert!(finding(&cart)["note"]["ts"].is_string());
    assert_eq!(finding(&cart)["accepted_by_note"], false);
    assert_eq!(value["noted_gaps"][0]["id"], gap);
    assert_eq!(value["noted_gaps"][0]["note"]["decision"], "later");

    let (_, sarif) = archguard(dir.path(), &["check", "--format", "sarif"]);
    let value: serde_json::Value = serde_json::from_str(&sarif).expect(&sarif);
    let results = value["runs"][0]["results"].as_array().unwrap();
    let noted = |decision: &str| {
        (results.iter())
            .find(|r| r["properties"]["note"]["decision"] == decision)
            .unwrap_or_else(|| panic!("{decision} in {sarif}"))
    };
    assert_eq!(
        noted("intentional")["suppressions"],
        serde_json::json!([{"kind": "external", "justification": "helper URL is fine here"}])
    );
    assert!(noted("fix").get("suppressions").is_none(), "{sarif}");
    assert_eq!(noted("fix")["level"], "error");
    assert_eq!(value["runs"][0]["properties"]["noted_gaps"][0]["id"], gap);

    record(&cart, "accept-risk", "both are fine");
    let (code, text) = archguard(dir.path(), &["check"]);
    assert_eq!(code, Some(0), "all accepted: {text}");
    assert!(text.starts_with("PASS"), "{text}");
}
