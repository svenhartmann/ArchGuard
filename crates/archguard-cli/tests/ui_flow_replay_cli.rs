//! `archguard ui-flow open|replay` (ADR 0021, decision 7). Nothing here runs
//! a browser or Playwright.
use std::{fs, path::Path, process::Command};

const FILES: &[(&str, &str)] = &[
    ("Web/Web.csproj", "<Project />"),
    (
        "Web/Pages/Home.razor",
        "@page \"/\"\n<a href=\"orders\">Orders</a>\n",
    ),
    (
        "Web/Pages/Orders.razor",
        "@page \"/orders\"\n@attribute [Authorize]\n<a href=\"@ItemUrl()\">Item</a>\n",
    ),
    ("Web/Pages/Item.razor", "@page \"/item/{id:int}\"\n"),
    ("Shop/Shop.csproj", "<Project />"),
    ("Shop/Pages/Home.razor", "@page \"/\"\n"),
];

const CONFIG: &str = r#"
[views.run.Web]
base_url = "https://localhost:7298"
[views.run.Web.users.Clerk]
login = "clerk@example.org"
password_env = "ARCHGUARD_PW_CLERK"
"#;

/// The project and, outside it, the configuration.
fn fixture() -> (tempfile::TempDir, tempfile::TempDir) {
    let project = tempfile::tempdir().unwrap();
    for (path, text) in FILES {
        let path = project.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("archguard.toml"), CONFIG).unwrap();
    (project, outside)
}

fn archguard(project: &Path, outside: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .arg("--config")
        .arg(outside.join("archguard.toml"))
        .args(["ui-flow"])
        .args(&args[..2])
        .arg(project)
        .args(&args[2..])
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn open_print_shows_the_local_url_of_a_screen() {
    let (project, outside) = fixture();
    let run = |args: &[&str]| archguard(project.path(), outside.path(), args);
    assert_eq!(
        run(&["open", "Orders", "--print"]),
        (0, "https://localhost:7298/orders\n".into(), String::new())
    );
    let (code, out, _) = run(&["open", "/item/{id}", "--print", "--param", "id=4711"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "https://localhost:7298/item/4711\n")
    );
    let (code, _, err) = run(&["open", "Item", "--print"]);
    assert_eq!(code, 2);
    assert!(err.contains("--param id="), "{err}");
    // The Shop application has no base URL.
    let (code, _, err) = run(&["open", "Shop:Pages/Home.razor", "--print"]);
    assert!(
        code == 2 && err.contains("[views.run.Shop] base_url"),
        "{err}"
    );
}

#[test]
fn an_ambiguous_screen_lists_the_candidates() {
    let (project, outside) = fixture();
    let (code, out, err) = archguard(project.path(), outside.path(), &["open", "Home", "--print"]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(err.contains("Home names several screens"), "{err}");
    assert!(
        err.contains("Web/Pages/Home.razor") && err.contains("Shop/Pages/Home.razor"),
        "{err}"
    );
}

#[test]
fn replay_out_writes_a_playwright_script_outside_the_project() {
    let (project, outside) = fixture();
    let file = outside.path().join("orders.spec.ts");
    let file_arg = file.to_str().unwrap();
    let (code, out, err) = archguard(
        project.path(),
        outside.path(),
        &["replay", "Orders", "--role", "Clerk", "--out", file_arg],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.starts_with(&format!(
            "Wrote {file_arg} · 1 steps to Orders for role Clerk"
        )),
        "{out}"
    );
    let script = fs::read_to_string(&file).unwrap();
    for line in [
        "const BASE = \"https://localhost:7298\";",
        "process.env[\"ARCHGUARD_PW_CLERK\"]",
        "await clickTrigger(page, \"Orders\");",
        "/orders(?:[?#].*)?$",
    ] {
        assert!(script.contains(line), "{line} in:\n{script}");
    }
    // Anonymous: Orders needs a signed-in user, so there is no path.
    let (code, _, err) = archguard(
        project.path(),
        outside.path(),
        &["replay", "Orders", "--anonymous", "--out", file_arg],
    );
    assert!(code == 1 && err.contains("no known path to"), "{err}");
    let before: Vec<_> = fs::read_dir(project.path()).unwrap().collect();
    assert_eq!(before.len(), 2, "nothing written into the project");
}
