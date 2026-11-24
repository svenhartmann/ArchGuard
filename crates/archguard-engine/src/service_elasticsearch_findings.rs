//! Findings of the Elasticsearch map (ADR 0023, items 8 and 9): declared artifacts compared with
//! code usages. Pure over the model; names that are unknown never produce a finding.
use archguard_core::{
    ElasticsearchModel, SearchArtifact, SearchFinding, ServiceEvidence, ServiceTopology,
};
use std::collections::{BTreeMap, BTreeSet};

/// (kind, subject) -> (services, evidence)
type Hits<'a> = BTreeMap<(&'a str, String), (BTreeSet<String>, Vec<ServiceEvidence>)>;

const INCOMPLETE: &str = "Declarations incomplete: an Elasticsearch name in the Terraform files is not a literal, so a declaration may be missing.";

pub(crate) fn augment(report: &mut ServiceTopology) {
    let incomplete = report
        .diagnostics
        .iter()
        .any(|d| d.message.contains("Elasticsearch name is not a literal"))
        || report
            .elasticsearch
            .artifacts
            .iter()
            .any(|a| a.name.is_none());
    report.elasticsearch.findings = findings(&report.elasticsearch, incomplete);
}

/// Glob with `*` only (any run of characters, also none); everything else is literal.
fn glob(pattern: &str, value: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let (first, rest) = parts.split_first().expect("split yields one part");
    let Some(mut tail) = value.strip_prefix(first) else {
        return false;
    };
    let Some((last, middle)) = rest.split_last() else {
        return tail.is_empty();
    };
    for m in middle {
        match tail.find(m) {
            Some(i) => tail = &tail[i + m.len()..],
            None => return false,
        }
    }
    tail.ends_with(last)
}

/// Pattern against name is a glob; pattern against pattern: either one matches the other with
/// its `*` removed.
fn covers(a: &str, b: &str) -> bool {
    glob(a, &b.replace('*', "")) || glob(b, &a.replace('*', ""))
}

/// Names of declared indices and aliases plus template patterns.
fn declared(arts: &[SearchArtifact]) -> Vec<&str> {
    let templates = arts.iter().filter(|a| a.kind == "index_template");
    arts.iter()
        .filter(|a| matches!(a.kind.as_str(), "index" | "alias"))
        .filter_map(|a| a.name.as_deref())
        .chain(templates.flat_map(|a| a.patterns.iter().map(String::as_str)))
        .collect()
}

fn add<'a>(
    out: &mut Hits<'a>,
    kind: &'a str,
    subject: &str,
    service: Option<&str>,
    ev: &[ServiceEvidence],
) {
    let e = out.entry((kind, subject.to_string())).or_default();
    e.0.extend(service.map(String::from));
    e.1.extend(ev.iter().cloned());
}

/// (a), (d) and (e): what the declarations say about themselves.
fn declaration_hits(arts: &[SearchArtifact], out: &mut Hits<'_>) {
    let pipelines: BTreeSet<&str> = arts
        .iter()
        .filter(|a| a.kind == "ingest_pipeline")
        .filter_map(|a| a.name.as_deref())
        .collect();
    let declared = declared(arts);
    for a in arts {
        for r in a.references.iter().filter(|r| r.kind == "ingest_pipeline") {
            if !pipelines.contains(r.name.as_str()) {
                add(out, "undeclared_pipeline", &r.name, None, &a.evidence);
            }
        }
        let kind = match a.kind.as_str() {
            "data_view" => "uncovered_pattern",
            "alerting_rule" => "alert_undeclared_index",
            _ => continue,
        };
        for p in a
            .patterns
            .iter()
            .filter(|p| !declared.iter().any(|d| covers(d, p)))
        {
            add(out, kind, p, None, &a.evidence);
        }
    }
}

/// (b) and (c): what the code does with the declared indices.
fn usage_hits(model: &ElasticsearchModel, out: &mut Hits<'_>) {
    let arts = &model.artifacts;
    let declared = declared(arts);
    let covered = |p: &str| declared.iter().any(|d| covers(d, p));
    if arts.iter().any(|a| a.address.contains("elasticstack_")) {
        for u in &model.usages {
            let parts = u.index.split(',').map(str::trim);
            if parts
                .filter(|p| !p.is_empty() && !p.starts_with('-'))
                .any(|p| !covered(p))
            {
                add(
                    out,
                    "undeclared_index",
                    &u.index,
                    Some(&u.service),
                    &u.evidence,
                );
            }
        }
    }
    for u in model.usages.iter().filter(|u| u.access == "write") {
        add(
            out,
            "shared_write_index",
            &target(arts, &u.index),
            Some(&u.service),
            &u.evidence,
        );
    }
}

fn findings(model: &ElasticsearchModel, incomplete: bool) -> Vec<SearchFinding> {
    let mut out = Hits::new();
    declaration_hits(&model.artifacts, &mut out);
    usage_hits(model, &mut out);
    // A single writer is no finding.
    out.retain(|(k, _), (s, _)| *k != "shared_write_index" || s.len() >= 2);

    out.into_iter()
        .map(|((kind, subject), (services, mut evidence))| {
            if incomplete {
                evidence.push(ServiceEvidence {
                    file: String::new(),
                    pointer: String::new(),
                    line: None,
                    description: INCOMPLETE.into(),
                });
            }
            SearchFinding {
                kind: kind.into(),
                subject,
                services: services.into_iter().collect(),
                evidence,
            }
        })
        .collect()
}

/// An alias with exactly one declared target counts as that target.
fn target(arts: &[SearchArtifact], name: &str) -> String {
    let targets: BTreeSet<&str> = arts
        .iter()
        .filter(|a| a.kind == "alias" && a.name.as_deref() == Some(name))
        .flat_map(|a| a.references.iter().filter(|r| r.kind == "index"))
        .map(|r| r.name.as_str())
        .collect();
    match targets.iter().next() {
        Some(t) if targets.len() == 1 => (*t).into(),
        _ => name.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{covers, glob};

    #[test]
    fn glob_edge_cases() {
        assert!(glob("logs-*", "logs-"));
        assert!(glob("a*c*e", "abcde"));
        assert!(!glob("a*c*e", "abde"));
        assert!(glob("*", ""));
        assert!(!glob("a*a", "a"));
        assert!(glob("a.b", "a.b") && !glob("a.b", "axb"));
        assert!(covers("logs-*", "logs-app-*") && covers("logs-app-*", "logs-*"));
        assert!(!covers("logs-*", "metrics-*"));
    }
}
