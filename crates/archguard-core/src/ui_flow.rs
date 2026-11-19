//! `ui-flow/v1` (ADR 0021): screens, navigation, actions and backend access in
//! one framework-neutral model, plus the pure simulator over it.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const UI_FLOW_DEFINITION: &str = "ui-flow/v1";
/// `source` of every entry read from `.archguard/ui-flow.json`.
pub const UI_FLOW_IMPORT_SOURCE: &str = "import";

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Transition {
    pub from: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Action {
    pub screen: String,
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
    pub method: String,
    pub path: String,
    pub access: AccessState,
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
/// neither its condition nor the target screen's access is denied; unresolved
/// targets and unknown screen ids are skipped. Empty when `start` is unknown
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
            let (Some(&from), Target::Screen(to)) =
                (reached.get(&transition.from), &transition.target)
            else {
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

pub fn path_patterns_match(a: &str, b: &str) -> bool {
    normalize_path_pattern(a) == normalize_path_pattern(b)
}

/// Each HTTP action with the endpoints it hits (same method and path shape).
/// An empty list means no backend endpoint was found for the call.
pub fn join_actions_to_endpoints(flow: &UiFlow) -> Vec<(&Action, Vec<&EndpointAccess>)> {
    flow.actions
        .iter()
        .filter_map(|action| {
            let Call::Http { method, path } = &action.call else {
                return None;
            };
            let hits = flow
                .endpoints
                .iter()
                .filter(|e| {
                    e.method.eq_ignore_ascii_case(method) && path_patterns_match(&e.path, path)
                })
                .collect();
            Some((action, hits))
        })
        .collect()
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
            from: from.into(),
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
            screen: "orders".into(),
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
            }],
            complete: false,
            ..UiFlow::default()
        };
        let json = serde_json::to_string(&flow).unwrap();
        assert!(json.contains(r#""definition":"ui-flow/v1""#), "{json}");
        assert!(json.contains(r#""access":"undeclared""#), "{json}");
        assert!(json.contains(r#""target":{"unresolved":"url"}"#), "{json}");
        assert_eq!(serde_json::from_str::<UiFlow>(&json).unwrap(), flow);
    }
}
