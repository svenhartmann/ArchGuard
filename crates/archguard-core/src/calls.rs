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

/// The fixed coverage notes analyzers report among their diagnostics.
/// Clients recognize them by their text (e.g. to translate them); other
/// diagnostics (file errors, counts, compiler output) vary and stay as
/// reported.
impl CallAnalyzer {
    pub const RUST_SYNTAX: &str = "Declarations, complexity and Halstead per function come from syntax. Traits count as abstract; structs, enums and unions as concrete. Implemented traits (impl Trait for Type) and supertraits resolve from syntax within the crate; traits of std, core and alloc are external, other crates' traits, prelude names and blanket impls stay unresolved. Rust has no implementation inheritance: DIT/NOC are not applicable. Files with item macro invocations or syntax errors keep their types unknown, and macro bodies that are not expressions stay opaque.";
    pub const RUST_UNTRUSTED: &str = "Rust calls and type references stay unresolved. Set ARCHGUARD_TRUST_PROJECT_COMPILER=1 outside the repository to resolve them with rust-analyzer; build scripts and proc macros are never run.";
    pub const RUST_RESOLVED: &str = "Calls, implemented traits, supertraits and type references in fields, signatures and impl bodies are resolved with rust-analyzer without build scripts, proc macros or cfg(test); code they generate, test-only code and macro bodies that are not expressions stay unresolved.";
    pub const PYTHON_SYNTAX: &str = "Syntax-only analysis: classes, functions, complexity and Halstead counts. Call targets are resolved statically from imports, module functions, classes, self/cls and super(); calls on other values, rebound names and star imports stay unresolved. Types are not resolved; abstractness is read from ABC/Protocol bases and @abstractmethod.";
    pub const PHP_TOKENS: &str = "Token-based analysis: classes, functions, complexity and Halstead counts. Call targets are resolved statically from namespaces, use imports, project classes, traits, $this, self, static (read as self) and parent; calls on other variables, dynamic names and $this inside closures stay unresolved; callables passed to call_user_func are not followed. Types are not resolved; abstractness is read from interface and abstract class declarations.";
    pub const KOTLIN_TOKENS: &str = "Token-based analysis: classes, objects, functions, complexity and Halstead counts. Call targets are resolved statically from the package, imports (aliases; star imports with a single candidate), Kotlin classes, objects, companion objects, top-level functions, this, super and the project class hierarchy; calls on variables, properties and call results, extension functions, every call inside lambdas, local and extension functions, and Java classes of the project stay unresolved; overloads are not told apart. Types are not resolved; abstractness is read from interface, abstract class and sealed declarations. Property accessors, constructors, init blocks and property initializers are not measured, and calls in them are not counted.";
    pub const TYPESCRIPT_MISSING: &str = "No installed TypeScript compiler found. Install TypeScript in the project or configure semantic.typescript.compiler; ArchGuard never downloads packages.";
    pub const TYPESCRIPT_BLOCKED: &str = "A TypeScript compiler was found through this repository but not loaded, because running it would execute repository code. Set ARCHGUARD_TRUST_PROJECT_COMPILER=1 for repositories you trust.";
    /// All of the notes above.
    pub const NOTES: [&str; 8] = [
        Self::RUST_SYNTAX,
        Self::RUST_UNTRUSTED,
        Self::RUST_RESOLVED,
        Self::PYTHON_SYNTAX,
        Self::PHP_TOKENS,
        Self::KOTLIN_TOKENS,
        Self::TYPESCRIPT_MISSING,
        Self::TYPESCRIPT_BLOCKED,
    ];
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

impl CallAnalysis {
    /// Rewrite every class, method and field identity of a report, so reports
    /// of several providers or compiler scopes can share one id space.
    pub fn reidentify(&mut self, identity: &dyn Fn(&str) -> String) {
        for class in &mut self.classes {
            class.id = identity(&class.id);
            class.supertypes = class.supertypes.iter().map(|id| identity(id)).collect();
            for site in class.supertype_sites.iter_mut().flatten() {
                site.target = identity(&site.target);
            }
        }
        for method in &mut self.methods {
            method.id = identity(&method.id);
            method.class_id = identity(&method.class_id);
        }
        for call in &mut self.calls {
            call.from_class = identity(&call.from_class);
            call.from_method = call.from_method.as_deref().map(identity);
            call.to_class = call.to_class.as_deref().map(identity);
            call.to_method = call.to_method.as_deref().map(identity);
        }
        let Some(details) = &mut self.details else {
            return;
        };
        for used in &mut details.type_uses {
            used.from_class = identity(&used.from_class);
            used.from_method = used.from_method.as_deref().map(identity);
            used.to_class = used.to_class.as_deref().map(identity);
        }
        for state in &mut details.class_states {
            state.class_id = identity(&state.class_id);
            for field in &mut state.fields {
                field.id = identity(&field.id);
            }
            for access in &mut state.accesses {
                access.method = identity(&access.method);
                access.field = identity(&access.field);
            }
        }
        for metric in &mut details.method_metrics {
            metric.method_id = identity(&metric.method_id);
        }
        for parent in &mut details.inheritance {
            parent.class_id = identity(&parent.class_id);
            parent.base_class = parent.base_class.as_deref().map(identity);
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
