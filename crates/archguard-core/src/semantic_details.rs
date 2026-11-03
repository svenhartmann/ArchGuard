//! Optional compiler evidence. Missing details in legacy reports mean unknown.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticDetails {
    /// True only when the provider enumerated all selected declarations,
    /// type uses and calls without reaching an evidence budget. Independent
    /// of symbol resolution (a full enumeration may still contain errors).
    #[serde(default)]
    pub enumeration_complete: Option<bool>,
    pub definition: String,
    pub type_uses: Vec<SemanticTypeUse>,
    pub class_states: Vec<ClassState>,
    #[serde(default)]
    pub cohesion: Vec<ClassCohesion>,
    #[serde(default)]
    pub method_metrics: Vec<SourceMethodMetrics>,
    #[serde(default)]
    pub inheritance: Vec<ClassInheritance>,
}
impl Default for SemanticDetails {
    fn default() -> Self {
        Self {
            enumeration_complete: None,
            definition: "source-types-state/v1".into(),
            type_uses: vec![],
            class_states: vec![],
            cohesion: vec![],
            method_metrics: vec![],
            inheritance: vec![],
        }
    }
}
/// Raw AST counts. Derived floating-point metrics are computed by the engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMethodMetrics {
    pub method_id: String,
    pub definition: String,
    pub cognitive: u32,
    pub distinct_operators: u32,
    pub distinct_operands: u32,
    pub operators: u32,
    pub operands: u32,
    pub source_lines: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassInheritance {
    pub class_id: String,
    pub base_class: Option<String>,
    /// Compiler-known superclass edges to the root, including external bases.
    /// Interfaces/implements are excluded; unavailable chains remain null.
    pub depth: Option<u32>,
    pub status: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticTypeUse {
    pub from_class: String,
    pub from_method: Option<String>,
    pub to_class: Option<String>,
    pub target_label: String,
    /// field, parameter, return, class_annotation, annotation or local.
    pub role: String,
    pub resolution: String,
    pub file: String,
    pub line: u32,
    pub code: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassState {
    pub class_id: String,
    /// ready or partial for state-access analysis in this class.
    pub status: String,
    pub fields: Vec<StateField>,
    pub accesses: Vec<FieldAccess>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateField {
    pub id: String,
    pub name: String,
    pub is_static: bool,
    pub file: String,
    pub line: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldAccess {
    pub method: String,
    pub field: String,
    pub file: String,
    pub line: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassCohesion {
    pub class_id: String,
    pub definition: String,
    pub status: String,
    pub methods: usize,
    pub fields: usize,
    pub lcom4: Option<usize>,
    pub groups: Vec<Vec<String>>,
    pub cyclomatic_sum: Option<u32>,
    pub reason: String,
}
