//! React and Next.js extractor for ui-flow/v1 (ADR 0021 phase 6).
use archguard_core::UiFlow;
use std::path::Path;

/// Slot; the extractor agent fills it.
pub fn extract(_root: &Path) -> UiFlow {
    UiFlow::default()
}
