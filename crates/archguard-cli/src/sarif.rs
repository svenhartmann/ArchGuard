//! Minimal SARIF 2.1.0 rendering of a report's findings for `check --format sarif`.
use std::collections::BTreeMap;

use archguard_engine::analysis::Baseline;
use archguard_engine::core::{Finding, FindingKind, Report, Severity};
use archguard_engine::{ViewNotes, accepted_by_baseline, accepted_by_note, note_value};
use serde_json::{Value, json};

use crate::output::to_pretty_json;

/// Minimal inline SARIF 2.1.0 serializer over the report's findings
/// (T25 — deliberately hand-rolled, no third-party SARIF crate): one
/// run, one driver rule per distinct [`FindingKind`], one result per
/// finding. Findings name modules, not files, so members become
/// `logicalLocations` rather than physical artifact locations. Findings the
/// `baseline` or an intentional note accepts are reported as `note`: they
/// no longer fail the gate. Notes go to the result's `properties`
/// ([`annotate`]); notes on gaps without a finding to the run's.
pub(crate) fn to_sarif(
    report: &Report,
    source_paths: &BTreeMap<String, String>,
    baseline: Option<&Baseline>,
    notes: &ViewNotes,
) -> String {
    let mut rule_ids: Vec<String> = Vec::new();
    let results: Vec<Value> = report
        .findings
        .iter()
        .map(|finding| {
            let id = rule_id(finding);
            if !rule_ids.contains(&id) {
                rule_ids.push(id.clone());
            }
            let mut locations:Vec<_>=finding.evidence.iter().filter_map(|reference|source_paths.get(&reference.file).map(|path|{
                let uri:String=path.bytes().map(|byte|if byte.is_ascii_alphanumeric()||b"-._~/".contains(&byte){(byte as char).to_string()}else{format!("%{byte:02X}")}).collect();
                json!({"physicalLocation":{"artifactLocation":{"uri":uri},"region":{"startLine":reference.line.max(1)}},"message":{"text":reference.raw}})
            })).collect();
            if locations.is_empty(){locations=finding.nodes.iter().map(|node|json!({"logicalLocations":[{"name":node,"kind":"module"}]})).collect();}
            let mut result = json!({
                "ruleId": id,
                "level": if accepted_by_baseline(baseline, finding) || accepted_by_note(notes, finding) {
                    "note"
                } else {
                    sarif_level(finding.severity)
                },
                "message": {
                    "text": finding.message.clone().unwrap_or_else(||format!(
                        "{} {}: {}",
                        finding.id,
                        archguard_engine::core::finding_kind_name(finding.kind),
                        finding.nodes.join(", "),
                    )),
                },
                "locations": locations,
            });
            annotate(&mut result, finding, notes);
            result
        })
        .collect();
    let rules: Vec<Value> = rule_ids
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "name": id.rsplit('/').next().unwrap_or(id),
                "shortDescription": { "text": id },
            })
        })
        .collect();
    let mut doc = json!({
        "version": "2.1.0",
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/sarif-2.1/schema/sarif-schema-2.1.0.json",
        "runs": [{
            "properties": {
                "semantic": report.semantic,
                "absence_of_cycles_proven": false,
                "dependency_scope": "Imports plus compiler-resolved calls and inheritance; type-only references are not covered. No detected cycles is not proof of an acyclic codebase.",
            },
            "tool": {
                "driver": {
                    "name": "archguard",
                    "version": env!("CARGO_PKG_VERSION"),
                    "rules": rules,
                },
            },
            "results": results,
        }],
    });
    let gaps = archguard_engine::noted_gaps_value(notes);
    if gaps.as_array().is_some_and(|g| !g.is_empty()) {
        doc["runs"][0]["properties"]["noted_gaps"] = gaps;
    }
    to_pretty_json(&doc)
}

/// The finding's note in `properties`; an intentional one that accepts the
/// finding also suppresses the result (kind `external`, the note as
/// justification), as a reviewer's decision outside the code.
fn annotate(result: &mut Value, finding: &Finding, notes: &ViewNotes) {
    let Some(d) = notes.get(&finding.id) else {
        return;
    };
    result["properties"] = json!({ "note": note_value(d) });
    if accepted_by_note(notes, finding) {
        result["suppressions"] = json!([{ "kind": "external", "justification": d.note }]);
    }
}

/// The SARIF rule id for a finding: one per kind, one per convention
/// (`archguard/convention/<rule>` from the id `view-<rule>:…`).
fn rule_id(finding: &Finding) -> String {
    let id = match finding.kind {
        FindingKind::Cycle => "archguard/cycle",
        FindingKind::LayerViolation => "archguard/layer-violation",
        FindingKind::MetricThreshold => "archguard/metric-threshold",
        FindingKind::LayerUnranked => "archguard/layer-unranked",
        FindingKind::DependencyRule => "archguard/dependency-rule",
        FindingKind::OwnershipRule => "archguard/ownership-rule",
        FindingKind::Orphan => "archguard/orphan",
        FindingKind::TargetArchitecture => "archguard/target-architecture",
        FindingKind::License => "archguard/license",
        FindingKind::SearchQuality => {
            let rule = finding.id.strip_prefix("es-").unwrap_or(&finding.id);
            return format!(
                "archguard/search-quality/{}",
                rule.split(':').next().unwrap_or(rule)
            );
        }
        FindingKind::Convention => {
            let rule = finding.id.strip_prefix("view-").unwrap_or(&finding.id);
            return format!(
                "archguard/convention/{}",
                rule.split(':').next().unwrap_or(rule)
            );
        }
    };
    id.to_string()
}

/// Severity → SARIF result level.
fn sarif_level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    }
}
