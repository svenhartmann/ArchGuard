//! `ui_flow`: screens, navigation, actions and backend access read from code
//! (ADR 0021), with the role simulation, read-only and paginated within 4 KiB.
use crate::engineering_tools::EngineeringParams;
use crate::server::ArchguardTools;
use archguard_engine::core::{
    Assignment, Origin, Outcome, ScreenReach, UiFlow, evaluate_access, resolve_assignment,
    screen_reaches, start_screen_ids, ui_flow_findings, ui_flow_summary,
};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct UiFlowParams {
    #[serde(flatten)]
    pub page: EngineeringParams,
    /// Simulate a signed-in user holding exactly these roles; [] means signed in without a role.
    pub roles: Option<Vec<String>>,
    /// true simulates an anonymous visitor; cannot be combined with roles.
    pub anonymous: Option<bool>,
    /// Exact application (project) name: keeps its screens, transitions and actions and
    /// simulates from its start screens. Endpoints, findings and diagnostics stay complete.
    pub application: Option<String>,
    /// Revision from a previous page. Rejects a changed flow; restart at offset 0.
    pub expected_revision: Option<String>,
}

pub fn validate(p: &UiFlowParams) -> Result<(), ErrorData> {
    crate::engineering_tools::validate(&p.page)?;
    let bad = |v: &str| v.is_empty() || v.len() > 256 || v.contains('\0');
    if p.roles
        .as_ref()
        .is_some_and(|r| r.len() > 64 || r.iter().any(|v| bad(v)))
        || p.application.as_deref().is_some_and(bad)
        || p.expected_revision.as_deref().is_some_and(bad)
    {
        return Err(crate::tools::invalid_params(
            "roles (at most 64), application and expected_revision must be 1..=256 bytes without NUL",
        ));
    }
    if p.anonymous == Some(true) && p.roles.is_some() {
        return Err(crate::tools::invalid_params(
            "anonymous cannot be combined with roles",
        ));
    }
    Ok(())
}

/// Stable within one build: binds continuation pages to unchanged input.
pub(crate) fn revision(value: &impl serde::Serialize) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    serde_json::to_string(value)
        .expect("model serializes")
        .hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(crate) fn check_revision(revision: &str, expected: Option<&str>) -> Result<(), ErrorData> {
    if expected.is_some_and(|e| e != revision) {
        return Err(crate::tools::invalid_params(
            "Model changed. Restart at offset 0 without expected_revision.",
        ));
    }
    Ok(())
}

fn application_of<'a>(flow: &'a UiFlow, origin: &'a Origin) -> &'a str {
    match origin {
        Origin::Screen(id) => (flow.screens.iter())
            .find(|s| &s.id == id)
            .and_then(|s| s.application.as_deref())
            .unwrap_or(""),
        Origin::AnyScreen { application } => application,
    }
}

/// The chosen role as an assignment; none without roles or anonymous.
pub(crate) fn assignment(roles: Option<&[String]>, anonymous: Option<bool>) -> Option<Assignment> {
    (roles.is_some() || anonymous.is_some()).then(|| Assignment {
        authenticated: anonymous != Some(true),
        roles: roles.into_iter().flatten().cloned().collect(),
        ..Assignment::default()
    })
}

/// The start screens in scope and the reach of every screen.
fn simulate(
    flow: &UiFlow,
    app: Option<&str>,
    a: &Assignment,
) -> (Vec<String>, BTreeMap<String, ScreenReach>) {
    let in_app = |id: &&str| {
        let screen = flow.screens.iter().find(|s| s.id == *id);
        app.is_none_or(|app| screen.is_some_and(|s| s.application.as_deref() == Some(app)))
    };
    let starts = (start_screen_ids(flow).into_iter().filter(in_app))
        .map(String::from)
        .collect();
    (starts, screen_reaches(flow, a))
}

fn outcome(o: Outcome) -> &'static str {
    match o {
        Outcome::Allowed => "allowed",
        Outcome::Denied => "denied",
        Outcome::Unknown => "unknown",
    }
}

/// Role and policy rows, and for the metadata the role names and each policy's
/// requirement in words (the full defs with file and line are the rows).
fn catalog(flow: &UiFlow) -> (Vec<Value>, Value) {
    let roles = (flow.roles.iter()).map(|d| json!({"kind":"role","role":d}));
    let policies = (flow.policies.iter())
        .map(|d| json!({"kind":"policy","policy":d,"requires_text":d.requires.text()}));
    let names: std::collections::BTreeSet<&str> =
        flow.roles.iter().map(|d| d.name.as_str()).collect();
    let needs: BTreeMap<&str, String> = (flow.policies.iter())
        .map(|p| (p.name.as_str(), p.requires.text()))
        .collect();
    (
        roles.chain(policies).collect(),
        json!({"roles": names, "policies": needs}),
    )
}

fn simulation_metadata(starts: &[String], resolved: &Assignment, chosen: &Assignment) -> Value {
    let implied: Vec<&String> = resolved.roles.difference(&chosen.roles).collect();
    json!({"anonymous":!chosen.authenticated,"roles":chosen.roles,"implied_roles":implied,"policies":resolved.policies,"start_screens":starts,"custom_and_undefined_policies":"unknown"})
}

pub fn ui_flow_value(flow: &UiFlow, p: &UiFlowParams) -> Result<Value, ErrorData> {
    let app = p.application.as_deref();
    if app.is_some_and(|a| {
        !flow
            .screens
            .iter()
            .any(|s| s.application.as_deref() == Some(a))
    }) {
        return Err(crate::tools::invalid_params("Unknown application"));
    }
    let rev = revision(&(flow, &p.application, &p.roles, &p.anonymous));
    check_revision(&rev, p.expected_revision.as_deref())?;
    let sim = assignment(p.roles.as_deref(), p.anonymous)
        .map(|c| (simulate(flow, app, &c), resolve_assignment(flow, &c), c));
    let in_app = |origin: &Origin| app.is_none_or(|a| application_of(flow, origin) == a);
    let mut rows: Vec<Value> = (ui_flow_findings(flow).into_iter())
        .map(|f| json!({"kind":"finding","finding":f}))
        .collect();
    for s in flow
        .screens
        .iter()
        .filter(|s| app.is_none_or(|a| s.application.as_deref() == Some(a)))
    {
        let mut row = json!({"kind":"screen","id":s.id,"name":s.name,"route":s.route,"application":s.application,"access":s.access,"file":s.file,"line":s.line});
        if let Some(((_, reached), ..)) = &sim {
            let reach = json!(reached[&s.id]);
            for (key, value) in reach.as_object().into_iter().flatten() {
                row[key] = value.clone();
            }
        }
        rows.push(row);
    }
    rows.extend(
        flow.transitions
            .iter()
            .filter(|t| in_app(&t.from))
            .map(|t| json!({"kind":"transition","transition":t})),
    );
    rows.extend(
        flow.actions
            .iter()
            .filter(|a| in_app(&a.screen))
            .map(|a| json!({"kind":"action","action":a})),
    );
    for e in &flow.endpoints {
        let mut row = json!({"kind":"endpoint","endpoint":e});
        if let Some((_, a, _)) = &sim {
            row["simulated"] = json!(outcome(evaluate_access(&e.access, a)));
        }
        rows.push(row);
    }
    let (catalog_rows, catalog) = catalog(flow);
    rows.extend(catalog_rows);
    rows.extend(
        flow.diagnostics
            .iter()
            .map(|d| json!({"kind":"diagnostic","diagnostic":d})),
    );
    let q = p.page.query.as_deref().unwrap_or("").trim().to_lowercase();
    let simulation = (sim.as_ref())
        .map(|((starts, _), resolved, chosen)| simulation_metadata(starts, resolved, chosen));
    crate::engineering_tools::page(
        if flow.complete {
            "complete"
        } else {
            "incomplete"
        },
        json!({"definition":flow.definition,"revision":rev,"summary":ui_flow_summary(flow),"application":app,"simulation":simulation,"roles":catalog["roles"],"policies":catalog["policies"],
            "limitations":"Static evidence from rules over the code (ADR 0021), no running application. undeclared means no declaration was found and is never allowed. A role implies the roles it includes (hierarchy) and the permission ids it covers; a policy defined in code resolves to what it requires, an explicit switch wins; custom checks and undefined policies stay unknown. Reach from the start screens (marked as start by the extractor, e.g. MAUI Shell or WPF StartupUri, or routed `/`): sure = every guard allowed, possible = some guard unknown, denied = the screen's own access denies, no_known_path = not denied but no resolved path (unresolved_transitions_in_application counts links the rules could not read, so the gap may be ours), no_start_screen = the application has no start screen."}),
        rows.iter()
            .filter(|r| q.is_empty() || r.to_string().to_lowercase().contains(&q)),
        &p.page,
        Clone::clone,
    )
}

#[tool_router(router = ui_flow_router, vis = "pub(crate)")]
impl ArchguardTools {
    #[tool(
        description = "Read the UI flow from code (ADR 0021): screens (route, name, access, file, line), navigation transitions, actions with their HTTP calls, backend endpoint access (protected/open/undeclared), frontend/backend mismatch findings, summary and gap diagnostics. Built-in rules for Blazor, Angular, ASP.NET and Spring plus .archguard/ui-flow.json. Also the roles, permissions and policies the code defines (metadata roles/policies; role and policy rows with file:line). With roles or anonymous: true it simulates that user (role hierarchy and policy definitions applied): each screen sure/possible/denied/no_known_path/no_start_screen from the start screens (marked or routed `/`) and each endpoint allowed/denied/unknown. Static evidence, not a running application; undeclared never means allowed. application narrows screens, transitions and actions. Read-only; flattened 4 KiB pages, follow next_offset with metadata.revision as expected_revision."
    )]
    pub(crate) async fn ui_flow(
        &self,
        Parameters(params): Parameters<UiFlowParams>,
    ) -> Result<String, ErrorData> {
        validate(&params)?;
        let root = crate::tools::resolve_root(&self.root, params.page.path.as_deref())?;
        let loaded = crate::tools::load_config_at(&root)?;
        let ctx = archguard_engine::extract_ctx::ExtractContext::new(&root, &loaded.config);
        let flow =
            archguard_engine::ui_flow::analyze_with(&ctx).map_err(crate::server::insight_error)?;
        ui_flow_value(&flow, &params).map(|v| v.to_string())
    }
}

/// Role catalog in the tool, on a hand-built flow.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering_tools::EngineeringParams;
    use archguard_engine::core::{
        Condition, PolicyDef, RoleDef, RoleKind, Screen, ScreenKind, Target, Transition,
    };

    fn screen(id: &str, route: &str, access: Option<Condition>) -> Screen {
        Screen {
            id: id.into(),
            name: id.into(),
            kind: ScreenKind::Page,
            route: Some(route.into()),
            application: None,
            file: format!("{id}.razor"),
            line: 1,
            access,
            source: "test".into(),
        }
    }

    fn flow() -> UiFlow {
        UiFlow {
            screens: vec![
                screen("home", "/", None),
                screen(
                    "purge",
                    "/purge",
                    Some(Condition::Policy("CanPurge".into())),
                ),
            ],
            transitions: vec![Transition {
                from: Origin::Screen("home".into()),
                target: Target::Screen("purge".into()),
                trigger: None,
                file: "home.razor".into(),
                line: 2,
                condition: None,
                source: "test".into(),
            }],
            roles: vec![RoleDef {
                name: "Owner".into(),
                kind: RoleKind::Role,
                includes: vec!["Administrator".into()],
                file: "Roles.cs".into(),
                line: 3,
                source: "aspnet".into(),
            }],
            policies: vec![PolicyDef {
                name: "CanPurge".into(),
                requires: Condition::Role("Administrator".into()),
                file: "Program.cs".into(),
                line: 4,
                source: "aspnet".into(),
            }],
            ..UiFlow::default()
        }
    }

    fn page(roles: Option<&[&str]>) -> Value {
        let params = UiFlowParams {
            page: EngineeringParams {
                limit: Some(200),
                ..EngineeringParams::default()
            },
            roles: roles.map(|r| r.iter().map(|s| s.to_string()).collect()),
            ..UiFlowParams::default()
        };
        ui_flow_value(&flow(), &params).unwrap()
    }

    fn purge_reach(page: &Value) -> Value {
        let items = page["items"].as_array().unwrap();
        items.iter().find(|r| r["id"] == "purge").unwrap()["reach"].clone()
    }

    #[test]
    fn catalog_is_listed_and_the_simulation_resolves_it() {
        let plain = page(None);
        assert_eq!(plain["metadata"]["roles"], json!(["Owner"]));
        assert_eq!(
            plain["metadata"]["policies"]["CanPurge"],
            "role Administrator"
        );
        let items = plain["items"].as_array().unwrap();
        let role = items.iter().find(|r| r["kind"] == "role").unwrap();
        assert_eq!(role["role"]["includes"], json!(["Administrator"]));
        assert!(items.iter().any(|r| r["kind"] == "policy"));

        let owner = page(Some(&["Owner"]));
        assert_eq!(purge_reach(&owner), "sure");
        let simulation = &owner["metadata"]["simulation"];
        assert_eq!(simulation["implied_roles"], json!(["Administrator"]));
        assert_eq!(simulation["policies"]["CanPurge"], true);
        assert_eq!(purge_reach(&page(Some(&[]))), "denied");
    }
}
