use crate::engineering_tools::EngineeringParams;
use archguard_engine::ArchitecturePatternReport;
use rmcp::ErrorData;
use serde_json::{Value, json};
/// Flatten assessments before paging so evidence and nodes never disappear
/// merely because one candidate is larger than the transport budget.
pub fn value(
    report: &ArchitecturePatternReport,
    params: &EngineeringParams,
) -> Result<Value, ErrorData> {
    let query = params.query.as_deref().unwrap_or("").to_lowercase();
    let rows = records(report, &query);
    crate::engineering_tools::page(
        &report.status,
        json!({"definition":report.definition,"assessments_total":report.assessments_total,"coverage":report.coverage,"interpretation":"Architecture hypotheses, not policy or quality gates; no probability is inferred. Query selects whole matching assessments. Follow next_offset while inputs remain unchanged."}),
        rows.iter(),
        params,
        |r| r.clone(),
    )
}

pub(crate) fn records(report: &ArchitecturePatternReport, query: &str) -> Vec<Value> {
    let mut rows = vec![];
    for p in &report.assessments {
        if !query.is_empty()
            && !serde_json::to_string(p)
                .unwrap()
                .to_lowercase()
                .contains(query)
        {
            continue;
        }
        rows.push(json!({"kind":"pattern","id":p.id,"pattern":p.pattern,"scope":p.scope,"status":p.status,"basis":p.basis,"metrics":p.metrics,"nodes_total":p.nodes_total,"services_total":p.services_total,"evidence_total":p.evidence_total,"counter_evidence_total":p.counter_evidence_total}));
        for node in &p.nodes {
            rows.push(json!({"kind":"pattern_node","assessment":p.id,"node":node}));
        }
        for service in &p.services {
            rows.push(json!({"kind":"pattern_service","assessment":p.id,"service":service}));
        }
        for (kind, items) in [
            ("supporting_evidence", &p.evidence),
            ("counter_evidence", &p.counter_evidence),
        ] {
            for e in items {
                rows.push(json!({"kind":kind,"assessment":p.id,"evidence":e}));
            }
        }
        for m in &p.missing_evidence {
            rows.push(json!({"kind":"missing_evidence","assessment":p.id,"message":m}));
        }
    }
    for m in &report.diagnostics {
        rows.push(json!({"kind":"diagnostic","message":m}));
    }
    for m in &report.limitations {
        rows.push(json!({"kind":"limitation","message":m}));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pattern_pages_keep_coverage_and_all_retained_evidence_in_bounded_pages() {
        let root = tempfile::tempdir().unwrap();
        let graph =
            archguard_engine::core::GraphBuilder::new().build(root.path().display().to_string());
        let mut r =
            archguard_engine::architecture_patterns(root.path(), &Default::default(), &graph)
                .unwrap();
        r.assessments[0].nodes = (0..100).map(|i| format!("source{i}.ts")).collect();
        r.assessments[0].nodes_total = 100;
        let mut offset = 0;
        let mut nodes = 0;
        let mut seen = 0;
        loop {
            let result = value(
                &r,
                &EngineeringParams {
                    offset: Some(offset),
                    limit: Some(200),
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(result.to_string().len() <= 4096);
            assert_eq!(result["metadata"]["assessments_total"], r.assessments_total);
            nodes += result["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| v["kind"] == "pattern_node")
                .count();
            seen += result["processed"].as_u64().unwrap();
            if let Some(next) = result["next_offset"].as_u64() {
                assert!(next as usize > offset);
                offset = next as usize;
            } else {
                assert_eq!(seen, result["total"].as_u64().unwrap());
                break;
            }
        }
        assert_eq!(nodes, 100);
    }
}
