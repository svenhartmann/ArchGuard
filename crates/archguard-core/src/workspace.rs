//! Manifest-backed project dependencies; distinct from deployment and runtime calls.
use crate::services::{ServiceEvidence, ServiceProject};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceReport {
    pub schema_version: u32,
    pub root: String,
    /// ready, partial or not_applicable; completeness is limited to supported static manifests.
    pub status: String,
    pub projects: Vec<ServiceProject>,
    pub dependencies: Vec<WorkspaceDependency>,
    /// Cyclic strongly connected project groups, not ordered runtime paths.
    pub cycles: Vec<Vec<String>>,
    pub unresolved: Vec<WorkspaceUnresolved>,
    pub diagnostics: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceDependency {
    pub from: String,
    pub to: String,
    /// compile, runtime, test, build, provided, optional, or peer.
    pub scope: String,
    /// maven_dependency, npm_workspace, npm_file, dotnet_project_reference, cargo_path.
    pub kind: String,
    pub evidence: Vec<ServiceEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceUnresolved {
    pub project: String,
    pub target: String,
    pub reason: String,
    pub evidence: Vec<ServiceEvidence>,
}
