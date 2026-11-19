//! `archguard ui-flow`: screens, navigations and permissions of UI frontends
//! against backend endpoints, with an optional permission simulation (ADR 0021).

use std::collections::BTreeMap;
use std::path::Path;

use archguard_engine::core::{
    Assignment, Condition, UiFlow, evaluate_access, screen_reaches, ui_flow_findings,
    ui_flow_summary,
};
use serde_json::{Value, json};

use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};

/// Diagnostics shown in text output without `-v`.
const DIAGNOSTICS_SHOWN: usize = 10;

pub(crate) fn run(cli: &Cli, root: &Path, roles: &[String], anonymous: bool) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("ui-flow supports --format text|json");
    }
    let flow = match archguard_engine::ui_flow::analyze(root) {
        Ok(flow) => flow,
        Err(error) => return analysis_error(error),
    };
    let simulation = (anonymous || !roles.is_empty()).then(|| simulate(&flow, roles));
    if cli.quiet {
        return EXIT_OK;
    }
    if cli.format == Format::Json {
        let mut value = json!({
            "summary": ui_flow_summary(&flow),
            "findings": ui_flow_findings(&flow),
            "flow": flow,
        });
        if let Some(simulation) = simulation {
            value["simulation"] = simulation;
        }
        emit(&serde_json::to_string_pretty(&value).expect("ui flow serializes"));
    } else {
        print_text(&flow, simulation.as_ref(), cli.verbose);
    }
    EXIT_OK
}

/// The simulated assignment: `--role` implies a signed-in user, no role
/// with `--anonymous` an anonymous one.
fn simulate(flow: &UiFlow, roles: &[String]) -> Value {
    let assignment = Assignment {
        authenticated: !roles.is_empty(),
        roles: roles.iter().cloned().collect(),
        ..Assignment::default()
    };
    let reaches = screen_reaches(flow, &assignment);
    let screens: Vec<Value> = (flow.screens.iter())
        .map(|s| {
            let mut row = json!({"id": s.id, "route": s.route, "application": s.application});
            let reach = json!(reaches[&s.id]);
            for (key, value) in reach.as_object().into_iter().flatten() {
                row[key] = value.clone();
            }
            row
        })
        .collect();
    let endpoints: Vec<Value> = (flow.endpoints.iter())
        .map(|e| {
            json!({
                "method": e.method, "path": e.path, "application": e.application,
                "file": e.file, "line": e.line,
                "outcome": evaluate_access(&e.access, &assignment),
            })
        })
        .collect();
    json!({
        "anonymous": roles.is_empty(),
        "roles": assignment.roles,
        "screens": screens,
        "endpoints": endpoints,
    })
}

fn application(name: Option<&str>) -> &str {
    name.unwrap_or("")
}

fn print_text(flow: &UiFlow, simulation: Option<&Value>, verbose: bool) {
    let s = ui_flow_summary(flow);
    let e = s.endpoints_by_access;
    outln!(
        "Screens {} · navigations {} of {} resolved · actions {}, {} unresolved · endpoints: {} protected, {} open, {} undeclared · complete: {}",
        s.screens,
        s.transitions_resolved,
        s.transitions_resolved + s.transitions_unresolved,
        s.actions,
        s.actions_unresolved,
        e.protected,
        e.open,
        e.undeclared,
        yes_no(s.complete)
    );
    if let Some(simulation) = simulation {
        outln!("Simulated: {}", assignment_text(simulation));
    }
    print_screens(flow, simulation);
    if let Some(simulation) = simulation {
        print_endpoints(simulation);
    }
    let findings = ui_flow_findings(flow);
    outln!("Findings: {}", findings.len());
    for f in &findings {
        let at = (f.action.as_ref())
            .map(|a| format!(" {}:{}", a.file, a.line))
            .unwrap_or_default();
        outln!(
            "  [{}] {}{at}: {}",
            label(&f.severity),
            label(&f.kind),
            f.message
        );
    }
    let gaps: Vec<String> = (flow.diagnostics.iter())
        .map(|d| {
            format!(
                "{} {}:{}: {}",
                label(&d.severity),
                d.file,
                d.line,
                d.message
            )
        })
        .collect();
    print_gaps(&gaps, verbose);
}

fn print_screens(flow: &UiFlow, simulation: Option<&Value>) {
    let reach: BTreeMap<&str, &Value> = simulation
        .and_then(|s| s["screens"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|s| Some((s["id"].as_str()?, s)))
        .collect();
    let mut by_application: BTreeMap<&str, Vec<_>> = BTreeMap::new();
    for screen in &flow.screens {
        let app = application(screen.application.as_deref());
        by_application.entry(app).or_default().push(screen);
    }
    for (app, screens) in by_application {
        outln!(
            "application {}",
            if app.is_empty() { "(none)" } else { app }
        );
        for screen in screens {
            let access = screen.access.as_ref().map_or("none".into(), condition_text);
            let reached = reach
                .get(screen.id.as_str())
                .map(|r| format!(" · {}", reach_text(r, &access)))
                .unwrap_or_default();
            outln!(
                "  {} · {} · access: {access} · {}:{}{reached}",
                screen.route.as_deref().unwrap_or("-"),
                screen.name,
                screen.file,
                screen.line
            );
        }
    }
}

fn print_endpoints(simulation: &Value) {
    let endpoints = simulation["endpoints"].as_array().map_or(&[][..], |v| v);
    outln!("Endpoints: {}", endpoints.len());
    for e in endpoints {
        outln!(
            "  {} {} {} · {}:{}",
            e["outcome"].as_str().unwrap_or("?"),
            e["method"].as_str().unwrap_or("?"),
            e["path"].as_str().unwrap_or("?"),
            e["file"].as_str().unwrap_or("?"),
            e["line"]
        );
    }
}

/// Gap diagnostics as preformatted rows: the count, then the first
/// [`DIAGNOSTICS_SHOWN`] or, with `-v`, all.
pub(crate) fn print_gaps(rows: &[String], verbose: bool) {
    let shown = if verbose {
        rows.len()
    } else {
        DIAGNOSTICS_SHOWN
    };
    let more = if rows.len() > shown {
        format!(" (first {shown}; -v for all)")
    } else {
        String::new()
    };
    outln!("Gaps: {}{more}", rows.len());
    for row in rows.iter().take(shown) {
        outln!("  {row}");
    }
}

fn assignment_text(simulation: &Value) -> String {
    if simulation["anonymous"] == true {
        return "anonymous".into();
    }
    let roles: Vec<&str> = (simulation["roles"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    format!("signed in with roles {}", roles.join(", "))
}

/// A simulated screen row in words; `access` is the screen's access in words.
fn reach_text(row: &Value, access: &str) -> String {
    match row["reach"].as_str().unwrap_or("") {
        "sure" => "reached surely".into(),
        "possible" => "reached possibly".into(),
        "denied" => format!("denied (needs: {access})"),
        "no_known_path" => {
            let n = row["unresolved_transitions_in_application"]
                .as_u64()
                .unwrap_or(0);
            let links = if n == 1 { "link" } else { "links" };
            format!("no known path ({n} {links} in this app unresolved)")
        }
        _ => "no start screen".into(),
    }
}

/// A condition in words: `role Admin or policy P`, `signed in`, `not (…)`.
pub(crate) fn condition_text(condition: &Condition) -> String {
    let join = |items: &[Condition], word: &str| {
        let parts: Vec<String> = (items.iter())
            .map(|c| match c {
                Condition::All(_) | Condition::Any(_) => format!("({})", condition_text(c)),
                _ => condition_text(c),
            })
            .collect();
        parts.join(word)
    };
    match condition {
        Condition::All(items) => join(items, " and "),
        Condition::Any(items) => join(items, " or "),
        Condition::Not(inner) => format!("not ({})", condition_text(inner)),
        Condition::Authenticated => "signed in".into(),
        Condition::Never => "never".into(),
        Condition::Role(name) => format!("role {name}"),
        Condition::Policy(name) => format!("policy {name}"),
        Condition::Custom(name) => format!("custom {name}"),
        Condition::Unknown(text) => format!("unknown ({text})"),
    }
}

/// The snake_case serde name of a unit enum value, from its `Debug` name.
pub(crate) fn label(value: &impl std::fmt::Debug) -> String {
    let mut out = String::new();
    for (i, c) in format!("{value:?}").chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

pub(crate) fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::Outcome;

    #[test]
    fn conditions_read_as_words() {
        let condition = Condition::All(vec![
            Condition::Authenticated,
            Condition::Any(vec![
                Condition::Role("A".into()),
                Condition::Policy("P".into()),
            ]),
            Condition::Not(Box::new(Condition::Custom("x".into()))),
        ]);
        assert_eq!(
            condition_text(&condition),
            "signed in and (role A or policy P) and not (custom x)"
        );
        assert_eq!(label(&Outcome::Denied), "denied");
        assert_eq!(
            label(&archguard_engine::core::UiFlowFindingKind::ShownButRefused),
            "shown_but_refused"
        );
    }
}
