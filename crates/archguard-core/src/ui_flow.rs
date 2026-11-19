//! `ui-flow/v1` (ADR 0021): screens, navigation, actions and backend access in
//! one framework-neutral model, plus the pure simulator over it.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const UI_FLOW_DEFINITION: &str = "ui-flow/v1";
/// `source` of every entry read from `.archguard/ui-flow.json`.
pub const UI_FLOW_IMPORT_SOURCE: &str = "import";
/// Endpoint method that accepts every HTTP method (Spring `@RequestMapping`
/// without `method`); the join matches it with any action method.
pub const ANY_METHOD: &str = "ANY";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiFlow {
    pub definition: String,
    #[serde(default)]
    pub screens: Vec<Screen>,
    #[serde(default)]
    pub transitions: Vec<Transition>,
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub endpoints: Vec<EndpointAccess>,
    #[serde(default)]
    pub diagnostics: Vec<UiFlowDiagnostic>,
    /// False as soon as one route, target or role could not be read literally.
    pub complete: bool,
}
impl Default for UiFlow {
    /// Empty and complete, so it is the identity of [`merge_ui_flows`].
    fn default() -> Self {
        Self {
            definition: UI_FLOW_DEFINITION.into(),
            screens: vec![],
            transitions: vec![],
            actions: vec![],
            endpoints: vec![],
            diagnostics: vec![],
            complete: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenKind {
    Page,
    Window,
    Dialog,
    View,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Screen {
    pub id: String,
    pub name: String,
    pub kind: ScreenKind,
    /// Route pattern such as `/order/:id`; none for screens without a URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Owning application; tells apart equal routes of different apps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<String>,
    pub file: String,
    pub line: u32,
    /// Guard on the screen itself; none means no guard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<Condition>,
    /// Extractor name, or `import`.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Screen(String),
    /// The source text of a target the rules could not read literally.
    Unresolved(String),
}

/// Where a transition or action starts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Screen(String),
    /// Every screen of one application, e.g. a layout or menu component.
    /// `""` stands for the screens without an application.
    AnyScreen {
        application: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Transition {
    pub from: Origin,
    pub target: Target,
    /// Element id or label that triggers the navigation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    pub file: String,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Call {
    Http {
        method: String,
        path: String,
    },
    /// A method id for calls that are not HTTP.
    Method(String),
    /// Source text of a call the rules could not read, e.g. a computed URL.
    Unresolved(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Action {
    pub screen: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    pub call: Call,
    pub file: String,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,
    pub source: String,
}

/// The backend side: who may call an HTTP endpoint.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EndpointAccess {
    /// Upper-case HTTP method, or [`ANY_METHOD`].
    pub method: String,
    pub path: String,
    pub access: AccessState,
    /// Owning project (directory name of the nearest build file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<String>,
    pub file: String,
    pub line: u32,
    pub source: String,
}

/// Never two states: `undeclared` must not look `open` (ADR 0021, decision 2).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessState {
    Protected(Condition),
    /// Declared anonymous, e.g. `[AllowAnonymous]`, `permitAll`.
    Open,
    /// No declaration found; the code may still check by hand.
    Undeclared,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Not(Box<Condition>),
    Authenticated,
    /// Never holds, e.g. `@DenyAll`.
    Never,
    Role(String),
    Policy(String),
    Custom(String),
    /// Source text the rules cannot read.
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UiFlowDiagnostic {
    pub file: String,
    pub line: u32,
    pub message: String,
    /// Older files without the field read as `gap`.
    #[serde(default)]
    pub severity: DiagnosticSeverity,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    /// An element is missing or unknown; the flow is not complete.
    #[default]
    Gap,
    /// Information only, e.g. rules that exist but are not evaluated.
    Note,
}

/// Simulator input: the chosen role sets `authenticated` and `roles`; policy
/// and custom atoms are switches, and a switch missing here is unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assignment {
    pub authenticated: bool,
    pub roles: BTreeSet<String>,
    pub policies: BTreeMap<String, bool>,
    pub custom: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Allowed,
    Denied,
    Unknown,
}

fn known(value: Option<&bool>) -> Outcome {
    match value {
        Some(true) => Outcome::Allowed,
        Some(false) => Outcome::Denied,
        None => Outcome::Unknown,
    }
}

/// Kleene AND: one denied decides, else one unknown keeps it open.
fn and(a: Outcome, b: Outcome) -> Outcome {
    match (a, b) {
        (Outcome::Denied, _) | (_, Outcome::Denied) => Outcome::Denied,
        (Outcome::Unknown, _) | (_, Outcome::Unknown) => Outcome::Unknown,
        _ => Outcome::Allowed,
    }
}

/// Three-valued evaluation (Kleene logic).
pub fn evaluate_condition(condition: &Condition, assignment: &Assignment) -> Outcome {
    match condition {
        Condition::All(items) => items.iter().fold(Outcome::Allowed, |acc, item| {
            and(acc, evaluate_condition(item, assignment))
        }),
        // any(x) = not(all(not x)): one allowed decides, else one unknown keeps it open.
        Condition::Any(items) => negate(items.iter().fold(Outcome::Allowed, |acc, item| {
            and(acc, negate(evaluate_condition(item, assignment)))
        })),
        Condition::Not(inner) => negate(evaluate_condition(inner, assignment)),
        Condition::Authenticated => known(Some(&assignment.authenticated)),
        Condition::Never => Outcome::Denied,
        Condition::Role(name) => known(Some(&assignment.roles.contains(name))),
        Condition::Policy(name) => known(assignment.policies.get(name)),
        Condition::Custom(name) => known(assignment.custom.get(name)),
        Condition::Unknown(_) => Outcome::Unknown,
    }
}

fn negate(outcome: Outcome) -> Outcome {
    match outcome {
        Outcome::Allowed => Outcome::Denied,
        Outcome::Denied => Outcome::Allowed,
        Outcome::Unknown => Outcome::Unknown,
    }
}

/// `undeclared` is unknown, never allowed.
pub fn evaluate_access(access: &AccessState, assignment: &Assignment) -> Outcome {
    match access {
        AccessState::Protected(condition) => evaluate_condition(condition, assignment),
        AccessState::Open => Outcome::Allowed,
        AccessState::Undeclared => Outcome::Unknown,
    }
}

fn evaluate_optional(condition: Option<&Condition>, assignment: &Assignment) -> Outcome {
    condition.map_or(Outcome::Allowed, |c| evaluate_condition(c, assignment))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    /// Some path has every guard allowed.
    Sure,
    /// Reached only over paths with an unknown guard.
    Possible,
}

/// Screens reachable from `start` (included). A transition is followed when
/// neither its condition nor the target screen's access is denied; an
/// `any_screen` transition starts at every reached screen of its application.
/// Unresolved targets and unknown screen ids are skipped. Empty when `start` is unknown
/// or its own access is denied.
pub fn reachable_screens(
    flow: &UiFlow,
    start: &str,
    assignment: &Assignment,
) -> BTreeMap<String, Reach> {
    let screens: BTreeMap<&str, &Screen> =
        flow.screens.iter().map(|s| (s.id.as_str(), s)).collect();
    let level = |outcome| match outcome {
        Outcome::Allowed => Some(Reach::Sure),
        Outcome::Unknown => Some(Reach::Possible),
        Outcome::Denied => None,
    };
    let mut reached = BTreeMap::new();
    let Some(first) = screens
        .get(start)
        .and_then(|s| level(evaluate_optional(s.access.as_ref(), assignment)))
    else {
        return reached;
    };
    reached.insert(start.to_string(), first);
    // Each screen improves at most twice (none -> possible -> sure), so this ends on cycles.
    let mut changed = true;
    while changed {
        changed = false;
        for transition in &flow.transitions {
            let from = origin_reach(&transition.from, &reached, &screens);
            let (Some(from), Target::Screen(to)) = (from, &transition.target) else {
                continue;
            };
            let Some(target) = screens.get(to.as_str()) else {
                continue;
            };
            let edge = and(
                evaluate_optional(transition.condition.as_ref(), assignment),
                evaluate_optional(target.access.as_ref(), assignment),
            );
            let Some(edge) = level(edge) else { continue };
            let via = from.max(edge); // Sure < Possible: the weaker link wins.
            if reached.get(to).is_none_or(|old| via < *old) {
                reached.insert(to.clone(), via);
                changed = true;
            }
        }
    }
    reached
}

/// Path shape: leading/trailing slashes dropped, literal segments lower-case,
/// every parameter (`:id`, `{id}`, `{id:int}`) becomes `{}`.
pub fn normalize_path_pattern(path: &str) -> String {
    let segments: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.starts_with(':') || (s.starts_with('{') && s.ends_with('}')) {
                "{}".to_string()
            } else {
                s.to_lowercase()
            }
        })
        .collect();
    format!("/{}", segments.join("/"))
}

/// How well a transition's origin is reached: an `any_screen` origin as well
/// as the best reached screen of its application.
fn origin_reach(
    origin: &Origin,
    reached: &BTreeMap<String, Reach>,
    screens: &BTreeMap<&str, &Screen>,
) -> Option<Reach> {
    match origin {
        Origin::Screen(id) => reached.get(id).copied(),
        Origin::AnyScreen { application } => reached
            .iter()
            .filter(|(id, _)| application_of(screens, id) == Some(application.as_str()))
            .map(|(_, reach)| *reach)
            .min(),
    }
}

/// Application of a screen id: `""` for a screen without one, none for an unknown id.
fn application_of<'a>(screens: &BTreeMap<&str, &'a Screen>, id: &str) -> Option<&'a str> {
    screens
        .get(id)
        .map(|s| s.application.as_deref().unwrap_or(""))
}

fn is_parameter(segment: &str) -> bool {
    segment.starts_with(':') || (segment.starts_with('{') && segment.ends_with('}'))
}

/// How well the route `pattern` matches `path`: none when it does not, else
/// the number of literal segments that are equal (higher is more specific).
/// Segments match when both are parameters, when they are equal literals
/// (case-insensitive), or when the pattern has a parameter and the path a
/// literal (`/orders/:id` matches `/orders/42`). A parameter in `path` never
/// matches a literal in `pattern`, so pattern against pattern compares shape.
pub fn path_match_score(pattern: &str, path: &str) -> Option<usize> {
    let segments = |p: &str| -> Vec<String> {
        p.split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let (pattern, path) = (segments(pattern), segments(path));
    if pattern.len() != path.len() {
        return None;
    }
    pattern.iter().zip(&path).try_fold(0, |score, (p, s)| {
        match (is_parameter(p), is_parameter(s)) {
            (true, _) => Some(score),
            (false, false) if p == s => Some(score + 1),
            _ => None,
        }
    })
}

/// Whether the route `pattern` matches `path`; see [`path_match_score`].
pub fn path_patterns_match(pattern: &str, path: &str) -> bool {
    path_match_score(pattern, path).is_some()
}

/// The one candidate whose route matches `path` best (an exact literal route
/// beats a parameter route). `Err` holds the tied best candidates, empty when
/// none matches: a tie is never guessed. A candidate may come with several routes.
pub fn best_route<'a, T: PartialEq>(
    path: &str,
    candidates: impl IntoIterator<Item = (T, &'a str)>,
) -> Result<T, Vec<T>> {
    let (mut best, mut tied) = (None, Vec::new());
    for (candidate, route) in candidates {
        let Some(score) = path_match_score(route, path) else {
            continue;
        };
        match best.cmp(&Some(score)) {
            std::cmp::Ordering::Less => (best, tied) = (Some(score), vec![candidate]),
            std::cmp::Ordering::Equal if !tied.contains(&candidate) => tied.push(candidate),
            _ => {}
        }
    }
    if tied.len() == 1 {
        Ok(tied.remove(0))
    } else {
        Err(tied)
    }
}

/// Each HTTP action with the endpoints it hits. An endpoint hits when its
/// method is equal or [`ANY_METHOD`] and its route matches the call path;
/// within one application only its best-matching routes count (a literal route
/// beats a parameter route, as in ASP.NET and Spring). When some hit belongs to
/// the action's own application, only those stay; otherwise hits of every
/// application stay, since frontends call other services. Unresolved and
/// method calls are skipped; an empty list means no endpoint was found.
pub fn join_actions_to_endpoints(flow: &UiFlow) -> Vec<(&Action, Vec<&EndpointAccess>)> {
    let screens: BTreeMap<&str, &Screen> =
        flow.screens.iter().map(|s| (s.id.as_str(), s)).collect();
    flow.actions
        .iter()
        .filter_map(|action| {
            let Call::Http { method, path } = &action.call else {
                return None;
            };
            let application = match &action.screen {
                Origin::Screen(id) => application_of(&screens, id),
                Origin::AnyScreen { application } => Some(application.as_str()),
            };
            Some((action, endpoint_hits(flow, application, method, path)))
        })
        .collect()
}

fn endpoint_hits<'a>(
    flow: &'a UiFlow,
    application: Option<&str>,
    method: &str,
    path: &str,
) -> Vec<&'a EndpointAccess> {
    let scored: Vec<(usize, &EndpointAccess)> = flow
        .endpoints
        .iter()
        .filter(|e| e.method == ANY_METHOD || e.method.eq_ignore_ascii_case(method))
        .filter_map(|e| path_match_score(&e.path, path).map(|score| (score, e)))
        .collect();
    let mut best: BTreeMap<Option<&str>, usize> = BTreeMap::new();
    for (score, e) in &scored {
        let top = best.entry(e.application.as_deref()).or_default();
        *top = (*top).max(*score);
    }
    let mut hits: Vec<&EndpointAccess> = scored
        .into_iter()
        .filter(|(score, e)| best[&e.application.as_deref()] == *score)
        .map(|(_, e)| e)
        .collect();
    let own =
        |e: &&EndpointAccess| application.is_some_and(|a| e.application.as_deref() == Some(a));
    if hits.iter().any(own) {
        hits.retain(own);
    }
    hits
}

/// Union of two flows (extractor result + import): sorted, identical entries
/// once, complete only when both are.
pub fn merge_ui_flows(a: UiFlow, b: UiFlow) -> UiFlow {
    fn union<T: Ord>(mut a: Vec<T>, b: Vec<T>) -> Vec<T> {
        a.extend(b);
        a.sort();
        a.dedup();
        a
    }
    UiFlow {
        definition: UI_FLOW_DEFINITION.into(),
        screens: union(a.screens, b.screens),
        transitions: union(a.transitions, b.transitions),
        actions: union(a.actions, b.actions),
        endpoints: union(a.endpoints, b.endpoints),
        diagnostics: union(a.diagnostics, b.diagnostics),
        complete: a.complete && b.complete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(name: &str) -> Condition {
        Condition::Role(name.into())
    }
    fn policy(name: &str) -> Condition {
        Condition::Policy(name.into())
    }
    fn admin() -> Assignment {
        Assignment {
            authenticated: true,
            roles: BTreeSet::from(["admin".to_string()]),
            ..Assignment::default()
        }
    }
    fn screen(id: &str, access: Option<Condition>) -> Screen {
        Screen {
            id: id.into(),
            name: id.into(),
            kind: ScreenKind::Page,
            route: Some(format!("/{id}")),
            application: None,
            file: "App.razor".into(),
            line: 1,
            access,
            source: "test".into(),
        }
    }
    fn go(from: &str, to: &str, condition: Option<Condition>) -> Transition {
        Transition {
            from: Origin::Screen(from.into()),
            target: Target::Screen(to.into()),
            trigger: None,
            file: "App.razor".into(),
            line: 2,
            condition,
            source: "test".into(),
        }
    }
    fn http_action(method: &str, path: &str) -> Action {
        Action {
            screen: Origin::Screen("orders".into()),
            trigger: Some("save".into()),
            call: Call::Http {
                method: method.into(),
                path: path.into(),
            },
            file: "Orders.razor".into(),
            line: 9,
            condition: None,
            source: "test".into(),
        }
    }
    fn endpoint(method: &str, path: &str, access: AccessState) -> EndpointAccess {
        EndpointAccess {
            method: method.into(),
            path: path.into(),
            access,
            application: None,
            file: "Api.cs".into(),
            line: 3,
            source: "test".into(),
        }
    }

    #[test]
    fn kleene_any_all_not_with_unknown() {
        let a = admin();
        let unknown = policy("missing");
        assert_eq!(
            evaluate_condition(&Condition::Any(vec![role("admin"), unknown.clone()]), &a),
            Outcome::Allowed
        );
        assert_eq!(
            evaluate_condition(&Condition::Any(vec![role("x"), unknown.clone()]), &a),
            Outcome::Unknown
        );
        assert_eq!(
            evaluate_condition(&Condition::Any(vec![role("x"), role("y")]), &a),
            Outcome::Denied
        );
        assert_eq!(
            evaluate_condition(&Condition::All(vec![role("x"), unknown.clone()]), &a),
            Outcome::Denied
        );
        assert_eq!(
            evaluate_condition(&Condition::All(vec![role("admin"), unknown.clone()]), &a),
            Outcome::Unknown
        );
        assert_eq!(
            evaluate_condition(
                &Condition::All(vec![role("admin"), Condition::Authenticated]),
                &a
            ),
            Outcome::Allowed
        );
        assert_eq!(
            evaluate_condition(&Condition::Not(Box::new(unknown.clone())), &a),
            Outcome::Unknown
        );
        assert_eq!(
            evaluate_condition(&Condition::Not(Box::new(role("admin"))), &a),
            Outcome::Denied
        );
        assert_eq!(
            evaluate_condition(&Condition::Unknown("IsOwner()".into()), &a),
            Outcome::Unknown
        );
        let mut switched = a.clone();
        switched.policies.insert("missing".into(), true);
        switched.custom.insert("beta".into(), false);
        assert_eq!(evaluate_condition(&unknown, &switched), Outcome::Allowed);
        assert_eq!(
            evaluate_condition(&Condition::Custom("beta".into()), &switched),
            Outcome::Denied
        );
        assert_eq!(
            evaluate_condition(&Condition::Authenticated, &Assignment::default()),
            Outcome::Denied
        );
        assert_eq!(evaluate_condition(&Condition::Never, &a), Outcome::Denied);
        assert_eq!(
            evaluate_condition(&Condition::Any(vec![Condition::Never, unknown]), &a),
            Outcome::Unknown
        );
    }

    #[test]
    fn undeclared_is_never_allowed() {
        for assignment in [Assignment::default(), admin()] {
            assert_eq!(
                evaluate_access(&AccessState::Undeclared, &assignment),
                Outcome::Unknown
            );
            assert_eq!(
                evaluate_access(&AccessState::Open, &assignment),
                Outcome::Allowed
            );
        }
        assert_eq!(
            evaluate_access(&AccessState::Protected(role("admin")), &admin()),
            Outcome::Allowed
        );
        assert_eq!(
            evaluate_access(
                &AccessState::Protected(role("admin")),
                &Assignment::default()
            ),
            Outcome::Denied
        );
    }

    #[test]
    fn reachability_handles_cycles_denied_and_unknown_guards() {
        let flow = UiFlow {
            screens: vec![
                screen("home", None),
                screen("orders", Some(Condition::Authenticated)),
                screen("admin", Some(role("admin"))),
                screen("beta", None),
                screen("beta_detail", None),
                screen("island", None),
            ],
            transitions: vec![
                go("home", "orders", None),
                go("orders", "home", None), // cycle
                go("orders", "admin", None),
                go("home", "beta", Some(Condition::Custom("beta".into()))),
                go("beta", "beta_detail", None),
                go("beta_detail", "home", None),
                Transition {
                    target: Target::Unresolved("Nav.NavigateTo(url)".into()),
                    ..go("home", "x", None)
                },
            ],
            ..UiFlow::default()
        };
        let user = Assignment {
            authenticated: true,
            ..Assignment::default()
        };
        let reached = reachable_screens(&flow, "home", &user);
        assert_eq!(
            reached,
            BTreeMap::from([
                ("home".to_string(), Reach::Sure),
                ("orders".to_string(), Reach::Sure),
                ("beta".to_string(), Reach::Possible),
                ("beta_detail".to_string(), Reach::Possible),
            ])
        );
        let reached = reachable_screens(&flow, "home", &admin());
        assert_eq!(reached.get("admin"), Some(&Reach::Sure));
        // A sure path upgrades a screen first seen as possible.
        let mut upgraded = flow.clone();
        upgraded.transitions.push(go("orders", "beta_detail", None));
        let reached = reachable_screens(&upgraded, "home", &user);
        assert_eq!(reached.get("beta_detail"), Some(&Reach::Sure));
        assert!(reachable_screens(&flow, "nowhere", &user).is_empty());
        assert!(reachable_screens(&flow, "admin", &user).is_empty());
    }

    #[test]
    fn any_screen_transitions_start_at_every_reached_screen_of_the_application() {
        let in_app = |app: &str, s: Screen| Screen {
            application: Some(app.into()),
            ..s
        };
        let menu = |app: &str, to: &str, condition: Option<Condition>| Transition {
            from: Origin::AnyScreen {
                application: app.into(),
            },
            ..go("x", to, condition)
        };
        let flow = UiFlow {
            screens: vec![
                in_app("shop", screen("home", None)),
                in_app("shop", screen("cart", None)),
                in_app("shop", screen("beta", None)),
                in_app("other", screen("help", None)),
                screen("loose", None),
            ],
            transitions: vec![
                go("home", "beta", Some(Condition::Custom("beta".into()))),
                menu("shop", "cart", None),
                menu("other", "help", None),
                menu("", "loose", None),
            ],
            ..UiFlow::default()
        };
        let reached = reachable_screens(&flow, "home", &admin());
        assert_eq!(
            reached,
            BTreeMap::from([
                ("beta".to_string(), Reach::Possible),
                ("cart".to_string(), Reach::Sure),
                ("home".to_string(), Reach::Sure),
            ])
        );
        // From a screen reached only possibly, the menu is only possible too.
        let mut gated = flow.clone();
        gated.transitions[1].from = Origin::Screen("beta".into());
        assert_eq!(
            reachable_screens(&gated, "home", &admin()).get("cart"),
            Some(&Reach::Possible)
        );
        assert_eq!(reachable_screens(&flow, "loose", &admin()).len(), 1);
    }

    #[test]
    fn path_patterns_compare_by_shape() {
        for other in ["/order/{id}", "order/{id:int}", "/Order/{id}/"] {
            assert!(path_patterns_match("/order/:id", other), "{other}");
        }
        assert_eq!(normalize_path_pattern("/Order/{id}/"), "/order/{}");
        assert_eq!(normalize_path_pattern("/"), "/");
        assert!(!path_patterns_match("/order/:id", "/order/:id/items"));
        assert!(!path_patterns_match("/order/:id", "/orders/:id"));
    }

    #[test]
    fn literal_paths_match_parameter_routes_and_exact_routes_win() {
        assert!(path_patterns_match("/orders/:id", "/orders/42"));
        assert!(path_patterns_match("/item/{id:int}", "item/42"));
        // A parameter in the path is no literal: pattern against pattern is shape.
        assert!(!path_patterns_match("/item/new", "/item/{p}"));
        assert!(!path_patterns_match("/orders/:id", "/orders/42/items"));
        assert_eq!(path_match_score("/Item/New", "/item/new"), Some(2));
        let routes = [("detail", "/item/{id}"), ("new", "/item/new")];
        assert_eq!(best_route("/item/new", routes), Ok("new"));
        assert_eq!(best_route("/item/7", routes), Ok("detail"));
        assert_eq!(best_route("/item/{p}", routes), Ok("detail"));
        assert_eq!(best_route("/cart", routes), Err(vec![]));
        // Two equally good candidates are never guessed.
        let tied = [("a", "/x/{id}"), ("b", "/{name}/1")];
        assert_eq!(best_route("/x/1", tied), Err(vec!["a", "b"]));
        // Several routes of one candidate are no tie.
        let one = [("b", "/b"), ("b", "/bee"), ("b", "/{x}")];
        assert_eq!(best_route("/bee", one), Ok("b"));
    }

    #[test]
    fn join_uses_any_method_best_routes_and_own_application() {
        let in_app = |app: &str, e: EndpointAccess| EndpointAccess {
            application: Some(app.into()),
            ..e
        };
        let flow = UiFlow {
            screens: vec![Screen {
                application: Some("Shop".into()),
                ..screen("orders", None)
            }],
            actions: vec![
                http_action("GET", "/api/items/new"),
                http_action("PUT", "/api/orders/7"),
                Action {
                    screen: Origin::AnyScreen {
                        application: "Shop".into(),
                    },
                    ..http_action("GET", "/api/orders/7")
                },
                Action {
                    call: Call::Unresolved("Http.GetAsync(url)".into()),
                    ..http_action("GET", "/")
                },
            ],
            endpoints: vec![
                in_app("Api", endpoint("GET", "/api/items/{id}", AccessState::Open)),
                in_app(
                    "Api",
                    endpoint("GET", "/api/items/new", AccessState::Undeclared),
                ),
                in_app(
                    "Other",
                    endpoint("ANY", "/api/items/{id}", AccessState::Open),
                ),
                in_app(
                    "Api",
                    endpoint("ANY", "/api/orders/{id}", AccessState::Open),
                ),
                in_app(
                    "Shop",
                    endpoint("GET", "/api/orders/{id}", AccessState::Open),
                ),
            ],
            ..UiFlow::default()
        };
        let joined = join_actions_to_endpoints(&flow);
        assert_eq!(joined.len(), 3, "unresolved calls are skipped");
        let e = &flow.endpoints;
        // Per application the literal route wins; other applications stay.
        assert_eq!(joined[0].1, vec![&e[1], &e[2]]);
        // ANY matches every method.
        assert_eq!(joined[1].1, vec![&e[3]]);
        // The action's own application wins when it serves the path.
        assert_eq!(joined[2].1, vec![&e[4]]);
    }

    #[test]
    fn actions_join_endpoints_by_method_and_shape() {
        let flow = UiFlow {
            actions: vec![
                http_action("post", "/api/orders/{orderId}"),
                http_action("DELETE", "/api/orders/:id"),
                Action {
                    call: Call::Method("OrderService.Save".into()),
                    ..http_action("GET", "/")
                },
            ],
            endpoints: vec![
                endpoint("POST", "api/orders/{id:int}", AccessState::Undeclared),
                endpoint("GET", "/api/orders/{id}", AccessState::Open),
            ],
            ..UiFlow::default()
        };
        let joined = join_actions_to_endpoints(&flow);
        assert_eq!(joined.len(), 2);
        assert_eq!(joined[0].1, vec![&flow.endpoints[0]]);
        assert!(joined[1].1.is_empty());
    }

    #[test]
    fn merge_is_deterministic_and_ands_complete() {
        let a = UiFlow {
            screens: vec![screen("b", None), screen("a", None)],
            ..UiFlow::default()
        };
        let b = UiFlow {
            screens: vec![screen("a", None), screen("c", None)],
            diagnostics: vec![UiFlowDiagnostic {
                file: "x.ts".into(),
                line: 4,
                message: "unresolved target".into(),
                severity: DiagnosticSeverity::Gap,
            }],
            complete: false,
            ..UiFlow::default()
        };
        let ab = merge_ui_flows(a.clone(), b.clone());
        assert_eq!(ab, merge_ui_flows(b, a));
        let ids: Vec<_> = ab.screens.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(ab.diagnostics.len(), 1);
        assert!(!ab.complete);
        assert!(merge_ui_flows(UiFlow::default(), UiFlow::default()).complete);
    }

    #[test]
    fn serde_round_trip_of_a_full_flow() {
        let flow = UiFlow {
            screens: vec![screen(
                "home",
                Some(Condition::Not(Box::new(Condition::Unknown("x".into())))),
            )],
            transitions: vec![
                go(
                    "home",
                    "orders",
                    Some(Condition::Any(vec![role("a"), policy("p")])),
                ),
                Transition {
                    target: Target::Unresolved("url".into()),
                    trigger: Some("btn-next".into()),
                    ..go("home", "x", None)
                },
            ],
            actions: vec![
                http_action("GET", "/api/orders"),
                Action {
                    call: Call::Method("Svc.Load".into()),
                    condition: Some(Condition::All(vec![
                        Condition::Authenticated,
                        Condition::Custom("c".into()),
                    ])),
                    ..http_action("GET", "/")
                },
            ],
            endpoints: vec![
                endpoint("GET", "/api/orders", AccessState::Protected(role("a"))),
                endpoint("GET", "/health", AccessState::Open),
                endpoint("POST", "/api/x", AccessState::Undeclared),
            ],
            diagnostics: vec![UiFlowDiagnostic {
                file: "a".into(),
                line: 1,
                message: "m".into(),
                severity: DiagnosticSeverity::Note,
            }],
            complete: false,
            ..UiFlow::default()
        };
        let json = serde_json::to_string(&flow).unwrap();
        assert!(json.contains(r#""definition":"ui-flow/v1""#), "{json}");
        assert!(json.contains(r#""access":"undeclared""#), "{json}");
        assert!(json.contains(r#""target":{"unresolved":"url"}"#), "{json}");
        assert!(json.contains(r#""from":{"screen":"home"}"#), "{json}");
        assert!(json.contains(r#""severity":"note""#), "{json}");
        assert_eq!(serde_json::from_str::<UiFlow>(&json).unwrap(), flow);
    }

    #[test]
    fn json_shapes_of_the_refinements() {
        let any = Origin::AnyScreen {
            application: "Shop".into(),
        };
        assert_eq!(
            serde_json::to_string(&any).unwrap(),
            r#"{"any_screen":{"application":"Shop"}}"#
        );
        assert_eq!(
            serde_json::to_string(&Call::Unresolved("url".into())).unwrap(),
            r#"{"unresolved":"url"}"#
        );
        assert_eq!(
            serde_json::to_string(&Condition::Never).unwrap(),
            r#""never""#
        );
        let mut api = endpoint("ANY", "/x", AccessState::Protected(Condition::Never));
        api.application = Some("Api".into());
        let json = serde_json::to_string(&api).unwrap();
        assert!(json.contains(r#""application":"Api""#), "{json}");
        assert!(json.contains(r#""access":{"protected":"never"}"#), "{json}");
        // Files written before `severity` existed read every diagnostic as a gap.
        let old: UiFlowDiagnostic =
            serde_json::from_str(r#"{"file":"a","line":1,"message":"m"}"#).unwrap();
        assert_eq!(old.severity, DiagnosticSeverity::Gap);
    }
}
