//! The content of an index, template or ghost detail page as plain data (no Slint types; `page_view`
//! converts it). Pure function of the topology; coverage is the engine's
//! `search_artifact_covers`.

use archguard_engine::core::{SearchArtifact, SearchIndexDetail, ServiceTopology};
use archguard_engine::{SearchNode, is_detail_gap, search_artifact_covers, short_scope};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct IndexPage {
    pub overview: Overview,
    pub mapping: Vec<MappingRow>,
    pub settings: Vec<(String, String)>,
    pub analysis: Vec<AnalysisRow>,
    /// Why mapping and settings are empty; `None` when a detail was read.
    pub note: Option<DetailNote>,
    pub links: Vec<Link>,
    pub usages: Vec<UsageRow>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Overview {
    /// File and line of the declaration; a ghost has none.
    pub declaration: Option<(String, Option<u32>)>,
    pub scope: String,
    pub address: String,
    pub origins: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct MappingRow {
    pub depth: usize,
    /// Last path segment.
    pub name: String,
    pub kind: String,
    pub analyzer: Option<String>,
    pub search_analyzer: Option<String>,
    pub normalizer: Option<String>,
    /// `object` or `nested`: its children follow.
    pub parent: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AnalysisRow {
    pub kind: String,
    pub name: String,
    pub definition: String,
}

/// Translation key of the hint shown instead of mapping and settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailNote {
    NotRead,
    NotDeclared,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Link {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct UsageRow {
    pub service: String,
    pub access: String,
    pub file: String,
    pub line: Option<u32>,
    /// False for a usage of another project of a set (`<project>:` prefix).
    pub clickable: bool,
}

/// Settings that come first, in this order; the link settings live under "Verknüpfungen".
const FIRST: [&str; 3] = ["number_of_shards", "number_of_replicas", "refresh_interval"];
const LINKED: [&str; 3] = ["default_pipeline", "final_pipeline", "lifecycle.name"];

/// The page of a node; a ghost (no artifact) is an undeclared index named by its title.
pub fn index_page(t: &ServiceTopology, node: &SearchNode) -> IndexPage {
    let es = &t.elasticsearch;
    let ghost;
    let a = match node.artifact.and_then(|i| es.artifacts.get(i)) {
        Some(a) => a,
        None => {
            ghost = SearchArtifact {
                kind: node.kind.clone(),
                name: Some(node.title.clone()),
                address: node.title.clone(),
                scope: String::new(),
                patterns: Vec::new(),
                references: Vec::new(),
                evidence: Vec::new(),
                origins: Vec::new(),
                detail: None,
            };
            &ghost
        }
    };
    let name = a.name.as_deref().unwrap_or_default();
    let aliases: Vec<&SearchArtifact> = es
        .artifacts
        .iter()
        .filter(|x| x.kind == "alias" && refers_to(x, &[name]))
        .collect();
    let mut names = vec![name];
    names.extend(aliases.iter().filter_map(|x| x.name.as_deref()));
    let mut page = IndexPage {
        overview: Overview {
            declaration: a.evidence.first().map(|e| (e.file.clone(), e.line)),
            scope: short_scope(&a.scope).to_string(),
            address: a.address.clone(),
            origins: a.origins.clone(),
        },
        links: links(t, a, &names, &aliases),
        usages: usages(t, a, &aliases),
        ..Default::default()
    };
    match &a.detail {
        Some(d) => detail(&mut page, d),
        None => {
            let read_gap = t.diagnostics.iter().any(|d| is_detail_gap(d, &a.address));
            page.note = Some(if read_gap {
                DetailNote::NotRead
            } else {
                DetailNote::NotDeclared
            });
        }
    }
    page
}

fn detail(page: &mut IndexPage, d: &SearchIndexDetail) {
    page.mapping = d
        .fields
        .iter()
        .map(|f| MappingRow {
            depth: f.path.matches('.').count(),
            name: f.path.rsplit('.').next().unwrap_or_default().into(),
            kind: f.kind.clone(),
            analyzer: f.analyzer.clone(),
            search_analyzer: f.search_analyzer.clone(),
            normalizer: f.normalizer.clone(),
            parent: matches!(f.kind.as_str(), "object" | "nested"),
        })
        .collect();
    let get = |k: &str| d.settings.get(k).map(|v| (k.to_string(), v.clone()));
    page.settings = FIRST.iter().filter_map(|k| get(k)).collect();
    page.settings.extend(
        d.settings
            .iter()
            .filter(|(k, _)| !FIRST.contains(&k.as_str()) && !LINKED.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    page.analysis = d
        .analysis
        .iter()
        .map(|x| AnalysisRow {
            kind: x.kind.clone(),
            name: x.name.clone(),
            definition: x.definition.clone(),
        })
        .collect();
}

fn refers_to(a: &SearchArtifact, names: &[&str]) -> bool {
    a.references
        .iter()
        .any(|r| r.kind == "index" && names.contains(&r.name.as_str()))
}

fn link(kind: &str, a: &SearchArtifact) -> Link {
    Link {
        kind: kind.into(),
        name: a.name.clone().unwrap_or_else(|| a.address.clone()),
    }
}

fn links(
    t: &ServiceTopology,
    a: &SearchArtifact,
    names: &[&str],
    aliases: &[&SearchArtifact],
) -> Vec<Link> {
    let mut out: Vec<Link> = aliases.iter().map(|x| link("alias", x)).collect();
    let setting = |k: &str| a.detail.as_ref().and_then(|d| d.settings.get(k));
    for (key, kind) in [
        ("default_pipeline", "ingest_pipeline"),
        ("final_pipeline", "ingest_pipeline"),
        ("lifecycle.name", "ilm_policy"),
    ] {
        out.extend(setting(key).map(|n| Link {
            kind: kind.into(),
            name: n.clone(),
        }));
    }
    let name = names[0];
    for x in &t.elasticsearch.artifacts {
        let hit = match x.kind.as_str() {
            "index_template" => a.kind == "index" && search_artifact_covers(x, name),
            "transform" | "enrich_policy" | "snapshot_lifecycle" => refers_to(x, names),
            "data_view" | "alerting_rule" => names.iter().any(|n| search_artifact_covers(x, n)),
            "watch" => refers_to(x, names) || names.iter().any(|n| search_artifact_covers(x, n)),
            _ => false,
        };
        if hit {
            out.push(link(&x.kind, x));
        }
    }
    out
}

fn usages(t: &ServiceTopology, a: &SearchArtifact, aliases: &[&SearchArtifact]) -> Vec<UsageRow> {
    let projects = &t.elasticsearch.projects;
    t.elasticsearch
        .usages
        .iter()
        .filter(|u| {
            search_artifact_covers(a, &u.index)
                || aliases.iter().any(|x| search_artifact_covers(x, &u.index))
        })
        .map(|u| {
            let e = u.evidence.first();
            UsageRow {
                service: u.service.clone(),
                access: u.access.clone(),
                file: e.map(|e| e.file.clone()).unwrap_or_default(),
                line: e.and_then(|e| e.line),
                clickable: !projects.iter().any(|p| {
                    u.service
                        .strip_prefix(p.as_str())
                        .is_some_and(|r| r.starts_with(':'))
                }),
            }
        })
        .collect()
}
