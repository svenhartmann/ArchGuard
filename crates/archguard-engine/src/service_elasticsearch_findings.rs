//! Findings of the Elasticsearch map (ADR 0023, items 8 and 9): declared artifacts compared with
//! code usages. Pure over the model; names that are unknown never produce a finding.
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
        .any(|d| d.message.contains(EsGap::NotLiteral.text()))
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

/// The names of a usage's index list: comma-split, trimmed, without empty parts and `-`
/// exclusions (an exclusion names nothing). The one split rule for findings and covers.
pub(super) fn index_parts(index: &str) -> impl Iterator<Item = &str> {
    index
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty() && !p.starts_with('-'))
}

/// Whether `a` names or matches a name of the usage's index list `index`: indices and aliases
/// by name, templates, data views and alerting rules by their patterns; every other kind
/// covers nothing.
pub fn search_artifact_covers(a: &SearchArtifact, index: &str) -> bool {
    index_parts(index).any(|p| match a.kind.as_str() {
        "index" | "alias" => a.name.as_deref() == Some(p),
        "index_template" | "data_view" | "alerting_rule" => a.patterns.iter().any(|q| covers(q, p)),
        _ => false,
    })
}

/// The Elasticsearch reader gaps: the only way to create such a diagnostic is [`es_gap`], the
/// only way to recognize one is [`is_elasticsearch_gap`]. One list defines the variants, `ALL`
/// and their texts, so a variant cannot exist without being recognized.
macro_rules! es_gaps {
    ($($(#[$m:meta])* $name:ident => $text:expr,)+) => {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub(super) enum EsGap { $($(#[$m])* $name,)+ }

        impl EsGap {
            pub(super) const ALL: &'static [EsGap] = &[$(EsGap::$name,)+];

            /// The fixed text; for kinds with a dynamic part, the part next to it (see `render`).
            pub(super) fn text(self) -> &'static str {
                match self { $(EsGap::$name => $text,)+ }
            }
        }
    };
}

es_gaps! {
    SameDir => "Several services build from the same source directory; indices named in this file are attributed to none of them.",
    Unread => "Source file not read: larger than 1 MiB, not a regular file or not readable; its indices are unknown.",
    IndexNotLiteral => "Index name is not a literal (constant, variable or expression); the index stays unknown. Source line ",
    ScanStop => "Index scan of the service stopped after ",
    JsonUnread => "JSON file next to Terraform could not be read; search templates in it are unknown.",
    JsonTooLarge => "JSON file next to Terraform is larger than 1 MiB and contains a mustache lang; it is not evaluated as a search template.",
    SavedObjects => ": saved objects are not analyzed.",
    NotLiteral => ": Elasticsearch name is not a literal; ",
    NoParamsIndex => ": alerting rule has no top-level params index; its index comes from a data view or is unknown.",
    DetailNotLiteral => ": index mapping, settings or analysis are not literal (file(), local., var. and references are not evaluated); not read: ",
}

const SCAN_STOP_TAIL: &str = " files; indices in later files are unknown.";
const NOT_LITERAL_TAIL: &str =
    " is unknown (variables, locals, each, module outputs and function calls are not evaluated).";

impl EsGap {
    /// `parts`: line number, file count, address, or address and what is unknown.
    fn render(self, parts: &[&str]) -> String {
        let (t, p) = (self.text(), |i: usize| {
            parts.get(i).copied().unwrap_or_default()
        });
        match self {
            Self::IndexNotLiteral => format!("{t}{}.", p(0)),
            Self::ScanStop => format!("{t}{}{SCAN_STOP_TAIL}", p(0)),
            Self::SavedObjects | Self::NoParamsIndex => format!("{}{t}", p(0)),
            Self::NotLiteral => format!("{}{t}{}{NOT_LITERAL_TAIL}", p(0), p(1)),
            Self::DetailNotLiteral => format!("{}{t}{}.", p(0), p(1)),
            _ => t.into(),
        }
    }

    fn matches(self, m: &str) -> bool {
        let t = self.text();
        match self {
            Self::IndexNotLiteral => m.starts_with(t),
            Self::ScanStop => m.starts_with(t) && m.ends_with(SCAN_STOP_TAIL),
            Self::SavedObjects | Self::NoParamsIndex => m.ends_with(t),
            Self::NotLiteral | Self::DetailNotLiteral => m.contains(t),
            _ => m == t,
        }
    }
}

/// The one constructor of Elasticsearch diagnostics. Saved-object notes are no gap: the report
/// stays complete.
pub(super) fn es_gap(
    report: &mut ServiceTopology,
    kind: EsGap,
    file: &str,
    service: Option<&str>,
    parts: &[&str],
) {
    report.complete &= kind == EsGap::SavedObjects;
    report.diagnostics.push(ServiceDiagnostic {
        file: file.into(),
        service: service.map(str::to_owned),
        message: kind.render(parts),
    });
}

/// Whether the diagnostic comes from one of the Elasticsearch readers.
pub fn is_elasticsearch_gap(d: &ServiceDiagnostic) -> bool {
    EsGap::ALL.iter().any(|k| k.matches(&d.message))
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
            if index_parts(&u.index).any(|p| !covered(p)) {
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
    use super::{EsGap, covers, es_gap, glob, is_elasticsearch_gap, search_artifact_covers};
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
            origins: vec![],
            detail: None,
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
        for kind in ["index_template", "data_view", "alerting_rule", "watch"] {
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

    #[test]
    fn gap_texts_are_distinct() {
        let texts: std::collections::BTreeSet<_> = EsGap::ALL.iter().map(|k| k.text()).collect();
        assert_eq!(texts.len(), EsGap::ALL.len());
    }

    #[test]
    fn every_gap_kind_is_recognized_with_its_dynamic_part() {
        for &k in EsGap::ALL {
            let mut r = archguard_core::ServiceTopology::default();
            es_gap(&mut r, k, "f", None, &["7", "what"]);
            es_gap(&mut r, k, "f", None, &[]);
            assert!(r.diagnostics.iter().all(is_elasticsearch_gap), "{k:?}");
        }
    }

    /// The constructor is the only place that builds a diagnostic: no other token sequence
    /// `ServiceDiagnostic {` or `diagnostic (` in the three producers (whitespace and line
    /// breaks do not matter).
    #[test]
    fn producers_build_diagnostics_only_through_es_gap() {
        let files = [
            ("code", include_str!("service_elasticsearch_code.rs")),
            (
                "constants",
                include_str!("service_elasticsearch_constants.rs"),
            ),
            ("tf", include_str!("service_elasticsearch_tf.rs")),
        ];
        for (name, text) in files {
            let text = text.split("#[cfg(test)]").next().unwrap();
            let flat: String = text.split_whitespace().collect::<Vec<_>>().join("");
            for bad in ["ServiceDiagnostic{", "diagnostic(", "diagnostics.push("] {
                assert!(!flat.contains(bad), "{name}: `{bad}` outside es_gap");
            }
        }
    }
}
