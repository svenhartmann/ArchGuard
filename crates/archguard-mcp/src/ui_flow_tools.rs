//! `ui_flow`: screens, navigation, actions and backend access read from code
//! (ADR 0021), with the role simulation, read-only and paginated within 4 KiB.
use crate::engineering_tools::EngineeringParams;
use crate::server::ArchguardTools;
use archguard_engine::core::{
    Assignment, Origin, Outcome, ScreenReach, UiFlow, evaluate_access, screen_reaches,
    start_screen_ids, ui_flow_findings, ui_flow_summary,
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
    let sim = assignment(p.roles.as_deref(), p.anonymous).map(|a| (simulate(flow, app, &a), a));
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
        if let Some(((_, reached), _)) = &sim {
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
        if let Some((_, a)) = &sim {
            row["simulated"] = json!(outcome(evaluate_access(&e.access, a)));
        }
        rows.push(row);
    }
    rows.extend(
        flow.diagnostics
            .iter()
            .map(|d| json!({"kind":"diagnostic","diagnostic":d})),
    );
    let q = p.page.query.as_deref().unwrap_or("").trim().to_lowercase();
    let simulation = sim.as_ref().map(|((starts, _), a)| json!({"anonymous":!a.authenticated,"roles":a.roles,"start_screens":starts,"policies_and_custom":"unknown"}));
    crate::engineering_tools::page(
        if flow.complete {
            "complete"
        } else {
            "incomplete"
        },
        json!({"definition":flow.definition,"revision":rev,"summary":ui_flow_summary(flow),"application":app,"simulation":simulation,
            "limitations":"Static evidence from rules over the code (ADR 0021), no running application. undeclared means no declaration was found and is never allowed. Policy and custom checks stay unknown. Reach from the start screens (marked as start by the extractor, e.g. MAUI Shell or WPF StartupUri, or routed `/`): sure = every guard allowed, possible = some guard unknown, denied = the screen's own access denies, no_known_path = not denied but no resolved path (unresolved_transitions_in_application counts links the rules could not read, so the gap may be ours), no_start_screen = the application has no start screen."}),
        rows.iter()
            .filter(|r| q.is_empty() || r.to_string().to_lowercase().contains(&q)),
        &p.page,
        Clone::clone,
    )
}

#[tool_router(router = ui_flow_router, vis = "pub(crate)")]
impl ArchguardTools {
    #[tool(
        description = "Read the UI flow from code (ADR 0021): screens (route, name, access, file, line), navigation transitions, actions with their HTTP calls, backend endpoint access (protected/open/undeclared), frontend/backend mismatch findings, summary and gap diagnostics. Built-in rules for Blazor, Angular, ASP.NET and Spring plus .archguard/ui-flow.json. With roles or anonymous: true it simulates that user: each screen sure/possible/denied/no_known_path/no_start_screen from the start screens (marked or routed `/`) and each endpoint allowed/denied/unknown. Static evidence, not a running application; undeclared never means allowed. application narrows screens, transitions and actions. Read-only; flattened 4 KiB pages, follow next_offset with metadata.revision as expected_revision."
    )]
    pub(crate) async fn ui_flow(
        &self,
        Parameters(params): Parameters<UiFlowParams>,
    ) -> Result<String, ErrorData> {
        validate(&params)?;
        let root = crate::tools::resolve_root(&self.root, params.page.path.as_deref())?;
        let flow =
            archguard_engine::ui_flow::analyze(&root).map_err(crate::server::insight_error)?;
        ui_flow_value(&flow, &params).map(|v| v.to_string())
    }
}
