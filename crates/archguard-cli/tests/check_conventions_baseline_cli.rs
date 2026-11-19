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
