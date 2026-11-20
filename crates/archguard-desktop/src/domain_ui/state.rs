//! The Domain tab's state: the model, the UI flow for the chains (ADR 0022
//! decision 4), the selected entity and state, the simulated user.

use std::sync::Arc;

use archguard_engine::core::domain_model::DomainModel;
use archguard_engine::core::{Assignment, UiFlow, resolve_assignment};

use crate::screens_ui::{role_assignment, role_names};

pub struct DomainState {
    pub model: Arc<Result<DomainModel, String>>,
    pub flow: Arc<Result<UiFlow, String>>,
    /// Index into the model's entities.
    pub selected: Option<usize>,
    /// A state of the selected entity's state machine.
    pub state: Option<String>,
    /// Role names of the flow; `role` indexes them as the Screens tab does.
    pub roles: Vec<String>,
    pub role: usize,
}

pub fn build_from(model: Result<DomainModel, String>, flow: Result<UiFlow, String>) -> DomainState {
    let roles = flow.as_ref().map_or_else(|_| Vec::new(), role_names);
    DomainState {
        model: Arc::new(model),
        flow: Arc::new(flow),
        selected: None,
        state: None,
        roles,
        role: 0,
    }
}

impl DomainState {
    pub fn select_entity(&mut self, i: usize) {
        if (*self.model).as_ref().is_ok_and(|d| i < d.entities.len()) {
            self.selected = Some(i);
            self.state = None;
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

    /// The chosen user, resolved by the flow's role and policy catalog.
    pub fn assignment(&self) -> Assignment {
        let mut a = role_assignment(self.role, &self.roles);
        match &*self.flow {
            Ok(f) => {
                crate::screens_ui::with_prefix(f, &mut a);
                resolve_assignment(f, &a)
            }
            Err(_) => a,
        }
    }
}
