//! Class diagram view of the Elasticsearch tab: shared types and the module skeleton.

pub mod boxes;
pub mod details;
pub mod diagram;
pub mod layout;
pub mod list;
pub mod view;

#[cfg(test)]
mod boxes_check;
#[cfg(test)]
mod canvas_check;
#[cfg(test)]
mod details_check;
#[cfg(test)]
mod details_view_check;
#[cfg(test)]
mod diagram_check;
#[cfg(test)]
mod layout_check;
#[cfg(test)]
mod list_check;
#[cfg(test)]
mod list_view_check;
#[cfg(test)]
mod probe_check;
#[cfg(test)]
mod state_check;

// Shared types live in `types` so `state` does not import this module (it would close a cycle
// through `state_check`).
mod types;
#[allow(unused_imports)] // Unused until ticket 12.
pub use types::{Group, Key, Switch, Switches};
