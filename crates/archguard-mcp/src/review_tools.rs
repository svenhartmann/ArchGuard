//! Read-only pages of the exact recorded review; never edits decisions.
use archguard_engine::SavedReview;
use rmcp::{ErrorData, schemars};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ReviewContextParams {
    #[serde(flatten)]
    pub page: crate::engineering_tools::EngineeringParams,
    /// Revision returned on the previous page. Restart when it changes.
    pub expected_revision: Option<String>,
}
pub fn value(
    saved: &SavedReview,
    revision: &str,
    params: &ReviewContextParams,
) -> Result<Value, ErrorData> {
    crate::engineering_tools::validate(&params.page)?;
    if params
        .expected_revision
        .as_deref()
        .is_some_and(|r| r != revision)
    {
        return Err(crate::tools::invalid_params(
            "review changed; restart at offset 0",
        ));
    }
    let mut rows = vec![];
    for d in &saved.diagnostics {
        rows.push(json!({"kind":"diagnostic","message":d}));
    }
    if let Some(r) = &saved.report {
        if let Some(patterns) = r.architecture.get("patterns") {
            match serde_json::from_value(patterns.clone()) {
                Ok(report) => rows.extend(crate::pattern_tools::records(&report, "")),
                Err(error) => rows.push(json!({"kind":"diagnostic","message":format!("Recorded pattern report could not be read: {error}")})),
            }
        }
        for f in &r.findings {
            rows.push(json!({"kind":"finding","id":f.id,"fingerprint":f.fingerprint,"category":f.category,"severity":f.severity,"title":f.title,"change":f.change,"disposition":f.disposition,"source_status":f.source_status,"nodes_total":f.nodes.len(),"suggested_owners":f.suggested_owners,"decision":f.decision}));
            for node in &f.nodes {
                rows.push(json!({"kind":"finding_node","finding_id":f.id,"node":node}));
            }
            rows.push(json!({"kind":"finding_evidence","finding_id":f.id,"data":f.evidence}));
        }
        for g in &r.gaps {
            rows.push(json!({"kind":"evidence_gap","data":g}));
        }
        for e in &r.evidence {
            rows.push(json!({"kind":"evidence_status","data":e}));
        }
        for d in &r.unmatched_decisions {
            rows.push(json!({"kind":"unmatched_decision","data":d}));
        }
        for name in ["affected_files", "affected_components"] {
            for item in r.impact[name].as_array().into_iter().flatten() {
                rows.push(json!({"kind":name,"data":item}));
            }
        }
        for test in r.impact["tests"]["selected"]
            .as_array()
            .into_iter()
            .flatten()
        {
            rows.push(json!({"kind":"recommended_test","data":test}));
        }
        for question in r.architecture["questions"].as_array().into_iter().flatten() {
            rows.push(json!({"kind":"architecture_question","data":question}));
        }
        for d in r.architecture["intent"]["decisions"]
            .as_array()
            .into_iter()
            .flatten()
        {
            rows.push(json!({"kind":"architecture_decision","data":d}));
        }
        for (file, status) in r
            .changes
            .as_ref()
            .and_then(|c| c["files"].as_object())
            .into_iter()
            .flatten()
        {
            rows.push(json!({"kind":"changed_file","file":file,"status":status}));
        }
        for note in &r.limitations {
            rows.push(json!({"kind":"limitation","message":note}));
        }
    }
    let query = params.page.query.as_deref().unwrap_or("").to_lowercase();
    let filtered = rows
        .iter()
        .filter(|r| query.is_empty() || r.to_string().to_lowercase().contains(&query));
    let metadata = json!({"revision":revision,"review_revision":saved.report.as_ref().map(|r|&r.revision),"assessment_status":saved.report.as_ref().map(|r|&r.status),"summary":saved.report.as_ref().map(|r|&r.summary),"records_total":rows.len(),"safe_to_skip_other_tests":false,"read_only":true});
    crate::engineering_tools::page(&saved.status, metadata, filtered, &params.page, |r| {
        r.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::ReviewFinding;
    #[test]
    fn saved_review_pages_include_recorded_pattern_assessments() {
        let dir = tempfile::tempdir().unwrap();
        let (report, _) =
            archguard_engine::generate_review(dir.path(), None, None, 1_793_232_000).unwrap();
        let expected = report.architecture["patterns"]["assessments_total"]
            .as_u64()
            .unwrap();
        let saved = SavedReview {
            status: "stale".into(),
            report: Some(report),
            diagnostics: vec![],
        };
        let mut params = ReviewContextParams::default();
        let mut count = 0;
        loop {
            let page = value(&saved, "saved-patterns", &params).unwrap();
            assert_eq!(page["status"], "stale");
            count += page["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["kind"] == "pattern")
                .count();
            match page["next_offset"].as_u64() {
                Some(next) => params.page.offset = Some(next as usize),
                None => break,
            }
        }
        assert!(expected >= 6);
        assert_eq!(count as u64, expected);
    }
    #[test]
    fn pages_preserve_stale_status_global_counts_and_advance_past_oversized_evidence() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.py"), "a=1\n").unwrap();
        let (mut report, _) =
            archguard_engine::generate_review(dir.path(), None, None, 1_793_232_000).unwrap();
        report.findings = (0..30)
            .map(|i| ReviewFinding {
                id: format!("id-{i}"),
                fingerprint: "a".repeat(64),
                category: "architecture".into(),
                severity: "error".into(),
                title: format!("Finding {i}"),
                nodes: vec!["a.py".into()],
                change: "new".into(),
                source_status: "analyzed".into(),
                evidence: json!({"raw":"x".repeat(5000)}),
                suggested_owners: vec![],
                disposition: "open".into(),
                decision: None,
            })
            .collect();
        report.summary = json!({"open":30,"findings":30,"evidence_gaps":2});
        let saved = SavedReview {
            status: "stale".into(),
            report: Some(report),
            diagnostics: vec!["Source changed".into()],
        };
        let mut params = ReviewContextParams::default();
        let mut offset = 0;
        let mut seen = std::collections::BTreeSet::new();
        let mut omissions = 0;
        loop {
            params.page.offset = Some(offset);
            let page = value(&saved, "revision-1", &params).unwrap();
            assert!(page.to_string().len() <= 4096);
            assert_eq!(page["status"], "stale");
            assert_eq!(page["metadata"]["summary"]["open"], 30);
            assert_eq!(page["metadata"]["revision"], "revision-1");
            for row in page["items"].as_array().unwrap() {
                if row["kind"] == "finding" {
                    seen.insert(row["id"].as_str().unwrap().to_owned());
                }
                if row["omitted"] == true {
                    omissions += 1;
                }
            }
            if let Some(next) = page["next_offset"].as_u64() {
                assert!(next as usize > offset);
                offset = next as usize;
            } else {
                break;
            }
        }
        assert_eq!(seen.len(), 30);
        assert_eq!(omissions, 30);
        params.page.query = Some("no matching records".into());
        params.page.offset = None;
        let filtered = value(&saved, "revision-1", &params).unwrap();
        assert_eq!(filtered["total"], 0);
        assert_eq!(filtered["metadata"]["summary"]["open"], 30);
        params.expected_revision = Some("revision-0".into());
        assert!(value(&saved, "revision-1", &params).is_err());
    }
}
