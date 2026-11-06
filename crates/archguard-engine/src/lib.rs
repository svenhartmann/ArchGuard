//! `archguard-engine` — L2: loads the config, orchestrates the
//! extract → analyze → report pipeline and caches the graph. Lands in T21–T23.
//!
//! Synchronous by design: async lives only in `archguard-mcp`.

mod atlas;
mod world;
pub use atlas::{ArchitectureAtlas, AtlasArea, AtlasConnection, architecture_atlas};
pub use world::{ArchitectureWorld, WorldCoverage, WorldEntity, architecture_world};
mod api_contracts;
mod arc42;
mod architecture;
mod cache;
mod change_context;
mod change_context_graph;
mod class_cohesion;
mod code_metrics;
mod knowledge_metrics;
mod measurement_runs;
pub use code_metrics::{
    ClassMetrics, CodeMetricsReport, CountMetric, HalsteadMetrics, MethodMetrics, code_metrics,
};
pub use knowledge_metrics::{Contributor, KnowledgeMetric, KnowledgeReport, knowledge_metrics};
pub use measurement_runs::{
    MeasurementGroup, MeasurementReport, MeasurementRun, import_measurement_runs,
    measurement_runs_fingerprint, read_measurement_runs,
};
mod co_change;
mod cohesion;
pub use class_cohesion::class_cohesion;
mod comparison;
mod staged;
pub use staged::{GitBranch, GitBranches, compare_branches, compare_staged, list_git_branches};
mod config;
mod coverage_review;
mod csharp_calls;
mod dep_inventory;
mod duplicates;
mod effective;
mod evidence_import;
mod go_calls;
mod go_sources;
mod java_calls;
mod kpi_payload;
mod mutation_report;
mod ownership;
mod patterns;
mod performance;
mod php_calls;
mod pipeline;
mod project_files;
mod python_calls;
mod quality_findings;
mod quality_gate;
mod quality_history;
mod reference_paths;
mod review_metrics;
mod review_register;
mod review_session;
mod review_sources;
mod risk_gap;
mod rust_calls;
mod rust_scip;
mod security;
// Declared here, not in their modules: a test file that imports its parent
// back would close a file cycle.
#[cfg(test)]
mod manifest_render_tests;
#[cfg(test)]
mod rust_imports_tests;
#[cfg(test)]
mod security_tests;
mod semantic_calls;
mod semantic_graph;
mod service_projects;
mod services;
mod snapshot;
mod target_architecture;
mod test_coverage;
mod test_evidence;
mod typescript_calls;
mod workspace;
mod workspace_sources;

pub use api_contracts::{
    ApiChange, ApiContract, ApiContractReport, ApiOperation, ApiUsage, api_contracts_fingerprint,
    import_api_contract, read_api_contracts,
};
pub use arc42::{arc42_document, arc42_from_evidence};
pub use architecture::architecture_context;
pub use cache::{fingerprint, load_graph_if_fresh, save_graph, source_fingerprint};
pub use change_context::{ChangeContext, ChangeContextOptions, build_change_context};
pub use co_change::{CoChangeFile, CoChangeOptions, CoChangePair, CoChangeReport, co_change};
pub use cohesion::{
    CohesionBoundary, CohesionGroup, CohesionPackage, CohesionReport, analyze_cohesion,
};
pub use comparison::{Comparison, compare_to_ref};
pub use config::{DEFAULT_CONFIG_PATH, LoadedConfig, default_config_string, load_config};
pub use coverage_review::{CoverageReview, CoverageReviewFile, test_coverage_review};
pub use csharp_calls::analyze_csharp_calls;
pub use dep_inventory::{DependencyInventory, ExternalDependency, dependency_inventory};
pub use duplicates::{
    DuplicateGroup, DuplicateOccurrence, DuplicateScanOptions, DuplicationReport,
    duplication_fingerprint, import_duplicates, read_duplicates, scan_duplicates,
};
pub use effective::{
    AnalysisRun, analyze, analyze_cached, analyze_effective, analyze_run, analyze_run_quiet,
    derive_project_hints, effective_config_string,
};
pub use java_calls::analyze_java_calls;
pub use kpi_payload::ReviewKpis;
pub use mutation_report::{
    MutationFile, MutationLocation, MutationPosition, MutationRecord, MutationReport,
    MutationSummary, import_mutations, mutation_fingerprint, read_mutations,
};
pub use ownership::{
    OwnerAssignment, OwnershipDependency, OwnershipReport, OwnershipStatus, analyze_ownership,
};
pub use patterns::{
    ArchitecturePatternReport, PatternAssessment, PatternEvidence, analyze_architecture_patterns,
    architecture_patterns, pattern_inputs_fingerprint,
};
pub use performance::{analyze_performance, review_zones};
pub use pipeline::Level;
pub use project_files::write_project_file;
pub use quality_findings::{
    QualityFinding, QualityReport, import_quality_findings, quality_findings_fingerprint,
    read_quality_findings,
};
pub use quality_gate::{
    ChangedCodeCoverage, ChangedFileCoverage, QualityGateFinding, QualityGateMethod,
    QualityGateOptions, QualityGateReport, evaluate_quality_gate,
};
pub use quality_history::{
    ModuleQualityDelta, ModuleQualitySnapshot, QualityAnalyzerIdentity, QualityComparison,
    QualityHistoryEntry, QualityHistoryPoint, QualityHistoryReport, QualityMetricDelta,
    QualityModulePoint, QualitySnapshot, RecordedMetric, build_quality_snapshot,
    compare_quality_snapshots, quality_history_fingerprint, read_quality_history,
    record_quality_snapshot,
};
pub use reference_paths::source_reference_paths;
pub use review_metrics::{ReviewMetrics, review_metrics};
pub use review_register::{
    LEGACY_DECISIONS_REL_PATH, REGISTER_REL_PATH, iso_from_unix, load_register, register_path,
    save_register, upsert_decision,
};
pub use review_session::{
    ReviewDecisionImport, ReviewFinding, ReviewReport, SavedReview, apply_review_decisions,
    generate_review, import_review_decisions, read_saved_review, review_session_fingerprint,
    save_review,
};
pub use review_sources::collect_review_sources;
pub use risk_gap::risk_coverage_gaps;
pub use security::{
    SecurityFinding, SecurityReport, SecurityScanOptions, SecurityTarget, read_security,
    scan_security, security_fingerprint,
};
pub use semantic_calls::{analyze_calls, semantic_fingerprints};
pub use services::analyze_services;
pub use snapshot::{
    Snapshot, SnapshotRow, latest_snapshot, read_snapshots, read_trend, snapshot_record,
    write_snapshot,
};
pub use snapshot::{snapshot_scope, write_snapshot_with_scope};
pub use target_architecture::analyze_target_architecture;
pub use test_coverage::{
    CoverageCounter, CoverageLine, TestCoverageFile, TestCoverageReport, import_test_coverage,
    read_test_coverage, test_coverage_fingerprint,
};
pub use test_evidence::{
    TestCaseEvidence, TestEvidenceReport, TestImpactMatch, TestImpactReport, TestSourceMapping,
    import_test_evidence, read_test_evidence, test_evidence_fingerprint, test_impact,
};
pub use typescript_calls::analyze_typescript_calls;
pub use workspace::{analyze_workspace, analyze_workspace_with_topology};
pub use workspace_sources::{
    WorkspaceImpactedTest, WorkspaceProjectCoverage, WorkspaceSourceFile, WorkspaceSourceReport,
    WorkspaceTestEvidence, WorkspaceTestImpact, WorkspaceTestMapping, analyze_workspace_sources,
    workspace_sources_fingerprint, workspace_test_impact,
};

/// Facade for L3 consumers, mirroring [`core`] for the baseline types
/// ([`archguard_analysis::Baseline`], [`archguard_analysis::cycle_hash`]) the
/// CLI's `check --baseline` and `baseline write|diff` need (Kap. 9).
pub use archguard_analysis as analysis;
/// Facade for L3 consumers (Kap. 3.2: CLI→ENG and MCP→ENG are the only
/// edges into the lower layers): lets `archguard-cli`/`archguard-mcp` name
/// [`archguard_core`] types without a direct dependency.
pub use archguard_core as core;
/// Facade for L3 consumers, mirroring [`core`] for the renderers.
pub use archguard_report as report;
/// Facade for L3 consumers, mirroring [`core`] for the graph-DB mirror
/// ([`archguard_store::GraphStore`], [`archguard_store::QueryResult`],
/// [`archguard_store::sanitize_query`] and — behind the `db` feature —
/// [`archguard_store::LadybugStore`]) the CLI's `db sync|query` and the MCP's
/// `graph_query` need (Kap. 8/9, T33).
pub use archguard_store as store;

/// Placeholder kept from the crate skeleton; [`analyze`] is the real API.
pub fn placeholder() -> &'static str {
    "archguard-engine"
}

#[cfg(test)]
mod tests {
    use super::placeholder;

    #[test]
    fn placeholder_returns_crate_name() {
        assert_eq!(placeholder(), "archguard-engine");
    }
}
