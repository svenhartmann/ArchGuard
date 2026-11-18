//! GraphQL contract records (ADR 0014), shared by reading and comparing.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphqlReport {
    /// `ready`, `partial` (diagnostics present).
    pub status: String,
    pub schemas: Vec<GraphqlContract>,
    pub links: Vec<GraphqlLink>,
    pub unresolved: Vec<GraphqlUnresolved>,
    /// `no_baseline`, `compared` or `baseline_invalid`.
    pub comparison_status: String,
    pub changes: Vec<GraphqlChange>,
    pub diagnostics: Vec<String>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphqlContract {
    /// Owning service id; `None` when no service scope contains the files.
    pub provider: Option<String>,
    pub files: Vec<String>,
    pub sha256: String,
    /// Root fields as schema coordinates (`Query.user`, `Mutation.createUser`).
    pub operations: Vec<String>,
    pub types: BTreeMap<String, GraphqlType>,
    /// Directive uses as written (`User @key(fields: "id")`); not interpreted.
    pub directives: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphqlType {
    pub kind: String,
    /// Fields, input fields, enum values or union members (empty type).
    pub fields: BTreeMap<String, GraphqlField>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphqlField {
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub arguments: BTreeMap<String, GraphqlArgument>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphqlArgument {
    #[serde(rename = "type")]
    pub ty: String,
    pub has_default: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphqlLink {
    pub consumer: String,
    pub provider: String,
    /// `query GetUser`; empty for a configured endpoint.
    pub operation: String,
    pub root_fields: Vec<String>,
    pub file: String,
    pub line: Option<u32>,
    /// `operation_document` or `configured_endpoint`.
    pub evidence: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphqlUnresolved {
    pub consumer: Option<String>,
    pub operation: String,
    pub file: String,
    pub line: Option<u32>,
    pub reason: String,
}
/// A schema change against the baseline, shaped like the OpenAPI `ApiChange`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphqlChange {
    /// Schema coordinate: `User`, `User.email`, `Mutation.createUser(email:)`.
    pub operation: String,
    pub kind: String,
    /// `breaking`, `review` or `info`.
    pub severity: String,
    pub message: String,
    /// Services whose linked operations select the changed root field.
    pub consumers: Vec<String>,
}

/// Contracts match across runs by provider, or by files when unowned.
pub(crate) fn key(c: &GraphqlContract) -> String {
    c.provider
        .clone()
        .unwrap_or_else(|| format!("file:{}", c.files.join(",")))
}
