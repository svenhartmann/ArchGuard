//! Gather existing evidence; documentation never invokes builds or scanners.
use archguard_core::{
    ArchguardError, CallAnalysis, Config, Cut, Finding, Graph, Report, ServiceTopology, aggregate,
};
use archguard_report::{Arc42Document, Arc42Inputs};
use serde_json::{Value, json};
use std::path::Path;

pub fn arc42_from_evidence(
    graph: &Graph,
    config: &Config,
    findings: &[Finding],
    calls: &CallAnalysis,
    services: &ServiceTopology,
    insights: &Value,
) -> Result<Arc42Document, ArchguardError> {
    let view = aggregate(graph, &Cut::at_depth(graph, config.modules.depth as usize));
    let report = Report {
        schema_version: 1,
        semantic: graph.semantic.clone(),
        review_kpis: graph.review_kpis.clone(),
        summary: archguard_analysis::summarize(
            &view,
            &archguard_analysis::find_sccs(&view),
            findings,
        ),
        config: Default::default(),
        findings: findings.to_vec(),
        metrics: Default::default(),
    };
    let paths = crate::reference_paths::source_reference_paths(Path::new(&graph.root), config)?;
    Ok(archguard_report::render_arc42(Arc42Inputs {
        source_graph: graph,
        view: &view,
        report: &report,
        config,
        calls,
        services,
        insights,
        source_paths: &paths,
    }))
}

pub fn arc42_document(
    root: &Path,
    config: &Config,
    graph: &Graph,
    report: &Report,
) -> Result<Arc42Document, ArchguardError> {
    let calls = crate::semantic_calls::analyze_calls(root, config);
    let services = crate::services::analyze_services(root)?;
    let insights = json!({
        "patterns":crate::patterns::analyze_architecture_patterns(root,config,graph,&calls,&services),
        "code_metrics":crate::code_metrics(&calls),
        "test_coverage":crate::test_coverage::read_test_coverage(root,config,graph),
        "quality":crate::quality_findings::read_quality_findings(root,config,graph),
        "security":crate::security::read_security(root,config),
        "contracts":crate::api_contracts::read_api_contracts(root,&services),
        "measurement_runs":crate::measurement_runs::read_measurement_runs(root),
        "mutations":crate::mutation_report::read_mutations(root,config,graph),
        "duplicates":crate::duplicates::read_duplicates(root,config,graph)
    });
    arc42_from_evidence(
        graph,
        config,
        &report.findings,
        &calls,
        &services,
        &insights,
    )
}
