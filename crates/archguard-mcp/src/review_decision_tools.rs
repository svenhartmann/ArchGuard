//! Decision-register tool for the review dashboard (R32 skeleton, R34 tool).
//!
//! The `review_decisions` tool reads and records the register at
//! `.archguard/reviews.json` — the only MCP tool that writes, and it
//! writes only the register, never through any other path. All
//! persistence goes through the engine register API (R19:
//! [`archguard_engine::load_register`] / [`archguard_engine::upsert_decision`],
//! atomic tmp-file-plus-rename), mirroring the CLI semantics of R29:
//! id defaults to the finding, author defaults to `"mcp"` (CLI: `"cli"`),
//! ts is ISO-8601 UTC, and a `accept-risk` without `adr` hits the
//! `[review] adr_required_on_accept_risk` gate (R30/R31) as -32602.

use archguard_engine::core::{DecisionKind, ReviewDecision, ReviewRegister};
use archguard_engine::{REGISTER_REL_PATH, load_register, upsert_decision};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;

use crate::server::ArchguardTools;

/// Input of the `review_decisions` tool (R34:
/// `{ path?, action?, finding?, decision?, adr?, note?, author?, id?,
/// offset?, limit? }`).
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ReviewDecisionParams {
    /// Project directory inside the MCP server root.
    pub path: Option<String>,
    /// `"list"` (default) or `"record"`.
    pub action: Option<String>,
    /// Record: finding the decision refers to, e.g. `scc-3`.
    pub finding: Option<String>,
    /// Record: `"fix"`, `"defer"` or `"accept-risk"`.
    pub decision: Option<String>,
    /// Record: ADR link; the `[review]` gate (R30) may require one.
    pub adr: Option<String>,
    /// Record: free-text note.
    pub note: Option<String>,
    /// Record: author; defaults to `"mcp"`.
    pub author: Option<String>,
    /// Record: decision id; defaults to the finding.
    pub id: Option<String>,
    /// List: first record of the sorted list; follow next_offset.
    pub offset: Option<usize>,
    /// List: 1..=200 processed records, further bounded by 4 KiB.
    pub limit: Option<usize>,
}

/// Reject unknown actions, unknown decision kinds, oversized text and
/// out-of-range pages before any file is read or written.
pub fn validate(params: &ReviewDecisionParams) -> Result<(), ErrorData> {
    if !(1..=200).contains(&params.limit.unwrap_or(crate::server::DEFAULT_LIMIT)) {
        return Err(crate::tools::invalid_params("limit must be 1..=200"));
    }
    match params.action.as_deref().unwrap_or("list") {
        "list" | "record" => {}
        other => {
            return Err(crate::tools::invalid_params(format!(
                "action must be \"list\" or \"record\", got `{other}`"
            )));
        }
    }
    if let Some(decision) = &params.decision
        && !matches!(decision.as_str(), "fix" | "defer" | "accept-risk")
    {
        return Err(crate::tools::invalid_params(format!(
            "decision must be \"fix\", \"defer\" or \"accept-risk\", got `{decision}`"
        )));
    }
    for (field, value) in [
        ("finding", &params.finding),
        ("adr", &params.adr),
        ("note", &params.note),
        ("author", &params.author),
        ("id", &params.id),
    ] {
        if value
            .as_ref()
            .is_some_and(|v| v.len() > 1024 || v.contains('\0'))
        {
            return Err(crate::tools::invalid_params(format!(
                "{field} must be at most 1024 bytes without NUL"
            )));
        }
    }
    Ok(())
}

/// Unix seconds to `YYYY-MM-DDTHH:MM:SSZ` (proleptic Gregorian, UTC) —
/// the civil-from-days conversion of the CLI (R29), no date crate needed.
fn iso_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem / 60) % 60,
        rem % 60
    )
}

fn register_error(err: archguard_engine::core::ArchguardError) -> ErrorData {
    ErrorData::internal_error(format!("review register: {err}"), None)
}

/// Deterministic list order: finding first, then id (same as the CLI R29).
fn sorted(register: &ReviewRegister) -> Vec<&ReviewDecision> {
    let mut decisions: Vec<&ReviewDecision> = register.decisions.iter().collect();
    decisions.sort_by(|a, b| {
        a.finding_ref
            .cmp(&b.finding_ref)
            .then_with(|| a.id.cmp(&b.id))
    });
    decisions
}

/// Build the paginated `list` answer for one register; a missing file
/// reads as an empty register (never an error), and an empty register
/// says "unreviewed", not "accepted".
pub fn list_value(
    register: &ReviewRegister,
    params: &ReviewDecisionParams,
) -> Result<Value, ErrorData> {
    validate(params)?;
    let rows = sorted(register);
    let total = rows.len();
    let offset = params.offset.unwrap_or(0).min(total);
    let mut reply = json!({
        "status": if total == 0 { "empty" } else { "ok" },
        "action": "list",
        "register": REGISTER_REL_PATH,
        "coverage_scope": "Records are exactly what the team recorded through this register (MCP or CLI `archguard review-decision`); they are team statements, not verified outcomes — a `fix` decision keeps the finding open until it is gone, only `defer`/`accept-risk` mark it decided. Order is finding_ref, then id.",
        "empty_note": (total == 0).then_some("No decisions recorded — a missing register means unreviewed, never accepted."),
        "total": total, "offset": offset, "returned": 0, "processed": 0,
        "next_offset": null, "truncated": false, "records": [],
    });
    let mut processed = 0;
    let mut returned = 0;
    let mut omitted = false;
    for decision in rows
        .into_iter()
        .skip(offset)
        .take(params.limit.unwrap_or(crate::server::DEFAULT_LIMIT))
    {
        let record = serde_json::to_value(decision).expect("decisions serialize");
        reply["records"]
            .as_array_mut()
            .unwrap()
            .push(record.clone());
        // Reserve cursor/count space even on the final, largest page.
        if reply.to_string().len() > crate::tools::MAX_RESPONSE_BYTES - 100 {
            reply["records"].as_array_mut().unwrap().pop();
            if returned > 0 {
                break;
            }
            reply["records"].as_array_mut().unwrap().push(json!({
                "omitted": true, "id": decision.id, "finding_ref": decision.finding_ref,
                "index": offset + processed,
                "reason": "Record exceeds response byte budget; use `archguard review-decision list --format json` for the complete document"
            }));
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

/// Build the `record` answer: apply the ADR gate, upsert through the
/// engine (R19) and return exactly what the register now stores plus the
/// register total. No field is invented beyond the documented defaults
/// (id → finding, author → "mcp", ts → now UTC).
pub fn record_value(root: &Path, params: &ReviewDecisionParams) -> Result<Value, ErrorData> {
    validate(params)?;
    let finding = crate::tools::required_str(&params.finding, "finding")?;
    let kind = match params.decision.as_deref() {
        Some("fix") => DecisionKind::Fix,
        Some("defer") => DecisionKind::Defer,
        Some("accept-risk") => DecisionKind::AcceptRisk,
        Some(other) => {
            return Err(crate::tools::invalid_params(format!(
                "decision must be \"fix\", \"defer\" or \"accept-risk\", got `{other}`"
            )));
        }
        None => {
            return Err(crate::tools::invalid_params(
                "record requires `decision` (\"fix\", \"defer\" or \"accept-risk\")",
            ));
        }
    };
    if kind == DecisionKind::AcceptRisk && params.adr.as_deref().is_none_or(str::is_empty) {
        // R30's gate, mirrored from the CLI (R29): with the gate on, an
        // accept-risk without an ADR is rejected (-32602), not recorded;
        // with the gate off it is recorded like any other decision.
        let loaded = crate::tools::load_config_at(root)?;
        if loaded.config.review.adr_required_on_accept_risk {
            return Err(crate::tools::invalid_params(
                "[review] adr_required_on_accept_risk is set — pass `adr` (e.g. docs/adr/NNN.md) to record an accept-risk decision",
            ));
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let decision = ReviewDecision {
        id: params
            .id
            .clone()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| finding.to_string()),
        finding_ref: finding.to_string(),
        decision: kind,
        adr_link: params.adr.clone().filter(|v| !v.is_empty()),
        note: params.note.clone().unwrap_or_default(),
        author: params
            .author
            .clone()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "mcp".to_string()),
        ts: iso_from_unix(now),
    };
    let id = decision.id.clone();
    let register = upsert_decision(root, decision).map_err(register_error)?;
    let stored = register
        .decisions
        .iter()
        .find(|d| d.id == id)
        .expect("upsert keeps the id");
    Ok(json!({
        "status": "recorded",
        "action": "record",
        "register": REGISTER_REL_PATH,
        "decision": serde_json::to_value(stored).expect("decisions serialize"),
        "total": register.decisions.len(),
        "coverage_scope": "The register stores exactly what is shown; re-recording the same id replaces the entry (idempotent upsert, atomic write). Decisions are team statements, not verified outcomes — a `fix` decision keeps the finding open until it disappears from the findings.",
    }))
}

#[tool_router(router = review_decision_router, vis = "pub(crate)")]
impl ArchguardTools {
    /// Read and record the review decision register (R34).
    #[tool(
        description = "Review decision register at .archguard/reviews.json: action=list (default) returns the recorded decisions (id, finding_ref, decision fix/defer/accept-risk, adr, note, author, ISO-8601 ts) sorted by finding then id, paginated within 4 KiB; a missing register is empty, not an error, and empty means unreviewed — never accepted. action=record upserts one decision (the only MCP tool that writes, and it writes only this register, atomically through the engine): finding and decision are required, id defaults to the finding, author defaults to 'mcp'. accept-risk without adr hits the [review] adr_required_on_accept_risk gate (R30): gate on → invalid params naming the gate, gate off → recorded. Decisions are team statements, not verified outcomes: a fix decision keeps the finding open until it is gone."
    )]
    async fn review_decisions(
        &self,
        Parameters(params): Parameters<ReviewDecisionParams>,
    ) -> Result<String, ErrorData> {
        validate(&params)?;
        let root = crate::tools::resolve_root(&self.root, params.path.as_deref())?;
        let value = match params.action.as_deref().unwrap_or("list") {
            "record" => record_value(&root, &params)?,
            _ => {
                let register = load_register(&root).map_err(register_error)?;
                list_value(&register, &params)?
            }
        };
        Ok(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ArchguardTools;
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::ErrorCode;

    fn decision(id: &str, finding: &str, kind: DecisionKind) -> ReviewDecision {
        ReviewDecision {
            id: id.to_string(),
            finding_ref: finding.to_string(),
            decision: kind,
            adr_link: None,
            note: String::new(),
            author: "sven".to_string(),
            ts: "2026-10-30T09:00:00Z".to_string(),
        }
    }

    fn register_of(decisions: Vec<ReviewDecision>) -> ReviewRegister {
        ReviewRegister {
            version: 1,
            decisions,
        }
    }

    fn list_params(offset: Option<usize>, limit: Option<usize>) -> ReviewDecisionParams {
        ReviewDecisionParams {
            offset,
            limit,
            ..Default::default()
        }
    }

    fn record_params(finding: &str, decision: &str) -> ReviewDecisionParams {
        ReviewDecisionParams {
            action: Some("record".into()),
            finding: Some(finding.into()),
            decision: Some(decision.into()),
            ..Default::default()
        }
    }

    #[test]
    fn review_decisions_tool_is_registered() {
        let tools = ArchguardTools::full_router().list_all();
        let tool = tools
            .iter()
            .find(|tool| tool.name == "review_decisions")
            .expect("the decision register must be discoverable over MCP");
        let schema = serde_json::to_value(&tool.input_schema).unwrap();
        for field in [
            "path", "action", "finding", "decision", "adr", "note", "author", "id", "offset",
            "limit",
        ] {
            assert!(schema["properties"].get(field).is_some(), "missing {field}");
        }
        let description = tool.description.as_deref().unwrap();
        assert!(
            description.contains("only MCP tool that writes"),
            "the write semantics must be visible: {description}"
        );
        assert!(description.contains("adr_required_on_accept_risk"));
        assert!(description.contains("empty means unreviewed"));
    }

    #[tokio::test]
    async fn record_then_list_roundtrip_upserts_by_id_and_defaults_are_honest() {
        let root = tempfile::tempdir().unwrap();
        let handler = ArchguardTools::new(root.path().into());
        // A missing register lists as empty, not as an error.
        let empty: Value = serde_json::from_str(
            &handler
                .review_decisions(Parameters(ReviewDecisionParams::default()))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(empty["status"], "empty");
        assert_eq!(empty["total"], 0);
        assert!(empty["records"].as_array().unwrap().is_empty());
        assert_eq!(empty["next_offset"], Value::Null);
        assert!(
            empty["empty_note"]
                .as_str()
                .unwrap()
                .contains("unreviewed, never accepted")
        );
        // Record with defaults: id → finding, author → "mcp", ts ISO UTC.
        let recorded: Value = serde_json::from_str(
            &handler
                .review_decisions(Parameters(record_params("scc-1", "fix")))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(recorded["status"], "recorded");
        assert_eq!(recorded["total"], 1);
        assert_eq!(recorded["register"], ".archguard/reviews.json");
        let stored = &recorded["decision"];
        assert_eq!(stored["id"], "scc-1");
        assert_eq!(stored["finding_ref"], "scc-1");
        assert_eq!(stored["decision"], "fix");
        assert_eq!(stored["author"], "mcp");
        assert!(stored["adr_link"].is_null());
        let ts = stored["ts"].as_str().unwrap();
        assert_eq!(ts.len(), 20, "ISO-8601 UTC: {ts}");
        assert_eq!(&ts[10..11], "T");
        assert!(ts.ends_with('Z'));
        assert!(root.path().join(".archguard/reviews.json").is_file());
        // A second decision with explicit id/author/note, then a list in
        // finding-then-id order.
        handler
            .review_decisions(Parameters(ReviewDecisionParams {
                action: Some("record".into()),
                finding: Some("app->core".into()),
                decision: Some("defer".into()),
                id: Some("d-2".into()),
                author: Some("sven".into()),
                note: Some("later".into()),
                ..Default::default()
            }))
            .await
            .unwrap();
        let list: Value = serde_json::from_str(
            &handler
                .review_decisions(Parameters(ReviewDecisionParams::default()))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(list["status"], "ok");
        assert_eq!(list["total"], 2);
        assert_eq!(list["records"][0]["id"], "d-2"); // sorted finding_ref: "app->core" < "scc-1"
        assert_eq!(list["records"][0]["finding_ref"], "app->core");
        assert_eq!(list["records"][0]["decision"], "defer");
        assert_eq!(list["records"][0]["author"], "sven");
        assert_eq!(list["records"][0]["note"], "later");
        assert_eq!(list["records"][1]["id"], "scc-1");
        assert_eq!(list["records"][1]["decision"], "fix");
        // Re-recording the same id replaces instead of duplicating.
        let again: Value = serde_json::from_str(
            &handler
                .review_decisions(Parameters(ReviewDecisionParams {
                    action: Some("record".into()),
                    finding: Some("scc-1".into()),
                    decision: Some("accept-risk".into()),
                    adr: Some("docs/adr/001.md".into()),
                    ..Default::default()
                }))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(again["total"], 2, "upsert by id never duplicates");
        assert_eq!(again["decision"]["decision"], "accept-risk");
        assert_eq!(again["decision"]["adr_link"], "docs/adr/001.md");
        // A path escaping the server root is rejected before any access.
        let escape = handler
            .review_decisions(Parameters(ReviewDecisionParams {
                path: Some("../outside".into()),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(escape.code, ErrorCode::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn accept_risk_without_adr_follows_the_review_gate() {
        let root = tempfile::tempdir().unwrap();
        let handler = ArchguardTools::new(root.path().into());
        // Gate off (the default): the accept-risk is recorded anyway.
        let recorded: Value = serde_json::from_str(
            &handler
                .review_decisions(Parameters(record_params("scc-9", "accept-risk")))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(recorded["status"], "recorded");
        assert_eq!(recorded["decision"]["decision"], "accept-risk");
        assert!(recorded["decision"]["adr_link"].is_null());
        // Gate on via .archguard/archguard.toml: -32602 naming the gate.
        std::fs::create_dir_all(root.path().join(".archguard")).unwrap();
        std::fs::write(
            root.path().join(".archguard/archguard.toml"),
            "[review]\nadr_required_on_accept_risk = true\n",
        )
        .unwrap();
        let err = handler
            .review_decisions(Parameters(record_params("scc-10", "accept-risk")))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(
            err.message
                .to_string()
                .contains("adr_required_on_accept_risk"),
            "{err:?}"
        );
        let list: Value = serde_json::from_str(
            &handler
                .review_decisions(Parameters(ReviewDecisionParams::default()))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            list["total"], 1,
            "the gated record must not have touched the register"
        );
        // With the gate on, an ADR satisfies it and other kinds are free.
        let with_adr: Value = serde_json::from_str(
            &handler
                .review_decisions(Parameters(ReviewDecisionParams {
                    adr: Some("docs/adr/002.md".into()),
                    ..record_params("scc-10", "accept-risk")
                }))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(with_adr["status"], "recorded");
        let defer: Value = serde_json::from_str(
            &handler
                .review_decisions(Parameters(record_params("scc-11", "defer")))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(defer["status"], "recorded");
        assert_eq!(list["total"], 1);
    }

    #[test]
    fn list_paginates_stably_and_stays_within_the_byte_budget() {
        let register = register_of(
            (0..30)
                .map(|i| {
                    decision(
                        &format!("d-{i:02}"),
                        &format!("scc-{i:02}"),
                        DecisionKind::Defer,
                    )
                })
                .collect(),
        );
        let first = list_value(&register, &list_params(None, None)).unwrap();
        assert_eq!(first["total"], 30);
        assert_eq!(first["truncated"], true);
        assert_eq!(first["next_offset"], 20); // DEFAULT_LIMIT
        let mut offset = 0;
        let mut ids = std::collections::BTreeSet::new();
        let mut order = Vec::new();
        loop {
            let page = list_value(&register, &list_params(Some(offset), Some(1))).unwrap();
            assert!(page.to_string().len() <= crate::tools::MAX_RESPONSE_BYTES);
            assert_eq!(page["returned"], 1);
            assert_eq!(page["total"], 30);
            assert_eq!(page["truncated"], true);
            let row = &page["records"][0];
            assert!(ids.insert(row["id"].as_str().unwrap().to_string()));
            order.push(row["finding_ref"].as_str().unwrap().to_string());
            if let Some(next) = page["next_offset"].as_u64() {
                assert!(next as usize > offset);
                offset = next as usize;
            } else {
                break;
            }
        }
        assert_eq!(ids.len(), 30);
        assert!(
            order.windows(2).all(|w| w[0] <= w[1]),
            "sorted by finding_ref: {order:?}"
        );
        let last = list_value(&register, &list_params(Some(29), Some(1))).unwrap();
        assert_eq!(last["next_offset"], Value::Null);
        assert_eq!(last["returned"], 1);
        // An oversized record is omitted explicitly and the cursor keeps
        // advancing toward it: the page ending before it stops with the
        // small record and points at its index.
        let mut big = decision("d-big", "scc-big", DecisionKind::Defer);
        big.note = "n".repeat(10_000);
        let mut decisions = register.decisions.clone();
        decisions.push(big);
        let before = list_value(
            &register_of(decisions.clone()),
            &list_params(Some(29), Some(2)),
        )
        .unwrap();
        assert_eq!(before["total"], 31);
        assert_eq!(
            before["returned"], 1,
            "the oversized sibling never crowds out the small one"
        );
        assert_eq!(before["processed"], 1);
        assert_eq!(before["records"][0]["id"], "d-29");
        assert_eq!(before["next_offset"], 30);
        assert_eq!(before["truncated"], true);
        // On its own page the oversized record is omitted explicitly, and
        // as the final record the cursor is exhausted (Null, honestly).
        let page = list_value(&register_of(decisions), &list_params(Some(30), Some(1))).unwrap();
        assert_eq!(page["total"], 31);
        assert!(page.to_string().len() <= crate::tools::MAX_RESPONSE_BYTES);
        assert_eq!(page["returned"], 0);
        assert_eq!(page["processed"], 1);
        assert_eq!(
            page["next_offset"],
            Value::Null,
            "oversized was the last record"
        );
        assert_eq!(page["truncated"], true);
        assert_eq!(page["records"][0]["omitted"], true);
        assert_eq!(page["records"][0]["id"], "d-big");
    }

    #[test]
    fn invalid_params_are_rejected_before_any_io() {
        for bad in [
            ReviewDecisionParams {
                limit: Some(0),
                ..Default::default()
            },
            ReviewDecisionParams {
                limit: Some(201),
                ..Default::default()
            },
            ReviewDecisionParams {
                action: Some("delete".into()),
                ..Default::default()
            },
            ReviewDecisionParams {
                decision: Some("wontfix".into()),
                ..Default::default()
            },
            ReviewDecisionParams {
                note: Some("a\0b".into()),
                ..Default::default()
            },
            ReviewDecisionParams {
                adr: Some("x".repeat(1025)),
                ..Default::default()
            },
        ] {
            assert!(validate(&bad).is_err(), "must reject: {bad:?}");
        }
    }

    #[test]
    fn iso_civil_from_unix_is_exact() {
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(951_868_800), "2000-03-01T00:00:00Z");
        assert_eq!(iso_from_unix(1_767_225_600), "2026-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(42_611), "1970-01-01T11:50:11Z");
    }
}
