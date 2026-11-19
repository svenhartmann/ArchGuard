//! The Screens tab's state: the UI flow (ADR 0021), the simulated user
//! and the open-condition switches, the selection.

use std::collections::BTreeSet;
use std::sync::Arc;

use archguard_engine::core::{
    AccessState, Assignment, Condition, UiFlow, UiFlowFinding, UiFlowSummary, ui_flow_findings,
    ui_flow_summary,
};

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
}

pub const LISTS: [List; 4] = [
    List::Gaps,
    List::ShownButRefused,
    List::HiddenButUnprotected,
    List::Undeclared,
];

pub struct ScreensState {
    pub flow: Arc<Result<UiFlow, String>>,
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
    let (summary, findings, roles, atoms) = match &flow {
        Ok(f) => {
            let (roles, atoms) = atoms_of(f);
            (ui_flow_summary(f), ui_flow_findings(f), roles, atoms)
        }
        Err(_) => Default::default(),
    };
    ScreensState {
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
    }
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

    /// The simulator input for the chosen user and switches.
    pub fn assignment(&self) -> Assignment {
        let mut a = role_assignment(self.role, &self.roles);
        for (atom, value) in self.atoms.iter().zip(&self.switches) {
            let Some(v) = *value else { continue };
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

    pub fn select_screen(&mut self, i: usize) {
        if self.flow().is_some_and(|f| i < f.screens.len()) {
            self.selected = Some(i);
            self.list = None;
        }
    }

    pub fn open_list(&mut self, i: usize) {
        self.list = LISTS.get(i).copied();
    }

    /// The list of the unresolved targets of a source (transition index).
    pub fn open_group(&mut self, k: usize) {
        if self.flow().is_some_and(|f| k < f.transitions.len()) {
            self.list = Some(List::Unreadable(k));
        }
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

    pub fn close_details(&mut self) {
        self.selected = None;
        self.list = None;
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
