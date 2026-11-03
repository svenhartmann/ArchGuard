use crate::engineering_tools::EngineeringParams;
use archguard_engine::core::StagedArchitectureDiff;
use rmcp::{ErrorData, schemars};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct StagedDiffParams {
    #[serde(flatten)]
    pub page: EngineeringParams,
    /// Commit to compare against; defaults to HEAD.
    pub base: Option<String>,
    /// Return the revision from page one on every subsequent page.
    pub expected_revision: Option<String>,
}
pub fn value(diff: &StagedArchitectureDiff, params: &StagedDiffParams) -> Result<Value, ErrorData> {
    let revision = format!("{}:{}", diff.base_commit, diff.index_fingerprint);
    if params
        .expected_revision
        .as_ref()
        .is_some_and(|v| v != &revision)
        || (params.page.offset.unwrap_or(0) > 0 && params.expected_revision.is_none())
    {
        return Err(crate::tools::invalid_params(
            "staged comparison changed or expected_revision missing; restart from offset 0",
        ));
    }
    let mut rows = vec![];
    for (kind, nodes) in [
        ("added_node", &diff.added_nodes),
        ("removed_node", &diff.removed_nodes),
    ] {
        for node in nodes {
            rows.push(json!({"kind":kind,"node":node}));
        }
    }
    for (kind, edges) in [
        ("added_edge", &diff.added_edges),
        ("removed_edge", &diff.removed_edges),
        ("changed_edge", &diff.changed_edges),
    ] {
        for e in edges {
            rows.push(json!({"kind":kind,"from":e.from,"to":e.to,"before_total":e.before.len(),"after_total":e.after.len()}));
            for (side, refs) in [("base", &e.before), ("index", &e.after)] {
                for r in refs {
                    rows.push(json!({"kind":"edge_evidence","side":side,"from":e.from,"to":e.to,"evidence":r}));
                }
            }
        }
    }
    for (kind, findings) in [
        ("new_finding", &diff.new_findings),
        ("resolved_finding", &diff.resolved_findings),
    ] {
        for f in findings {
            rows.push(json!({"kind":kind,"id":f.id,"finding_kind":f.kind,"severity":f.severity,"message":f.message,"nodes_total":f.nodes.len()}));
            for node in &f.nodes {
                rows.push(json!({"kind":"finding_node","change":kind,"finding":f.id,"node":node}));
            }
            for e in &f.evidence {
                rows.push(
                    json!({"kind":"finding_evidence","change":kind,"finding":f.id,"evidence":e}),
                );
            }
        }
    }
    for (side, graph) in [("base", &diff.base_graph), ("index", &diff.staged_graph)] {
        if let Some(coverage) = &graph.semantic {
            for diagnostic in &coverage.diagnostics {
                rows.push(json!({"kind":"coverage_diagnostic","side":side,"message":diagnostic}));
            }
        }
    }
    for message in &diff.limitations {
        rows.push(json!({"kind":"limitation","message":message}));
    }
    let query = params.page.query.as_deref().unwrap_or("").to_lowercase();
    crate::engineering_tools::page(
        "ready",
        json!({"revision":revision,"definition":diff.definition,"target":"index","base_commit":diff.base_commit,"policy_source":diff.policy_source,"added_edges":diff.added_edges.len(),"removed_edges":diff.removed_edges.len(),"changed_edges":diff.changed_edges.len(),"new_findings":diff.new_findings.len(),"resolved_findings":diff.resolved_findings.len(),"base_semantic_status":diff.base_graph.semantic.as_ref().map(|s|&s.status),"index_semantic_status":diff.staged_graph.semantic.as_ref().map(|s|&s.status)}),
        rows.iter()
            .filter(|r| query.is_empty() || r.to_string().to_lowercase().contains(&query)),
        &params.page,
        |r| r.clone(),
    )
}
