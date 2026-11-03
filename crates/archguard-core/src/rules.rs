//! Explicit forbidden dependency and ownership directions.

use crate::model::Severity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DependencyRule {
    pub name: String,
    /// Globs over normalized source node names, independent of view depth.
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub severity: Severity,
    pub message: String,
}

impl Default for DependencyRule {
    fn default() -> Self {
        Self {
            name: String::new(),
            from: vec![],
            to: vec![],
            severity: Severity::Error,
            message: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OwnershipRule {
    /// Exact CODEOWNERS tokens, such as `@example/frontend`.
    pub from: String,
    pub to: String,
    pub severity: Severity,
    pub message: String,
}

impl Default for OwnershipRule {
    fn default() -> Self {
        Self {
            from: String::new(),
            to: String::new(),
            severity: Severity::Error,
            message: String::new(),
        }
    }
}
