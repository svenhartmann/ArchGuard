//! File → Compare architecture…: two Git refs, or a ref and the Git index,
//! compared by the engine and shown as a package tree with added, removed
//! and changed packages, files and relations and cycle changes. UI-free;
//! `compare_ui.rs` shows it.

pub mod diff;
pub mod git;
mod packages;
#[cfg(test)]
mod tests;
