//! Bounded, read-only test-coverage evidence for agents.
use archguard_engine::{CoverageReview, TestCoverageReport};
use rmcp::{ErrorData, schemars};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TestCoverageParams {
    /// Project directory inside the MCP server root.
    pub path: Option<String>,
    /// Case-insensitive substring of the graph-node or physical source path.
    pub query: Option<String>,
    /// First ranked file, following next_offset.
    pub offset: Option<usize>,
    /// 1..=200 files; the 4 KiB byte budget may shorten the page.
    pub limit: Option<usize>,
}
pub fn validate(params: &TestCoverageParams) -> Result<(), ErrorData> {
    if !(1..=200).contains(&params.limit.unwrap_or(crate::server::DEFAULT_LIMIT)) {
        return Err(crate::tools::invalid_params("limit must be 1..=200"));
    }
    if params
        .query
        .as_ref()
        .is_some_and(|q| q.len() > 512 || q.contains('\0'))
    {
        return Err(crate::tools::invalid_params(
            "query must be at most 512 bytes without NUL",
        ));
    }
    Ok(())
}
pub fn coverage_value(
    report: &TestCoverageReport,
    review: &CoverageReview,
    params: &TestCoverageParams,
) -> Result<Value, ErrorData> {
    validate(params)?;
    let q = params.query.as_deref().unwrap_or("").trim().to_lowercase();
    let files: std::collections::BTreeMap<_, _> = report
        .files
        .iter()
        .map(|f| ((f.node.as_str(), f.path.as_str()), f))
        .collect();
    let rows: Vec<_> = review
        .files
        .iter()
        .filter(|r| r.node.to_lowercase().contains(&q) || r.path.to_lowercase().contains(&q))
        .collect();
    let total = rows.len();
    let offset = params.offset.unwrap_or(0).min(total);
    let mut reply = json!({
        "status":report.status,"format":report.format,"imported_at_unix":report.imported_at_unix,
        "report_modified_unix":report.report_modified_unix,"report_path":report.report_path,
        "source_revision_verified":false,"coverage_scope":"Only mapped files in the imported report; absent files and missing counters are unknown, never zero. Hashes detect later edits to mapped source files, not test/build configuration changes.",
        "score_definition":review.definition,"score_note":"Uncovered line fraction × (1 + observed transitive file dependents). Heuristic, not defect probability; stale or unknown values have no score.",
        "semantic_status":review.semantic_status,"mapped_files":report.files.len(),"unmatched_files":report.unmatched_files,
        "diagnostics":report.diagnostics.iter().take(3).collect::<Vec<_>>(),"diagnostics_total":report.diagnostics.len(),"metadata_truncated":report.diagnostics.len()>3,
        "total":total,"offset":offset,"returned":0,"processed":0,"next_offset":null,"truncated":false,"files":[],
        "refresh":"archguard coverage <project> --report <jacoco.xml|cobertura.xml|clover.xml|lcov.info> [--source-root src/main/java]"
    });
    while reply.to_string().len() > crate::tools::MAX_RESPONSE_BYTES / 2 {
        reply["metadata_truncated"] = true.into();
        if !reply["diagnostics"].as_array().unwrap().is_empty() {
            reply["diagnostics"].as_array_mut().unwrap().pop();
        } else {
            reply["report_path"] = Value::Null;
            break;
        }
    }
    let mut processed = 0;
    let mut returned = 0;
    let mut omitted = false;
    for rank in rows
        .into_iter()
        .skip(offset)
        .take(params.limit.unwrap_or(crate::server::DEFAULT_LIMIT))
    {
        let Some(file) = files.get(&(rank.node.as_str(), rank.path.as_str())) else {
            continue;
        };
        let row = json!({"node":file.node,"path":file.path,"lines":file.lines,"branches":file.branches,"status":file.status,"dependents":rank.dependents,"score":rank.score});
        reply["files"].as_array_mut().unwrap().push(row);
        // Reserve cursor/count space even on the final, largest numeric page.
        if reply.to_string().len() > crate::tools::MAX_RESPONSE_BYTES - 100 {
            reply["files"].as_array_mut().unwrap().pop();
            if returned > 0 {
                break;
            }
            reply["files"].as_array_mut().unwrap().push(json!({"omitted":true,"reason":"File record exceeds response byte budget","index":offset+processed}));
            omitted = true;
        } else {
            returned += 1;
        }
        processed += 1;
    }
    reply["returned"] = returned.into();
    reply["processed"] = processed.into();
    reply["next_offset"] = if offset + processed < total {
        (offset + processed).into()
    } else {
        Value::Null
    };
    reply["truncated"] = (offset > 0 || offset + processed < total || omitted).into();
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::{CoverageCounter, CoverageReviewFile, TestCoverageFile};
    fn reports() -> (TestCoverageReport, CoverageReview) {
        let report = TestCoverageReport {
            schema_version: 1,
            status: "partial".into(),
            format: Some("jacoco".into()),
            report_path: Some("coverage.xml".into()),
            imported_at_unix: Some(12),
            report_modified_unix: Some(10),
            files: (0..45)
                .map(|i| TestCoverageFile {
                    node: format!("lib/file{i}.java"),
                    path: format!("src/lib/file{i}.java"),
                    lines: Some(CoverageCounter {
                        covered: 1,
                        total: 5,
                    }),
                    branches: None,
                    line_hits: None,
                    source_sha256: "hash".into(),
                    status: if i == 1 {
                        "changed_since_import"
                    } else {
                        "unchanged_since_import"
                    }
                    .into(),
                })
                .collect(),
            unmatched_files: 3,
            diagnostics: vec!["Producer revision unverified".into()],
        };
        let review = CoverageReview {
            definition: "uncovered-line-fraction-times-impact-v1".into(),
            semantic_status: Some("partial".into()),
            files: report
                .files
                .iter()
                .enumerate()
                .map(|(i, f)| CoverageReviewFile {
                    node: f.node.clone(),
                    path: f.path.clone(),
                    dependents: Some(2),
                    score: if i == 1 { None } else { Some(2.4) },
                })
                .collect(),
        };
        (report, review)
    }
    #[test]
    fn bounded_pages_keep_unknown_stale_and_whole_report_context() {
        let (report, review) = reports();
        let mut offset = 0;
        let mut seen = 0;
        loop {
            let value = coverage_value(
                &report,
                &review,
                &TestCoverageParams {
                    offset: Some(offset),
                    limit: Some(200),
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(value.to_string().len() <= crate::tools::MAX_RESPONSE_BYTES);
            assert_eq!(value["status"], "partial");
            assert_eq!(value["mapped_files"], 45);
            assert_eq!(value["unmatched_files"], 3);
            for row in value["files"].as_array().unwrap() {
                assert!(row["branches"].is_null());
                if row["status"] == "changed_since_import" {
                    assert!(row["score"].is_null());
                }
            }
            seen += value["returned"].as_u64().unwrap();
            if let Some(next) = value["next_offset"].as_u64() {
                assert!(next as usize > offset);
                offset = next as usize;
            } else {
                break;
            }
        }
        assert_eq!(seen, 45);
        let filtered = coverage_value(
            &report,
            &review,
            &TestCoverageParams {
                query: Some("FILE1.JAVA".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered["total"], 1);
        assert!(filtered["files"][0]["score"].is_null());
    }
    #[test]
    fn oversized_records_advance_and_invalid_limits_fail() {
        let (mut report, mut review) = reports();
        report.report_path = Some("r".repeat(20000));
        report.diagnostics = vec!["d".repeat(10000)];
        report.files[0].node = "x".repeat(10000);
        review.files[0].node = report.files[0].node.clone();
        let value = coverage_value(
            &report,
            &review,
            &TestCoverageParams {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(value.to_string().len() <= 4096);
        assert_eq!(value["metadata_truncated"], true);
        assert_eq!(value["returned"], 0);
        assert_eq!(value["processed"], 1);
        assert_eq!(value["next_offset"], 1);
        assert_eq!(value["files"][0]["omitted"], true);
        assert!(
            validate(&TestCoverageParams {
                limit: Some(0),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            validate(&TestCoverageParams {
                query: Some("a\0b".into()),
                ..Default::default()
            })
            .is_err()
        );
    }
}
