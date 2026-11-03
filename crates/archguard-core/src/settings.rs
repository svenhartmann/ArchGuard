//! The `[project]`, `[modules]`, `[[layers]]`, `[rules]` and `[store]` sections of the configuration.

use serde::{Deserialize, Serialize};

/// `[project]` — where to look for sources and what to skip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectConfig {
    pub source_roots: Vec<String>,
    /// Globs excluded from the analysis.
    pub exclude: Vec<String>,
    /// `"auto"` or explicit language ids, e.g. `["ts", "py"]`.
    pub languages: Vec<String>,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            source_roots: vec!["src".to_string()],
            exclude: vec![
                "**/tests/**".to_string(),
                "**/*.spec.*".to_string(),
                "**/generated/**".to_string(),
            ],
            languages: vec!["auto".to_string()],
        }
    }
}

/// `[modules]` — the zoom level at which modules are named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModulesConfig {
    /// Default zoom: `app/billing/x.ts` is seen as node `app/billing`.
    pub depth: u32,
}

impl Default for ModulesConfig {
    fn default() -> Self {
        Self { depth: 2 }
    }
}

/// How a [`LayerConfig`] came to be, deciding the severity of its
/// violations (Kap. 11): declared layers are `Error` gates, derived ones
/// are `Info` hints that never fail `check`.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum LayerOrigin {
    /// Written by the user in `archguard.toml`.
    #[default]
    Declared,
    /// Produced by the engine's layer derivation.
    Derived,
}

/// One `[[layers]]` entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayerConfig {
    pub name: String,
    /// Globs matching node names (source-root-relative).
    pub contains: Vec<String>,
    /// Layers this one may depend on; empty means bottom layer.
    pub may_depend_on: Vec<String>,
    /// Set by the engine during the merge (Kap. 11), never parsed from
    /// toml and never serialized back.
    #[serde(default, skip)]
    pub origin: LayerOrigin,
}

/// `[rules]` — cycle policy and quality gates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RulesConfig {
    pub deny_cycles: DenyCycles,
    /// Largest allowed cyclic SCC size; independent of cycle denial and
    /// baselines. `0` and `1` both reject every cyclic SCC (not zero-off).
    pub max_scc_size: u32,
    /// Path of the baseline file.
    pub baseline: String,
    /// Quality gate for average component dependency (including self)
    /// in the configured module view; `0` disables it.
    pub max_acd: f32,
    /// Quality gate for the instability of bottom-layer modules; `0` disables it.
    pub max_instability_core: f32,
}

impl Default for RulesConfig {
    fn default() -> Self {
        Self {
            deny_cycles: DenyCycles::Module,
            max_scc_size: 1,
            baseline: ".archguard/baseline.json".to_string(),
            max_acd: 0.0,
            max_instability_core: 0.0,
        }
    }
}

/// The `deny_cycles` policy (Kap. 11).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum DenyCycles {
    /// Cycles between modules are forbidden.
    #[default]
    Module,
    /// Even file-level cycles are forbidden.
    File,
    /// Cycles are allowed.
    Off,
}

/// `[store]` — the graph DB mirror (Kap. 8, feature "db").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StoreConfig {
    /// Path of the graph DB mirror.
    pub db: String,
    /// `true`: every `archguard check` refreshes the DB mirror.
    pub sync_on_check: bool,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            db: ".archguard/graph.db".to_string(),
            sync_on_check: false,
        }
    }
}
