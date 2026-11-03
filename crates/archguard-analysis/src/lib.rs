//! `archguard-analysis` — L1: graph analysis (Tarjan SCC, cycles, metrics,
//! layer checks, cut suggestions, baseline diff). Lands in T12–T17.
//!
//! T12 contributes the SCC detection of [`find_sccs`], T13 the cycle
//! sampling of [`sample_cycles`], T14 the per-module metrics of
//! [`compute_metrics`] and the [`summarize`]d run summary; T15 adds the
//! layer-rule check [`check_layers`]; T16 adds the ranked cut suggestions
//! of [`suggest_cuts`]; T17 adds the baseline ratchet ([`cycle_hash`],
//! [`Baseline`], [`diff`]); T45 adds the zero-config derivations
//! [`suggest_depth`] and [`infer_layers`]; T36 adds the Lakos metrics
//! [`lakos_metrics`] (CCD/ACD/NCCD, Kap. 7); T37 adds the virtual
//! refactoring simulation ([`SimOp`], [`apply_ops`], [`simulate`],
//! [`SimDelta`]); the later tasks build their findings on top of them.

mod baseline;
mod cuts;
mod cycles;
mod derive;
mod health;
mod lakos;
mod layer_drift;
mod layers;
mod leaky;
mod metrics;
mod pc;
mod review;
mod rules;
mod scc;
mod sdp;
mod simulate;
mod tangles;
#[cfg(test)]
pub(crate) mod tests_helpers;
mod zones;

pub use baseline::{Baseline, BaselineDiff, cycle_hash, diff};
pub use cuts::suggest_cuts;
pub use cycles::{CycleSample, DEFAULT_EDGE_BUDGET, sample_cycles, sample_cycles_bounded};
pub use derive::{DerivedLayers, infer_layers, suggest_depth};
pub use health::health_index;
pub use lakos::lakos_metrics;
pub use layer_drift::layer_drift;
pub use layers::{check_layer_pairs, check_layers};
pub use leaky::leaky_encapsulation;
pub use metrics::{compute_metrics, summarize};
pub use pc::propagation_cost;
pub use review::{analyze_review, match_review_entry_points};
pub use rules::{check_dependency_rules, check_orphans, validate_dependency_rules};
pub use scc::{Scc, find_sccs};
pub use sdp::sdp_violations;
pub use simulate::{SimDelta, SimOp, apply_ops, simulate};
pub use tangles::{hub_gini, tangledness};
pub use zones::zone_stats;
