//! Jump to the running application and path replay (ADR 0021, decision 7).
use std::collections::BTreeMap;

use archguard_core::{
    Assignment, Condition, Config, Origin, Screen, ScreenKind, Target, Transition, UiFlow,
};
use archguard_engine::ui_flow_replay::{find_screens, path_to, playwright_script, url_for};

const CONFIG: &str = r#"
[views.run.Web]
base_url = "https://localhost:7298/"
login_path = "/login"
[views.run.Web.users.Admin]
login = "admin@example.org"
password_env = "ARCHGUARD_PW_ADMIN"
[views.run.Web.users.Clerk]
login = "clerk@example.org"
password_keychain = "archguard-web-clerk"
"#;

fn config() -> Config {
    toml::from_str(CONFIG).unwrap()
}

fn screen(id: &str, route: &str, access: Option<Condition>) -> Screen {
    Screen {
        weak: false,
        id: id.into(),
        name: id.into(),
        kind: ScreenKind::Page,
        route: Some(route.into()),
        application: Some("Web".into()),
        file: format!("Web/{id}.razor"),
        line: 1,
        access: archguard_core::AccessState::from_parts(access, false),
        source: "test".into(),
    }
}

fn link(from: Origin, to: &str, trigger: Option<&str>, condition: Option<Condition>) -> Transition {
    Transition {
        weak: false,
        from,
        target: Target::Screen(to.into()),
        trigger: trigger.map(Into::into),
        file: "Web/x.razor".into(),
        line: 3,
        condition: archguard_core::AccessState::from_parts(condition, false),
        source: "test".into(),
    }
}

fn on(id: &str) -> Origin {
    Origin::Screen(id.into())
}

/// Home → Orders → Detail, a long way Home → Admin → Detail guarded by a
/// role, a menu link to Help, and Lost without any link.
fn flow() -> UiFlow {
    let admin = Some(Condition::Role("Admin".into()));
    UiFlow {
        screens: vec![
            screen("Home", "/", None),
            screen("Orders", "/orders", Some(Condition::Authenticated)),
            screen("Detail", "/orders/{id:int}", None),
            screen("Admin", "/admin", admin.clone()),
            screen("Help", "/help", None),
            screen("Lost", "/lost", None),
        ],
        transitions: vec![
            link(on("Home"), "Admin", Some("admin-link"), None),
            link(on("Admin"), "Detail", Some("Open"), admin),
            link(on("Home"), "Orders", Some("orders-link"), None),
            link(on("Orders"), "Detail", None, None),
            link(
                Origin::AnyScreen {
                    application: "Web".into(),
                },
                "Help",
                Some("Help"),
                None,
            ),
            Transition {
                target: Target::Unresolved("@Url()".into()),
                ..link(on("Home"), "x", None, None)
            },
        ],
        ..UiFlow::default()
    }
}

fn signed_in(roles: &[&str]) -> Assignment {
    Assignment {
        authenticated: true,
        roles: roles.iter().map(|r| r.to_string()).collect(),
        ..Assignment::default()
    }
}

fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn run_config_parses_users_with_env_or_keychain_password() {
    let run = &config().views.run["Web"];
    assert_eq!(run.base_url.as_deref(), Some("https://localhost:7298/"));
    assert_eq!(
        run.users["Admin"].password_env.as_deref(),
        Some("ARCHGUARD_PW_ADMIN")
    );
    assert_eq!(
        run.users["Clerk"].password_keychain.as_deref(),
        Some("archguard-web-clerk")
    );
}

#[test]
fn a_literal_password_is_a_config_error() {
    let text = "[views.run.Web.users.Admin]\nlogin = \"a\"\npassword = \"secret\"\n";
    let error = toml::from_str::<Config>(text).unwrap_err().to_string();
    assert!(
        error.contains("a password must not be written into the configuration"),
        "{error}"
    );
    assert!(!error.contains("secret\""), "{error}");
    let neither = "[views.run.Web.users.Admin]\nlogin = \"a\"\n";
    let error = toml::from_str::<Config>(neither).unwrap_err().to_string();
    assert!(error.contains("exactly one of password_env"), "{error}");
}

#[test]
fn a_literal_password_fails_load_config() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("archguard.toml");
    std::fs::write(
        &file,
        "[views.run.Web.users.Admin]\nlogin = \"a\"\npassword = \"x\"\n",
    )
    .unwrap();
    let error = archguard_engine::load_config(Some(&file), dir.path()).unwrap_err();
    assert!(error.to_string().contains("password must not be written"));
}

#[test]
fn url_for_joins_base_url_and_route_with_parameters() {
    let (flow, config) = (flow(), config());
    assert_eq!(
        url_for(&flow, "Orders", &config, &params(&[])).unwrap(),
        "https://localhost:7298/orders"
    );
    assert_eq!(
        url_for(&flow, "Detail", &config, &params(&[("id", "4711")])).unwrap(),
        "https://localhost:7298/orders/4711"
    );
    let missing = url_for(&flow, "Detail", &config, &params(&[])).unwrap_err();
    assert!(missing.contains("--param id="), "{missing}");
    let none = url_for(&flow, "Orders", &Config::default(), &params(&[])).unwrap_err();
    assert!(none.contains("[views.run.Web] base_url"), "{none}");
    let unknown = url_for(&flow, "Nope", &config, &params(&[])).unwrap_err();
    assert!(unknown.contains("Nope"), "{unknown}");
}

#[test]
fn path_to_takes_the_shortest_allowed_path() {
    let flow = flow();
    let ids = |path: Vec<&Transition>| -> Vec<String> {
        path.iter()
            .map(|t| match &t.target {
                Target::Screen(id) => id.clone(),
                Target::Unresolved(text) => text.clone(),
            })
            .collect()
    };
    // Signed in without the Admin role: the guarded way is denied.
    let clerk = path_to(&flow, "Detail", &signed_in(&["Clerk"])).unwrap();
    assert_eq!(ids(clerk), ["Orders", "Detail"]);
    // Anonymous: Orders is denied, the Admin way too.
    assert!(path_to(&flow, "Detail", &Assignment::default()).is_none());
    assert!(path_to(&flow, "Lost", &signed_in(&["Admin"])).is_none());
    assert!(path_to(&flow, "Orders", &Assignment::default()).is_none());
    assert_eq!(
        ids(path_to(&flow, "Help", &Assignment::default()).unwrap()),
        ["Help"]
    );
    assert!(
        path_to(&flow, "Home", &Assignment::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn find_screens_matches_id_name_or_route() {
    let flow = flow();
    let ids = |q| -> Vec<&str> {
        find_screens(&flow, q)
            .iter()
            .map(|s| s.id.as_str())
            .collect()
    };
    assert_eq!(ids("Orders"), ["Orders"]);
    assert_eq!(ids("/orders/{id}"), ["Detail"]);
    assert_eq!(ids("/ORDERS"), ["Orders"]);
    assert!(ids("nope").is_empty());
}

#[test]
fn playwright_script_signs_in_from_env_and_clicks_by_test_id() {
    let (flow, config) = (flow(), config());
    // The Orders way; the role only picks the test user.
    let path = path_to(&flow, "Detail", &signed_in(&["Clerk"])).unwrap();
    let script = playwright_script(&flow, "Detail", &path, &config, Some("Admin")).unwrap();
    for line in [
        "import { test, expect, type Page } from '@playwright/test';",
        "const BASE = \"https://localhost:7298\";",
        "process.env[\"ARCHGUARD_PW_ADMIN\"]",
        "await signIn(page, \"admin@example.org\", password, \"/login\");",
        "await clickTrigger(page, \"orders-link\");",
        "page.getByTestId(trigger)",
        "await expect(page).toHaveURL(new RegExp(\"^\" + escape(BASE) + \"/orders(?:[?#].*)?$\", 'i'));",
        "// no trigger read: direct navigation",
        "/orders/[^/?#]+(?:[?#].*)?$",
    ] {
        assert!(script.contains(line), "{line} in:\n{script}");
    }
    // The direct step to a parameterized route cannot guess the value.
    assert!(script.contains("throw new Error("), "{script}");
    assert!(!script.contains("secret"));
}

#[test]
fn playwright_script_reads_keychain_passwords_from_an_env_var_the_cli_fills() {
    let (flow, config) = (flow(), config());
    let path = path_to(&flow, "Help", &signed_in(&["Clerk"])).unwrap();
    let script = playwright_script(&flow, "Help", &path, &config, Some("Clerk")).unwrap();
    assert!(
        script.contains("process.env[\"ARCHGUARD_REPLAY_PASSWORD\"]"),
        "{script}"
    );
    assert!(script.contains("await clickTrigger(page, \"Help\");"));
    let anonymous = playwright_script(&flow, "Help", &path, &config, None).unwrap();
    assert!(!anonymous.contains("await signIn("), "{anonymous}");
    let error = playwright_script(&flow, "Help", &path, &config, Some("Guest")).unwrap_err();
    assert!(error.contains("[views.run.Web.users.Guest]"), "{error}");
}

/// A desktop/MAUI app has no `/` route; its start comes from `start_screens`.
#[test]
fn path_to_starts_at_a_marked_start_screen_without_a_root_route() {
    let mut login = screen("Login", "//Login", None);
    login.route = None;
    let flow = UiFlow {
        screens: vec![login, screen("Basket", "Basket", None)],
        transitions: vec![link(on("Login"), "Basket", Some("Login"), None)],
        start_screens: vec!["Login".into()],
        ..UiFlow::default()
    };
    let path =
        path_to(&flow, "Basket", &Assignment::default()).expect("path from the marked start");
    assert_eq!(path.len(), 1);
}
