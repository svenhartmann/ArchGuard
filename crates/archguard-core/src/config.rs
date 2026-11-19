//! The parsed `archguard.toml` root that composes the sections of every feature.

use serde::{Deserialize, Serialize};

use crate::architecture::ArchitectureConfig;
use crate::calls::SemanticConfig;
use crate::diagrams::Arc42Config;
use crate::licenses::LicensesConfig;
use crate::performance::PerformanceConfig;
use crate::review::ReviewConfig;
use crate::rules::{DependencyRule, OwnershipRule};
use crate::runtime::RuntimeConfig;
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
    /// `[runtime]`: thresholds of the runtime findings (ADR 0004).
    pub runtime: RuntimeConfig,
    pub architecture: ArchitectureConfig,
    pub semantic: SemanticConfig,
    pub store: StoreConfig,
    /// `[arc42]`: which catalogue diagrams the document shows (ADR 0006).
    pub arc42: Arc42Config,
    /// `[licenses]`: licence policy of third-party dependencies (ADR 0016).
    pub licenses: LicensesConfig,
    /// `[views]`: conventions `check` enforces on the UI-flow and domain
    /// views (ADR 0021 and 0022, decision 6); none by default.
    pub views: ViewsConfig,
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
            runtime: RuntimeConfig::default(),
            architecture: ArchitectureConfig::default(),
            semantic: SemanticConfig::default(),
            store: StoreConfig::default(),
            arc42: Arc42Config::default(),
            licenses: LicensesConfig::default(),
            views: ViewsConfig::default(),
        }
    }
}

/// `[views]`: opt-in conventions; an empty list checks nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ViewsConfig {
    pub conventions: Vec<ViewConvention>,
}

/// One entry of `[views] conventions`; an unknown name is a config error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewConvention {
    NavigationLiteral,
    EndpointAccessDeclared,
    HttpUrlLiteral,
    FrontendBackendAgree,
    StateChangesInEntity,
    EventsThroughRaiseFunction,
}

impl ViewConvention {
    /// The configuration name; also the rule part of a finding id. The engine
    /// matches on it because `archguard_core` re-exports only [`Config`].
    pub fn name(self) -> &'static str {
        match self {
            Self::NavigationLiteral => "navigation_literal",
            Self::EndpointAccessDeclared => "endpoint_access_declared",
            Self::HttpUrlLiteral => "http_url_literal",
            Self::FrontendBackendAgree => "frontend_backend_agree",
            Self::StateChangesInEntity => "state_changes_in_entity",
            Self::EventsThroughRaiseFunction => "events_through_raise_function",
        }
    }
}
