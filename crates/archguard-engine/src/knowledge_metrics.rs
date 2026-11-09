//! Contribution concentration in a bounded Git history window, not actual expertise.
use crate::config::validate_review_since;
use crate::pipeline::NamedGraph;
use crate::review_sources::{
    MAX_COMMITS, MAX_LOG_BYTES, collect_git_history, collect_source_evidence,
};
use archguard_core::{ArchguardError, Config, GitStatus};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contributor {
    pub author: String,
    pub touches: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeMetric {
    pub node: String,
    pub touches: u64,
    pub contributors: usize,
    pub top_share: Option<f64>,
    pub bus_factor_50: Option<usize>,
    pub authors: Vec<Contributor>,
    pub status: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeReport {
    pub definition: String,
    pub status: String,
    pub since: String,
    pub commits: usize,
    pub files: Vec<KnowledgeMetric>,
    pub packages: Vec<KnowledgeMetric>,
    pub diagnostics: Vec<String>,
}
fn row(node: String, touches: BTreeMap<String, u64>, complete: bool) -> KnowledgeMetric {
    let total = touches.values().sum();
    let mut authors: Vec<_> = touches
        .into_iter()
        .map(|(author, touches)| Contributor { author, touches })
        .collect();
    authors.sort_by(|a, b| {
        b.touches
            .cmp(&a.touches)
            .then_with(|| a.author.cmp(&b.author))
    });
    let mut cumulative = 0;
    let mut factor = 0;
    for a in &authors {
        cumulative += a.touches;
        factor += 1;
        if cumulative * 2 >= total {
            break;
        }
    }
    KnowledgeMetric {
        node,
        touches: total,
        contributors: authors.len(),
        top_share: (total > 0).then(|| authors[0].touches as f64 / total as f64),
        bus_factor_50: (complete && total > 0).then_some(factor),
        authors,
        status: if !complete {
            "partial"
        } else if total == 0 {
            "unavailable"
        } else {
            "ready"
        }
        .into(),
    }
}
/// Pass `(graph, names)` from the run that built `graph` to skip re-extraction.
pub fn knowledge_metrics<'a>(
    root: &Path,
    config: &Config,
    graph: impl Into<NamedGraph<'a>>,
) -> Result<KnowledgeReport, ArchguardError> {
    let graph = graph.into();
    validate_review_since(&config.review.git_since)?;
    let history = collect_git_history(root, &config.review.git_since, MAX_COMMITS, MAX_LOG_BYTES);
    let complete = history.evidence.status == GitStatus::Available
        && !history.evidence.shallow
        && !history.evidence.truncated
        && history.commits.iter().all(|c| !c.author.trim().is_empty());
    let nodes: BTreeMap<_, _> = graph.graph.nodes.iter().map(|n| (n.id, n)).collect();
    let mut file_touches = BTreeMap::<String, BTreeMap<String, u64>>::new();
    let mut package_touches = BTreeMap::<String, BTreeMap<String, u64>>::new();
    let mut paths = BTreeMap::new();
    for source in collect_source_evidence(root, config, graph)? {
        if let Some(node) = nodes.get(&source.node_id) {
            file_touches.entry(node.name.clone()).or_default();
            if let Some(path) = source.path {
                paths.insert(path, node.name.clone());
            }
        }
    }
    for commit in &history.commits {
        if commit.author.trim().is_empty() {
            continue;
        }
        for path in &commit.paths {
            if let Some(name) = paths.get(path) {
                *file_touches
                    .entry(name.clone())
                    .or_default()
                    .entry(commit.author.clone())
                    .or_default() += 1;
                let package = name.rsplit_once('/').map(|(p, _)| p).unwrap_or(".");
                *package_touches
                    .entry(package.into())
                    .or_default()
                    .entry(commit.author.clone())
                    .or_default() += 1;
            }
        }
    }
    Ok(KnowledgeReport{definition:"git-touch-concentration/v1".into(),status:if history.evidence.status!=GitStatus::Available{"unavailable"}else if complete{"ready"}else{"partial"}.into(),since:config.review.git_since.clone(),commits:history.commits.len(),files:file_touches.into_iter().map(|(n,t)|row(n,t,complete)).collect(),packages:package_touches.into_iter().map(|(n,t)|row(n,t,complete)).collect(),diagnostics:vec!["Bus factor 50: smallest number of authors accounting for at least 50% of file/commit touches in the configured window. Mailmap-normalized author emails identify contributors. This is contribution concentration, not actual expertise or staff availability. Merge commits and rename history are excluded; package counts sum file touches. Shallow/truncated history has no bus-factor score; untouched files remain unknown.".into(),history.evidence.message.unwrap_or_default()]})
}
