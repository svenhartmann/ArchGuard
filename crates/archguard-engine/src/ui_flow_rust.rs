//! Rust UI frameworks other than Slint (Tauri commands, Dioxus, Leptos, Yew,
//! egui, iced) for ui-flow/v1 (ADR 0021). Slot; the extractor agent fills it.
use crate::extract_ctx::{ExtractContext, with_root_config};
use archguard_core::UiFlow;
use std::path::Path;

pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run.
pub fn extract_with(_ctx: &ExtractContext) -> UiFlow {
    UiFlow::default()
}
