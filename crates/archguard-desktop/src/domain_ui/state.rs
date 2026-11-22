//! The Domain tab's state: the model, the UI flow for the chains (ADR 0022
//! decision 4), the services, what the centre focuses, the chosen state,
//! the simulated user (shared with the Screens tab), notes and code jumps.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use archguard_engine::core::domain_model::DomainModel;
use archguard_engine::core::{Assignment, Config, ReviewDecision, UiFlow, resolve_assignment};

use super::overview;
use super::services::Services;
use crate::screens_ui::code::Code;
use crate::screens_ui::{role_assignment, role_names};

/// What the centre shows; the index points into the model's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Entity(usize),
    Integration(usize),
    /// A domain event (`DomainModel::events`).
    Event(usize),
    UseCase(usize),
}

impl Focus {
    /// The kind number of the window: 0 entity, 1 integration event, 2
    /// domain event, 3 use case.
    pub fn kind(self) -> i32 {
        match self {
            Focus::Entity(_) => 0,
            Focus::Integration(_) => 1,
            Focus::Event(_) => 2,
            Focus::UseCase(_) => 3,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Focus::Entity(i) | Focus::Integration(i) | Focus::Event(i) | Focus::UseCase(i) => i,
        }
    }

    pub fn of(kind: i32, i: usize) -> Option<Focus> {
        Some(match kind {
            0 => Focus::Entity(i),
            1 => Focus::Integration(i),
            2 => Focus::Event(i),
            3 => Focus::UseCase(i),
            _ => return None,
        })
    }
}

pub struct DomainState {
    pub model: Arc<Result<DomainModel, String>>,
    pub flow: Arc<Result<UiFlow, String>>,
    pub services: Services,
    /// The service shown; `None` for all.
    pub service: Option<String>,
    pub focus: Option<Focus>,
    /// A state of the focused entity's state machine.
    pub state: Option<String>,
    /// The centre shows every entity by aggregate instead of the focus.
    pub whole: bool,
    /// The module the whole-model diagram is drilled into; `None` for the
    /// overview (or the plain band diagram of a small model).
    pub module: Option<String>,
    /// Tree groups the user closed.
    pub folded: BTreeSet<String>,
    /// The tree's "Details" section is open; open until the user closes it.
    pub details_open: bool,
    /// Role names of the flow; `role` indexes them as the Screens tab does.
    pub roles: Vec<String>,
    pub role: usize,
    pub root: std::path::PathBuf,
    pub config: Option<Config>,
    /// Notes for the coding agent by item id (the review register).
    pub notes: BTreeMap<String, ReviewDecision>,
    /// The item whose note form is open: its id and what it is about.
    pub note_for: Option<(String, String)>,
    /// Why the last note or editor jump failed.
    pub problem: String,
    pub code: Code,
}

pub fn build_from(model: Result<DomainModel, String>, flow: Result<UiFlow, String>) -> DomainState {
    let roles = flow.as_ref().map_or_else(|_| Vec::new(), role_names);
    let mut s = DomainState {
        model: Arc::new(model),
        flow: Arc::new(flow),
        services: Services::default(),
        service: None,
        focus: None,
        state: None,
        whole: false,
        module: None,
        folded: BTreeSet::new(),
        details_open: true,
        roles,
        role: 0,
        root: Default::default(),
        config: None,
        notes: BTreeMap::new(),
        note_for: None,
        problem: String::new(),
        code: Code::default(),
    };
    s.focus = s.first_entity().map(Focus::Entity);
    s
}

impl DomainState {
    pub fn model(&self) -> Option<&DomainModel> {
        (*self.model).as_ref().ok()
    }

    /// The focused entity.
    pub fn selected(&self) -> Option<usize> {
        match self.focus {
            Some(Focus::Entity(i)) => Some(i),
            _ => None,
        }
    }

    /// The first aggregate root with a state machine, else the first
    /// aggregate root, else the first entity.
    fn first_entity(&self) -> Option<usize> {
        let d = self.model()?;
        let root = |i: &usize| {
            d.entities[*i].kind == archguard_engine::core::domain_model::EntityKind::AggregateRoot
        };
        let machine = |i: &usize| {
            d.state_machines
                .iter()
                .any(|m| m.entity == d.entities[*i].id)
        };
        let shown = |i: &usize| self.in_service(&d.entities[*i].file);
        let all = || (0..d.entities.len()).filter(shown);
        (all().find(|i| root(i) && machine(i)))
            .or_else(|| all().find(root))
            .or_else(|| all().next())
    }

    pub fn in_service(&self, file: &str) -> bool {
        self.service
            .as_deref()
            .is_none_or(|s| self.services.of(file) == s)
    }

    pub fn select(&mut self, f: Focus) {
        let Some(d) = self.model() else { return };
        let len = match f {
            Focus::Entity(_) => d.entities.len(),
            Focus::Integration(_) => d.integration_events.len(),
            Focus::Event(_) => d.events.len(),
            Focus::UseCase(_) => d.use_cases.len(),
        };
        if f.index() < len {
            self.focus = Some(f);
            self.state = None;
            self.whole = false;
            self.note_for = None;
        }
    }

    /// Drill the diagram into module `i` of [`overview::modules`].
    pub fn open_module(&mut self, i: usize) {
        let found = (self.model()).and_then(|d| overview::modules(d).into_iter().nth(i));
        // An index past the modules (the external stub) changes nothing.
        if found.is_some() {
            self.module = found;
        }
    }

    pub fn select_state(&mut self, name: &str) {
        self.state = (!name.is_empty()).then(|| name.to_string());
    }

    pub fn select_role(&mut self, i: usize) {
        if i < self.roles.len() + 2 {
            self.role = i;
        }
    }

    /// Show service `i` of the picker: 0 all, then `services.names`; the
    /// focus moves to its first entity when it lies elsewhere.
    pub fn select_service(&mut self, i: usize) {
        self.service = i
            .checked_sub(1)
            .and_then(|k| self.services.names.get(k).cloned());
        if let Some(name) = &self.service {
            self.folded.remove(&format!("s:{name}"));
        }
        let inside = match (self.focus, self.model()) {
            (Some(Focus::Entity(e)), Some(d)) => self.in_service(&d.entities[e].file),
            (Some(Focus::UseCase(u)), Some(d)) => self.in_service(&d.use_cases[u].file),
            _ => true,
        };
        if !inside {
            self.focus = self.first_entity().map(Focus::Entity);
            self.state = None;
        }
    }

    /// Close every service group but the focused entity's, so the tree
    /// opens where the centre is.
    pub fn fold_other_services(&mut self) {
        let Some(file) =
            (self.selected().zip(self.model())).map(|(i, d)| d.entities[i].file.clone())
        else {
            return;
        };
        let mine = self.services.of(&file).to_string();
        let others = self.services.names.iter().filter(|n| **n != mine);
        let keys: Vec<String> = others.map(|n| format!("s:{n}")).collect();
        self.folded.extend(keys);
    }

    pub fn toggle_group(&mut self, key: &str) {
        if !self.folded.remove(key) {
            self.folded.insert(key.to_string());
        }
    }

    pub fn open_note(&mut self, id: &str, subject: &str) {
        if !id.is_empty() {
            self.note_for = Some((id.to_string(), subject.to_string()));
            self.problem.clear();
        }
    }

    /// The chosen user, resolved by the flow's role and policy catalog.
    pub fn assignment(&self) -> Assignment {
        let a = role_assignment(self.role, &self.roles);
        match &*self.flow {
            Ok(f) => resolve_assignment(f, &a),
            Err(_) => a,
        }
    }
}
