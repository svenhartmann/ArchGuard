//! Review KPI tool for the review dashboard (R32 skeleton, R33 tool).
//!
//! The `review_kpis` tool exposes the [`archguard_engine::core::ReviewKpis`]
//! payload the pipeline computes (R23) as flat, stably identified records,
//! paginated under the same 4 KiB guard as the existing tools. The KPIs are
//! read from the analysis payload — the tool never computes, caches or
//! records anything, and `None` in the payload stays "not measured", never
//! an invented zero.

use archguard_engine::core::ReviewKpis;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::server::ArchguardTools;

/// Input of the `review_kpis` tool (R33: `{ path?, query?, offset?, limit? }`).
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ReviewKpiParams {
    /// Project directory inside the MCP server root.
    pub path: Option<String>,
    /// Case-insensitive substring of record identities or descriptions.
    pub query: Option<String>,
    /// First flattened record; follow next_offset.
    pub offset: Option<usize>,
    /// 1..=200 processed records, further bounded by 4 KiB.
    pub limit: Option<usize>,
}

/// Reject out-of-range pages and oversized queries before any analysis runs.
pub fn validate(params: &ReviewKpiParams) -> Result<(), ErrorData> {
    if !(1..=200).contains(&params.limit.unwrap_or(crate::server::DEFAULT_LIMIT)) {
        return Err(crate::tools::invalid_params("limit must be 1..=200"));
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

/// One flattened KPI record: stable group + id, the response value itself
/// and the lowercased corpus the `query` filter matches against.
struct Record {
    group: &'static str,
    text: String,
    value: Value,
}

fn record(group: &'static str, id: String, description: String, mut value: Value) -> Record {
    value["group"] = group.into();
    value["id"] = id.clone().into();
    Record {
        group,
        text: format!("{group} {id} {description}").to_lowercase(),
        value,
    }
}

fn serialize<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("review KPI records serialize")
}

/// Flatten the payload into records, groups in this fixed order; every
/// within-group order is the producer's deterministic order. Optional
/// groups without a measurement contribute no record — absence is carried
/// by the `measured` map instead of a zero-valued record.
fn flatten(kpis: &ReviewKpis) -> Vec<Record> {
    let mut records = Vec::new();
    for pc in &kpis.propagation_cost {
        records.push(record(
            "propagation_cost",
            pc.scc_id.clone(),
            format!("size {} affected {}", pc.size, pc.affected),
            serialize(pc),
        ));
    }
    for violation in &kpis.sdp_violations {
        records.push(record(
            "sdp_violations",
            format!("{}->{}", violation.from, violation.to),
            violation.evidence.join(" "),
            serialize(violation),
        ));
    }
    if let Some(zones) = &kpis.zones {
        records.push(record(
            "zones",
            "zones".into(),
            "martin a/i zones".into(),
            serialize(zones),
        ));
    }
    if let Some(tangledness) = &kpis.tangledness {
        records.push(record(
            "tangledness",
            "tangledness".into(),
            format!(
                "{} of {} nodes in cycles",
                tangledness.tangled_nodes, tangledness.total
            ),
            serialize(tangledness),
        ));
    }
    if let Some(hub_gini) = &kpis.hub_gini {
        records.push(record(
            "hub_gini",
            "hub_gini".into(),
            "fan-in gini and top hub share".into(),
            serialize(hub_gini),
        ));
    }
    if let Some(drift) = &kpis.layer_drift {
        let described = [
            drift.declared_only.as_slice(),
            drift.derived_only.as_slice(),
            drift.violated.as_slice(),
        ]
        .concat()
        .join(" ");
        records.push(record(
            "layer_drift",
            "layer_drift".into(),
            described,
            serialize(drift),
        ));
    }
    if let Some(debt) = &kpis.debt {
        records.push(record(
            "debt",
            "debt".into(),
            debt.by_kind.keys().cloned().collect::<Vec<_>>().join(" "),
            serialize(debt),
        ));
    }
    for (index, surface) in kpis.api_surface.iter().enumerate() {
        records.push(record(
            "api_surface",
            format!("api-surface-{index}"),
            surface
                .top
                .iter()
                .map(|(file, _)| file.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            serialize(surface),
        ));
    }
    for leak in &kpis.leaks {
        records.push(record(
            "leaks",
            format!("{}::{}", leak.from_file, leak.symbol),
            format!("{} {}", leak.in_file, leak.evidence.join(" ")),
            serialize(leak),
        ));
    }
    for gap in &kpis.risk_gaps {
        records.push(record(
            "risk_gaps",
            gap.path.clone(),
            "risk-weighted coverage gap".into(),
            serialize(gap),
        ));
    }
    if let Some(inventory) = &kpis.dependencies {
        records.push(record(
            "dependencies",
            "dependencies".into(),
            inventory.notes.join(" "),
            json!({
                "total": inventory.total,
                "by_ecosystem": inventory.by_ecosystem,
                "stale_count": inventory.stale_count,
                "notes": inventory.notes.iter().take(3).collect::<Vec<_>>(),
                "notes_total": inventory.notes.len(),
                "dependencies_total": inventory.dependencies.len(),
            }),
        ));
        for dependency in &inventory.dependencies {
            records.push(record(
                "dependencies",
                format!("{}:{}", dependency.ecosystem, dependency.name),
                dependency.version.clone().unwrap_or_default(),
                serialize(dependency),
            ));
        }
    }
    if let Some(health) = &kpis.health {
        records.push(record(
            "health",
            "health".into(),
            health.parts.keys().cloned().collect::<Vec<_>>().join(" "),
            serialize(health),
        ));
    }
    records
}

/// Whole-payload record count per group; kept even when `query` filters
/// the page, so filters never hide the size of what they filtered.
fn counts(records: &[Record]) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for record in records {
        *counts.entry(record.group).or_insert(0usize) += 1;
    }
    counts
}

/// Build the paginated `review_kpis` answer for one payload; `None` is an
/// explicit not-measured state, never a zeroed report.
pub fn kpi_value(kpis: Option<&ReviewKpis>, params: &ReviewKpiParams) -> Result<Value, ErrorData> {
    validate(params)?;
    let query = params.query.as_deref().unwrap_or("").trim().to_lowercase();
    let (status, measured, records) = match kpis {
        None => ("not_measured", Value::Null, Vec::new()),
        Some(kpis) => {
            let mut measured = BTreeMap::<&'static str, bool>::new();
            measured.insert("propagation_cost", true);
            measured.insert("sdp_violations", true);
            measured.insert("zones", kpis.zones.is_some());
            measured.insert("tangledness", kpis.tangledness.is_some());
            measured.insert("hub_gini", kpis.hub_gini.is_some());
            measured.insert("layer_drift", kpis.layer_drift.is_some());
            measured.insert("debt", kpis.debt.is_some());
            measured.insert("api_surface", true);
            measured.insert("leaks", true);
            measured.insert("risk_gaps", true);
            measured.insert("dependencies", kpis.dependencies.is_some());
            measured.insert("health", kpis.health.is_some());
            (
                "measured",
                serde_json::to_value(measured).expect("measured map serializes"),
                flatten(kpis),
            )
        }
    };
    let counts = counts(&records);
    let rows: Vec<&Record> = records
        .iter()
        .filter(|record| query.is_empty() || record.text.contains(&query))
        .collect();
    let total = rows.len();
    let offset = params.offset.unwrap_or(0).min(total);
    let mut reply = json!({
        "status": status,
        "coverage_scope": "Records come from the review_kpis block the pipeline attached to the current analysis payload; the tool never computes KPIs itself. An unmeasured optional KPI contributes no record — the measured map marks it false and no value is invented as zero. Empty vec groups are measured findings of none. Propagation cost, tangledness, hub Gini, risk gaps and the health index are review heuristics, not defect proofs.",
        "not_measured_note": (status == "not_measured").then_some("The analysis payload carries no review_kpis block (graph written before the KPI pass or a pipeline without it). Absence means not measured, never clean — no KPI is reported as zero here."),
        "measured": measured,
        "counts": counts,
        "total": total, "offset": offset, "returned": 0, "processed": 0,
        "next_offset": null, "truncated": false, "records": [],
        "refresh": "archguard analyze <project> — the pipeline recomputes review_kpis; MCP only reads them"
    });
    let mut processed = 0;
    let mut returned = 0;
    let mut omitted = false;
    for record in rows
        .into_iter()
        .skip(offset)
        .take(params.limit.unwrap_or(crate::server::DEFAULT_LIMIT))
    {
        reply["records"]
            .as_array_mut()
            .unwrap()
            .push(record.value.clone());
        // Reserve cursor/count space even on the final, largest page.
        if reply.to_string().len() > crate::tools::MAX_RESPONSE_BYTES - 100 {
            reply["records"].as_array_mut().unwrap().pop();
            if returned > 0 {
                break;
            }
            reply["records"].as_array_mut().unwrap().push(json!({
                "omitted": true, "group": record.group, "index": offset + processed,
                "reason": "Record exceeds response byte budget; use the CLI JSON report for the complete document"
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

#[tool_router(router = review_kpi_router, vis = "pub(crate)")]
impl ArchguardTools {
    /// Review-dashboard KPIs as flat, paginated records (R33).
    #[tool(
        description = "Read the review-dashboard KPIs the pipeline attaches to the analysis payload: propagation cost per SCC, SDP violations, Martin A/I zones, tangledness, hub Gini, layer drift, debt density, API surface, leaky encapsulation, risk coverage gaps, external dependency inventory and the composite health index, flattened into stably identified records (group + id) and paginated within 4 KiB. Query filters identities and descriptions case-insensitively; whole-payload counts and the measured map survive filtering. None means not measured, never zero: an unmeasured optional KPI yields no record and no invented value. Read-only: reads the cached module analysis, never computes, scans or records anything — refresh with archguard analyze."
    )]
    async fn review_kpis(
        &self,
        Parameters(params): Parameters<ReviewKpiParams>,
    ) -> Result<String, ErrorData> {
        validate(&params)?;
        let run = crate::tools::analyze_run(&self.root, params.path.as_deref())?;
        // The effective report inherits the KPIs from the graph the
        // pipeline computed them on (R23/R26); fall back to the graph for
        // payloads that carry them only there.
        let kpis = run
            .report
            .review_kpis
            .as_ref()
            .or(run.file_graph.review_kpis.as_ref());
        kpi_value(kpis, &params).map(|value| value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ArchguardTools;
    use archguard_engine::core::{
        ApiSurface, DebtDensity, DependencyInventory, ExternalDependency, HealthIndex, HealthLevel,
        HubGini, LayerDrift, Leak, PropagationCost, RiskCoverageGap, SdpViolation, Tangledness,
    };
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::ErrorCode;

    fn kpis() -> ReviewKpis {
        ReviewKpis {
            propagation_cost: vec![
                PropagationCost {
                    scc_id: "scc-1".into(),
                    size: 2,
                    affected: 4,
                    total: 10,
                    pc: 0.4,
                },
                PropagationCost {
                    scc_id: "scc-2".into(),
                    size: 3,
                    affected: 9,
                    total: 10,
                    pc: 0.9,
                },
            ],
            sdp_violations: vec![SdpViolation {
                from: "app".into(),
                to: "core".into(),
                i_from: Some(0.8),
                i_to: None,
                evidence: vec!["src/app/a.ts:3".into()],
            }],
            zones: None,
            tangledness: Some(Tangledness {
                tangled_nodes: 5,
                total: 10,
                quote: 0.5,
            }),
            hub_gini: Some(HubGini {
                gini: 0.3,
                top_share: 0.6,
            }),
            layer_drift: Some(LayerDrift {
                agreement: Some(0.5),
                declared_only: vec!["api".into()],
                derived_only: vec![],
                violated: vec!["app".into()],
            }),
            debt: Some(DebtDensity {
                markers: 5,
                loc: 2000,
                per_kloc: Some(2.5),
                by_kind: [("todo".to_string(), 5usize)].into_iter().collect(),
            }),
            api_surface: vec![ApiSurface {
                exported_symbols: 12,
                files: 4,
                per_file: Some(3.0),
                top: vec![("src/app/a.ts".into(), 7)],
            }],
            leaks: vec![Leak {
                from_file: "src/app/a.ts".into(),
                symbol: "Secret".into(),
                in_file: "src/domain/s.ts".into(),
                evidence: vec!["src/app/a.ts:9".into()],
            }],
            risk_gaps: vec![RiskCoverageGap {
                path: "src/app/a.ts".into(),
                hotspot: 80.0,
                coverage: None,
                gap: None,
            }],
            dependencies: Some(DependencyInventory {
                total: 2,
                by_ecosystem: [("cargo".to_string(), 2usize)].into_iter().collect(),
                stale_count: None,
                dependencies: vec![
                    ExternalDependency {
                        ecosystem: "cargo".into(),
                        name: "serde".into(),
                        version: Some("1.0.200".into()),
                        stale: None,
                    },
                    ExternalDependency {
                        ecosystem: "cargo".into(),
                        name: "tokio".into(),
                        version: None,
                        stale: None,
                    },
                ],
                notes: vec!["read Cargo.lock".into()],
            }),
            health: Some(HealthIndex {
                score: Some(72.5),
                level: Some(HealthLevel::Yellow),
                trend: None,
                parts: [("tangledness".to_string(), 0.5f64)].into_iter().collect(),
            }),
        }
    }

    fn params(offset: Option<usize>, limit: Option<usize>, query: Option<&str>) -> ReviewKpiParams {
        ReviewKpiParams {
            path: None,
            query: query.map(str::to_string),
            offset,
            limit,
        }
    }

    #[test]
    fn review_kpis_tool_is_registered() {
        let tools = ArchguardTools::full_router().list_all();
        let tool = tools
            .iter()
            .find(|tool| tool.name == "review_kpis")
            .expect("review dashboard KPIs must be discoverable over MCP");
        let schema = serde_json::to_value(&tool.input_schema).unwrap();
        for field in ["path", "query", "offset", "limit"] {
            assert!(schema["properties"].get(field).is_some(), "missing {field}");
        }
        let description = tool.description.as_deref().unwrap();
        assert!(description.contains("never computes"));
        assert!(description.contains("not measured, never zero"));
    }

    #[test]
    fn records_flatten_query_paginate_and_stay_within_the_byte_budget() {
        let first = kpi_value(Some(&kpis()), &params(None, None, None)).unwrap();
        assert_eq!(first["status"], "measured");
        assert_eq!(first["counts"]["propagation_cost"], 2);
        assert_eq!(first["counts"]["dependencies"], 3); // aggregate + 2 deps
        assert_eq!(first["measured"]["zones"], false);
        assert_eq!(first["measured"]["tangledness"], true);
        let total = first["total"].as_u64().unwrap() as usize;
        assert_eq!(total, 14); // 2+1+1+1+1+1+1+1+1+3+1
        let mut offset = 0;
        let mut ids = std::collections::BTreeSet::new();
        loop {
            let page = kpi_value(Some(&kpis()), &params(Some(offset), Some(1), None)).unwrap();
            assert!(page.to_string().len() <= crate::tools::MAX_RESPONSE_BYTES);
            assert_eq!(page["returned"], 1);
            assert_eq!(page["total"], total as u64);
            assert_eq!(page["truncated"], true);
            for row in page["records"].as_array().unwrap() {
                assert!(ids.insert(row["id"].as_str().unwrap().to_string()));
            }
            if let Some(next) = page["next_offset"].as_u64() {
                assert!(next as usize > offset);
                offset = next as usize;
            } else {
                break;
            }
        }
        assert_eq!(ids.len(), total);
        let last = kpi_value(Some(&kpis()), &params(Some(total - 1), Some(1), None)).unwrap();
        assert_eq!(last["next_offset"], Value::Null);
        assert_eq!(last["returned"], 1);
        // Query filters identities/descriptions case-insensitively.
        let filtered = kpi_value(Some(&kpis()), &params(None, None, Some("SCC-2"))).unwrap();
        assert_eq!(filtered["total"], 1);
        assert_eq!(filtered["records"][0]["group"], "propagation_cost");
        assert_eq!(filtered["records"][0]["pc"], 0.9);
        assert_eq!(filtered["counts"]["propagation_cost"], 2); // whole payload
        let by_symbol = kpi_value(Some(&kpis()), &params(None, None, Some("secret"))).unwrap();
        assert_eq!(by_symbol["records"][0]["group"], "leaks");
        // Unknown coverage stays null inside the risk-gap record.
        let gaps = kpi_value(Some(&kpis()), &params(None, None, Some("risk_gaps"))).unwrap();
        assert!(gaps["records"][0]["coverage"].is_null());
        assert!(gaps["records"][0]["gap"].is_null());
    }

    #[test]
    fn missing_payload_is_not_measured_and_default_invents_no_zeros() {
        let missing = kpi_value(None, &params(None, None, None)).unwrap();
        assert_eq!(missing["status"], "not_measured");
        assert_eq!(missing["total"], 0);
        assert_eq!(missing["returned"], 0);
        assert_eq!(missing["next_offset"], Value::Null);
        assert!(missing["records"].as_array().unwrap().is_empty());
        assert!(missing["measured"].is_null());
        assert!(
            missing["not_measured_note"]
                .as_str()
                .unwrap()
                .contains("not measured, never clean")
        );
        assert!(
            !missing.to_string().contains("\"gini\""),
            "no fabricated measurement may appear: {missing}"
        );
        // A payload that ran every scan but found nothing: measured groups
        // stay measurable, unmeasured optionals yield no record and no zero.
        let empty = kpi_value(Some(&ReviewKpis::default()), &params(None, None, None)).unwrap();
        assert_eq!(empty["status"], "measured");
        assert_eq!(empty["total"], 0);
        assert!(empty["records"].as_array().unwrap().is_empty());
        assert_eq!(empty["measured"]["zones"], false);
        assert_eq!(empty["measured"]["dependencies"], false);
        assert_eq!(empty["measured"]["propagation_cost"], true);
        assert!(
            !empty.to_string().contains("\"gini\""),
            "an unmeasured hub Gini must not serialize as 0: {empty}"
        );
    }

    #[test]
    fn oversized_records_omit_explicitly_and_invalid_params_fail() {
        let mut kpis = kpis();
        kpis.sdp_violations[0].evidence = vec!["e".repeat(10000)];
        // A second, small violation matching the same query so the cursor
        // must keep advancing past the omitted oversized record.
        kpis.sdp_violations.push(SdpViolation {
            from: "app".into(),
            to: "core2".into(),
            i_from: Some(0.9),
            i_to: Some(0.1),
            evidence: vec![],
        });
        let page = kpi_value(Some(&kpis), &params(None, Some(1), Some("app->core"))).unwrap();
        assert_eq!(page["total"], 2);
        assert!(page.to_string().len() <= crate::tools::MAX_RESPONSE_BYTES);
        assert_eq!(page["returned"], 0);
        assert_eq!(page["processed"], 1);
        assert_eq!(page["next_offset"], 1);
        assert_eq!(page["records"][0]["omitted"], true);
        assert_eq!(page["records"][0]["group"], "sdp_violations");
        for bad in [
            params(None, Some(0), None),
            params(None, Some(201), None),
            params(None, None, Some("a\0b")),
            params(None, None, Some(&"x".repeat(513))),
        ] {
            assert!(validate(&bad).is_err());
        }
    }

    #[tokio::test]
    async fn handler_reads_the_pipeline_payload_of_a_cyclic_project() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src/app")).unwrap();
        std::fs::create_dir_all(root.path().join("src/lib")).unwrap();
        std::fs::write(
            root.path().join("src/app/one.ts"),
            "import { two } from \"../lib/two\";\n// TODO cyclic\nexport const one = (n: number): string => two(n + 1);\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("src/lib/two.ts"),
            "import { one } from \"../app/one\";\nexport const two = (n: number): string => one(n);\n",
        )
        .unwrap();
        let handler = ArchguardTools::new(root.path().into());
        let answer = handler
            .review_kpis(Parameters(ReviewKpiParams::default()))
            .await
            .unwrap();
        assert!(answer.len() <= crate::tools::MAX_RESPONSE_BYTES);
        let value: Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(value["status"], "measured");
        assert_eq!(value["counts"]["propagation_cost"], 1);
        assert_eq!(value["measured"]["tangledness"], true);
        assert_eq!(value["measured"]["hub_gini"], true);
        assert_eq!(value["measured"]["zones"], false);
        let page = kpi_rows(&value);
        let pc = page
            .iter()
            .find(|row| row["group"] == "propagation_cost")
            .expect("the 2-module cycle must carry a propagation-cost record");
        assert_eq!(pc["size"], 2);
        assert_eq!(pc["affected"], 2);
        let tangled = page
            .iter()
            .find(|row| row["group"] == "tangledness")
            .expect("both modules are cycle members");
        assert_eq!(tangled["tangled_nodes"], 2);
        assert_eq!(tangled["total"], 2);
        assert_eq!(tangled["quote"], 1.0);
        // Paging and the query filter work over the live payload, too.
        let small = handler
            .review_kpis(Parameters(params(Some(0), Some(1), None)))
            .await
            .unwrap();
        let small: Value = serde_json::from_str(&small).unwrap();
        assert_eq!(small["returned"], 1);
        assert!(small["next_offset"].is_number());
        assert_eq!(small["truncated"], true);
        let filtered = handler
            .review_kpis(Parameters(params(None, None, Some("TANGLEDNESS"))))
            .await
            .unwrap();
        let filtered: Value = serde_json::from_str(&filtered).unwrap();
        // The query matches the tangledness record and — via its parts —
        // the health index, but nothing else.
        assert!(filtered["total"].as_u64().unwrap() >= 1);
        assert_eq!(filtered["records"][0]["id"], "tangledness");
        for row in filtered["records"].as_array().unwrap() {
            assert!(
                matches!(row["group"].as_str().unwrap(), "tangledness" | "health"),
                "unexpected match: {row}"
            );
        }
        let escape = handler
            .review_kpis(Parameters(ReviewKpiParams {
                path: Some("../outside".into()),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(escape.code, ErrorCode::INVALID_PARAMS);
    }

    fn kpi_rows(value: &Value) -> Vec<Value> {
        assert_eq!(
            value["returned"], value["total"],
            "fixture must fit one page"
        );
        value["records"].as_array().unwrap().clone()
    }
}
