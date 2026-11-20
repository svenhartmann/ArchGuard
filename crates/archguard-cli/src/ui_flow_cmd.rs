//! `archguard ui-flow`: screens, navigations and permissions of UI frontends
//! against backend endpoints, with an optional permission simulation (ADR 0021).

use std::collections::BTreeMap;
use std::path::Path;

use archguard_engine::core::{
    ArchguardError, Assignment, Condition, MatrixKind, PermissionMatrix, UiFlow, evaluate_access,
    merge_ui_flows, permission_matrix, resolve_assignment, screen_reaches, ui_flow_findings,
    ui_flow_summary,
};
use archguard_engine::extract_ctx::{ExtractContext, with_root_config};
use serde_json::{Value, json};

use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};

/// Diagnostics shown in text output without `-v`.
const DIAGNOSTICS_SHOWN: usize = 10;

/// Runs an extraction of `root` with the configuration given by `--config`
/// (an error when it cannot be read), else with the one `root` holds (an
/// unreadable one is a note of the extractors that use it).
pub(crate) fn extract<T>(
    cli: &Cli,
    root: &Path,
    run: impl FnOnce(&ExtractContext) -> Result<T, ArchguardError>,
) -> Result<T, ArchguardError> {
    match cli.config.as_deref() {
        None => with_root_config(root, run),
        Some(file) => {
            let loaded = archguard_engine::load_config(Some(file), root)?;
            run(&ExtractContext::new(root, &loaded.config)
                .with_config_file(file.display().to_string()))
        }
    }
}

pub(crate) fn run(cli: &Cli, root: &Path, roles: &[String], anonymous: bool) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("ui-flow supports --format text|json");
    }
    let flow = match extract(cli, root, archguard_engine::ui_flow::analyze_with) {
        Ok(flow) => flow,
        Err(error) => return analysis_error(error),
    };
    let roles = defined_names(&flow, roles);
    let simulation = (anonymous || !roles.is_empty()).then(|| simulate(&flow, &roles));
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
pub(crate) fn assignment(roles: &[String]) -> Assignment {
    Assignment {
        authenticated: !roles.is_empty(),
        roles: roles.iter().cloned().collect(),
        ..Assignment::default()
    }
}

/// `--role` names as the flow defines them: `ROLE_X` reads `X` when only `X`
/// is defined (Spring drops the prefix); an undefined name gets a warning.
fn defined_names(flow: &UiFlow, roles: &[String]) -> Vec<String> {
    let defined = |n: &str| flow.roles.iter().any(|d| d.name == n);
    (roles.iter())
        .map(|role| {
            let stripped = role.strip_prefix("ROLE_").filter(|s| defined(s));
            let name = stripped.filter(|_| !defined(role)).unwrap_or(role);
            if !flow.roles.is_empty() && !defined(name) {
                eprintln!(
                    "warning: role {name:?} is not defined in the code; `archguard ui-flow roles` lists the defined ones"
                );
            }
            name.to_string()
        })
        .collect()
}

fn simulate(flow: &UiFlow, roles: &[String]) -> Value {
    let chosen = assignment(roles);
    let assignment = resolve_assignment(flow, &chosen);
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
    let implied: Vec<&String> = assignment.roles.difference(&chosen.roles).collect();
    json!({
        "anonymous": roles.is_empty(),
        "roles": chosen.roles,
        "implied_roles": implied,
        "policies": assignment.policies,
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
            let access = screen
                .access
                .as_ref()
                .map_or("none".into(), Condition::text);
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
    let implied: Vec<&str> = (simulation["implied_roles"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    let policies: Vec<String> = (simulation["policies"].as_object().into_iter().flatten())
        .map(|(name, allowed)| {
            format!(
                "{name} {}",
                if allowed == true { "allowed" } else { "denied" }
            )
        })
        .collect();
    let mut text = format!("signed in with roles {}", roles.join(", "));
    if !implied.is_empty() {
        text += &format!(" (implies {})", implied.join(", "));
    }
    if !policies.is_empty() {
        text += &format!("; policies: {}", policies.join(", "));
    }
    text
}

/// `ui-flow <action>`: the catalog actions here, `open` and `replay` in `ui_flow_run_cmd`.
pub(crate) fn action(cli: &Cli, root: &Path, action: &crate::ui_flow_run_cmd::UiFlowAction) -> i32 {
    use crate::ui_flow_run_cmd::UiFlowAction as A;
    match action {
        A::Roles { .. } => roles(cli, root),
        A::Matrix { kind, .. } => matrix(cli, root, *kind),
        _ => crate::ui_flow_run_cmd::run(cli.config.as_deref(), root, action),
    }
}

/// `archguard ui-flow matrix`: the permission matrix as table, JSON or CSV.
pub(crate) fn matrix(cli: &Cli, root: &Path, kind: crate::ui_flow_run_cmd::MatrixKindArg) -> i32 {
    use crate::ui_flow_run_cmd::MatrixKindArg as K;
    if !matches!(
        cli.format,
        Format::Text | Format::Table | Format::Json | Format::Csv
    ) {
        return usage("ui-flow matrix supports --format table|json|csv");
    }
    let flow = match extract(cli, root, archguard_engine::ui_flow::analyze_with) {
        Ok(flow) => flow,
        Err(error) => return analysis_error(error),
    };
    let kind = match kind {
        K::Screens => MatrixKind::Screens,
        K::Endpoints => MatrixKind::Endpoints,
        K::Both => MatrixKind::Both,
    };
    let matrix = permission_matrix(&flow, kind);
    if cli.quiet {
        return EXIT_OK;
    }
    match cli.format {
        Format::Json => emit(&serde_json::to_string_pretty(&matrix).expect("matrix serializes")),
        Format::Csv => emit(&matrix_csv(&matrix)),
        _ => emit(&matrix_table(&matrix)),
    }
    EXIT_OK
}

/// Aligned columns: kind, row, file:line, then one per role.
fn matrix_table(m: &PermissionMatrix) -> String {
    let mut table = vec![
        ["kind", "row", "where"]
            .into_iter()
            .map(String::from)
            .chain(m.roles.iter().cloned())
            .collect::<Vec<_>>(),
    ];
    for r in &m.rows {
        let cells = r.cells.iter().map(label);
        table.push(
            [
                label(&r.kind),
                r.label.clone(),
                format!("{}:{}", r.file, r.line),
            ]
            .into_iter()
            .chain(cells)
            .collect(),
        );
    }
    let widths: Vec<usize> = (0..table[0].len())
        .map(|c| {
            table
                .iter()
                .map(|row| row[c].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let lines: Vec<String> = (table.iter())
        .map(|row| {
            let padded: Vec<String> = (row.iter().zip(&widths))
                .map(|(cell, w)| format!("{cell:<w$}"))
                .collect();
            padded.join("  ").trim_end().to_string()
        })
        .collect();
    lines.join("\n")
}

fn matrix_csv(m: &PermissionMatrix) -> String {
    let quote = |s: &str| {
        if s.contains([',', '"', '\n']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    };
    let head = ["kind", "row", "file", "line", "requires"]
        .into_iter()
        .map(String::from)
        .chain(m.roles.iter().map(|r| quote(r)));
    let mut lines = vec![head.collect::<Vec<_>>().join(",")];
    for r in &m.rows {
        let fixed = [
            label(&r.kind),
            quote(&r.label),
            quote(&r.file),
            r.line.to_string(),
            quote(&r.requires),
        ];
        lines.push(
            fixed
                .into_iter()
                .chain(r.cells.iter().map(label))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    lines.join("\n")
}

/// `archguard ui-flow roles`: the roles, permissions and policies the code
/// defines (rules and import file), with where each was read.
pub(crate) fn roles(cli: &Cli, root: &Path) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("ui-flow roles supports --format text|json");
    }
    let import = match archguard_engine::ui_flow::read_ui_flow(root) {
        Ok(flow) => flow.unwrap_or_default(),
        Err(error) => return analysis_error(error),
    };
    let roles = extract(cli, root, |ctx| {
        Ok(archguard_engine::ui_flow_roles::extract_with(ctx))
    });
    let flow = match roles {
        Ok(roles) => merge_ui_flows(import, roles),
        Err(error) => return analysis_error(error),
    };
    if cli.quiet {
        return EXIT_OK;
    }
    if cli.format == Format::Json {
        let policies: Vec<Value> = (flow.policies.iter())
            .map(|p| {
                let mut row = json!(p);
                row["requires_text"] = json!(p.requires.text());
                row
            })
            .collect();
        let value =
            json!({"roles": flow.roles, "policies": policies, "diagnostics": flow.diagnostics});
        emit(&serde_json::to_string_pretty(&value).expect("roles serialize"));
        return EXIT_OK;
    }
    outln!("Roles: {}", flow.roles.len());
    for d in &flow.roles {
        let includes = if d.includes.is_empty() {
            String::new()
        } else {
            format!(" · includes {}", d.includes.join(", "))
        };
        outln!(
            "  {} · {}{includes} · {}:{} ({})",
            d.name,
            label(&d.kind),
            d.file,
            d.line,
            d.source
        );
    }
    outln!("Policies: {}", flow.policies.len());
    for p in &flow.policies {
        outln!(
            "  {} · needs: {} · {}:{} ({})",
            p.name,
            p.requires.text(),
            p.file,
            p.line,
            p.source
        );
    }
    let notes: Vec<String> = (flow.diagnostics.iter())
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
    print_gaps(&notes, cli.verbose);
    EXIT_OK
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
    fn matrix_renders_as_table_and_csv() {
        use archguard_engine::core::{AccessState, EndpointAccess, MatrixKind, ScreenKind};
        let flow = UiFlow {
            screens: vec![archguard_engine::core::Screen {
                id: "a".into(),
                name: "a".into(),
                kind: ScreenKind::Page,
                route: Some("/a, b".into()),
                application: None,
                file: "A.razor".into(),
                line: 3,
                access: Some(Condition::Authenticated),
                source: "t".into(),
            }],
            endpoints: vec![EndpointAccess {
                method: "GET".into(),
                path: "/x".into(),
                access: AccessState::Undeclared,
                application: None,
                file: "X.cs".into(),
                line: 9,
                source: "t".into(),
                handler: None,
                sends: vec![],
            }],
            ..UiFlow::default()
        };
        let m = permission_matrix(&flow, MatrixKind::Both);
        assert_eq!(
            matrix_table(&m),
            "kind      row     where      anonymous  signed in\n\
             screen    /a, b   A.razor:3  denied     allowed\n\
             endpoint  GET /x  X.cs:9     unknown    unknown"
        );
        assert_eq!(
            matrix_csv(&m),
            "kind,row,file,line,requires,anonymous,signed in\n\
             screen,\"/a, b\",A.razor,3,signed in,denied,allowed\n\
             endpoint,GET /x,X.cs,9,undeclared,unknown,unknown"
        );
    }

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
            condition.text(),
            "signed in and (role A or policy P) and not (custom x)"
        );
        assert_eq!(label(&Outcome::Denied), "denied");
        assert_eq!(
            label(&archguard_engine::core::UiFlowFindingKind::ShownButRefused),
            "shown_but_refused"
        );
    }

    #[test]
    fn role_names_follow_the_catalog_and_the_simulation_shows_implied_roles() {
        use archguard_engine::core::{RoleDef, RoleKind};
        let def = |name: &str, includes: &[&str]| RoleDef {
            name: name.into(),
            kind: RoleKind::Role,
            includes: includes.iter().map(|s| s.to_string()).collect(),
            file: "Sec.java".into(),
            line: 1,
            source: "spring".into(),
        };
        let flow = UiFlow {
            roles: vec![def("ADMIN", &["USER"]), def("USER", &[])],
            ..UiFlow::default()
        };
        let names = defined_names(&flow, &["ROLE_ADMIN".into(), "Other".into()]);
        assert_eq!(names, ["ADMIN", "Other"]);
        let simulation = simulate(&flow, &["ADMIN".into()]);
        assert_eq!(simulation["implied_roles"], json!(["USER"]));
        assert_eq!(
            assignment_text(&simulation),
            "signed in with roles ADMIN (implies USER)"
        );
    }
}
