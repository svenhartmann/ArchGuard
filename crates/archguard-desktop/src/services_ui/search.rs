//! Services → Elasticsearch: the artifacts of `ServiceTopology.elasticsearch` by kind, the
//! open artifact's details, all findings and the reader gaps. Built like `contracts.rs`;
//! kinds are shown as the raw identifiers.

use std::collections::{BTreeSet, HashMap, HashSet};

use archguard_engine::core::{SearchArtifact, SearchFinding, ServiceEvidence, ServiceTopology};
use archguard_engine::{is_elasticsearch_gap, search_artifact_covers};
use slint::ModelRc;

use crate::tab_support::model;
use crate::ui::{
    SvcEvidence, SvcSearch, SvcSearchDetail, SvcSearchFinding, SvcSearchGap, SvcSearchKind,
    SvcSearchRow,
};

/// Artifacts, findings and gaps per page.
pub const PAGE: usize = 20;

/// The kinds in the documentation order of `SearchArtifact`; unknown kinds follow.
pub(crate) const KINDS: [&str; 14] = [
    "index",
    "alias",
    "index_template",
    "component_template",
    "ilm_policy",
    "ingest_pipeline",
    "transform",
    "watch",
    "enrich_policy",
    "snapshot_lifecycle",
    "stored_script",
    "search_template",
    "data_view",
    "alerting_rule",
];

/// How evidence is shown (`view::evidence`, passed in: `view` depends on the state, which holds
/// this section).
pub type Evidence = fn(&[ServiceEvidence]) -> ModelRc<SvcEvidence>;

/// The section's state: the open artifact (index into the topology's list) and the pages shown.
#[derive(Default)]
pub struct Search {
    open: Option<usize>,
    pages: usize,
}

/// Name, patterns and referenced names: what a finding's subject can denote.
fn names(a: &SearchArtifact) -> impl Iterator<Item = &str> {
    a.name
        .as_deref()
        .into_iter()
        .chain(a.patterns.iter().map(String::as_str))
        .chain(a.references.iter().map(|r| r.name.as_str()))
}

fn finding(f: &SearchFinding, evidence: Evidence) -> SvcSearchFinding {
    SvcSearchFinding {
        kind: f.kind.as_str().into(),
        subject: f.subject.as_str().into(),
        services: f.services.join(", ").into(),
        evidence: evidence(&f.evidence),
    }
}

/// Artifact indices grouped by kind, in [`KINDS`] order, unknown kinds by first appearance.
fn groups(arts: &[SearchArtifact]) -> Vec<(&str, Vec<usize>)> {
    let mut by_kind: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut unknown = Vec::new();
    for (i, a) in arts.iter().enumerate() {
        let kind = a.kind.as_str();
        if !KINDS.contains(&kind) && !by_kind.contains_key(kind) {
            unknown.push(kind);
        }
        by_kind.entry(kind).or_default().push(i);
    }
    KINDS
        .iter()
        .copied()
        .chain(unknown)
        .filter_map(|k| by_kind.remove(k).map(|v| (k, v)))
        .collect()
}

impl Search {
    /// Open artifact `i` of `len`, or close it when it is open.
    pub fn choose(&mut self, i: usize, len: usize) {
        self.open = (self.open != Some(i) && i < len).then_some(i);
    }

    pub fn more(&mut self) {
        self.pages += 1;
    }

    pub fn model(&self, t: &ServiceTopology, evidence: Evidence) -> SvcSearch {
        let es = &t.elasticsearch;
        let limit = PAGE * (self.pages + 1);
        let subjects: HashSet<&str> = es.findings.iter().map(|f| f.subject.as_str()).collect();
        let mut left = limit;
        let kinds = groups(&es.artifacts).into_iter().map(|(kind, all)| {
            let shown = all.len().min(left);
            left -= shown;
            let rows = all[..shown].iter().map(|&i| {
                let a = &es.artifacts[i];
                SvcSearchRow {
                    index: i as i32,
                    title: a.name.as_deref().unwrap_or(&a.address).into(),
                    warn: names(a).any(|n| subjects.contains(n)),
                    open: self.open == Some(i),
                }
            });
            SvcSearchKind {
                kind: kind.into(),
                count: all.len() as i32,
                rows: model(rows),
            }
        });
        let kinds: Vec<_> = kinds.collect();
        let gaps: Vec<_> = t
            .diagnostics
            .iter()
            .filter(|d| is_elasticsearch_gap(d))
            .collect();
        SvcSearch {
            total: es.artifacts.len() as i32,
            kinds: model(kinds),
            more: es.artifacts.len() > limit || es.findings.len() > limit || gaps.len() > limit,
            detail: self
                .open
                .and_then(|i| es.artifacts.get(i))
                .map(|a| detail(a, t, evidence))
                .unwrap_or_default(),
            findings: model(es.findings.iter().take(limit).map(|f| finding(f, evidence))),
            gaps: model(gaps.into_iter().take(limit).map(|d| SvcSearchGap {
                file: d.file.as_str().into(),
                service: d.service.as_deref().unwrap_or_default().into(),
                message: d.message.as_str().into(),
            })),
        }
    }
}

pub(crate) fn detail(
    a: &SearchArtifact,
    t: &ServiceTopology,
    evidence: Evidence,
) -> SvcSearchDetail {
    let es = &t.elasticsearch;
    let (mut writers, mut readers, mut others) =
        (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for u in es
        .usages
        .iter()
        .filter(|u| search_artifact_covers(a, &u.index))
    {
        let set = match u.access.as_str() {
            "write" => &mut writers,
            "read" => &mut readers,
            _ => &mut others,
        };
        set.insert(u.service.as_str());
    }
    let join = |s: BTreeSet<&str>| s.into_iter().collect::<Vec<_>>().join(", ").into();
    SvcSearchDetail {
        declaration: evidence(&a.evidence),
        scope: a.scope.as_str().into(),
        patterns: model(a.patterns.iter().map(|p| p.as_str().into())),
        references: model(
            a.references
                .iter()
                .map(|r| format!("{} {}", r.kind, r.name).into()),
        ),
        writers: join(writers),
        readers: join(readers),
        others: join(others),
        findings: model(
            es.findings
                .iter()
                .filter(|f| names(a).any(|n| n == f.subject))
                .map(|f| finding(f, evidence)),
        ),
    }
}

#[cfg(test)]
mod tests {
    use archguard_engine::core::{
        SearchArtifact, SearchFinding, SearchIndexUsage, SearchReference, ServiceDiagnostic,
        ServiceEvidence,
    };
    use slint::{Model, ModelRc};

    use super::*;

    fn ev(file: &str, line: u32) -> ServiceEvidence {
        ServiceEvidence {
            file: file.into(),
            pointer: String::new(),
            line: Some(line),
            description: "declared".into(),
        }
    }

    fn art(kind: &str, name: &str, patterns: &[&str], refs: &[(&str, &str)]) -> SearchArtifact {
        SearchArtifact {
            kind: kind.into(),
            name: Some(name.into()),
            address: format!("{kind}.{name}"),
            scope: "infra".into(),
            patterns: patterns.iter().map(|p| p.to_string()).collect(),
            references: refs
                .iter()
                .map(|(k, n)| SearchReference {
                    kind: k.to_string(),
                    name: n.to_string(),
                })
                .collect(),
            evidence: vec![ev("main.tf", 3)],
        }
    }

    /// `file:line` rows, as `view::evidence` (not imported: it would close a file cycle).
    fn shown_evidence(items: &[ServiceEvidence]) -> ModelRc<SvcEvidence> {
        crate::tab_support::model(items.iter().map(|e| SvcEvidence {
            location: format!("{}:{}", e.file, e.line.unwrap_or(0)).into(),
            description: e.description.as_str().into(),
        }))
    }

    fn usage(service: &str, index: &str, access: &str) -> SearchIndexUsage {
        SearchIndexUsage {
            service: service.into(),
            index: index.into(),
            access: access.into(),
            evidence: vec![],
        }
    }

    fn finding(kind: &str, subject: &str, services: &[&str]) -> SearchFinding {
        SearchFinding {
            kind: kind.into(),
            subject: subject.into(),
            services: services.iter().map(|s| s.to_string()).collect(),
            evidence: vec![ev("src/app.rs", 9)],
        }
    }

    /// Artifacts in scrambled order, an unknown kind, usages, findings and a gap.
    fn topology() -> ServiceTopology {
        let mut t = ServiceTopology::default();
        let es = &mut t.elasticsearch;
        es.artifacts = vec![
            art("ingest_pipeline", "p1", &[], &[]),
            art("zz_future", "z", &[], &[]),
            art(
                "index_template",
                "t1",
                &["logs-*"],
                &[("ingest_pipeline", "p1")],
            ),
            art("index", "logs-a", &[], &[]),
            art("index", "plain", &[], &[]),
            art("alias", "logs-all", &[], &[]),
        ];
        es.usages = vec![
            usage("api", "logs-a", "write"),
            usage("api", "logs-a", "write"),
            usage("web", "logs-a", "read"),
            usage("web", "logs-a", "unknown"),
            usage("web", "other", "read"),
        ];
        es.findings = vec![
            finding("undeclared_index", "orders-x", &["api"]),
            finding("shared_write_index", "logs-a", &["api", "web"]),
        ];
        t.diagnostics.push(ServiceDiagnostic {
            file: "main.tf".into(),
            service: Some("api".into()),
            message:
                "JSON file next to Terraform could not be read; search templates in it are unknown."
                    .into(),
        });
        t.diagnostics.push(ServiceDiagnostic {
            file: "x".into(),
            service: None,
            message: "unrelated".into(),
        });
        t
    }

    fn strings(m: ModelRc<slint::SharedString>) -> Vec<String> {
        m.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_model_is_default() {
        let m = Search::default().model(&ServiceTopology::default(), shown_evidence);
        assert_eq!(m.total, 0);
        assert_eq!(m.kinds.row_count(), 0);
        assert_eq!(m.gaps.row_count(), 0);
    }

    #[test]
    fn kinds_in_documented_order_with_counts_and_unknown_kinds_last() {
        let m = Search::default().model(&topology(), shown_evidence);
        let kinds: Vec<_> = m
            .kinds
            .iter()
            .map(|k| (k.kind.to_string(), k.count))
            .collect();
        let want = [
            ("index", 2),
            ("alias", 1),
            ("index_template", 1),
            ("ingest_pipeline", 1),
            ("zz_future", 1),
        ];
        assert_eq!(kinds, want.map(|(k, c)| (k.to_string(), c)));
        assert_eq!(m.total, 6);
    }

    #[test]
    fn warn_when_a_finding_names_the_artifact_its_pattern_or_reference() {
        let mut t = topology();
        t.elasticsearch.findings = vec![
            finding("shared_write_index", "logs-a", &[]),
            finding("uncovered_pattern", "logs-*", &[]),
            finding("undeclared_pipeline", "p2", &[]),
        ];
        t.elasticsearch.artifacts.push(art(
            "index_template",
            "t2",
            &[],
            &[("ingest_pipeline", "p2")],
        ));
        let m = Search::default().model(&t, shown_evidence);
        let warned: Vec<_> = m
            .kinds
            .iter()
            .flat_map(|k| {
                k.rows
                    .iter()
                    .filter(|r| r.warn)
                    .map(|r| r.title.to_string())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(warned.len(), 3, "{warned:?}");
        assert!(warned.iter().any(|w| w.contains("logs-a")));
        assert!(warned.iter().any(|w| w.contains("t1")));
        assert!(warned.iter().any(|w| w.contains("t2")));
    }

    #[test]
    fn choose_opens_and_closes_and_ignores_out_of_range() {
        let t = topology();
        let (mut s, len) = (Search::default(), t.elasticsearch.artifacts.len());
        s.choose(3, len);
        assert_eq!(s.model(&t, shown_evidence).detail.scope.as_str(), "infra");
        s.choose(3, len);
        assert_eq!(s.model(&t, shown_evidence).detail.scope.as_str(), "");
        s.choose(len, len);
        assert_eq!(s.model(&t, shown_evidence).detail.scope.as_str(), "");
    }

    #[test]
    fn detail_of_an_index_has_declaration_writers_readers_and_findings() {
        let t = topology();
        let mut s = Search::default();
        s.choose(3, 6);
        let d = s.model(&t, shown_evidence).detail;
        assert_eq!(
            d.declaration.row_data(0).unwrap().location.as_str(),
            "main.tf:3"
        );
        assert_eq!(d.writers.as_str(), "api");
        assert_eq!(d.readers.as_str(), "web");
        assert_eq!(d.others.as_str(), "web");
        assert_eq!(d.findings.row_count(), 1);
        assert_eq!(
            d.findings.row_data(0).unwrap().kind.as_str(),
            "shared_write_index"
        );
    }

    #[test]
    fn detail_of_a_template_has_patterns_references_and_covered_usage() {
        let t = topology();
        let mut s = Search::default();
        s.choose(2, 6);
        let d = s.model(&t, shown_evidence).detail;
        assert_eq!(strings_of(&d.patterns), ["logs-*"]);
        assert_eq!(strings_of(&d.references), ["ingest_pipeline p1"]);
        assert_eq!(d.writers.as_str(), "api");
        assert_eq!(d.readers.as_str(), "web");
    }

    fn strings_of(m: &ModelRc<slint::SharedString>) -> Vec<String> {
        strings(m.clone())
    }

    #[test]
    fn all_findings_and_only_es_gaps_are_listed() {
        let m = Search::default().model(&topology(), shown_evidence);
        assert_eq!(m.findings.row_count(), 2);
        let f = m.findings.row_data(0).unwrap();
        assert_eq!(
            (f.kind.as_str(), f.subject.as_str(), f.services.as_str()),
            ("undeclared_index", "orders-x", "api")
        );
        assert_eq!(
            f.evidence.row_data(0).unwrap().location.as_str(),
            "src/app.rs:9"
        );
        assert_eq!(m.gaps.row_count(), 1);
        let g = m.gaps.row_data(0).unwrap();
        assert_eq!((g.file.as_str(), g.service.as_str()), ("main.tf", "api"));
    }

    #[test]
    fn more_shows_the_next_page_of_artifacts() {
        let mut t = ServiceTopology::default();
        t.elasticsearch.artifacts = (0..PAGE + 1)
            .map(|i| art("index", &format!("i{i}"), &[], &[]))
            .collect();
        let mut s = Search::default();
        let shown = |m: &SvcSearch| m.kinds.iter().map(|k| k.rows.row_count()).sum::<usize>();
        let m = s.model(&t, shown_evidence);
        assert_eq!(
            (shown(&m), m.more, m.kinds.row_data(0).unwrap().count),
            (PAGE, true, PAGE as i32 + 1)
        );
        s.more();
        let m = s.model(&t, shown_evidence);
        assert_eq!((shown(&m), m.more), (PAGE + 1, false));
    }
}
