//! The Domain tab's state: the model and the selected entity.

use std::sync::Arc;

use archguard_engine::core::domain_model::DomainModel;

pub struct DomainState {
    pub model: Arc<Result<DomainModel, String>>,
    /// Index into the model's entities.
    pub selected: Option<usize>,
}

pub fn build_from(model: Result<DomainModel, String>) -> DomainState {
    DomainState {
        model: Arc::new(model),
        selected: None,
    }
}

impl DomainState {
    pub fn select_entity(&mut self, i: usize) {
        if (*self.model).as_ref().is_ok_and(|d| i < d.entities.len()) {
            self.selected = Some(i);
        }
    }
}
