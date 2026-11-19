//! Java and Kotlin extractor for domain-model/v1 (ADR 0022 phase 4).
use archguard_core::domain_model::DomainModel;
use std::path::Path;

/// Slot; the extractor agent fills it.
pub fn extract(_root: &Path) -> DomainModel {
    DomainModel::default()
}
