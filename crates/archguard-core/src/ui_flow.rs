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
    /// Ids of screens an extractor or import marks as where the application
    /// starts (MAUI Shell, WPF `StartupUri`, ...); see [`start_screen_ids`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub start_screens: Vec<String>,
    /// Roles, permissions and groups the project defines; see [`resolve_assignment`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<RoleDef>,
    /// Named policies and what they require; see [`resolve_assignment`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policies: Vec<PolicyDef>,
    /// Where endpoint handlers are declared; see [`UiFlow::handler_site`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handler_sites: Vec<HandlerSite>,
    /// How each application writes and compares role and permission names;
    /// an application without an entry compares exactly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub role_conventions: Vec<RoleConvention>,
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
            start_screens: vec![],
            roles: vec![],
            policies: vec![],
            handler_sites: vec![],
            role_conventions: vec![],
        }
    }
}

impl UiFlow {
    /// The convention an element of `application` is evaluated under: its own
    /// entry (an element without application: the only application's, when
    /// the flow has exactly one), with the whole-flow entry filling what the
    /// own one did not read. Permission format and action aliases always
    /// inherit; prefixes, claim and case only when the flow has a single
    /// application, so no application's prefix reaches another's elements.
    /// Default (exact) when nothing was read.
    pub fn convention(&self, application: Option<&str>) -> RoleConvention {
        let entry = |app: Option<&str>| {
            (self.role_conventions.iter()).find(|c| c.application.as_deref() == app)
        };
        let apps = self.applications();
        let single = apps.len() <= 1;
        let own = match (application, apps.as_slice()) {
            (Some(a), _) => entry(Some(a)),
            (None, [one]) => entry(Some(one.as_str())),
            _ => None,
        };
        let mut c = own.cloned().unwrap_or_default();
        let Some(w) = entry(None) else {
            return c;
        };
        if own.is_none() && single {
            return w.clone();
        }
        if c.permissions == PermissionFormat::Exact {
            c.permissions = w.permissions.clone();
        }
        if c.action_aliases.is_empty() {
            c.action_aliases = w.action_aliases.clone();
        }
        if single {
            c.role_prefix = c.role_prefix.or_else(|| w.role_prefix.clone());
            c.scope_prefix = c.scope_prefix.or_else(|| w.scope_prefix.clone());
            c.role_claim = c.role_claim.or_else(|| w.role_claim.clone());
            if c.role_case == NameCase::Exact {
                c.role_case = w.role_case;
            }
            if c.policy_case == NameCase::Exact {
                c.policy_case = w.policy_case;
            }
        }
        c
    }

    /// The applications of screens and endpoints, sorted.
    pub fn applications(&self) -> Vec<String> {
        let apps: BTreeSet<&String> = (self.screens.iter().filter_map(|s| s.application.as_ref()))
            .chain(self.endpoints.iter().filter_map(|e| e.application.as_ref()))
            .collect();
        apps.into_iter().cloned().collect()
    }

    /// Where the handler method of `e` is declared, when the extractor read it.
    pub fn handler_site(&self, e: &EndpointAccess) -> Option<&HandlerSite> {
        (self.handler_sites.iter()).find(|s| {
            s.endpoint_file == e.file
                && s.endpoint_line == e.line
                && e.handler.as_deref() == Some(s.handler.as_str())
        })
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
    /// Guard on the screen itself; missing or `null` in a file reads as `undeclared`.
    #[serde(
        default,
        skip_serializing_if = "AccessState::is_undeclared",
        serialize_with = "AccessState::serialize_element"
    )]
    pub access: AccessState,
    /// The guard only holds as a rule of thumb (layout load, parent loader):
    /// shown as "weakly protected", never changes the outcome.
    #[serde(default, skip_serializing_if = "wire::is_false")]
    pub weak: bool,
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
    #[serde(
        default,
        skip_serializing_if = "AccessState::is_undeclared",
        serialize_with = "AccessState::serialize_element"
    )]
    pub condition: AccessState,
    /// See [`Screen::weak`].
    #[serde(default, skip_serializing_if = "wire::is_false")]
    pub weak: bool,
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
    #[serde(
        default,
        skip_serializing_if = "AccessState::is_undeclared",
        serialize_with = "AccessState::serialize_element"
    )]
    pub condition: AccessState,
    /// See [`Screen::weak`].
    #[serde(default, skip_serializing_if = "wire::is_false")]
    pub weak: bool,
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
    /// The handler method, e.g. `OrdersApi.CreateOrderAsync`; none for a lambda
    /// or when it was not found (ADR 0022, decision 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    /// Request types the handler sends as written
    /// (`IdentifiedCommand<CreateOrderCommand, bool>`), the wrapped request
    /// (`CreateOrderCommand`), and use-case classes it calls directly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<String>,
}

/// The declaration of an endpoint's handler method, e.g. a minimal-API
/// method group in another file. It sits on [`UiFlow`], not on
/// [`EndpointAccess`], so code that builds endpoints literally keeps
/// compiling; the endpoint is named by its `file`, `line` and `handler`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HandlerSite {
    pub endpoint_file: String,
    pub endpoint_line: u32,
    /// [`EndpointAccess::handler`].
    pub handler: String,
    pub file: String,
    pub line: u32,
}

mod wire {
    pub(super) fn is_false(b: &bool) -> bool {
        !*b
    }
}

/// Never two states: `undeclared` must not look `open` (ADR 0021, decision 2).
/// Reads a missing field or `null` as `undeclared`, `"open"`, `"undeclared"`,
/// `{"protected": <condition>}`, and a bare condition of older files as `protected`.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessState {
    Protected(Condition),
    /// Declared anonymous, e.g. `[AllowAnonymous]`, `permitAll`.
    Open,
    /// No declaration found; the code may still check by hand.
    #[default]
    Undeclared,
}

impl<'de> Deserialize<'de> for AccessState {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Current {
            Protected(Condition),
            Open,
            Undeclared,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Null(()),
            Current(Current),
            Legacy(Condition),
        }
        Ok(match Wire::deserialize(d)? {
            Wire::Null(()) | Wire::Current(Current::Undeclared) => AccessState::Undeclared,
            Wire::Current(Current::Open) => AccessState::Open,
            Wire::Current(Current::Protected(c)) | Wire::Legacy(c) => AccessState::Protected(c),
        })
    }
}

impl AccessState {
    /// A condition the extractor read wins over an open marker; neither is undeclared.
    pub fn from_parts(condition: Option<Condition>, open: bool) -> Self {
        match (condition, open) {
            (Some(c), _) => AccessState::Protected(c),
            (None, true) => AccessState::Open,
            (None, false) => AccessState::Undeclared,
        }
    }

    /// Screens, links and actions keep writing a protected state as the bare
    /// condition (files stay as before) and an open one as `"open"`.
    fn serialize_element<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            AccessState::Protected(c) => c.serialize(s),
            AccessState::Open => s.serialize_str("open"),
            AccessState::Undeclared => s.serialize_unit(),
        }
    }

    pub fn is_undeclared(&self) -> bool {
        *self == AccessState::Undeclared
    }

    /// The condition when protected.
    pub fn condition(&self) -> Option<&Condition> {
        match self {
            AccessState::Protected(c) => Some(c),
            _ => None,
        }
    }

    pub fn condition_mut(&mut self) -> Option<&mut Condition> {
        match self {
            AccessState::Protected(c) => Some(c),
            _ => None,
        }
    }

    /// Both layers must hold. Undeclared is the identity (so folding layers
    /// starts from it); `Open` adds no condition.
    pub fn and(self, other: AccessState) -> AccessState {
        use AccessState::*;
        match (self, other) {
            (Undeclared, x) | (x, Undeclared) => x,
            (Open, x) | (x, Open) => x,
            (Protected(a), Protected(b)) => {
                let mut items = match a {
                    Condition::All(items) => items,
                    a => vec![a],
                };
                match b {
                    Condition::All(more) => items.extend(more),
                    b => items.push(b),
                }
                Protected(Condition::All(items))
            }
        }
    }

    /// Fallback/default policy: applies only when no declaration gave a state.
    pub fn or_default(self, default: AccessState) -> AccessState {
        if self == AccessState::Undeclared {
            default
        } else {
            self
        }
    }
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
impl Condition {
    /// In words: `role Admin or policy P`, `signed in`, `not (…)`.
    pub fn text(&self) -> String {
        let join = |items: &[Condition], word: &str| {
            let parts: Vec<String> = (items.iter())
                .map(|c| match c {
                    Condition::All(_) | Condition::Any(_) => format!("({})", c.text()),
                    _ => c.text(),
                })
                .collect();
            parts.join(word)
        };
        match self {
            Condition::All(items) => join(items, " and "),
            Condition::Any(items) => join(items, " or "),
            Condition::Not(inner) => format!("not ({})", inner.text()),
            Condition::Authenticated => "signed in".into(),
            Condition::Never => "never".into(),
            Condition::Role(name) => format!("role {name}"),
            Condition::Policy(name) => format!("policy {name}"),
            Condition::Custom(name) => format!("custom {name}"),
            Condition::Unknown(text) => format!("unknown ({text})"),
        }
    }
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
    /// How role names compare; filled by [`resolve_assignment`], exact otherwise.
    pub convention: RoleConvention,
    /// The roles as chosen, before the catalog implied more; filled by
    /// [`resolve_assignment`] so each application can imply under its own
    /// convention.
    pub held: BTreeSet<String>,
    /// One resolved copy per application, for [`Assignment::in_app`].
    pub by_app: BTreeMap<Option<String>, Assignment>,
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
        Condition::Role(name) => known(Some(&holds(
            &assignment.convention,
            AtomKind::Role,
            &assignment.roles,
            name,
        ))),
        Condition::Policy(name) => known(assignment.policy_switch(name)),
        Condition::Custom(name) => known(assignment.custom.get(name)),
        Condition::Unknown(_) => Outcome::Unknown,
    }
}

impl Assignment {
    /// The switch of a policy; names compare by `policy_case`.
    fn policy_switch(&self, name: &str) -> Option<&bool> {
        match self.convention.policy_case {
            NameCase::Exact => self.policies.get(name),
            NameCase::Insensitive => (self.policies.iter())
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v),
        }
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
impl Assignment {
    /// The assignment an element of `application` is judged by: roles held
    /// and implied under that application's convention, never another's.
    pub fn in_app(&self, application: Option<&str>) -> &Assignment {
        (self.by_app.get(&application.map(String::from))).unwrap_or(self)
    }
}

pub fn evaluate_access(access: &AccessState, assignment: &Assignment) -> Outcome {
    match access {
        AccessState::Protected(condition) => evaluate_condition(condition, assignment),
        AccessState::Open => Outcome::Allowed,
        AccessState::Undeclared => Outcome::Unknown,
    }
}

/// The findings keep reading a missing declaration as "no guard".
fn evaluate_optional(
    access: &AccessState,
    application: Option<&str>,
    assignment: &Assignment,
) -> Outcome {
    match access {
        AccessState::Undeclared => Outcome::Allowed,
        other => evaluate_access(other, assignment.in_app(application)),
    }
}

/// Order is strength, strongest first: a path is as weak as its weakest step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    /// Some path has every guard allowed.
    Sure,
    /// Not denied, but some step has no declaration (not "open"); the code may
    /// still check by hand. Separate from `Possible`, which is an unknown guard.
    Undeclared,
    /// Reached only over paths with an unknown guard.
    Possible,
}

impl AccessState {
    /// Reach level of one screen or link; none when denied.
    fn level(&self, assignment: &Assignment, application: Option<&str>) -> Option<Reach> {
        let assignment = assignment.in_app(application);
        match self {
            AccessState::Undeclared => Some(Reach::Undeclared),
            other => match evaluate_access(other, assignment) {
                Outcome::Allowed => Some(Reach::Sure),
                Outcome::Unknown => Some(Reach::Possible),
                Outcome::Denied => None,
            },
        }
    }
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
    let assignment = &resolve_assignment(flow, assignment);
    let screens: BTreeMap<&str, &Screen> =
        flow.screens.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut reached = BTreeMap::new();
    let Some(first) =
        (screens.get(start)).and_then(|s| s.access.level(assignment, s.application.as_deref()))
    else {
        return reached;
    };
    reached.insert(start.to_string(), first);
    // Each screen improves at most three times (none -> possible -> undeclared -> sure), so this ends on cycles.
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
            let from_app = match &transition.from {
                Origin::Screen(id) => screens
                    .get(id.as_str())
                    .and_then(|s| s.application.as_deref()),
                Origin::AnyScreen { application } => Some(application.as_str()),
            };
            let (Some(link), Some(screen)) = (
                transition.condition.level(assignment, from_app),
                target
                    .access
                    .level(assignment, target.application.as_deref()),
            ) else {
                continue;
            };
            let via = from.max(link).max(screen); // the weakest step wins
            if reached.get(to).is_none_or(|old| via < *old) {
                reached.insert(to.clone(), via);
                changed = true;
            }
        }
    }
    reached
}

/// Reach of one screen in a simulation, with the reason when not reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reach", rename_all = "snake_case")]
pub enum ScreenReach {
    Sure,
    Possible,
    /// Reached, but a screen or link on the way has no declaration.
    Undeclared,
    /// The screen's own access evaluates denied.
    Denied,
    /// Not denied, but no resolved path from a start screen. The count of
    /// unresolved transitions of the application tells whether the gap may be
    /// the extractor's rather than the application's.
    NoKnownPath {
        unresolved_transitions_in_application: usize,
    },
    /// The application has no start screen (none marked, none routed `/`).
    NoStartScreen,
}

/// The start screens, in screen order: those listed in `flow.start_screens`
/// and those routed `/`. Simulation and findings start only here.
pub fn start_screen_ids(flow: &UiFlow) -> Vec<&str> {
    let routed =
        |s: &Screen| s.route.as_deref().map(normalize_path_pattern).as_deref() == Some("/");
    (flow.screens.iter())
        .filter(|s| flow.start_screens.contains(&s.id) || routed(s))
        .map(|s| s.id.as_str())
        .collect()
}

/// Best reach of every screen over all `starts`.
fn reach_from(flow: &UiFlow, starts: &[&str], assignment: &Assignment) -> BTreeMap<String, Reach> {
    let mut reached = BTreeMap::new();
    for start in starts {
        for (id, reach) in reachable_screens(flow, start, assignment) {
            let best = reached.entry(id).or_insert(reach);
            *best = (*best).min(reach);
        }
    }
    reached
}

/// Every screen's reach from the start screens ([`start_screen_ids`]), with
/// the reason when not reached.
pub fn screen_reaches(flow: &UiFlow, assignment: &Assignment) -> BTreeMap<String, ScreenReach> {
    let assignment = &resolve_assignment(flow, assignment);
    let app = |s: &Screen| s.application.clone().unwrap_or_default();
    let starts = start_screen_ids(flow);
    let screens: BTreeMap<&str, &Screen> =
        flow.screens.iter().map(|s| (s.id.as_str(), s)).collect();
    let started: BTreeSet<String> = (starts.iter().map(|id| app(screens[id]))).collect();
    let mut unresolved: BTreeMap<&str, usize> = BTreeMap::new();
    for t in &flow.transitions {
        let from_app = match &t.from {
            Origin::Screen(id) => application_of(&screens, id),
            Origin::AnyScreen { application } => Some(application.as_str()),
        };
        if let (Target::Unresolved(_), Some(a)) = (&t.target, from_app) {
            *unresolved.entry(a).or_default() += 1;
        }
    }
    let reached = reach_from(flow, &starts, assignment);
    let reason = |s: &Screen| match reached.get(&s.id) {
        Some(Reach::Sure) => ScreenReach::Sure,
        Some(Reach::Undeclared) => ScreenReach::Undeclared,
        Some(Reach::Possible) => ScreenReach::Possible,
        None if s
            .access
            .level(assignment, s.application.as_deref())
            .is_none() =>
        {
            ScreenReach::Denied
        }
        None if !started.contains(&app(s)) => ScreenReach::NoStartScreen,
        None => ScreenReach::NoKnownPath {
            unresolved_transitions_in_application: unresolved
                .get(app(s).as_str())
                .copied()
                .unwrap_or(0),
        },
    };
    (flow.screens.iter())
        .map(|s| (s.id.clone(), reason(s)))
        .collect()
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

/// How well the route `pattern` matches `path`: none when it does not, else
/// the number of literal segments that are equal (higher is more specific).
/// Segments match when both are parameters, when they are equal literals
/// (case-insensitive), or when the pattern has a parameter and the path a
/// literal (`/orders/:id` matches `/orders/42`). A parameter in `path` never
/// matches a literal in `pattern`, so pattern against pattern compares shape.
/// A trailing `*` or `**` (React `[...slug]`, Angular `**`) matches one or
/// more remaining segments; [`best_route`] and the join rank such a match
/// below every match without one.
pub fn path_match_score(pattern: &str, path: &str) -> Option<usize> {
    route_rank(pattern, path).map(|(_, literals)| literals)
}

/// `(no catch-all, equal literal segments)`: ordered so the better match is greater.
fn route_rank(pattern: &str, path: &str) -> Option<(bool, usize)> {
    let segments = |p: &str| -> Vec<String> {
        p.split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let parameter = |s: &str| s.starts_with(':') || (s.starts_with('{') && s.ends_with('}'));
    let (mut pattern, path) = (segments(pattern), segments(path));
    let catch_all = pattern.last().is_some_and(|s| s == "*" || s == "**");
    if catch_all {
        pattern.pop();
    }
    let fits = if catch_all {
        path.len() > pattern.len()
    } else {
        path.len() == pattern.len()
    };
    if !fits {
        return None;
    }
    let literals = pattern.iter().zip(&path).try_fold(0, |score, (p, s)| {
        match (parameter(p), parameter(s)) {
            (true, _) => Some(score),
            (false, false) if p == s => Some(score + 1),
            _ => None,
        }
    })?;
    Some((!catch_all, literals))
}

/// Whether the route `pattern` matches `path`; see [`path_match_score`].
pub fn path_patterns_match(pattern: &str, path: &str) -> bool {
    path_match_score(pattern, path).is_some()
}

/// The one candidate whose route matches `path` best (an exact literal route
/// beats a parameter route, any route beats a catch-all). `Err` holds the tied best candidates, empty when
/// none matches: a tie is never guessed. A candidate may come with several routes.
pub fn best_route<'a, T: PartialEq>(
    path: &str,
    candidates: impl IntoIterator<Item = (T, &'a str)>,
) -> Result<T, Vec<T>> {
    let (mut best, mut tied) = (None, Vec::new());
    for (candidate, route) in candidates {
        let Some(score) = route_rank(route, path) else {
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
    let scored: Vec<((bool, usize), &EndpointAccess)> = flow
        .endpoints
        .iter()
        .filter(|e| e.method == ANY_METHOD || e.method.eq_ignore_ascii_case(method))
        .filter_map(|e| route_rank(&e.path, path).map(|score| (score, e)))
        .collect();
    let mut best: BTreeMap<Option<&str>, (bool, usize)> = BTreeMap::new();
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
        start_screens: union(a.start_screens, b.start_screens),
        roles: union(a.roles, b.roles),
        policies: union(a.policies, b.policies),
        handler_sites: union(a.handler_sites, b.handler_sites),
        role_conventions: union(a.role_conventions, b.role_conventions),
    }
}

pub use roles::{
    AtomKind, ConventionOrigin, NameCase, PermissionFormat, PolicyDef, RoleConvention, RoleDef,
    RoleKind, canonical, holds, label, resolve_assignment, resolve_assignment_for,
};

/// The role and policy catalog and how the simulation applies it.
mod roles {
    use super::*;

    /// A role, permission or group the project defines, where it was read.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    pub struct RoleDef {
        pub name: String,
        pub kind: RoleKind,
        /// Names this one implies (`ROLE_ADMIN > ROLE_USER`: ADMIN includes USER).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub includes: Vec<String>,
        /// Owning application; none for a catalog of the whole flow.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub application: Option<String>,
        pub file: String,
        pub line: u32,
        pub source: String,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum RoleKind {
        Role,
        /// A permission id; dotted ids cover each other by [`PermissionFormat::Dotted`].
        Permission,
        Group,
        /// An OAuth scope or token authority.
        Scope,
    }

    /// A named policy (ASP.NET `AddPolicy`) and what it requires.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    pub struct PolicyDef {
        pub name: String,
        pub requires: Condition,
        /// Owning application; none for a catalog of the whole flow.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub application: Option<String>,
        pub file: String,
        pub line: u32,
        pub source: String,
    }

    /// How ONE application writes and compares role and permission names, as
    /// its extractor read it. Nothing read means exact comparison.
    #[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    pub struct RoleConvention {
        /// Same key as `Screen::application`; none is the whole flow.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub application: Option<String>,
        /// Prefix the framework puts before role names (Spring `ROLE_`); only
        /// used to canonicalize user input and shorten labels.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub role_prefix: Option<String>,
        /// Prefix for token scopes and authorities (Spring `SCOPE_`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub scope_prefix: Option<String>,
        /// Claim that carries the roles; information only, never compared.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub role_claim: Option<String>,
        #[serde(default)]
        pub role_case: NameCase,
        #[serde(default)]
        pub policy_case: NameCase,
        #[serde(default)]
        pub permissions: PermissionFormat,
        /// Action aliases (CanCanCan `read` -> `index`, `show`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub action_aliases: Vec<(String, Vec<String>)>,
        /// Where each field came from.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub origins: Vec<ConventionOrigin>,
    }

    /// Where one [`RoleConvention`] field was read: a file position or
    /// `framework default <name> <version>` in `source`.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    pub struct ConventionOrigin {
        pub field: String,
        pub file: String,
        pub line: u32,
        pub source: String,
    }

    #[derive(
        Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
    )]
    #[serde(rename_all = "snake_case")]
    pub enum NameCase {
        #[default]
        Exact,
        Insensitive,
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum PermissionFormat {
        #[default]
        Exact,
        /// Dotted ids; a held id covers `wanted` when its segments are a
        /// prefix of `wanted`'s and each is equal or `wildcard` (Squidex:
        /// `app.{app}.*` and `app.{app}` cover `app.{app}.contents.read`).
        Dotted { wildcard: String },
        /// `"<action> <subject>"` (CASL, CanCanCan).
        ActionSubject {
            any_action: String,
            any_subject: String,
        },
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum AtomKind {
        Role,
        Policy,
    }

    fn same(case: NameCase, a: &str, b: &str) -> bool {
        match case {
            NameCase::Exact => a == b,
            NameCase::Insensitive => a.to_lowercase() == b.to_lowercase(),
        }
    }

    fn case_of(conv: &RoleConvention, kind: AtomKind) -> NameCase {
        match kind {
            AtomKind::Role => conv.role_case,
            AtomKind::Policy => conv.policy_case,
        }
    }

    /// Whether the held names satisfy `wanted` under the application's
    /// convention. The only place that compares names. A prefix is never
    /// guessed (names are stored as the runtime authority) and hierarchy is
    /// expanded beforehand by [`resolve_assignment`]. The permission format
    /// applies to both kinds: Squidex ids are roles today.
    pub fn holds(
        conv: &RoleConvention,
        kind: AtomKind,
        held: &BTreeSet<String>,
        wanted: &str,
    ) -> bool {
        let case = case_of(conv, kind);
        if case == NameCase::Exact && held.contains(wanted) {
            return true;
        }
        if held.iter().any(|h| same(case, h, wanted)) {
            return true;
        }
        // a typed name meets the stored authority through this convention's prefix
        if kind == AtomKind::Role
            && (held.iter()).any(|h| same(case, &canonical(conv, kind, h), wanted))
        {
            return true;
        }
        match &conv.permissions {
            PermissionFormat::Exact => false,
            PermissionFormat::Dotted { wildcard } => {
                (held.iter()).any(|h| dotted_covers(case, wildcard, h, wanted))
            }
            PermissionFormat::ActionSubject {
                any_action,
                any_subject,
            } => (held.iter())
                .any(|h| action_subject_covers(conv, case, any_action, any_subject, h, wanted)),
        }
    }

    fn dotted_covers(case: NameCase, wildcard: &str, held: &str, wanted: &str) -> bool {
        let h: Vec<&str> = held.split('.').collect();
        let w: Vec<&str> = wanted.split('.').collect();
        !same(case, held, wanted)
            && h.len() <= w.len()
            && h.iter()
                .zip(&w)
                .all(|(h, w)| *h == wildcard || same(case, h, w))
    }

    fn action_subject_covers(
        conv: &RoleConvention,
        case: NameCase,
        any_action: &str,
        any_subject: &str,
        held: &str,
        wanted: &str,
    ) -> bool {
        let (Some((ha, hs)), Some((wa, ws))) = (held.split_once(' '), wanted.split_once(' '))
        else {
            return false;
        };
        let action = same(case, ha, any_action)
            || same(case, ha, wa)
            || (conv.action_aliases.iter()).any(|(alias, actions)| {
                same(case, alias, ha) && actions.iter().any(|a| same(case, a, wa))
            });
        action && (same(case, hs, any_subject) || same(case, hs, ws))
    }

    fn strip_prefix_case<'a>(case: NameCase, prefix: &str, text: &'a str) -> Option<&'a str> {
        let head = text.get(..prefix.len())?;
        same(case, head, prefix).then(|| &text[prefix.len()..])
    }

    /// A user input (`--role ADMIN`) in the stored form (`ROLE_ADMIN`).
    /// Policies are stored as written.
    pub fn canonical(conv: &RoleConvention, kind: AtomKind, input: &str) -> String {
        match (kind, &conv.role_prefix) {
            (AtomKind::Role, Some(p)) if strip_prefix_case(conv.role_case, p, input).is_none() => {
                format!("{p}{input}")
            }
            _ => input.to_string(),
        }
    }

    /// The stored name without its prefix, for display only, never compared.
    pub fn label(conv: &RoleConvention, kind: AtomKind, stored: &str) -> String {
        let (prefix, case) = match kind {
            AtomKind::Role => (&conv.role_prefix, conv.role_case),
            AtomKind::Policy => (&conv.scope_prefix, conv.policy_case),
        };
        (prefix.as_deref())
            .and_then(|p| strip_prefix_case(case, p, stored))
            .unwrap_or(stored)
            .to_string()
    }

    /// The assignment the simulator evaluates, with the convention of the whole
    /// flow; see [`resolve_assignment_for`].
    pub fn resolve_assignment(flow: &UiFlow, assignment: &Assignment) -> Assignment {
        let mut out = resolve_assignment_for(flow, None, assignment);
        out.by_app = (flow.applications().into_iter())
            .map(|app| {
                let one = resolve_assignment_for(flow, Some(&app), assignment);
                (Some(app), one)
            })
            .collect();
        out
    }

    /// The assignment the simulator evaluates for one application's elements:
    /// it carries that application's [`RoleConvention`], every held role adds
    /// the roles it `includes` (transitively, cycle-safe) and the permission
    /// defs it covers, and every defined policy the assignment does not switch
    /// explicitly is set from what it requires, when that is known. Several
    /// defs of one name that disagree leave the policy unknown. Idempotent;
    /// [`reachable_screens`], [`screen_reaches`] and the findings apply it,
    /// [`evaluate_condition`] and [`evaluate_access`] take it as given.
    pub fn resolve_assignment_for(
        flow: &UiFlow,
        application: Option<&str>,
        assignment: &Assignment,
    ) -> Assignment {
        let mut out = assignment.clone();
        out.by_app.clear();
        // nothing defined or read: nothing is implied, the input stays as it is
        if !flow.roles.is_empty() || !flow.role_conventions.is_empty() {
            if out.held.is_empty() {
                out.held = out.roles.clone();
            }
            out.roles = out.held.clone();
        }
        out.convention = flow.convention(application);
        imply_roles(flow, &out.convention, &mut out.roles);
        resolve_policies(flow, &mut out);
        out
    }

    /// Adds the roles every held role includes and the permission defs it covers.
    fn imply_roles(flow: &UiFlow, conv: &RoleConvention, roles: &mut BTreeSet<String>) {
        let mut todo: Vec<String> = roles.iter().cloned().collect();
        while let Some(held) = todo.pop() {
            let one = BTreeSet::from([held.clone()]);
            let matching = flow.roles.iter().filter(|d| match d.kind {
                RoleKind::Permission => holds(conv, AtomKind::Role, &one, &d.name),
                _ => {
                    let stored = canonical(conv, AtomKind::Role, &held);
                    same(conv.role_case, &d.name, &held) || same(conv.role_case, &d.name, &stored)
                }
            });
            for d in matching {
                for name in d.includes.iter().chain([&d.name]) {
                    if roles.insert(name.clone()) {
                        todo.push(name.clone());
                    }
                }
            }
        }
    }

    /// Sets each defined policy the assignment leaves open, in rounds until nothing
    /// changes, so a policy may require another; a cycle stays unknown.
    fn resolve_policies(flow: &UiFlow, out: &mut Assignment) {
        let names: BTreeSet<&str> = flow.policies.iter().map(|p| p.name.as_str()).collect();
        loop {
            let open = names.iter().filter(|n| !out.policies.contains_key(**n));
            let known: Vec<(&str, bool)> = open
                .filter_map(|&name| Some((name, policy_value(flow, name, out)?)))
                .collect();
            if known.is_empty() {
                return;
            }
            for (name, value) in known {
                out.policies.insert(name.to_string(), value);
            }
        }
    }

    /// The policy's value when all its defs agree on a known outcome.
    fn policy_value(flow: &UiFlow, name: &str, assignment: &Assignment) -> Option<bool> {
        let outcomes: BTreeSet<Outcome> = (flow.policies.iter())
            .filter(|p| p.name == name)
            .map(|p| evaluate_condition(&p.requires, assignment))
            .collect();
        match outcomes.iter().collect::<Vec<_>>()[..] {
            [Outcome::Allowed] => Some(true),
            [Outcome::Denied] => Some(false),
            _ => None,
        }
    }
}

pub use findings::{
    EndpointCounts, SimulatedRole, UiFlowActionRef, UiFlowEndpointRef, UiFlowFinding,
    UiFlowFindingKind, UiFlowSummary, ui_flow_findings, ui_flow_summary,
};

/// Frontend/backend mismatch findings (ADR 0021, third question) and the
/// summary line of the flow view.
mod findings {
    use super::*;
    use crate::model::Severity;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum UiFlowFindingKind {
        /// The role reaches the action, but every matched endpoint refuses it.
        ShownButRefused,
        /// The frontend guards the action, but an endpoint is open or undeclared.
        HiddenButUnprotected,
        /// No scanned endpoint matches the HTTP call; the backend may be outside the scan.
        NoBackendEndpoint,
        /// The application has no start screen (none marked, none routed `/`);
        /// no reachability for it.
        NoStartScreen,
    }

    /// A role the findings simulate; policy and custom atoms stay unknown.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum SimulatedRole {
        Anonymous,
        /// Signed in without any role.
        Authenticated,
        /// Signed in with this one role.
        Role(String),
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    pub struct UiFlowActionRef {
        pub file: String,
        pub line: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub trigger: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    pub struct UiFlowEndpointRef {
        pub method: String,
        pub path: String,
        pub file: String,
        pub line: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub application: Option<String>,
    }

    /// Field order is the sort order: warnings first, then kind, then place.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    pub struct UiFlowFinding {
        /// `warning` for a mismatch, `info` for a note.
        pub severity: Severity,
        pub kind: UiFlowFindingKind,
        /// Only for `no_start_screen`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub application: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub action: Option<UiFlowActionRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub role: Option<SimulatedRole>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub endpoints: Vec<UiFlowEndpointRef>,
        pub message: String,
    }

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct EndpointCounts {
        pub protected: usize,
        pub open: usize,
        pub undeclared: usize,
    }

    /// The "12 of 13 navigations resolved" line of the flow view.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct UiFlowSummary {
        pub screens: usize,
        pub transitions_resolved: usize,
        pub transitions_unresolved: usize,
        pub actions: usize,
        pub actions_unresolved: usize,
        /// Undeclared endpoints are counted here instead of one finding each.
        pub endpoints_by_access: EndpointCounts,
        pub complete: bool,
    }

    pub fn ui_flow_summary(flow: &UiFlow) -> UiFlowSummary {
        let mut access = EndpointCounts::default();
        for endpoint in &flow.endpoints {
            *match endpoint.access {
                AccessState::Protected(_) => &mut access.protected,
                AccessState::Open => &mut access.open,
                AccessState::Undeclared => &mut access.undeclared,
            } += 1;
        }
        let unresolved = |t: &&Transition| matches!(t.target, Target::Unresolved(_));
        let transitions_unresolved = flow.transitions.iter().filter(unresolved).count();
        UiFlowSummary {
            screens: flow.screens.len(),
            transitions_resolved: flow.transitions.len() - transitions_unresolved,
            transitions_unresolved,
            actions: flow.actions.len(),
            actions_unresolved: (flow.actions.iter())
                .filter(|a| matches!(a.call, Call::Unresolved(_)))
                .count(),
            endpoints_by_access: access,
            complete: flow.complete,
        }
    }

    /// Mismatch findings, sorted. Simulates anonymous, signed in without a
    /// role, and each role named anywhere in the flow alone; policy and custom
    /// atoms stay unknown. Reachability starts at every start screen
    /// ([`start_screen_ids`]); an application without one gets a note and is
    /// not reached from itself.
    /// One finding per action and kind, naming the first role that shows it.
    pub fn ui_flow_findings(flow: &UiFlow) -> Vec<UiFlowFinding> {
        let (starts, mut findings) = start_screens(flow);
        let roles = simulated_roles(flow);
        let reached: Vec<(SimulatedRole, Assignment, BTreeMap<String, Reach>)> = roles
            .into_iter()
            .map(|role| {
                let assignment = resolve_assignment(flow, &assignment_of(&role));
                let reached = reach_from(flow, &starts, &assignment);
                (role, assignment, reached)
            })
            .collect();
        for (action, hits) in join_actions_to_endpoints(flow) {
            if hits.is_empty() {
                let text = format!(
                    "No scanned endpoint matches {}; the backend may be outside the scan.",
                    call_text(action)
                );
                findings.push(finding(
                    UiFlowFindingKind::NoBackendEndpoint,
                    action,
                    None,
                    &[],
                    text,
                ));
                continue;
            }
            let refused = reached.iter().find(|(_, assignment, reached)| {
                shown(flow, action, assignment, reached)
                    && (hits.iter()).all(|e| {
                        evaluate_access(&e.access, assignment.in_app(e.application.as_deref()))
                            == Outcome::Denied
                    })
            });
            if let Some((role, ..)) = refused {
                let text = format!(
                    "{} reaches the action calling {}, but the backend refuses it.",
                    role_text(role),
                    call_text(action)
                );
                findings.push(finding(
                    UiFlowFindingKind::ShownButRefused,
                    action,
                    Some(role.clone()),
                    &hits,
                    text,
                ));
            }
            let unprotected: Vec<&EndpointAccess> = (hits.iter().copied())
                .filter(|e| matches!(e.access, AccessState::Open | AccessState::Undeclared))
                .collect();
            if !unprotected.is_empty() && guarded_for_anonymous(flow, action) {
                let text = format!(
                    "The frontend guards the action calling {}, but the backend does not declare a guard.",
                    call_text(action)
                );
                findings.push(finding(
                    UiFlowFindingKind::HiddenButUnprotected,
                    action,
                    Some(SimulatedRole::Anonymous),
                    &unprotected,
                    text,
                ));
            }
        }
        findings.sort();
        findings
    }

    /// The start screen ids, and a note for each application without one.
    fn start_screens(flow: &UiFlow) -> (Vec<&str>, Vec<UiFlowFinding>) {
        let starts = start_screen_ids(flow);
        fn app(s: &Screen) -> &str {
            s.application.as_deref().unwrap_or("")
        }
        let apps: BTreeSet<&str> = flow.screens.iter().map(app).collect();
        let started: BTreeSet<&str> = (flow.screens.iter())
            .filter(|s| starts.contains(&s.id.as_str()))
            .map(app)
            .collect();
        let notes = apps
            .into_iter()
            .filter(|a| !started.contains(a))
            .map(|app| UiFlowFinding {
                severity: Severity::Info,
                kind: UiFlowFindingKind::NoStartScreen,
                application: Some(app.to_string()),
                action: None,
                role: None,
                endpoints: vec![],
                message: format!("Application {app:?} has no start screen (none routed `/` or marked as start), so its reachability is not simulated."),
            })
            .collect();
        (starts, notes)
    }

    /// Anonymous, authenticated, then every role name in the flow's conditions
    /// and policy requirements, sorted.
    fn simulated_roles(flow: &UiFlow) -> Vec<SimulatedRole> {
        fn collect(condition: &Condition, names: &mut BTreeSet<String>) {
            match condition {
                Condition::All(items) | Condition::Any(items) => {
                    items.iter().for_each(|c| collect(c, names))
                }
                Condition::Not(inner) => collect(inner, names),
                Condition::Role(name) => {
                    names.insert(name.clone());
                }
                _ => {}
            }
        }
        let mut names = BTreeSet::new();
        let conditions = (flow.screens.iter().filter_map(|s| s.access.condition()))
            .chain(
                flow.transitions
                    .iter()
                    .filter_map(|t| t.condition.condition()),
            )
            .chain(flow.actions.iter().filter_map(|a| a.condition.condition()))
            .chain(flow.endpoints.iter().filter_map(|e| match &e.access {
                AccessState::Protected(c) => Some(c),
                _ => None,
            }))
            .chain(flow.policies.iter().map(|p| &p.requires));
        conditions.for_each(|c| collect(c, &mut names));
        [SimulatedRole::Anonymous, SimulatedRole::Authenticated]
            .into_iter()
            .chain(names.into_iter().map(SimulatedRole::Role))
            .collect()
    }

    fn assignment_of(role: &SimulatedRole) -> Assignment {
        Assignment {
            authenticated: *role != SimulatedRole::Anonymous,
            roles: match role {
                SimulatedRole::Role(name) => BTreeSet::from([name.clone()]),
                _ => BTreeSet::new(),
            },
            ..Assignment::default()
        }
    }

    /// The action's screen is reached for sure (for `any_screen`, some screen
    /// of its application is) and its own condition is not denied.
    fn shown(
        flow: &UiFlow,
        action: &Action,
        assignment: &Assignment,
        reached: &BTreeMap<String, Reach>,
    ) -> bool {
        // An undeclared step counts as unguarded here, as before `Reach::Undeclared`.
        let sure = |id: &String| matches!(reached.get(id), Some(Reach::Sure | Reach::Undeclared));
        let on_sure_screen = match &action.screen {
            Origin::Screen(id) => sure(id),
            Origin::AnyScreen { application } => flow
                .screens
                .iter()
                .any(|s| s.application.as_deref().unwrap_or("") == application && sure(&s.id)),
        };
        on_sure_screen
            && evaluate_optional(&action.condition, action_app(flow, action), assignment)
                != Outcome::Denied
    }

    fn action_app<'a>(flow: &'a UiFlow, action: &'a Action) -> Option<&'a str> {
        match &action.screen {
            Origin::Screen(id) => {
                (flow.screens.iter().find(|s| s.id == *id)).and_then(|s| s.application.as_deref())
            }
            Origin::AnyScreen { application } => Some(application.as_str()),
        }
    }

    /// The screen's access and the action's condition deny an anonymous user.
    fn guarded_for_anonymous(flow: &UiFlow, action: &Action) -> bool {
        let anonymous = resolve_assignment(flow, &Assignment::default());
        let screen_access = match &action.screen {
            Origin::Screen(id) => (flow.screens.iter().find(|s| s.id == *id))
                .map_or(AccessState::Undeclared, |s| s.access.clone()),
            Origin::AnyScreen { .. } => AccessState::Undeclared,
        };
        and(
            evaluate_optional(&screen_access, action_app(flow, action), &anonymous),
            evaluate_optional(&action.condition, action_app(flow, action), &anonymous),
        ) == Outcome::Denied
    }

    fn finding(
        kind: UiFlowFindingKind,
        action: &Action,
        role: Option<SimulatedRole>,
        endpoints: &[&EndpointAccess],
        message: String,
    ) -> UiFlowFinding {
        let severity = match kind {
            UiFlowFindingKind::ShownButRefused | UiFlowFindingKind::HiddenButUnprotected => {
                Severity::Warning
            }
            _ => Severity::Info,
        };
        UiFlowFinding {
            severity,
            kind,
            application: None,
            action: Some(UiFlowActionRef {
                file: action.file.clone(),
                line: action.line,
                trigger: action.trigger.clone(),
            }),
            role,
            endpoints: (endpoints.iter())
                .map(|e| UiFlowEndpointRef {
                    method: e.method.clone(),
                    path: e.path.clone(),
                    file: e.file.clone(),
                    line: e.line,
                    application: e.application.clone(),
                })
                .collect(),
            message,
        }
    }

    fn call_text(action: &Action) -> String {
        match &action.call {
            Call::Http { method, path } => format!("{method} {path}"),
            Call::Method(id) | Call::Unresolved(id) => id.clone(),
        }
    }

    fn role_text(role: &SimulatedRole) -> String {
        match role {
            SimulatedRole::Anonymous => "An anonymous user".into(),
            SimulatedRole::Authenticated => "A signed-in user without roles".into(),
            SimulatedRole::Role(name) => format!("Role {name:?}"),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn screen(id: &str, app: &str, route: &str, access: Option<Condition>) -> Screen {
            Screen {
                weak: false,
                id: id.into(),
                name: id.into(),
                kind: ScreenKind::Page,
                route: Some(route.into()),
                application: Some(app.into()),
                file: format!("{id}.razor"),
                line: 1,
                access: AccessState::from_parts(access, false),
                source: "test".into(),
            }
        }
        fn go(from: &str, to: Target) -> Transition {
            Transition {
                weak: false,
                from: Origin::Screen(from.into()),
                target: to,
                trigger: None,
                file: "Home.razor".into(),
                line: 2,
                condition: AccessState::Undeclared,
                source: "test".into(),
            }
        }
        fn act(screen: Origin, line: u32, call: Call, condition: Option<Condition>) -> Action {
            Action {
                weak: false,
                screen,
                trigger: Some(format!("t{line}")),
                call,
                file: "Ui.razor".into(),
                line,
                condition: AccessState::from_parts(condition, false),
                source: "test".into(),
            }
        }
        fn http(method: &str, path: &str) -> Call {
            Call::Http {
                method: method.into(),
                path: path.into(),
            }
        }
        fn api(method: &str, path: &str, line: u32, access: AccessState) -> EndpointAccess {
            EndpointAccess {
                method: method.into(),
                path: path.into(),
                access,
                application: Some("api".into()),
                file: "Api.cs".into(),
                line,
                source: "test".into(),
                handler: None,
                sends: vec![],
            }
        }
        fn on(id: &str) -> Origin {
            Origin::Screen(id.into())
        }
        fn any_web() -> Origin {
            Origin::AnyScreen {
                application: "web".into(),
            }
        }
        fn role(name: &str) -> Condition {
            Condition::Role(name.into())
        }
        fn guarded(condition: Condition) -> AccessState {
            AccessState::Protected(condition)
        }

        /// `web` starts at `home`; `other` has no start screen.
        fn fixture() -> UiFlow {
            UiFlow {
                screens: vec![
                    screen("home", "web", "/", None),
                    screen("orders", "web", "/orders", Some(Condition::Authenticated)),
                    screen("admin", "web", "/admin/", Some(role("Admin"))),
                    screen("lone", "other", "/lone", None),
                ],
                transitions: vec![
                    go("home", Target::Screen("orders".into())),
                    go("home", Target::Screen("admin".into())),
                    go("home", Target::Unresolved("url".into())),
                ],
                actions: vec![
                    // Every signed-in user sees it; only `Manager` may call it.
                    act(
                        on("orders"),
                        10,
                        http("POST", "/api/orders/42/cancel"),
                        None,
                    ),
                    // `Banned` appears only in the endpoint.
                    act(on("home"), 11, http("POST", "/api/feedback"), None),
                    act(on("admin"), 12, http("DELETE", "/api/cache"), None),
                    act(
                        any_web(),
                        13,
                        http("POST", "/api/logout"),
                        Some(Condition::Authenticated),
                    ),
                    act(any_web(), 14, http("GET", "/api/basket"), None),
                    act(on("home"), 15, http("GET", "/api/missing"), None),
                    act(on("home"), 16, Call::Unresolved("url".into()), None),
                    // Not reached: `other` has no start screen.
                    act(on("lone"), 17, http("POST", "/api/never"), None),
                    // An unknown endpoint guard decides nothing.
                    act(on("home"), 18, http("GET", "/api/unknown"), None),
                    act(on("home"), 19, Call::Method("Svc.Load".into()), None),
                ],
                endpoints: vec![
                    api(
                        "POST",
                        "/api/orders/{id}/cancel",
                        1,
                        guarded(role("Manager")),
                    ),
                    api(
                        "POST",
                        "/api/feedback",
                        2,
                        guarded(Condition::Not(Box::new(role("Banned")))),
                    ),
                    api("DELETE", "/api/cache", 3, AccessState::Undeclared),
                    api("POST", "/api/logout", 4, AccessState::Open),
                    api("GET", "/api/basket", 5, guarded(Condition::Never)),
                    api("POST", "/api/never", 6, guarded(Condition::Never)),
                    api(
                        "GET",
                        "/api/unknown",
                        7,
                        guarded(Condition::Policy("P".into())),
                    ),
                    api("GET", "/api/health", 8, AccessState::Open),
                ],
                complete: false,
                ..UiFlow::default()
            }
        }

        fn rows(
            findings: &[UiFlowFinding],
        ) -> Vec<(UiFlowFindingKind, Option<u32>, Option<SimulatedRole>)> {
            findings
                .iter()
                .map(|f| (f.kind, f.action.as_ref().map(|a| a.line), f.role.clone()))
                .collect()
        }

        #[test]
        fn every_kind_with_its_role_in_a_fixed_order() {
            use SimulatedRole::*;
            use UiFlowFindingKind::*;
            let findings = ui_flow_findings(&fixture());
            assert_eq!(
                rows(&findings),
                vec![
                    (ShownButRefused, Some(10), Some(Authenticated)),
                    (ShownButRefused, Some(11), Some(Role("Banned".into()))),
                    (ShownButRefused, Some(14), Some(Anonymous)),
                    (HiddenButUnprotected, Some(12), Some(Anonymous)),
                    (HiddenButUnprotected, Some(13), Some(Anonymous)),
                    (NoBackendEndpoint, Some(15), None),
                    (NoStartScreen, None, None),
                ]
            );
            assert!(
                findings[..5]
                    .iter()
                    .all(|f| f.severity == Severity::Warning)
            );
            assert!(findings[5..].iter().all(|f| f.severity == Severity::Info));
            assert_eq!(findings[6].application.as_deref(), Some("other"));
            assert!(findings.iter().all(|f| !f.message.is_empty()));
            let refused = &findings[0];
            assert_eq!(
                refused.action.as_ref().unwrap().trigger.as_deref(),
                Some("t10")
            );
            assert_eq!(
                refused.endpoints,
                vec![UiFlowEndpointRef {
                    method: "POST".into(),
                    path: "/api/orders/{id}/cancel".into(),
                    file: "Api.cs".into(),
                    line: 1,
                    application: Some("api".into()),
                }]
            );
            assert_eq!(findings[3].endpoints[0].line, 3);
            assert!(findings[5].endpoints.is_empty());
        }

        #[test]
        fn denied_action_condition_is_not_shown_and_an_allowed_endpoint_clears_it() {
            let mut flow = fixture();
            // The action hides itself from everyone, so no role sees it refused.
            flow.actions[0].condition = AccessState::Protected(Condition::Never);
            // A second endpoint of the same shape that admits everyone signed in.
            flow.endpoints.push(EndpointAccess {
                application: Some("api2".into()),
                ..api(
                    "POST",
                    "/api/feedback",
                    9,
                    guarded(Condition::Authenticated),
                )
            });
            let lines: Vec<_> = ui_flow_findings(&flow)
                .iter()
                .filter(|f| f.kind == UiFlowFindingKind::ShownButRefused)
                .map(|f| f.action.as_ref().unwrap().line)
                .collect();
            // Line 11 goes: for `Banned` the second endpoint admits it.
            assert_eq!(lines, vec![14]);
        }

        #[test]
        fn findings_serialize_compactly() {
            let findings = ui_flow_findings(&fixture());
            let json = serde_json::to_value(&findings[1]).unwrap();
            assert_eq!(json["kind"], "shown_but_refused");
            assert_eq!(json["severity"], "warning");
            assert_eq!(json["role"], serde_json::json!({"role": "Banned"}));
            assert!(json.get("application").is_none());
            let anonymous = serde_json::to_value(&findings[2]).unwrap();
            assert_eq!(anonymous["role"], "anonymous");
            let back: UiFlowFinding = serde_json::from_value(json).unwrap();
            assert_eq!(back, findings[1]);
        }

        #[test]
        fn summary_counts_navigations_actions_and_access() {
            assert_eq!(
                ui_flow_summary(&fixture()),
                UiFlowSummary {
                    screens: 4,
                    transitions_resolved: 2,
                    transitions_unresolved: 1,
                    actions: 10,
                    actions_unresolved: 1,
                    endpoints_by_access: EndpointCounts {
                        protected: 5,
                        open: 2,
                        undeclared: 1,
                    },
                    complete: false,
                }
            );
        }
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
            weak: false,
            id: id.into(),
            name: id.into(),
            kind: ScreenKind::Page,
            route: Some(format!("/{id}")),
            application: None,
            file: "App.razor".into(),
            line: 1,
            access: AccessState::from_parts(access, false),
            source: "test".into(),
        }
    }
    fn go(from: &str, to: &str, condition: Option<Condition>) -> Transition {
        Transition {
            weak: false,
            from: Origin::Screen(from.into()),
            target: Target::Screen(to.into()),
            trigger: None,
            file: "App.razor".into(),
            line: 2,
            condition: AccessState::from_parts(condition, false),
            source: "test".into(),
        }
    }
    fn http_action(method: &str, path: &str) -> Action {
        Action {
            weak: false,
            screen: Origin::Screen("orders".into()),
            trigger: Some("save".into()),
            call: Call::Http {
                method: method.into(),
                path: path.into(),
            },
            file: "Orders.razor".into(),
            line: 9,
            condition: AccessState::Undeclared,
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
            handler: None,
            sends: vec![],
        }
    }

    #[test]
    fn endpoint_without_handler_fields_reads_and_writes_as_before() {
        let old = r#"{"method":"GET","path":"/a","access":"undeclared","file":"Api.cs","line":3,"source":"test"}"#;
        let e: EndpointAccess = serde_json::from_str(old).unwrap();
        assert_eq!(e, endpoint("GET", "/a", AccessState::Undeclared));
        assert_eq!(serde_json::to_string(&e).unwrap(), old);
    }

    #[test]
    fn garbage_access_is_rejected_not_read_as_open() {
        for bad in [r#""foo""#, "{}"] {
            let json = format!(
                r#"{{"method":"GET","path":"/a","access":{bad},"file":"Api.cs","line":3,"source":"test"}}"#
            );
            assert!(
                serde_json::from_str::<EndpointAccess>(&json).is_err(),
                "{bad}"
            );
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
                ("home".to_string(), Reach::Undeclared),
                ("orders".to_string(), Reach::Undeclared),
                ("beta".to_string(), Reach::Possible),
                ("beta_detail".to_string(), Reach::Possible),
            ])
        );
        let reached = reachable_screens(&flow, "home", &admin());
        assert_eq!(reached.get("admin"), Some(&Reach::Undeclared));
        // A sure path upgrades a screen first seen as possible.
        let mut upgraded = flow.clone();
        upgraded.transitions.push(go("orders", "beta_detail", None));
        let reached = reachable_screens(&upgraded, "home", &user);
        assert_eq!(reached.get("beta_detail"), Some(&Reach::Undeclared));
        assert!(reachable_screens(&flow, "nowhere", &user).is_empty());
        assert!(reachable_screens(&flow, "admin", &user).is_empty());
    }

    #[test]
    fn every_screen_gets_a_reach_with_its_reason() {
        let in_app = |app: &str, s: Screen| Screen {
            application: Some(app.into()),
            ..s
        };
        let start = |s: Screen| Screen {
            route: Some("/".into()),
            ..s
        };
        let unresolved = |from: &str| Transition {
            target: Target::Unresolved("Url.Item(id)".into()),
            ..go(from, "x", None)
        };
        let flow = UiFlow {
            screens: vec![
                in_app("shop", start(screen("home", None))),
                in_app("shop", screen("beta", None)),
                in_app("shop", screen("admin", Some(role("admin")))),
                in_app("shop", screen("item", None)),
                in_app("shop", screen("hidden", None)),
                in_app("tools", screen("help", None)),
                in_app("tools", screen("locked", Some(Condition::Never))),
            ],
            transitions: vec![
                go("home", "beta", Some(Condition::Custom("beta".into()))),
                go("home", "admin", None),
                go("home", "hidden", Some(Condition::Never)),
                unresolved("home"),
                unresolved("beta"),
                unresolved("help"),
            ],
            ..UiFlow::default()
        };
        let reaches = screen_reaches(&flow, &Assignment::default());
        let no_path = |n| ScreenReach::NoKnownPath {
            unresolved_transitions_in_application: n,
        };
        assert_eq!(
            reaches,
            BTreeMap::from([
                ("home".to_string(), ScreenReach::Undeclared),
                ("beta".to_string(), ScreenReach::Possible),
                ("admin".to_string(), ScreenReach::Denied),
                // Open, but only an unresolved link could lead there.
                ("item".to_string(), no_path(2)),
                // Behind a denied transition, not behind its own access.
                ("hidden".to_string(), no_path(2)),
                ("help".to_string(), ScreenReach::NoStartScreen),
                ("locked".to_string(), ScreenReach::Denied),
            ])
        );
        assert_eq!(
            screen_reaches(&flow, &admin())["admin"],
            ScreenReach::Undeclared
        );
        assert_eq!(
            serde_json::to_value(no_path(2)).unwrap(),
            serde_json::json!({"reach": "no_known_path", "unresolved_transitions_in_application": 2})
        );
        assert_eq!(
            serde_json::to_value(ScreenReach::Denied).unwrap(),
            serde_json::json!({"reach": "denied"})
        );
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
                ("cart".to_string(), Reach::Undeclared),
                ("home".to_string(), Reach::Undeclared),
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
    fn catch_all_routes_match_the_rest_and_rank_last() {
        // React `[...slug]` and `path="*"` become `*`, Angular `**`.
        assert_eq!(path_match_score("/docs/*", "/docs/a/b"), Some(1));
        assert_eq!(path_match_score("/**", "/anything"), Some(0));
        assert!(path_patterns_match("/docs/*", "/docs/{id}"));
        // One or more segments: the catch-all needs something to catch.
        assert!(!path_patterns_match("/docs/*", "/docs"));
        assert!(!path_patterns_match("/docs/*", "/blog/a"));
        // A literal or parameter route beats a catch-all, also one with more literals.
        let routes = [
            ("all", "/**"),
            ("docs", "/docs/*"),
            ("page", "/docs/:id"),
            ("intro", "/docs/intro"),
        ];
        assert_eq!(best_route("/docs/intro", routes), Ok("intro"));
        assert_eq!(best_route("/docs/x", routes), Ok("page"));
        assert_eq!(best_route("/docs/x/y", routes), Ok("docs"));
        assert_eq!(best_route("/cart", routes), Ok("all"));
        let deep = [("catch", "/a/b/*"), ("param", "/a/:x/:y")];
        assert_eq!(best_route("/a/b/c", deep), Ok("param"));
        // The join ranks the same way.
        let flow = UiFlow {
            actions: vec![http_action("GET", "/api/docs/7")],
            endpoints: vec![
                endpoint("GET", "/api/**", AccessState::Open),
                endpoint("GET", "/api/docs/{id}", AccessState::Undeclared),
            ],
            ..UiFlow::default()
        };
        let joined = join_actions_to_endpoints(&flow);
        let paths: Vec<&str> = joined[0].1.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["/api/docs/{id}"]);
    }

    #[test]
    fn marked_start_screens_start_the_simulation() {
        let in_app = |s: Screen| Screen {
            application: Some("ClientApp".into()),
            route: Some(format!("//Main/{}", s.id)),
            ..s
        };
        let mut flow = UiFlow {
            screens: vec![
                in_app(screen("login", None)),
                in_app(screen("catalog", None)),
                in_app(screen("orders", Some(Condition::Authenticated))),
            ],
            transitions: vec![go("login", "catalog", None), go("catalog", "orders", None)],
            ..UiFlow::default()
        };
        let anonymous = Assignment::default();
        assert_eq!(
            screen_reaches(&flow, &anonymous)["login"],
            ScreenReach::NoStartScreen
        );
        let kinds: Vec<_> = ui_flow_findings(&flow).iter().map(|f| f.kind).collect();
        assert_eq!(kinds, [UiFlowFindingKind::NoStartScreen]);
        flow.start_screens = vec!["login".into()];
        assert_eq!(start_screen_ids(&flow), ["login"]);
        let reaches = screen_reaches(&flow, &anonymous);
        assert_eq!(reaches["login"], ScreenReach::Undeclared);
        assert_eq!(reaches["catalog"], ScreenReach::Undeclared);
        assert_eq!(reaches["orders"], ScreenReach::Denied);
        assert!(ui_flow_findings(&flow).is_empty());
        // A screen routed `/` stays a start screen next to a marked one.
        flow.screens.push(Screen {
            route: Some("/".into()),
            ..screen("home", None)
        });
        assert_eq!(start_screen_ids(&flow), ["login", "home"]);
        // JSON: the list, absent when empty; older files read without it.
        let json = serde_json::to_string(&flow).unwrap();
        assert!(json.contains(r#""start_screens":["login"]"#), "{json}");
        assert_eq!(serde_json::from_str::<UiFlow>(&json).unwrap(), flow);
        let empty = serde_json::to_string(&UiFlow::default()).unwrap();
        assert!(!empty.contains("start_screens"), "{empty}");
        let merged = merge_ui_flows(flow.clone(), UiFlow::default());
        assert_eq!(merged.start_screens, ["login"]);
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
    fn handler_sites_are_found_per_endpoint_merged_and_optional_in_json() {
        let e = EndpointAccess {
            handler: Some("OrdersApi.CreateOrderAsync".into()),
            ..endpoint("POST", "/api/orders", AccessState::Undeclared)
        };
        let site = |endpoint_line: u32, line: u32| HandlerSite {
            endpoint_file: e.file.clone(),
            endpoint_line,
            handler: "OrdersApi.CreateOrderAsync".into(),
            file: "Apis/OrdersApi.cs".into(),
            line,
        };
        let a = UiFlow {
            endpoints: vec![e.clone()],
            handler_sites: vec![site(e.line + 1, 7)],
            ..UiFlow::default()
        };
        let b = UiFlow {
            handler_sites: vec![site(e.line, 40)],
            ..UiFlow::default()
        };
        assert_eq!(a.handler_site(&e), None, "another endpoint's site");
        let merged = merge_ui_flows(a, b);
        assert_eq!(merged.handler_site(&e).map(|s| s.line), Some(40));
        let lambda = EndpointAccess {
            handler: None,
            ..e.clone()
        };
        assert_eq!(merged.handler_site(&lambda), None);
        // Older files have no sites; an empty list is left out.
        let old: UiFlow = serde_json::from_str(r#"{"definition":"ui-flow/v1","complete":true}"#)
            .expect("reads without handler_sites");
        assert!(old.handler_sites.is_empty());
        assert!(
            !serde_json::to_string(&old)
                .unwrap()
                .contains("handler_sites")
        );
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
                    condition: AccessState::Protected(Condition::All(vec![
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
