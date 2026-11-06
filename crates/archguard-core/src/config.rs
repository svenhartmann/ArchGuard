//! The parsed `archguard.toml` root that composes the sections of every feature.

use serde::{Deserialize, Serialize};

use crate::architecture::ArchitectureConfig;
use crate::calls::SemanticConfig;
use crate::performance::PerformanceConfig;
use crate::review::ReviewConfig;
use crate::rules::{DependencyRule, OwnershipRule};
use crate::settings::{LayerConfig, ModulesConfig, ProjectConfig, RulesConfig, StoreConfig};

// ------------------------------------------------------- configuration (Kap. 11)

/// Parsed `archguard.toml` (Kap. 11).
///
/// Every section and every field is optional (`#[serde(default)]`
/// throughout); [`Default`] yields exactly the defaults documented in
/// Kap. 11, so an empty file parses to the default configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Top-level switch of the layer derivation (Kap. 11); `false` keeps
    /// only the layers written in `archguard.toml`.
    pub derive_layers: bool,
    pub project: ProjectConfig,
    pub modules: ModulesConfig,
    /// The `[[layers]]` entries in file order; no layers by default.
    pub layers: Vec<LayerConfig>,
    pub dependency_rules: Vec<DependencyRule>,
    pub ownership_rules: Vec<OwnershipRule>,
    pub rules: RulesConfig,
    pub review: ReviewConfig,
    pub performance: PerformanceConfig,
    pub architecture: ArchitectureConfig,
    pub semantic: SemanticConfig,
    pub store: StoreConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            derive_layers: true,
            project: ProjectConfig::default(),
            modules: ModulesConfig::default(),
            layers: Vec::new(),
            dependency_rules: Vec::new(),
            ownership_rules: Vec::new(),
            rules: RulesConfig::default(),
            review: ReviewConfig::default(),
            performance: PerformanceConfig::default(),
            architecture: ArchitectureConfig::default(),
            semantic: SemanticConfig::default(),
            store: StoreConfig::default(),
        }
    }
}
