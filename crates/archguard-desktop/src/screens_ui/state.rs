//! The Screens tab's state: the UI flow (ADR 0021), the chosen
//! application, the simulated user, how unknown conditions count, the
//! route tree's search and filter, the selection with its history.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use archguard_engine::core::{
    AccessState, Assignment, Condition, Config, UiFlow, UiFlowFinding, UiFlowSummary,
    resolve_assignment, start_screen_ids, ui_flow_findings, ui_flow_summary,
};

use archguard_engine::core::ReviewDecision;
use archguard_engine::core::domain_model::DomainModel;

use super::notes::Ids;
use super::tree::Filter;

/// Who is simulated: index 0 anonymous, 1 signed in without a role, then
/// one entry per role of [`ScreensState::roles`].
pub type RoleIndex = usize;

/// An open condition the user switches: a policy or a custom atom.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Atom {
    pub policy: bool,
    pub name: String,
}

/// A list the left panel opens in the details panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    Gaps,
    ShownButRefused,
    HiddenButUnprotected,
    Undeclared,
    /// The unresolved targets of one source: its first unresolved transition.
    Unreadable(usize),
    /// The unresolved targets of a screen, its own and its layout's.
    UnreadableFrom(usize),
    /// Shown but refused and hidden but unprotected together.
    Mismatches,
}

/// How a policy or custom condition without its own switch counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Unknown {
    /// Unknown: the simulator keeps both outcomes.
    #[default]
    Open,
    AllHold,
    NoneHold,
}

/// The centre: the selected screen's neighbours, two rings, or every band.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Focus,
    TwoSteps,
    Whole,
}

pub const LISTS: [List; 4] = [
    List::Gaps,
    List::ShownButRefused,
    List::HiddenButUnprotected,
    List::Undeclared,
];

pub struct ScreensState {
    pub flow: Arc<Result<UiFlow, String>>,
    /// The flow with every unreadable condition (`unknown(text)`) as a
    /// custom atom of that text, so the switches and [`Unknown`] reach it;
    /// every simulation runs on this one.
    pub sim: UiFlow,
    /// The project configuration (`[views.run]`, `[views.access]`); none
    /// when it could not be read.
    pub config: Option<Config>,
    /// The application shown; `""` for screens without one.
    pub app: String,
    pub unknown: Unknown,
    pub mode: Mode,
    /// Earlier selections, for Back.
    pub back: Vec<usize>,
    pub search: String,
    pub filter: Filter,
    /// Route groups the user closed.
    pub folded: BTreeSet<String>,
    /// The details panel was closed; the next selection opens it again.
    pub hide_details: bool,
    /// Why the browser did not open, shown under the button.
    pub browser_note: String,
    /// The project root, to read a screen's file for "where the meaning
    /// comes from".
    pub root: std::path::PathBuf,
    pub summary: UiFlowSummary,
    pub findings: Vec<UiFlowFinding>,
    pub roles: Vec<String>,
    pub atoms: Vec<Atom>,
    /// Per atom: `None` open (unknown), else on or off.
    pub switches: Vec<Option<bool>>,
    pub role: RoleIndex,
    /// Index into the flow's screens.
    pub selected: Option<usize>,
    pub list: Option<List>,
    /// Applications whose band shows only its title.
    pub collapsed: BTreeSet<String>,
    /// The domain model for the drill-down; none when it could not be read.
    pub domain: Option<DomainModel>,
    /// The action (index into the flow's actions) the drill-down shows.
    pub drill: Option<usize>,
    /// The stable ids of gaps, mismatches and undeclared endpoints.
    pub ids: Ids,
    /// Notes for the coding agent by item id (the review register).
    pub notes: BTreeMap<String, ReviewDecision>,
    /// The item whose note form is open: its id and what it is about.
    pub note_for: Option<(String, String)>,
    /// Why the last note or editor jump failed.
    pub problem: String,
    /// "Show here": the jumps the code overlay shows.
    pub code: super::code::Code,
    /// The tree's "Details" section is open; open until the user closes it.
    pub details_open: bool,
}

pub fn build_from(flow: Result<UiFlow, String>) -> ScreensState {
    let sim = flow.as_ref().map(simulable).unwrap_or_default();
    let (summary, findings, roles, atoms) = match &flow {
        Ok(f) => {
            let atoms = atoms_of(&sim).1;
            (
                ui_flow_summary(f),
                ui_flow_findings(f),
                role_names(f),
                atoms,
            )
        }
        Err(_) => Default::default(),
    };
    let app = default_app(&sim);
    let ids = Ids::of(&sim, &findings);
    let mut s = ScreensState {
        domain: None,
        drill: None,
        ids,
        notes: BTreeMap::new(),
        note_for: None,
        problem: String::new(),
        code: Default::default(),
        details_open: true,
        sim,
        config: None,
        app,
        unknown: Unknown::Open,
        mode: Mode::Focus,
        back: vec![],
        search: String::new(),
        filter: Filter::All,
        folded: BTreeSet::new(),
        hide_details: false,
        browser_note: String::new(),
        root: Default::default(),
        switches: vec![None; atoms.len()],
        flow: Arc::new(flow),
        summary,
        findings,
        roles,
        atoms,
        role: 0,
        selected: None,
        list: None,
        collapsed: BTreeSet::new(),
    };
    s.selected = s.first_screen();
    s
}

/// The applications in band order: those with a start screen first, most
/// reached screens first (the whole graph's order).
pub fn applications(flow: &UiFlow) -> Vec<String> {
    super::layout::band_names(flow)
}

/// The application shown first: the first in band order.
pub fn default_app(flow: &UiFlow) -> String {
    applications(flow).into_iter().next().unwrap_or_default()
}

/// `unknown(text)` becomes `custom(text)`; everything else stays.
fn simulable(flow: &UiFlow) -> UiFlow {
    fn map(c: &mut Condition) {
        match c {
            Condition::All(items) | Condition::Any(items) => items.iter_mut().for_each(map),
            Condition::Not(inner) => map(inner),
            Condition::Unknown(text) => *c = Condition::Custom(std::mem::take(text)),
            _ => {}
        }
    }
    let mut f = flow.clone();
    let screens = f
        .screens
        .iter_mut()
        .filter_map(|s| s.access.condition_mut());
    let transitions = f
        .transitions
        .iter_mut()
        .filter_map(|t| t.condition.condition_mut());
    let actions = f
        .actions
        .iter_mut()
        .filter_map(|a| a.condition.condition_mut());
    let endpoints = f.endpoints.iter_mut().filter_map(|e| match &mut e.access {
        AccessState::Protected(c) => Some(c),
        _ => None,
    });
    screens
        .chain(transitions)
        .chain(actions)
        .chain(endpoints)
        .for_each(map);
    f
}

/// Every condition of the flow: screens, transitions, actions, endpoints.
fn conditions(flow: &UiFlow) -> impl Iterator<Item = &Condition> {
    let screens = flow.screens.iter().filter_map(|s| s.access.condition());
    let transitions = flow
        .transitions
        .iter()
        .filter_map(|t| t.condition.condition());
    let actions = flow.actions.iter().filter_map(|a| a.condition.condition());
    let endpoints = flow.endpoints.iter().filter_map(|e| match &e.access {
        AccessState::Protected(c) => Some(c),
        _ => None,
    });
    screens.chain(transitions).chain(actions).chain(endpoints)
}

/// The sorted role names and policy/custom atoms named anywhere.
pub fn atoms_of(flow: &UiFlow) -> (Vec<String>, Vec<Atom>) {
    let (mut roles, mut atoms) = (BTreeSet::new(), BTreeSet::new());
    let mut todo: Vec<&Condition> = conditions(flow).collect();
    while let Some(c) = todo.pop() {
        match c {
            Condition::All(items) | Condition::Any(items) => todo.extend(items),
            Condition::Not(inner) => todo.push(inner),
            Condition::Role(name) => {
                roles.insert(name.clone());
            }
            Condition::Policy(name) | Condition::Custom(name) => {
                let policy = matches!(c, Condition::Policy(_));
                atoms.insert(Atom {
                    policy,
                    name: name.clone(),
                });
            }
            _ => {}
        }
    }
    (roles.into_iter().collect(), atoms.into_iter().collect())
}

/// The roles to simulate: the catalog's (`flow.roles`) and every role a
/// condition names, sorted, in the stored form (the runtime authority).
pub fn role_names(flow: &UiFlow) -> Vec<String> {
    let named = atoms_of(flow).0.into_iter();
    let defined = flow.roles.iter().map(|r| r.name.clone());
    named
        .chain(defined)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The simulated user the Screens and Domain tabs share, per document: each
/// tab records its choice here and takes it over before it shows.
pub struct SharedRole(pub RoleIndex);

/// Record `role` as the document's simulated user.
pub fn share_role(role: RoleIndex) {
    crate::documents::with_active_state_or(|| SharedRole(role), |r: &mut SharedRole| r.0 = role);
}

/// The document's simulated user, when a tab chose one.
pub fn shared_role() -> Option<RoleIndex> {
    crate::documents::with_active_state(|r: &mut SharedRole| r.0)
}

/// Anonymous (0), signed in without a role (1), else signed in with
/// `roles[role - 2]`.
pub fn role_assignment(role: RoleIndex, roles: &[String]) -> Assignment {
    let mut a = Assignment {
        authenticated: role > 0,
        ..Assignment::default()
    };
    if let Some(name) = role.checked_sub(2).and_then(|i| roles.get(i)) {
        a.roles.insert(name.clone());
    }
    a
}

fn atom_map<'a>(a: &'a mut Assignment, atom: &Atom) -> &'a mut BTreeMap<String, bool> {
    if atom.policy {
        &mut a.policies
    } else {
        &mut a.custom
    }
}

impl ScreensState {
    pub fn flow(&self) -> Option<&UiFlow> {
        (*self.flow).as_ref().ok()
    }

    /// The simulator input for the chosen user and switches, resolved by
    /// the role and policy catalog (`resolve_assignment`): included roles
    /// count, a defined policy is what it requires. A condition still
    /// without a value counts as [`Unknown`] says.
    pub fn assignment(&self) -> Assignment {
        let mut a = role_assignment(self.role, &self.roles);
        let atoms = self.atoms.iter().zip(&self.switches);
        for (atom, v) in atoms.clone().filter_map(|(a, v)| Some((a, (*v)?))) {
            atom_map(&mut a, atom).insert(atom.name.clone(), v);
        }
        let mut a = resolve_assignment(&self.sim, &a);
        let fallback = match self.unknown {
            Unknown::Open => return a,
            Unknown::AllHold => true,
            Unknown::NoneHold => false,
        };
        // the per-application copies the views read get the same fallback
        let atoms: Vec<&Atom> = atoms.map(|(atom, _)| atom).collect();
        let fill = |x: &mut Assignment| {
            for atom in &atoms {
                atom_map(x, atom)
                    .entry(atom.name.clone())
                    .or_insert(fallback);
            }
        };
        fill(&mut a);
        a.by_app.values_mut().for_each(fill);
        a
    }

    pub fn select_role(&mut self, i: usize) {
        if i < self.roles.len() + 2 {
            self.role = i;
        }
    }

    /// `value`: 0 open, 1 on, 2 off.
    pub fn set_switch(&mut self, i: usize, value: i32) {
        if let Some(s) = self.switches.get_mut(i) {
            *s = match value {
                1 => Some(true),
                2 => Some(false),
                _ => None,
            };
        }
    }

    /// Select screen `i`, show its application and remember the screen
    /// before for Back.
    pub fn select_screen(&mut self, i: usize) {
        let Some(screen) = self.flow().and_then(|f| f.screens.get(i)) else {
            return;
        };
        self.app = screen.application.clone().unwrap_or_default();
        if let Some(old) = self.selected.filter(|&old| old != i) {
            self.back.push(old);
        }
        self.selected = Some(i);
        self.list = None;
        self.drill = None;
        self.note_for = None;
        self.hide_details = false;
        self.browser_note.clear();
    }

    /// Show `file` at `line` in the code overlay; Back returns.
    pub fn show_code(&mut self, file: &str, line: u32) {
        self.code.show(&self.root, file, line);
    }

    /// Show the drill-down of action `a` (index into the flow's actions).
    pub fn open_drill(&mut self, a: usize) {
        if a < self.sim.actions.len() {
            self.drill = Some(a);
            self.note_for = None;
            self.hide_details = false;
        }
    }

    /// Open the note form of item `id` (`subject` says what it is about).
    pub fn open_note(&mut self, id: &str, subject: &str) {
        if !id.is_empty() {
            self.note_for = Some((id.to_string(), subject.to_string()));
            self.problem.clear();
            self.hide_details = false;
        }
    }

    pub fn go_back(&mut self) {
        if let Some(i) = self.back.pop() {
            self.select_screen(i);
            // Going back is not a step to remember.
            self.back.pop();
        }
    }

    /// A start screen of the application, else its first screen by name.
    fn first_screen(&self) -> Option<usize> {
        let f = self.flow()?;
        let starts = start_screen_ids(f);
        let of_app = |s: &&archguard_engine::core::Screen| {
            s.application.as_deref().unwrap_or("") == self.app
        };
        let screens = f.screens.iter().filter(of_app);
        let start = screens.clone().find(|s| starts.contains(&s.id.as_str()));
        let id = &start
            .or_else(|| screens.min_by(|a, b| a.name.cmp(&b.name)))?
            .id;
        f.screens.iter().position(|s| s.id == *id)
    }

    /// Show the application at `i` of [`applications`], at its first screen.
    pub fn select_app(&mut self, i: usize) {
        let Some(name) = self.flow().and_then(|f| applications(f).get(i).cloned()) else {
            return;
        };
        self.app = name;
        self.back.clear();
        self.selected = self.first_screen();
        self.list = None;
        self.hide_details = false;
    }

    pub fn set_unknown(&mut self, i: usize) {
        self.unknown = [Unknown::Open, Unknown::AllHold, Unknown::NoneHold][i.min(2)];
    }

    pub fn set_mode(&mut self, i: usize) {
        self.mode = [Mode::Focus, Mode::TwoSteps, Mode::Whole][i.min(2)];
    }

    pub fn set_filter(&mut self, i: usize) {
        self.filter = [Filter::All, Filter::Denied, Filter::NoPath][i.min(2)];
    }

    pub fn toggle_group(&mut self, key: &str) {
        if !self.folded.remove(key) {
            self.folded.insert(key.to_string());
        }
    }

    pub fn open_list(&mut self, i: usize) {
        self.list = LISTS.get(i).copied();
        self.drill = None;
        self.hide_details = false;
    }

    /// The list of the unresolved targets of a source (transition index);
    /// in the focus view all of the selected screen's.
    pub fn open_group(&mut self, k: usize) {
        if self.flow().is_some_and(|f| k < f.transitions.len()) {
            self.list = Some(match (self.mode, self.selected) {
                (Mode::Whole, _) | (_, None) => List::Unreadable(k),
                (_, Some(i)) => List::UnreadableFrom(i),
            });
            self.hide_details = false;
        }
    }

    pub fn open_mismatches(&mut self) {
        self.list = Some(List::Mismatches);
        self.drill = None;
        self.hide_details = false;
    }

    /// Collapse or expand the band at `i` (band order of the layout).
    pub fn toggle_band(&mut self, i: usize) {
        let Some(name) = self
            .flow()
            .and_then(|f| super::layout::band_names(f).get(i).cloned())
        else {
            return;
        };
        if !self.collapsed.remove(&name) {
            self.collapsed.insert(name);
        }
    }

    /// Close a list back to the screen; else close the screen's details.
    pub fn close_details(&mut self) {
        self.problem.clear();
        if self.note_for.take().is_none()
            && self.drill.take().is_none()
            && self.list.take().is_none()
        {
            self.hide_details = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Screen, ScreenKind};

    fn screen(id: &str, access: Option<Condition>) -> Screen {
        Screen {
            weak: false,
            id: id.into(),
            name: id.into(),
            kind: ScreenKind::Page,
            route: Some(format!("/{id}")),
            application: Some("App".into()),
            file: format!("{id}.razor"),
            line: 1,
            access: archguard_engine::core::AccessState::from_parts(access, false),
            source: "test".into(),
        }
    }

    fn flow() -> UiFlow {
        let admin = Condition::All(vec![
            Condition::Role("Admin".into()),
            Condition::Not(Box::new(Condition::Policy("Locked".into()))),
        ]);
        UiFlow {
            screens: vec![
                screen("a", Some(admin)),
                screen("b", Some(Condition::Custom("beta".into()))),
                screen("c", Some(Condition::Role("Clerk".into()))),
            ],
            ..UiFlow::default()
        }
    }

    #[test]
    fn the_details_section_starts_open_and_keeps_the_choice() {
        let mut s = build_from(Ok(flow()));
        assert!(s.details_open, "open by default");
        let texts = crate::tab_support::Texts::default();
        assert!(super::super::view::build(&s, &texts).details_open);
        s.details_open = false;
        s.select_screen(1);
        assert!(!super::super::view::build(&s, &texts).details_open, "kept");
    }

    #[test]
    fn roles_and_atoms_come_from_every_condition() {
        let (roles, atoms) = atoms_of(&flow());
        assert_eq!(roles, ["Admin", "Clerk"]);
        let names: Vec<(bool, &str)> = atoms.iter().map(|a| (a.policy, a.name.as_str())).collect();
        assert_eq!(names, [(false, "beta"), (true, "Locked")]);
    }

    #[test]
    fn the_unknown_fallback_reaches_every_applications_copy() {
        let mut f = flow();
        for screen in &mut f.screens {
            screen.application = Some("App".into());
        }
        let mut s = build_from(Ok(f));
        s.set_unknown(1);
        let a = s.assignment();
        assert!(!a.custom.is_empty() || !a.policies.is_empty());
        let copy = a.in_app(Some("App"));
        assert_eq!((&copy.custom, &copy.policies), (&a.custom, &a.policies));
    }

    #[test]
    fn the_assignment_follows_role_and_switches() {
        let mut s = build_from(Ok(flow()));
        // the per-application copies are a representation detail
        let anonymous = Assignment {
            by_app: Default::default(),
            ..s.assignment()
        };
        assert_eq!(anonymous, Assignment::default(), "anonymous");
        s.select_role(1);
        assert!(s.assignment().authenticated);
        assert!(s.assignment().roles.is_empty(), "signed in, no role");
        s.select_role(3);
        assert_eq!(
            s.assignment().roles.into_iter().collect::<Vec<_>>(),
            ["Clerk"]
        );
        s.select_role(9);
        assert_eq!(s.role, 3, "unknown index ignored");
        s.set_switch(1, 2);
        s.set_switch(0, 1);
        let a = s.assignment();
        assert_eq!(a.policies.get("Locked"), Some(&false));
        assert_eq!(a.custom.get("beta"), Some(&true));
        s.set_switch(1, 0);
        assert!(s.assignment().policies.is_empty(), "open again");
    }

    /// jhipster: ADMIN includes USER; the catalog also names a role no
    /// condition uses, and a policy requires ADMIN.
    #[test]
    fn the_catalog_fills_the_roles_and_resolves_hierarchy_and_policies() {
        use archguard_engine::core::{PolicyDef, RoleDef, RoleKind};
        let def = |name: &str, includes: &[&str]| RoleDef {
            application: None,
            name: name.into(),
            kind: RoleKind::Role,
            includes: includes.iter().map(|s| s.to_string()).collect(),
            file: "Roles.java".into(),
            line: 3,
            source: "t".into(),
        };
        let mut f = flow();
        f.roles = vec![def("Admin", &["Clerk"]), def("Auditor", &[])];
        f.policies = vec![PolicyDef {
            application: None,
            name: "Locked".into(),
            requires: Condition::Role("Auditor".into()),
            file: "P.cs".into(),
            line: 1,
            source: "t".into(),
        }];
        let mut s = build_from(Ok(f));
        assert_eq!(s.roles, ["Admin", "Auditor", "Clerk"]);
        s.select_role(2);
        let a = s.assignment();
        assert!(a.roles.contains("Clerk"), "Admin includes Clerk");
        assert_eq!(a.policies.get("Locked"), Some(&false), "requires Auditor");
        s.set_unknown(1);
        assert_eq!(
            s.assignment().policies.get("Locked"),
            Some(&false),
            "\"all hold\" does not override a policy the catalog decides"
        );
        assert_eq!(s.assignment().custom.get("beta"), Some(&true));
    }

    /// jhipster: catalog, route and backend all say ROLE_ADMIN; the picker lists
    /// it once and the assignment holds it as stored, no alias.
    #[test]
    fn a_spring_role_is_listed_once_in_its_stored_form() {
        use archguard_engine::core::{RoleDef, RoleKind};
        let mut f = flow();
        f.screens[2].access =
            archguard_engine::core::AccessState::Protected(Condition::Role("ROLE_ADMIN".into()));
        f.roles = vec![RoleDef {
            application: None,
            name: "ROLE_ADMIN".into(),
            kind: RoleKind::Role,
            includes: vec![],
            file: "AuthoritiesConstants.java".into(),
            line: 8,
            source: "t".into(),
        }];
        let mut s = build_from(Ok(f));
        assert_eq!(s.roles, ["Admin", "ROLE_ADMIN"]);
        s.select_role(3);
        let roles: Vec<String> = s.assignment().roles.into_iter().collect();
        assert_eq!(roles, ["ROLE_ADMIN"]);
    }
}
