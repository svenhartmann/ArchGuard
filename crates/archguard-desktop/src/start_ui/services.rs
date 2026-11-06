//! The services World ("Your services"): the web's repository overview
//! (`renderRepositoryWorld` and its filters in `viewer_repository.js`).
//! One 2×2 island per project or deployment, the topology's connections
//! between them as arrows, and filters by deployment, evidence, kind,
//! direction and cycles. Independent of the tab state and of Slint;
//! `services_view.rs` shows it.

mod draw;
mod inventory;
mod links;
mod world;

#[cfg(test)]
mod agreement;

pub use draw::{Drawn, draw, tone_index};
pub use links::{Direction, Source};
pub use world::{KINDS, ServicesWorld};
