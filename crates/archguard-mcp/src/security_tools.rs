//! Read-only access to saved dependency security findings.
use archguard_engine::SecurityReport;
use rmcp::{ErrorData, schemars};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SecurityFindingsParams {
    /// Project directory inside the server root.
    pub path: Option<String>,
    /// Exact advisory severity: critical, high, medium, low or unknown.
    pub severity: Option<String>,
    /// Case-insensitive substring in the package name or advisory ID (e.g. CVE).
    pub query: Option<String>,
    /// First matching finding. Follow next_offset from the response.
    pub offset: Option<usize>,
    /// Requested findings, 1..=200; the 4 KiB response budget may reduce the page.
    pub limit: Option<usize>,
}

pub fn validate(params: &SecurityFindingsParams) -> Result<(), ErrorData> {
    if params.severity.as_deref().is_some_and(|severity| {
        !matches!(severity, "critical" | "high" | "medium" | "low" | "unknown")
    }) {
        return Err(crate::tools::invalid_params(
            "severity must be critical, high, medium, low or unknown",
        ));
    }
    if !(1..=200).contains(&params.limit.unwrap_or(crate::server::DEFAULT_LIMIT)) {
        return Err(crate::tools::invalid_params("limit must be 1..=200"));
    }
    if params
        .query
        .as_ref()
        .is_some_and(|query| query.len() > 512 || query.contains('\0'))
    {
        return Err(crate::tools::invalid_params(
            "query must be at most 512 bytes without NUL characters",
        ));
    }
    Ok(())
}

fn size(value: &Value) -> usize {
    serde_json::to_vec(value).expect("plain JSON").len()
}

// Omit oversized metadata instead of changing a version, fingerprint or path.
// Counts and metadata_truncated identify the incomplete metadata sample.
fn metadata(value: Option<&str>, omitted: &mut bool) -> Value {
    match value {
        Some(value) if value.len() <= 256 => value.into(),
        Some(_) => {
            *omitted = true;
            Value::Null
        }
        None => Value::Null,
    }
}

pub fn security_value(
    report: &SecurityReport,
    params: &SecurityFindingsParams,
) -> Result<Value, ErrorData> {
    validate(params)?;
    let query = params.query.as_deref().unwrap_or("").trim().to_lowercase();
    let rows: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| {
            params
                .severity
                .as_deref()
                .is_none_or(|severity| finding.severity == severity)
                && (query.is_empty()
                    || finding.id.to_lowercase().contains(&query)
                    || finding.package.to_lowercase().contains(&query))
        })
        .collect();
    let total = rows.len();
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(crate::server::DEFAULT_LIMIT);
    let mut counts = json!({"critical":0,"high":0,"medium":0,"low":0,"unknown":0});
    for finding in &report.findings {
        let severity = match finding.severity.as_str() {
            "critical" | "high" | "medium" | "low" => finding.severity.as_str(),
            _ => "unknown",
        };
        counts[severity] = (counts[severity].as_u64().unwrap() + 1).into();
    }
    let mut omitted = report.targets.len() > 8 || report.diagnostics.len() > 3;
    let targets: Vec<_> = report.targets.iter().take(8).collect();
    let diagnostics: Vec<_> = report.diagnostics.iter().take(3).collect();
    let mut reply = json!({
        "schema_version":report.schema_version,"status":report.status,
        "scanner":metadata(Some(&report.scanner),&mut omitted),
        "scanner_version":metadata(report.scanner_version.as_deref(),&mut omitted),
        "scanned_at":metadata(report.scanned_at.as_deref(),&mut omitted),
        "database_updated_at":metadata(report.database_updated_at.as_deref(),&mut omitted),
        "fingerprint":metadata(Some(&report.fingerprint),&mut omitted),
        "coverage_scope":"whole_report","packages_scanned":report.packages_scanned,
        "targets":targets,"targets_total":report.targets.len(),
        "diagnostics":diagnostics,"diagnostics_total":report.diagnostics.len(),
        "metadata_truncated":false,"counts":counts,"findings_total":report.findings.len(),
        "total":total,"offset":offset,"returned":0,"next_offset":null,"truncated":false,
        "findings":[],"refresh":"Run archguard security <path> to refresh the saved report."
    });
    // Keep half of the byte budget for findings; source identifiers and paths
    // are either returned exactly or explicitly omitted, never shortened.
    while size(&reply) > crate::tools::MAX_RESPONSE_BYTES / 2 {
        omitted = true;
        if !reply["diagnostics"].as_array().unwrap().is_empty() {
            reply["diagnostics"].as_array_mut().unwrap().pop();
        } else if !reply["targets"].as_array().unwrap().is_empty() {
            reply["targets"].as_array_mut().unwrap().pop();
        } else {
            // Unusually large scalar metadata must not crowd out the page.
            for key in [
                "scanner",
                "scanner_version",
                "scanned_at",
                "database_updated_at",
                "fingerprint",
            ] {
                reply[key] = Value::Null;
            }
            break;
        }
    }
    reply["metadata_truncated"] = omitted.into();
    let mut returned = 0;
    let mut omitted_finding = false;
    for (index, row) in rows.into_iter().enumerate().skip(offset).take(limit) {
        reply["findings"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(row).expect("finding serializes"));
        reply["returned"] = (returned + 1).into();
        reply["next_offset"] = if index + 1 < total {
            (index + 1).into()
        } else {
            Value::Null
        };
        if size(&reply) > crate::tools::MAX_RESPONSE_BYTES {
            reply["findings"].as_array_mut().unwrap().pop();
            if returned > 0 {
                break;
            }
            reply["findings"].as_array_mut().unwrap().push(json!({
                "index":index,"omitted":true,"reason":"finding_exceeds_response_limit",
                "hint":"Read .archguard/security.json in the analyzed project for complete source evidence."
            }));
            omitted_finding = true;
        }
        returned += 1;
    }
    let next = offset.saturating_add(returned);
    reply["returned"] = returned.into();
    reply["next_offset"] = if next < total {
        next.into()
    } else {
        Value::Null
    };
    reply["truncated"] = (next < total || omitted || omitted_finding).into();
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Value {
        json!({
            "schema_version":1,"status":"ready","scanner":"trivy",
            "scanner_version":"0.69.1","scanned_at":"2026-10-27T12:00:00Z",
            "database_updated_at":"2026-10-27T09:00:00Z","fingerprint":"manifest-v1",
            "packages_scanned":73,"targets":[{"path":"package-lock.json","ecosystem":"npm","packages":73}],
            "findings":[
                {"id":"CVE-2026-1001","package":"@example/core","installed_version":"1.2.3-beta+build.4","fixed_version":"1.2.4","ecosystem":"npm","severity":"high","title":"Core advisory","url":"https://example.test/CVE-2026-1001","targets":["package-lock.json"]},
                {"id":"GHSA-example-2","package":"Example.Json","installed_version":"9.0.0","fixed_version":null,"ecosystem":"nuget","severity":"critical","title":"JSON advisory","url":null,"targets":["src/obj/project.assets.json"]},
                {"id":"CVE-2026-1003","package":"@example/core","installed_version":"2.0.0","fixed_version":"2.0.1","ecosystem":"npm","severity":"unknown","title":"Unrated advisory","url":null,"targets":["web/package-lock.json"]}
            ],"diagnostics":[]
        })
    }

    fn reply(report: Value, params: SecurityFindingsParams) -> Value {
        security_value(&serde_json::from_value(report).unwrap(), &params).unwrap()
    }

    #[test]
    fn security_filters_preserve_source_evidence_and_whole_report_coverage() {
        let source = fixture();
        let filtered = reply(
            source.clone(),
            SecurityFindingsParams {
                severity: Some("high".into()),
                query: Some("EXAMPLE/CORE".into()),
                ..Default::default()
            },
        );
        assert_eq!(filtered["total"], 1);
        assert_eq!(filtered["findings"], json!([source["findings"][0]]));
        assert_eq!(
            filtered["counts"],
            json!({"critical":1,"high":1,"medium":0,"low":0,"unknown":1})
        );
        assert_eq!(filtered["findings_total"], 3);
        assert_eq!(filtered["packages_scanned"], 73);
        assert_eq!(filtered["targets"], source["targets"]);
        assert_eq!(filtered["coverage_scope"], "whole_report");
        for field in [
            "status",
            "scanner",
            "scanner_version",
            "scanned_at",
            "database_updated_at",
            "fingerprint",
        ] {
            assert_eq!(filtered[field], source[field], "{field}");
        }
        let by_id = reply(
            source.clone(),
            SecurityFindingsParams {
                query: Some("cve-2026-1003".into()),
                ..Default::default()
            },
        );
        assert_eq!(by_id["findings"], json!([source["findings"][2]]));
        let none = reply(
            source,
            SecurityFindingsParams {
                query: Some("not-present".into()),
                ..Default::default()
            },
        );
        assert_eq!(none["total"], 0);
        assert_eq!(
            none["status"], "ready",
            "filtering never rewrites report status"
        );
        assert_eq!(none["findings_total"], 3);
    }

    #[test]
    fn security_unknown_scan_states_are_never_reported_as_clean() {
        for status in ["not_scanned", "partial", "stale", "error"] {
            let mut source = fixture();
            source["status"] = status.into();
            source["findings"] = json!([]);
            source["diagnostics"] = json!(["Coverage is unavailable or incomplete."]);
            let value = reply(source, SecurityFindingsParams::default());
            assert_eq!(value["status"], status);
            assert_eq!(value["total"], 0);
            assert_eq!(
                value["diagnostics"],
                json!(["Coverage is unavailable or incomplete."])
            );
            assert!(value.get("clean").is_none());
        }
    }

    #[test]
    fn security_rejects_invalid_filters_and_page_limits() {
        for params in [
            SecurityFindingsParams {
                severity: Some("severe".into()),
                ..Default::default()
            },
            SecurityFindingsParams {
                limit: Some(0),
                ..Default::default()
            },
            SecurityFindingsParams {
                limit: Some(201),
                ..Default::default()
            },
            SecurityFindingsParams {
                query: Some("x".repeat(513)),
                ..Default::default()
            },
            SecurityFindingsParams {
                query: Some("bad\0query".into()),
                ..Default::default()
            },
        ] {
            let error =
                security_value(&serde_json::from_value(fixture()).unwrap(), &params).unwrap_err();
            assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        }
    }

    #[test]
    fn security_pages_exact_original_rows_without_gaps_or_overflow() {
        let mut source = fixture();
        let original = source["findings"][0].clone();
        source["findings"] = (0..45)
            .map(|index| {
                let mut row = original.clone();
                row["id"] = format!("CVE-2026-{index:05}").into();
                row
            })
            .collect();
        let mut offset = 0;
        let mut received = Vec::new();
        loop {
            let value = reply(
                source.clone(),
                SecurityFindingsParams {
                    offset: Some(offset),
                    limit: Some(200),
                    ..Default::default()
                },
            );
            assert!(value.to_string().len() <= crate::tools::MAX_RESPONSE_BYTES);
            assert_eq!(value["total"], 45);
            let rows = value["findings"].as_array().unwrap();
            assert!(!rows.is_empty());
            assert_eq!(value["returned"], rows.len());
            received.extend(rows.iter().cloned());
            match value["next_offset"].as_u64() {
                Some(next) => {
                    assert_eq!(next as usize, offset + rows.len());
                    assert_eq!(value["truncated"], true);
                    offset = next as usize;
                }
                None => {
                    assert_eq!(value["truncated"], false);
                    break;
                }
            }
        }
        assert_eq!(received, source["findings"].as_array().unwrap().clone());
        let beyond = reply(
            source,
            SecurityFindingsParams {
                offset: Some(usize::MAX),
                ..Default::default()
            },
        );
        assert_eq!(beyond["findings"], json!([]));
        assert!(beyond["next_offset"].is_null());
    }

    #[test]
    fn security_oversized_evidence_is_explicit_and_advances_the_cursor() {
        let mut source = fixture();
        source["findings"][0]["id"] = "CVE-".repeat(2000).into();
        source["targets"] = (0..30).map(|i| json!({"path":format!("{i}/{}", "deep/".repeat(100)),"ecosystem":"npm","packages":1})).collect();
        source["diagnostics"] = json!(["詳細".repeat(5000)]);
        source["scanner_version"] = "version".repeat(5000).into();
        let first = reply(
            source.clone(),
            SecurityFindingsParams {
                limit: Some(1),
                ..Default::default()
            },
        );
        assert!(first.to_string().len() <= crate::tools::MAX_RESPONSE_BYTES);
        assert_eq!(first["findings"][0]["omitted"], true);
        assert_eq!(first["findings"][0]["index"], 0);
        assert_eq!(first["next_offset"], 1);
        assert_eq!(first["truncated"], true);
        assert_eq!(first["metadata_truncated"], true);
        assert_eq!(first["targets_total"], 30);
        assert_eq!(first["diagnostics_total"], 1);
        assert_eq!(first["counts"]["high"], 1);
        let next = reply(
            source.clone(),
            SecurityFindingsParams {
                offset: Some(1),
                limit: Some(1),
                ..Default::default()
            },
        );
        assert_eq!(next["findings"], json!([source["findings"][1]]));
    }

    #[tokio::test]
    async fn security_handler_reads_saved_snapshot_without_creating_analysis_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let handler = crate::server::ArchguardTools::new(root.path().to_path_buf());
        let call =
            |params| handler.security_findings(rmcp::handler::server::wrapper::Parameters(params));
        let missing: Value =
            serde_json::from_str(&call(SecurityFindingsParams::default()).await.unwrap()).unwrap();
        assert_eq!(missing["status"], "not_scanned");
        assert_eq!(missing["total"], 0);
        assert!(
            !root.path().join(".archguard").exists(),
            "reading never creates a project artifact directory"
        );
        let loaded = archguard_engine::load_config(None, root.path()).unwrap();
        let mut source = fixture();
        source["fingerprint"] = archguard_engine::security_fingerprint(root.path(), &loaded.config)
            .unwrap()
            .into();
        std::fs::create_dir(root.path().join(".archguard")).unwrap();
        let file = root.path().join(".archguard/security.json");
        let saved = serde_json::to_vec(&source).unwrap();
        std::fs::write(&file, &saved).unwrap();
        let value: Value = serde_json::from_str(
            &call(SecurityFindingsParams {
                severity: Some("critical".into()),
                ..Default::default()
            })
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(value["status"], "ready");
        assert_eq!(value["findings"], json!([source["findings"][1]]));
        assert_eq!(
            std::fs::read(&file).unwrap(),
            saved,
            "the stored report stays byte-for-byte unchanged"
        );
        assert_eq!(
            std::fs::read_dir(root.path().join(".archguard"))
                .unwrap()
                .count(),
            1,
            "no graph cache, scan or other artifact is written"
        );
    }

    #[tokio::test]
    async fn security_handler_rejects_paths_outside_the_server_root() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("project");
        let outside = base.path().join("outside");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let handler = crate::server::ArchguardTools::new(root);
        for path in [
            "../outside".to_owned(),
            outside.to_string_lossy().into_owned(),
        ] {
            let error = handler
                .security_findings(rmcp::handler::server::wrapper::Parameters(
                    SecurityFindingsParams {
                        path: Some(path),
                        ..Default::default()
                    },
                ))
                .await
                .unwrap_err();
            assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        }
        let nested = handler
            .security_findings(rmcp::handler::server::wrapper::Parameters(
                SecurityFindingsParams {
                    path: Some("sub".into()),
                    ..Default::default()
                },
            ))
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&nested).unwrap()["status"],
            "not_scanned"
        );
        let error = handler
            .security_findings(rmcp::handler::server::wrapper::Parameters(
                SecurityFindingsParams {
                    severity: Some("invalid".into()),
                    ..Default::default()
                },
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn security_handler_rejects_symlink_root_escapes() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("external")).unwrap();
        let error = crate::server::ArchguardTools::new(root.path().to_path_buf())
            .security_findings(rmcp::handler::server::wrapper::Parameters(
                SecurityFindingsParams {
                    path: Some("external".into()),
                    ..Default::default()
                },
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn security_tool_is_registered_with_read_only_refresh_guidance() {
        let tools = crate::server::ArchguardTools::full_router().list_all();
        let tool = tools
            .iter()
            .find(|tool| tool.name == "security_findings")
            .unwrap();
        let description = tool.description.as_deref().unwrap();
        assert!(description.contains("archguard security <path>"));
        assert!(description.contains("never runs a scanner or network request"));
        for parameter in ["path", "severity", "query", "offset", "limit"] {
            assert!(tool.input_schema["properties"].get(parameter).is_some());
        }
    }
}
