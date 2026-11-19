//! `archguard ui-flow open|replay` (ADR 0021, decision 7): open a screen of
//! the locally running application by URL, or write (and on request run) a
//! Playwright test that replays the view's click path to it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use archguard_engine::core::{Assignment, Config, Screen, UiFlow};
use archguard_engine::ui_flow_replay::{
    KEYCHAIN_PASSWORD_ENV, find_screens, path_to, playwright_script, url_for,
};
use clap::Subcommand;
use serde_json::Value;

use crate::output::{EXIT_FOUND, EXIT_OK, EXIT_USAGE, analysis_error, usage};

#[derive(Subcommand)]
pub(crate) enum UiFlowAction {
    /// Print a screen's local URL ([views.run.<app>] base_url + route) and open it.
    Open {
        /// Screen id, name or route.
        screen: String,
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Route parameter value (repeatable), e.g. `--param id=4711`.
        #[arg(long = "param", value_name = "KEY=VALUE")]
        params: Vec<String>,
        /// Only print the URL, do not open a browser.
        #[arg(long)]
        print: bool,
    },
    /// Write a Playwright test that replays the shortest click path to a screen.
    Replay {
        /// Screen id, name or route.
        screen: String,
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Sign in as this role's test user from [views.run.<app>.users.<role>].
        #[arg(long, value_name = "ROLE", conflicts_with = "anonymous")]
        role: Option<String>,
        /// Replay without signing in (the default without --role).
        #[arg(long)]
        anonymous: bool,
        /// Script file; default: a file in the system temp directory.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Run the script with `npx playwright test` and report every step.
        #[arg(long)]
        run: bool,
    },
}

impl UiFlowAction {
    pub(crate) fn path(&self) -> Option<&Path> {
        match self {
            Self::Open { path, .. } | Self::Replay { path, .. } => path.as_deref(),
        }
    }
}

pub(crate) fn run(config_file: Option<&Path>, root: &Path, action: &UiFlowAction) -> i32 {
    let config = match archguard_engine::load_config(config_file, root) {
        Ok(loaded) => loaded.config,
        Err(error) => return analysis_error(error),
    };
    let flow = match archguard_engine::ui_flow::analyze(root) {
        Ok(flow) => flow,
        Err(error) => return analysis_error(error),
    };
    let (UiFlowAction::Open { screen, .. } | UiFlowAction::Replay { screen, .. }) = action;
    let screen = match pick(&flow, screen) {
        Ok(screen) => screen,
        Err(code) => return code,
    };
    match action {
        UiFlowAction::Open { params, print, .. } => open(&flow, screen, &config, params, *print),
        UiFlowAction::Replay { role, out, run, .. } => {
            let file = out.clone().unwrap_or_else(|| default_file(screen));
            replay(&flow, screen, &config, role.as_deref(), &file, *run)
        }
    }
}

/// The one screen `query` names; none or several are a usage error.
fn pick<'a>(flow: &'a UiFlow, query: &str) -> Result<&'a Screen, i32> {
    match find_screens(flow, query)[..] {
        [screen] => Ok(screen),
        [] => Err(usage(format!("no screen with id, name or route {query}"))),
        ref several => {
            eprintln!("archguard: usage error: {query} names several screens:");
            for s in several {
                let route = s.route.as_deref().unwrap_or("-");
                eprintln!("  {} · {route} · {}:{}", s.id, s.file, s.line);
            }
            Err(EXIT_USAGE)
        }
    }
}

fn open(flow: &UiFlow, screen: &Screen, config: &Config, params: &[String], print: bool) -> i32 {
    let mut values = BTreeMap::new();
    for param in params {
        let Some((key, value)) = param.split_once('=') else {
            return usage(format!("--param {param}: expected KEY=VALUE"));
        };
        values.insert(key.to_string(), value.to_string());
    }
    let url = match url_for(flow, &screen.id, config, &values) {
        Ok(url) => url,
        Err(why) => return usage(why),
    };
    outln!("{url}");
    if print {
        return EXIT_OK;
    }
    // shortcut: `start` via cmd splits URLs at `&`; quote when Windows users hit it.
    let opener = if cfg!(target_os = "macos") {
        Process::new("open").arg(&url).status()
    } else if cfg!(windows) {
        Process::new("cmd")
            .args(["/C", "start", ""])
            .arg(&url)
            .status()
    } else {
        Process::new("xdg-open").arg(&url).status()
    };
    match opener {
        Ok(status) if status.success() => EXIT_OK,
        Ok(status) => analysis_error(format!("the browser opener ended with {status}")),
        Err(error) => analysis_error(format!("cannot start the browser opener: {error}")),
    }
}

/// `<temp>/archguard-replay/<screen name>.spec.ts`: never inside the project.
fn default_file(screen: &Screen) -> PathBuf {
    let name: String = (screen.name.chars())
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    std::env::temp_dir()
        .join("archguard-replay")
        .join(format!("{name}.spec.ts"))
}

fn replay(
    flow: &UiFlow,
    screen: &Screen,
    config: &Config,
    role: Option<&str>,
    file: &Path,
    run: bool,
) -> i32 {
    let assignment = Assignment {
        authenticated: role.is_some(),
        roles: role.into_iter().map(String::from).collect(),
        ..Assignment::default()
    };
    let who = role.map_or("anonymous".into(), |r| format!("role {r}"));
    let Some(path) = path_to(flow, &screen.id, &assignment) else {
        eprintln!(
            "archguard: no known path to {} for {who}; `archguard ui-flow` with --role or --anonymous says why",
            screen.id
        );
        return EXIT_FOUND;
    };
    let script = match playwright_script(flow, &screen.id, &path, config, role) {
        Ok(script) => script,
        Err(why) => return usage(why),
    };
    let written = (file.parent().map_or(Ok(()), std::fs::create_dir_all))
        .and_then(|()| std::fs::write(file, script));
    if let Err(error) = written {
        return analysis_error(format!("{}: {error}", file.display()));
    }
    let direct = path.iter().filter(|t| t.trigger.is_none()).count();
    outln!(
        "Wrote {} · {} steps to {} for {who} · {direct} without trigger (direct navigation, weaker evidence)",
        file.display(),
        path.len(),
        screen.name
    );
    if !run {
        return EXIT_OK;
    }
    let keychain = role
        .and_then(|r| {
            config
                .views
                .run
                .get(screen.application.as_deref().unwrap_or(""))?
                .users
                .get(r)
        })
        .and_then(|user| user.password_keychain.as_deref());
    run_playwright(file, keychain)
}

/// The password stored under `service` in the macOS keychain.
fn keychain_password(service: &str) -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("password_keychain is read on macOS only; use password_env".into());
    }
    let output = Process::new("security")
        .args(["find-generic-password", "-s", service, "-w"])
        .output()
        .map_err(|error| format!("cannot run security: {error}"))?;
    if !output.status.success() {
        return Err(format!("no keychain entry for service {service}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end_matches('\n')
        .to_string())
}

/// Runs the script with Playwright's JSON reporter and prints every step;
/// a failed step is a finding: view and application disagree.
fn run_playwright(file: &Path, keychain: Option<&str>) -> i32 {
    let npx_found = Process::new("npx").arg("--version").output();
    if !npx_found.is_ok_and(|o| o.status.success()) {
        return analysis_error("--run needs npx (Node.js) on PATH; the script is written");
    }
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
        return analysis_error(format!("{}: not a file path", file.display()));
    };
    let mut command = Process::new("npx");
    command
        .args(["playwright", "test", "--reporter=json"])
        .arg(name)
        .current_dir(dir);
    if let Some(service) = keychain {
        match keychain_password(service) {
            Ok(password) => command.env(KEYCHAIN_PASSWORD_ENV, password),
            Err(why) => return analysis_error(why),
        };
    }
    let output = match command.output() {
        Ok(output) => output,
        Err(error) => return analysis_error(format!("cannot run npx playwright: {error}")),
    };
    let Ok(report) = serde_json::from_slice::<Value>(&output.stdout) else {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        return analysis_error("playwright wrote no JSON report");
    };
    let failed = print_steps(&report);
    if failed || !output.status.success() {
        EXIT_FOUND
    } else {
        EXIT_OK
    }
}

/// Every result's `test.step`s from Playwright's JSON report; true when one failed.
fn print_steps(report: &Value) -> bool {
    let mut failed = false;
    let mut suites: Vec<&Value> = report["suites"].as_array().into_iter().flatten().collect();
    while let Some(suite) = suites.pop() {
        suites.extend(suite["suites"].as_array().into_iter().flatten());
        let specs = suite["specs"].as_array().into_iter().flatten();
        let tests = specs.flat_map(|s| s["tests"].as_array().into_iter().flatten());
        for result in tests.flat_map(|t| t["results"].as_array().into_iter().flatten()) {
            failed |= print_result(result);
        }
    }
    failed
}

/// One test result's steps; true when it did not pass.
fn print_result(result: &Value) -> bool {
    let first_line = |v: &Value| {
        v.as_str()
            .and_then(|m| m.lines().next())
            .unwrap_or("")
            .to_string()
    };
    let mut step_failed = false;
    for step in result["steps"].as_array().into_iter().flatten() {
        let title = step["title"].as_str().unwrap_or("?");
        if step["error"].is_null() {
            outln!("passed: {title}");
            continue;
        }
        step_failed = true;
        outln!("failed: {title}");
        let why = first_line(&step["error"]["message"]);
        outln!("finding: view and application disagree at {title}: {why}");
    }
    let passed = result["status"] == "passed";
    if !step_failed && !passed {
        outln!(
            "failed before the replay: {}",
            first_line(&result["error"]["message"])
        );
    }
    step_failed || !passed
}
