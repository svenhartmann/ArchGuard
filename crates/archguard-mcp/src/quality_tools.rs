//! Read imported SARIF evidence without invoking its producer or altering sources.
use archguard_engine::QualityReport;
use rmcp::{ErrorData, schemars};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct QualityFindingsParams {
    /// Project directory inside the MCP server root.
    pub path: Option<String>,
    /// Case-insensitive substring of tool, rule, message or mapped source path.
    pub query: Option<String>,
    /// Exact SARIF level: error, warning, note or none. Omit for all levels.
    pub level: Option<String>,
    /// First matching record, following next_offset.
    pub offset: Option<usize>,
    /// 1..=200 records, further bounded by the 4 KiB response budget.
    pub limit: Option<usize>,
}
pub fn validate(params: &QualityFindingsParams) -> Result<(), ErrorData> {
    if !(1..=200).contains(&params.limit.unwrap_or(crate::server::DEFAULT_LIMIT)) {
        return Err(crate::tools::invalid_params("limit must be 1..=200"));
    }
    if params
        .level
        .as_deref()
        .is_some_and(|level| !matches!(level, "error" | "warning" | "note" | "none"))
    {
        return Err(crate::tools::invalid_params(
            "level must be error, warning, note or none",
        ));
    }
    if params
        .query
        .as_ref()
        .is_some_and(|query| query.len() > 512 || query.contains('\0'))
    {
        return Err(crate::tools::invalid_params(
            "query must be at most 512 bytes without NUL",
        ));
    }
    Ok(())
}
pub fn quality_value(
    report: &QualityReport,
    params: &QualityFindingsParams,
) -> Result<Value, ErrorData> {
    validate(params)?;
    let query = params.query.as_deref().unwrap_or("").trim().to_lowercase();
    let rows: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| {
            params
                .level
                .as_deref()
                .is_none_or(|level| finding.level == level)
                && (query.is_empty()
                    || [
                        &finding.tool,
                        &finding.rule_id,
                        &finding.message,
                        finding.path.as_deref().unwrap_or(""),
                        finding.node.as_deref().unwrap_or(""),
                    ]
                    .iter()
                    .any(|text| text.to_lowercase().contains(&query)))
        })
        .collect();
    let mut levels = BTreeMap::<&str, usize>::new();
    let mut active_levels = BTreeMap::<&str, usize>::new();
    let mut active_findings = 0;
    for finding in &report.findings {
        *levels.entry(&finding.level).or_default() += 1;
        if !finding.suppressed && finding.baseline_state.as_deref() != Some("absent") {
            active_findings += 1;
            *active_levels.entry(&finding.level).or_default() += 1;
        }
    }
    let total = rows.len();
    let offset = params.offset.unwrap_or(0).min(total);
    let mut value = json!({
        "status":report.status,"report_path":report.report_path,"imported_at_unix":report.imported_at_unix,
        "tools":report.tools.iter().take(3).collect::<Vec<_>>(),"tools_total":report.tools.len(),"findings_total":report.findings.len(),"levels":levels,
        "active_findings":active_findings,"active_levels":active_levels,"historical_total":report.findings.iter().filter(|finding|finding.baseline_state.as_deref()==Some("absent")).count(),
        "suppressed_total":report.findings.iter().filter(|finding|finding.suppressed).count(),
        "coverage_scope":"Whole imported report. Filters preserve overall status. Active counts exclude accepted suppression and baselineState absent; all records remain visible and labeled. No findings is not proof of clean code.",
        "source_revision_verified":false,"freshness_note":"Source hashes detect later edits only. Report/source revision at import is unverified; historical findings do not prove that a problem still exists or has been fixed.",
        "diagnostics":report.diagnostics.iter().take(3).collect::<Vec<_>>(),"diagnostics_total":report.diagnostics.len(),"metadata_truncated":report.diagnostics.len()>3||report.tools.len()>3,
        "total":total,"offset":offset,"returned":0,"processed":0,"next_offset":null,"truncated":false,"findings":[],
        "refresh":"archguard quality <project> --report <results.sarif>"
    });
    while value.to_string().len() > crate::tools::MAX_RESPONSE_BYTES / 2 {
        value["metadata_truncated"] = true.into();
        if !value["diagnostics"].as_array().unwrap().is_empty() {
            value["diagnostics"].as_array_mut().unwrap().pop();
        } else if !value["tools"].as_array().unwrap().is_empty() {
            value["tools"].as_array_mut().unwrap().pop();
        } else {
            value["report_path"] = Value::Null;
            break;
        }
    }
    let mut processed = 0;
    let mut returned = 0;
    let mut omitted = false;
    for finding in rows
        .into_iter()
        .skip(offset)
        .take(params.limit.unwrap_or(crate::server::DEFAULT_LIMIT))
    {
        value["findings"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(finding).expect("quality finding serializes"));
        if value.to_string().len() > crate::tools::MAX_RESPONSE_BYTES - 100 {
            value["findings"].as_array_mut().unwrap().pop();
            if returned > 0 {
                break;
            }
            value["findings"].as_array_mut().unwrap().push(json!({"omitted":true,"index":offset+processed,"reason":"Finding exceeds response byte budget; use CLI JSON for the complete record"}));
            omitted = true;
        } else {
            returned += 1;
        }
        processed += 1;
    }
    value["returned"] = returned.into();
    value["processed"] = processed.into();
    value["next_offset"] = if offset + processed < total {
        (offset + processed).into()
    } else {
        Value::Null
    };
    value["truncated"] = (offset > 0 || offset + processed < total || omitted).into();
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report() -> QualityReport {
        serde_json::from_value(json!({"schema_version":1,"status":"partial","report_path":"lint.sarif","imported_at_unix":4,"tools":["ExampleLint"],"diagnostics":["Producer run incomplete"],
            "findings":(0..40).map(|i|json!({"id":format!("finding-{i}"),"tool":"ExampleLint","rule_id":"rule/1","level":if i%2==0 {"warning"}else{"error"},"message":format!("Message {i}"),"path":format!("src/a{i}.ts"),"node":format!("a{i}.ts"),"line":1,"column":2,"status":if i==0 {"changed_since_import"}else{"unchanged_since_import"},"source_sha256":"a".repeat(64),"suppressed":i==1,"baseline_state":if i==2 {Some("absent")}else{None},"help_uri":"https://example.test/rule"})).collect::<Vec<_>>()
        })).unwrap()
    }
    #[test]
    fn pages_keep_exact_evidence_status_and_whole_report_totals() {
        let report = report();
        let mut offset = 0;
        let mut ids = std::collections::BTreeSet::new();
        loop {
            let value = quality_value(
                &report,
                &QualityFindingsParams {
                    offset: Some(offset),
                    limit: Some(200),
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(value.to_string().len() <= 4096);
            assert_eq!(value["status"], "partial");
            assert_eq!(value["findings_total"], 40);
            assert_eq!(value["levels"]["error"], 20);
            assert_eq!(value["levels"]["warning"], 20);
            assert_eq!(value["suppressed_total"], 1);
            assert_eq!(value["historical_total"], 1);
            assert_eq!(value["active_findings"], 38);
            assert_eq!(value["active_levels"]["error"], 19);
            assert_eq!(value["active_levels"]["warning"], 19);
            assert_eq!(value["source_revision_verified"], false);
            for row in value["findings"].as_array().unwrap() {
                assert!(ids.insert(row["id"].as_str().unwrap().to_string()));
            }
            if let Some(next) = value["next_offset"].as_u64() {
                assert!(next as usize > offset);
                offset = next as usize;
            } else {
                break;
            }
        }
        assert_eq!(ids.len(), 40);
        let filtered = quality_value(
            &report,
            &QualityFindingsParams {
                query: Some("A1.TS".into()),
                level: Some("error".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered["total"], 1);
        assert_eq!(filtered["findings"][0]["id"], "finding-1");
        assert_eq!(filtered["findings"][0]["suppressed"], true);
        let stale = quality_value(
            &report,
            &QualityFindingsParams {
                query: Some("a0.ts".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stale["findings"][0]["status"], "changed_since_import");
    }
    #[test]
    fn oversized_records_advance_cursor_and_metadata_is_bounded() {
        let mut report = report();
        report.report_path = Some("r".repeat(10000));
        report.tools = vec!["t".repeat(10000)];
        report.diagnostics = vec!["d".repeat(10000)];
        report.findings[0].message = "x".repeat(10000);
        let value = quality_value(
            &report,
            &QualityFindingsParams {
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
        assert_eq!(value["findings"][0]["omitted"], true);
        for params in [
            QualityFindingsParams {
                limit: Some(0),
                ..Default::default()
            },
            QualityFindingsParams {
                limit: Some(201),
                ..Default::default()
            },
            QualityFindingsParams {
                level: Some("fatal".into()),
                ..Default::default()
            },
            QualityFindingsParams {
                query: Some("\0".into()),
                ..Default::default()
            },
        ] {
            assert!(validate(&params).is_err());
        }
    }
}
