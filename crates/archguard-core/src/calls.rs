//! Compiler-backed call analysis. Resolved project calls and inheritance can
//! supplement the dependency graph; the detailed call model remains a source
//! snapshot, independent of hypothetical graph edits.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallClass {
    /// Analyzer language; absent in legacy Java reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub id: String,
    pub name: String,
    pub package: String,
    pub file: String,
    pub graph_node: String,
    pub line: u32,
    pub kind: String,
    /// Compiler-declared abstractness; absent in older call reports.
    #[serde(default)]
    pub is_abstract: Option<bool>,
    /// Declared visibility, e.g. `public`, `protected`, `private`, `package`, or `internal`.
    #[serde(default)]
    pub visibility: Option<String>,
    /// Direct superclass/interface identities, sorted deterministically.
    #[serde(default)]
    pub supertypes: Vec<String>,
    /// Compiler-resolved base clauses and their actual source declaration.
    /// In C#, separate partial declarations can contribute different bases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supertype_sites: Option<Vec<CallSupertypeSite>>,
    /// All compiler-known source files declaring this type. A partial C# type
    /// has no unique file target for an implicit constructor or a base type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaration_files: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallSupertypeSite {
    pub target: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallMethod {
    pub id: String,
    pub class_id: String,
    pub name: String,
    pub signature: String,
    pub file: String,
    pub line: u32,
    /// Inclusive compiler source range; absent in older snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    pub is_static: bool,
    /// Source-body AST measurements; absent for legacy reports, declarations
    /// without a body, or invalid syntax. Not bytecode or runtime complexity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity: Option<MethodComplexity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodComplexity {
    pub cyclomatic: u32,
    pub max_nesting: u32,
    /// Versioned source-counting rule, currently `ast-decisions-v1`.
    pub definition: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodCall {
    pub from_class: String,
    pub from_method: Option<String>,
    pub to_class: Option<String>,
    pub to_method: Option<String>,
    pub target_label: String,
    /// `static`, `instance`, `interface`, `constructor`, or `unknown`.
    pub dispatch: String,
    /// `project`, `external`, or `unresolved`; never a runtime dispatch claim.
    pub resolution: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallAnalysis {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<crate::semantic_details::SemanticDetails>,
    /// `ready`, `partial`, `unavailable`, or `not_applicable`.
    pub status: String,
    /// Legacy Java API baseline; zero when no single Java baseline applies.
    #[serde(default)]
    pub java_release: u32,
    /// Independent coverage per language/project. Never infer that a working
    /// provider covers files belonging to a missing or unsupported provider.
    #[serde(default)]
    pub analyzers: Vec<CallAnalyzer>,
    pub files: usize,
    pub classes: Vec<CallClass>,
    pub methods: Vec<CallMethod>,
    pub calls: Vec<MethodCall>,
    pub diagnostics: Vec<String>,
}

impl Default for CallAnalysis {
    fn default() -> Self {
        Self {
            details: None,
            status: "not_applicable".into(),
            java_release: 17,
            analyzers: Vec::new(),
            files: 0,
            classes: Vec::new(),
            methods: Vec::new(),
            calls: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallAnalyzer {
    pub language: String,
    pub backend: String,
    pub status: String,
    pub files: usize,
    pub version: Option<String>,
    pub project: Option<String>,
    pub diagnostics: Vec<String>,
}

/// Coverage of compiler-resolved calls and inheritance projected into a graph.
/// Type-use evidence includes resolved declaration types when the provider
/// supplies semantic details; legacy reports have no such coverage. Counts refer to the
/// original source graph and remain unchanged by aggregation or simulation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticDependencyCoverage {
    /// `ready`, `partial`, `unavailable`, or `not_applicable` for this projection.
    pub status: String,
    pub analyzers: Vec<CallAnalyzer>,
    pub projected_calls: usize,
    pub projected_supertypes: usize,
    #[serde(default)]
    pub projected_type_uses: usize,
    #[serde(default)]
    pub skipped_type_uses: usize,
    pub skipped_project_calls: usize,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SemanticConfig {
    pub java: JavaSemanticConfig,
    pub csharp: CSharpSemanticConfig,
    pub typescript: TypeScriptSemanticConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TypeScriptSemanticConfig {
    /// Existing tsconfig; defaults to the root tsconfig.json if present.
    pub project: Option<String>,
    /// Installed typescript/lib/typescript.js; never downloaded by ArchGuard.
    pub compiler: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JavaSemanticConfig {
    pub release: Option<u32>,
    /// Existing jars/directories; relative paths resolve against the scan root.
    pub classpath: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CSharpSemanticConfig {
    pub project: Option<String>,
    pub configuration: String,
    pub target_framework: Option<String>,
}
impl Default for CSharpSemanticConfig {
    fn default() -> Self {
        Self {
            project: None,
            configuration: "Debug".into(),
            target_framework: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn method_complexity_is_optional_and_survives_roundtrip() {
        let legacy = serde_json::json!({"id":"A#run()","class_id":"A","name":"run","signature":"run()","file":"A.java","line":1,"is_static":false});
        let decoded: CallMethod = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
        let mut measured = legacy;
        measured["complexity"] =
            serde_json::json!({"cyclomatic":4,"max_nesting":2,"definition":"ast-decisions-v1"});
        measured["end_line"] = serde_json::json!(12);
        let decoded: CallMethod = serde_json::from_value(measured.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), measured);
    }

    #[test]
    fn analyzer_coverage_survives_wire_roundtrip_and_legacy_reports_load() {
        let legacy = serde_json::to_value(CallAnalysis::default()).unwrap();
        let mut report = legacy.clone();
        report["analyzers"] = serde_json::json!([{"language":"csharp","backend":"roslyn","status":"partial","files":2,"version":"8.0","project":"app.csproj","diagnostics":["Missing reference"]}]);
        let parsed: CallAnalysis = serde_json::from_value(report.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(parsed).unwrap()["analyzers"],
            report["analyzers"]
        );
        assert!(serde_json::from_value::<CallAnalysis>(legacy).is_ok());
    }
}
