//! Minimal SARIF 2.1.0 rendering of a report's findings for `check --format sarif`.
use std::collections::BTreeMap;

use archguard_engine::core::{FindingKind, Report, Severity};
use serde_json::{Value, json};

use crate::output::to_pretty_json;

/// Minimal inline SARIF 2.1.0 serializer over the report's findings
/// (T25 — deliberately hand-rolled, no third-party SARIF crate): one
/// run, one driver rule per distinct [`FindingKind`], one result per
/// finding. Findings name modules, not files, so members become
/// `logicalLocations` rather than physical artifact locations.
pub(crate) fn to_sarif(report: &Report, source_paths: &BTreeMap<String, String>) -> String {
    let mut rule_ids: Vec<&'static str> = Vec::new();
    let results: Vec<Value> = report
        .findings
        .iter()
        .map(|finding| {
            let id = rule_id(finding.kind);
            if !rule_ids.contains(&id) {
                rule_ids.push(id);
            }
            let mut locations:Vec<_>=finding.evidence.iter().filter_map(|reference|source_paths.get(&reference.file).map(|path|{
                let uri:String=path.bytes().map(|byte|if byte.is_ascii_alphanumeric()||b"-._~/".contains(&byte){(byte as char).to_string()}else{format!("%{byte:02X}")}).collect();
                json!({"physicalLocation":{"artifactLocation":{"uri":uri},"region":{"startLine":reference.line.max(1)}},"message":{"text":reference.raw}})
            })).collect();
            if locations.is_empty(){locations=finding.nodes.iter().map(|node|json!({"logicalLocations":[{"name":node,"kind":"module"}]})).collect();}
            json!({
                "ruleId": id,
                "level": sarif_level(finding.severity),
                "message": {
                    "text": finding.message.clone().unwrap_or_else(||format!(
                        "{} {}: {}",
                        finding.id,
                        kind_name(finding.kind),
                        finding.nodes.join(", "),
                    )),
                },
                "locations": locations,
            })
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
    let doc = json!({
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
    to_pretty_json(&doc)
}

/// The SARIF rule id for a finding kind.
fn rule_id(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::Cycle => "archguard/cycle",
        FindingKind::LayerViolation => "archguard/layer-violation",
        FindingKind::MetricThreshold => "archguard/metric-threshold",
        FindingKind::LayerUnranked => "archguard/layer-unranked",
        FindingKind::DependencyRule => "archguard/dependency-rule",
        FindingKind::OwnershipRule => "archguard/ownership-rule",
        FindingKind::Orphan => "archguard/orphan",
        FindingKind::TargetArchitecture => "archguard/target-architecture",
    }
}

/// Severity → SARIF result level.
fn sarif_level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    }
}

/// The snake_case name of a finding kind (as in the report JSON).
fn kind_name(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::Cycle => "cycle",
        FindingKind::LayerViolation => "layer_violation",
        FindingKind::MetricThreshold => "metric_threshold",
        FindingKind::LayerUnranked => "layer_unranked",
        FindingKind::DependencyRule => "dependency_rule",
        FindingKind::OwnershipRule => "ownership_rule",
        FindingKind::Orphan => "orphan",
        FindingKind::TargetArchitecture => "target_architecture",
    }
}
