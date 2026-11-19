//! Jump to the running application and path replay (ADR 0021 decision 7,
//! phase 5): the URL of a screen, the shortest resolved click path from a
//! start screen, and a Playwright test that replays it. Nothing here runs a
//! browser; the CLI decides whether to.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write;

use archguard_core::{
    Assignment, Condition, Config, Origin, Outcome, Screen, Target, Transition, UiFlow,
    evaluate_condition, normalize_path_pattern,
};

/// Environment variable through which the CLI hands a keychain password to
/// the generated script; the password never lands in the file.
pub const KEYCHAIN_PASSWORD_ENV: &str = "ARCHGUARD_REPLAY_PASSWORD";

/// Screens whose id, name (case-insensitive) or route shape equals `query`;
/// an exact id wins alone.
pub fn find_screens<'a>(flow: &'a UiFlow, query: &str) -> Vec<&'a Screen> {
    if let Some(exact) = flow.screens.iter().find(|s| s.id == query) {
        return vec![exact];
    }
    let shape = query
        .starts_with('/')
        .then(|| normalize_path_pattern(query));
    (flow.screens.iter())
        .filter(|s| {
            s.name.eq_ignore_ascii_case(query)
                || shape.is_some() && s.route.as_deref().map(normalize_path_pattern) == shape
        })
        .collect()
}

fn screen<'a>(flow: &'a UiFlow, id: &str) -> Result<&'a Screen, String> {
    (flow.screens.iter().find(|s| s.id == id)).ok_or_else(|| format!("no screen with id {id}"))
}

fn application(screen: &Screen) -> &str {
    screen.application.as_deref().unwrap_or("")
}

/// `[views.run.<application>] base_url` of the screen, without a trailing `/`.
fn base_url<'a>(config: &'a Config, screen: &Screen) -> Result<&'a str, String> {
    let app = application(screen);
    (config.views.run.get(app))
        .and_then(|run| run.base_url.as_deref())
        .map(|base| base.trim_end_matches('/'))
        .ok_or_else(|| format!("no base URL for application {app}: set [views.run.{app}] base_url"))
}

/// Name of a route parameter segment (`:id`, `{id}`, `{id:int}`, `{*rest}`, `{id?}`).
fn parameter(segment: &str) -> Option<&str> {
    let inner =
        (segment.strip_prefix(':')).or_else(|| segment.strip_prefix('{')?.strip_suffix('}'))?;
    inner.trim_start_matches('*').split([':', '?', '=']).next()
}

/// The route with every parameter filled from `params`; an optional or
/// catch-all parameter without a value is dropped.
fn fill_route(route: &str, params: &BTreeMap<String, String>) -> Result<String, String> {
    let mut out = String::new();
    for segment in route.split('/').filter(|s| !s.is_empty()) {
        let value = match parameter(segment) {
            None => segment,
            Some(name) => match params.get(name) {
                Some(value) => value,
                None if segment.ends_with("?}") || segment.starts_with("{*") => continue,
                None => {
                    return Err(format!(
                        "route {route} needs a value for {name}: pass --param {name}=…"
                    ));
                }
            },
        };
        // shortcut: values are not percent-encoded; upgrade when ids with `/`, `?` or `#` occur.
        write!(out, "/{value}").expect("write to string");
    }
    Ok(if out.is_empty() { "/".into() } else { out })
}

/// Base URL plus the screen's route with its parameters filled.
pub fn url_for(
    flow: &UiFlow,
    screen_id: &str,
    config: &Config,
    params: &BTreeMap<String, String>,
) -> Result<String, String> {
    let screen = screen(flow, screen_id)?;
    let route = (screen.route.as_deref())
        .ok_or_else(|| format!("screen {screen_id} has no route; only web screens open by URL"))?;
    Ok(format!(
        "{}{}",
        base_url(config, screen)?,
        fill_route(route, params)?
    ))
}

fn not_denied(condition: Option<&Condition>, assignment: &Assignment) -> bool {
    condition.is_none_or(|c| evaluate_condition(c, assignment) != Outcome::Denied)
}

/// A start screen: routed `/`, the same rule as `screen_reaches`.
fn is_start(screen: &Screen) -> bool {
    screen
        .route
        .as_deref()
        .map(normalize_path_pattern)
        .as_deref()
        == Some("/")
}

fn leaves(transition: &Transition, screen: &Screen) -> bool {
    match &transition.from {
        Origin::Screen(id) => *id == screen.id,
        Origin::AnyScreen { application: app } => app == application(screen),
    }
}

/// Shortest path of resolved transitions from a start screen to `screen_id`
/// (breadth first, in flow order), following only transitions whose own
/// condition and target access are not `denied`. Empty when the screen is a
/// start screen; none when it is unknown or not reached that way.
pub fn path_to<'a>(
    flow: &'a UiFlow,
    screen_id: &str,
    assignment: &Assignment,
) -> Option<Vec<&'a Transition>> {
    let open = |s: &Screen| not_denied(s.access.as_ref(), assignment);
    let mut via: BTreeMap<&str, Option<(&str, &Transition)>> = BTreeMap::new();
    let mut queue = VecDeque::new();
    for start in flow.screens.iter().filter(|s| is_start(s) && open(s)) {
        via.entry(&start.id).or_insert(None);
        queue.push_back(start);
    }
    while let Some(at) = queue.pop_front() {
        for t in flow.transitions.iter().filter(|t| leaves(t, at)) {
            let Target::Screen(to) = &t.target else {
                continue;
            };
            let Some(next) = flow.screens.iter().find(|s| s.id == *to) else {
                continue;
            };
            if via.contains_key(to.as_str())
                || !not_denied(t.condition.as_ref(), assignment)
                || !open(next)
            {
                continue;
            }
            via.insert(&next.id, Some((&at.id, t)));
            queue.push_back(next);
        }
    }
    let mut path = Vec::new();
    let mut at = screen_id;
    while let Some((from, transition)) = *via.get(at)? {
        path.push(transition);
        at = from;
    }
    path.reverse();
    Some(path)
}

/// JavaScript string literal.
fn js(text: &str) -> String {
    serde_json::to_string(text).expect("a string serializes")
}

/// Regex source of a route's path, anchored at the end, query and hash allowed.
fn route_regex(route: &str) -> String {
    let mut out = String::new();
    for segment in route.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        match parameter(segment) {
            Some(_) if segment.starts_with("{*") => out.push_str(".*"),
            Some(_) => out.push_str("[^/?#]+"),
            None => {
                for c in segment.chars() {
                    if r"\.^$*+?()[]{}|-".contains(c) {
                        out.push('\\');
                    }
                    out.push(c);
                }
            }
        }
    }
    if out.is_empty() {
        out.push_str("/?");
    }
    out + "(?:[?#].*)?$"
}

const PRELUDE: &str = r#"import { test, expect, type Page } from '@playwright/test';

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

// Click the element the view names: test id first, then aria-label, then text.
async function clickTrigger(page: Page, trigger: string) {
  const candidates = [
    page.getByTestId(trigger),
    page.getByLabel(trigger, { exact: true }),
    page.getByText(trigger, { exact: true }),
  ];
  await candidates[0].or(candidates[1]).or(candidates[2]).first().waitFor();
  for (const candidate of candidates) {
    if ((await candidate.count()) > 0) {
      await candidate.first().click();
      return;
    }
  }
}

// Fill the first password field and the user field before it, then submit.
async function signIn(page: Page, login: string, password: string, path: string) {
  await page.goto(BASE + path);
  const passwordField = page.locator('input[type=password]').first();
  await passwordField.waitFor();
  await page
    .locator('input[type=email], input[autocomplete=username], input[name*=user i], input[name*=login i], input[name*=email i]')
    .first()
    .fill(login);
  await passwordField.fill(password);
  await passwordField.press('Enter');
  await page.waitForLoadState();
}
"#;

/// The URL wait for `screen`, or a comment when it has no route.
fn url_check(out: &mut String, screen: &Screen) {
    match &screen.route {
        Some(route) => writeln!(
            out,
            "    await expect(page).toHaveURL(new RegExp(\"^\" + escape(BASE) + {}, 'i'));",
            js(&route_regex(route))
        ),
        None => writeln!(
            out,
            "    // screen {} has no route: URL not checked",
            screen.id
        ),
    }
    .expect("write to string");
}

/// One replay step: click the trigger, or navigate directly without one.
fn step(out: &mut String, n: usize, from: &str, transition: &Transition, to: &Screen) {
    let via = (transition.trigger.as_deref()).map_or(String::new(), |t| format!(" via {t:?}"));
    let title = format!(
        "step {n}: {from} -> {}{via} ({}:{})",
        to.name, transition.file, transition.line
    );
    writeln!(out, "  await test.step({}, async () => {{", js(&title)).expect("write");
    match &transition.trigger {
        Some(trigger) => writeln!(out, "    await clickTrigger(page, {});", js(trigger)),
        None => {
            out.push_str("    // no trigger read: direct navigation\n");
            match to.route.as_deref().map(|r| fill_route(r, &BTreeMap::new())) {
                Some(Ok(path)) => writeln!(out, "    await page.goto(BASE + {});", js(&path)),
                Some(Err(why)) => writeln!(out, "    throw new Error({});", js(&why)),
                None => writeln!(
                    out,
                    "    throw new Error({});",
                    js("no route to navigate to")
                ),
            }
        }
    }
    .expect("write to string");
    url_check(out, to);
    out.push_str("  });\n");
}

/// Sign-in lines for `role`: the password is read from the environment at
/// run time, from `password_env` or from [`KEYCHAIN_PASSWORD_ENV`].
fn sign_in(
    out: &mut String,
    config: &Config,
    app: &str,
    role: &str,
    start: &Screen,
) -> Result<(), String> {
    let run = config.views.run.get(app);
    let user = (run.and_then(|r| r.users.get(role))).ok_or_else(|| {
        format!("no test user for role {role}: set [views.run.{app}.users.{role}]")
    })?;
    let variable = user
        .password_env
        .as_deref()
        .unwrap_or(KEYCHAIN_PASSWORD_ENV);
    let login_path = (run.and_then(|r| r.login_path.as_deref()))
        .or(start.route.as_deref())
        .unwrap_or("/");
    writeln!(out, "  const password = process.env[{}];", js(variable)).expect("write");
    let missing = format!("set {variable} to the password of {}", user.login);
    writeln!(out, "  if (!password) throw new Error({});", js(&missing)).expect("write");
    writeln!(
        out,
        "  await signIn(page, {}, password, {});",
        js(&user.login),
        js(login_path)
    )
    .expect("write");
    Ok(())
}

/// The start screen of `path`: the first step's origin, or the target itself.
fn start_of<'a>(
    flow: &'a UiFlow,
    target: &'a Screen,
    path: &[&Transition],
) -> Result<&'a Screen, String> {
    match path.first().map(|t| &t.from) {
        None => Ok(target),
        Some(Origin::Screen(id)) => screen(flow, id),
        // shortcut: takes the first start screen of the application; exact when it has one.
        Some(Origin::AnyScreen { application: app }) => (flow.screens.iter())
            .find(|s| is_start(s) && application(s) == app)
            .ok_or_else(|| format!("application {app} has no start screen")),
    }
}

/// A self-contained Playwright test (TypeScript) that signs in as `role`'s
/// test user (none: anonymous), opens the start screen and replays `path` to
/// `screen_id` step by step. No password is written into the script.
pub fn playwright_script(
    flow: &UiFlow,
    screen_id: &str,
    path: &[&Transition],
    config: &Config,
    role: Option<&str>,
) -> Result<String, String> {
    let target = screen(flow, screen_id)?;
    let start = start_of(flow, target, path)?;
    let mut out = format!(
        "// Generated by `archguard ui-flow replay` (ADR 0021, decision 7).\n\
         // Replays the view's path to {} ({}:{}); a failing step means view and application disagree.\n",
        target.name, target.file, target.line
    );
    out.push_str(PRELUDE);
    writeln!(out, "\nconst BASE = {};", js(base_url(config, target)?)).expect("write");
    writeln!(
        out,
        "\ntest({}, async ({{ page }}) => {{",
        js(&format!("replay {}", target.name))
    )
    .expect("write");
    if let Some(role) = role {
        sign_in(&mut out, config, application(target), role, start)?;
    }
    writeln!(
        out,
        "  await test.step({}, async () => {{",
        js(&format!("start: {}", start.name))
    )
    .expect("write");
    let route = start.route.as_deref().unwrap_or("/");
    writeln!(out, "    await page.goto(BASE + {});", js(route)).expect("write");
    url_check(&mut out, start);
    out.push_str("  });\n");
    let mut from = start.name.clone();
    for (n, transition) in path.iter().enumerate() {
        let Target::Screen(id) = &transition.target else {
            return Err("path contains an unresolved transition".into());
        };
        let to = screen(flow, id)?;
        step(&mut out, n + 1, &from, transition, to);
        from = to.name.clone();
    }
    out.push_str("});\n");
    Ok(out)
}
