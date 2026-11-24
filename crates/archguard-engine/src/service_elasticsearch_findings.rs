//! Findings of the Elasticsearch map (ADR 0023, items 8 and 9): declared artifacts compared with
//! code usages. Pure over the model; names that are unknown never produce a finding.
use super::service_elasticsearch_code as code;
use super::service_elasticsearch_tf as tf;
use archguard_core::{
    ElasticsearchModel, SearchArtifact, SearchFinding, ServiceDiagnostic, ServiceEvidence,
    ServiceTopology,
};
use std::collections::{BTreeMap, BTreeSet};

/// (kind, subject) -> (services, evidence)
type Hits<'a> = BTreeMap<(&'a str, String), (BTreeSet<String>, Vec<ServiceEvidence>)>;

const INCOMPLETE: &str = "Declarations incomplete: an Elasticsearch name in the Terraform files is not a literal, so a declaration may be missing.";

pub(crate) fn augment(report: &mut ServiceTopology) {
    let incomplete = report
        .diagnostics
        .iter()
        .any(|d| d.message.contains(tf::NOT_LITERAL))
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

/// Whether `a` names or matches the index `index`: indices and aliases by name, templates,
/// data views and alerting rules by their patterns; every other kind covers nothing.
pub fn search_artifact_covers(a: &SearchArtifact, index: &str) -> bool {
    match a.kind.as_str() {
        "index" | "alias" => a.name.as_deref() == Some(index),
        "index_template" | "data_view" | "alerting_rule" => {
            a.patterns.iter().any(|p| covers(p, index))
        }
        _ => false,
    }
}

/// Whether the diagnostic comes from one of the Elasticsearch readers (fixed texts, or fixed
/// head/tail around a name, a line or a count).
pub fn is_elasticsearch_gap(d: &ServiceDiagnostic) -> bool {
    let m = d.message.as_str();
    [
        code::SAME_DIR,
        code::UNREAD,
        tf::JSON_UNREAD,
        tf::JSON_TOO_LARGE,
    ]
    .contains(&m)
        || m.starts_with(code::constants::GAP_HEAD)
        || (m.starts_with(code::SCAN_STOP_HEAD) && m.ends_with(code::SCAN_STOP_TAIL))
        || m.ends_with(tf::SAVED_OBJECTS)
        || m.ends_with(tf::NO_PARAMS_INDEX)
        || m.contains(tf::NOT_LITERAL)
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

/// An alias whose declarations all name exactly one index counts as that index; a declaration
/// without a known index target may be the write index, so the alias keeps its own name.
fn target(arts: &[SearchArtifact], name: &str) -> String {
    let mut targets = BTreeSet::new();
    let mut unknown = false;
    for a in arts
        .iter()
        .filter(|a| a.kind == "alias" && a.name.as_deref() == Some(name))
    {
        targets.extend(
            a.references
                .iter()
                .filter(|r| r.kind == "index")
                .map(|r| r.name.as_str()),
        );
        unknown |= a.references.iter().all(|r| r.kind != "index");
    }
    match targets.iter().next() {
        Some(t) if targets.len() == 1 && !unknown => (*t).into(),
        _ => name.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{covers, glob, is_elasticsearch_gap, search_artifact_covers};
    use archguard_core::{SearchArtifact, ServiceDiagnostic};

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

    fn art(kind: &str, name: Option<&str>, patterns: &[&str]) -> SearchArtifact {
        SearchArtifact {
            kind: kind.into(),
            name: name.map(String::from),
            address: String::new(),
            scope: String::new(),
            patterns: patterns.iter().map(|p| p.to_string()).collect(),
            references: vec![],
            evidence: vec![],
        }
    }

    #[test]
    fn artifact_covers_by_kind() {
        for kind in ["index", "alias"] {
            assert!(search_artifact_covers(
                &art(kind, Some("logs-a"), &[]),
                "logs-a"
            ));
            assert!(!search_artifact_covers(
                &art(kind, Some("logs-a"), &[]),
                "logs-b"
            ));
            // a name is no glob, and a nameless artifact covers nothing
            assert!(!search_artifact_covers(
                &art(kind, Some("logs-*"), &[]),
                "logs-a"
            ));
            assert!(!search_artifact_covers(
                &art(kind, None, &["logs-*"]),
                "logs-a"
            ));
        }
        for kind in ["index_template", "data_view", "alerting_rule"] {
            assert!(search_artifact_covers(
                &art(kind, None, &["logs-*"]),
                "logs-a"
            ));
            assert!(!search_artifact_covers(
                &art(kind, None, &["logs-*"]),
                "metrics-a"
            ));
            // the name does not count for pattern kinds
            assert!(!search_artifact_covers(
                &art(kind, Some("logs-a"), &[]),
                "logs-a"
            ));
        }
        assert!(!search_artifact_covers(
            &art("ingest_pipeline", Some("p"), &["p"]),
            "p"
        ));
    }

    fn diag(message: &str) -> ServiceDiagnostic {
        ServiceDiagnostic {
            file: "f".into(),
            service: None,
            message: message.into(),
        }
    }

    #[test]
    fn gap_check_knows_elasticsearch_diagnostics() {
        // every fixed text of the three producers
        for m in [
            "Several services build from the same source directory; indices named in this file are attributed to none of them.",
            "Source file not read: larger than 1 MiB, not a regular file or not readable; its indices are unknown.",
            "Index name is not a literal (constant, variable or expression); the index stays unknown. Source line 7.",
            "Index scan of the service stopped after 500 files; indices in later files are unknown.",
            "JSON file next to Terraform could not be read; search templates in it are unknown.",
            "JSON file next to Terraform is larger than 1 MiB and contains a mustache lang; it is not evaluated as a search template.",
            "a.b: Elasticsearch name is not a literal; the index is unknown (variables).",
            "a.b: alerting rule has no top-level params index; its index comes from a data view or is unknown.",
            "a.b: saved objects are not analyzed.",
        ] {
            assert!(is_elasticsearch_gap(&diag(m)), "{m}");
        }
    }

    #[test]
    fn gap_check_ignores_other_diagnostics() {
        assert!(!is_elasticsearch_gap(&diag(
            "Compose file could not be read; its services are unknown."
        )));
        assert!(!is_elasticsearch_gap(&diag("Index name is fine")));
    }

    /// `{CONST}` interpolation inside a format string.
    fn interpolates_const(text: &str) -> bool {
        text.split('{').skip(1).any(|r| {
            r.split('}').next().is_some_and(|n| {
                !n.is_empty() && n.chars().all(|c| c.is_ascii_uppercase() || c == '_')
            })
        })
    }

    /// Every diagnostic site of the three producers passes a constant, `gap(..)` or a message
    /// built from a constant, so `is_elasticsearch_gap` can recognize it. A new fixed text needs
    /// a constant and a clause in `is_elasticsearch_gap`.
    #[test]
    fn every_diagnostic_site_uses_a_constant() {
        let files = [
            (
                "service_elasticsearch_code.rs",
                include_str!("service_elasticsearch_code.rs"),
            ),
            (
                "service_elasticsearch_constants.rs",
                include_str!("service_elasticsearch_constants.rs"),
            ),
            (
                "service_elasticsearch_tf.rs",
                include_str!("service_elasticsearch_tf.rs"),
            ),
        ];
        let mut sites = 0;
        for (name, text) in files {
            let lines: Vec<&str> = text
                .lines()
                .take_while(|l| !l.contains("#[cfg(test)]"))
                .collect();
            for (n, line) in lines.iter().enumerate() {
                let at = format!("{name}:{} `{}`", n + 1, line.trim());
                let window = lines[n..lines.len().min(n + 6)].join("\n");
                let back = lines[n.saturating_sub(8)..n].join("\n");
                if line.contains("ServiceDiagnostic {") && line.contains("push(") {
                    sites += 1;
                    assert!(
                        interpolates_const(&window),
                        "{at}: message without constant; see is_elasticsearch_gap"
                    );
                } else if line.contains("diagnostic(") && !line.contains("fn ") {
                    sites += 1;
                    let rest = line.split("diagnostic(").nth(1).unwrap();
                    let arg = rest.splitn(4, ", ").nth(3).unwrap_or("");
                    let ident: String = arg
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    let ok = match ident.as_str() {
                        "gap" => true,
                        "message" => interpolates_const(back.rsplit("let message").next().unwrap()),
                        id => {
                            !id.is_empty() && id.chars().all(|c| c.is_ascii_uppercase() || c == '_')
                        }
                    };
                    assert!(ok, "{at}: text is not a constant; see is_elasticsearch_gap");
                }
            }
        }
        assert_eq!(
            sites, 10,
            "diagnostic sites changed; update is_elasticsearch_gap and this count"
        );
    }
}
