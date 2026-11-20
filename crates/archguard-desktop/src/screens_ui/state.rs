//! The Screens tab's state: the UI flow (ADR 0021), the chosen
//! application, the simulated user, how unknown conditions count, the
//! route tree's search and filter, the selection with its history.

use std::collections::BTreeSet;
use std::sync::Arc;

use archguard_engine::core::{
    AccessState, Assignment, Condition, Config, UiFlow, UiFlowFinding, UiFlowSummary,
    start_screen_ids, ui_flow_findings, ui_flow_summary,
};

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
}

pub fn build_from(flow: Result<UiFlow, String>) -> ScreensState {
    let sim = flow.as_ref().map(simulable).unwrap_or_default();
    let (summary, findings, roles, atoms) = match &flow {
        Ok(f) => {
            let (roles, atoms) = atoms_of(&sim);
            (ui_flow_summary(f), ui_flow_findings(f), roles, atoms)
        }
        Err(_) => Default::default(),
    };
    let app = default_app(&sim);
    let mut s = ScreensState {
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
    let screens = f.screens.iter_mut().filter_map(|s| s.access.as_mut());
    let transitions = f
        .transitions
        .iter_mut()
        .filter_map(|t| t.condition.as_mut());
    let actions = f.actions.iter_mut().filter_map(|a| a.condition.as_mut());
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
    let screens = flow.screens.iter().filter_map(|s| s.access.as_ref());
    let transitions = flow.transitions.iter().filter_map(|t| t.condition.as_ref());
    let actions = flow.actions.iter().filter_map(|a| a.condition.as_ref());
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

impl ScreensState {
    pub fn flow(&self) -> Option<&UiFlow> {
        (*self.flow).as_ref().ok()
    }

    /// The simulator input for the chosen user and switches; a condition
    /// without its own switch counts as [`Unknown`] says.
    pub fn assignment(&self) -> Assignment {
        let mut a = role_assignment(self.role, &self.roles);
        let fallback = match self.unknown {
            Unknown::Open => None,
            Unknown::AllHold => Some(true),
            Unknown::NoneHold => Some(false),
        };
        for (atom, value) in self.atoms.iter().zip(&self.switches) {
            let Some(v) = value.or(fallback) else {
                continue;
            };
            let map = if atom.policy {
                &mut a.policies
            } else {
                &mut a.custom
            };
            map.insert(atom.name.clone(), v);
        }
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
        self.hide_details = false;
        self.browser_note.clear();
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
        if self.list.take().is_none() {
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
            id: id.into(),
            name: id.into(),
            kind: ScreenKind::Page,
            route: Some(format!("/{id}")),
            application: Some("App".into()),
            file: format!("{id}.razor"),
            line: 1,
            access,
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
    fn roles_and_atoms_come_from_every_condition() {
        let (roles, atoms) = atoms_of(&flow());
        assert_eq!(roles, ["Admin", "Clerk"]);
        let names: Vec<(bool, &str)> = atoms.iter().map(|a| (a.policy, a.name.as_str())).collect();
        assert_eq!(names, [(false, "beta"), (true, "Locked")]);
    }

    #[test]
    fn the_assignment_follows_role_and_switches() {
        let mut s = build_from(Ok(flow()));
        assert_eq!(s.assignment(), Assignment::default(), "anonymous");
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
}
