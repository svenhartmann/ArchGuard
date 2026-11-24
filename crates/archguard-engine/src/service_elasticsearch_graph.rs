//! The Elasticsearch graph (ticket 20): one view-neutral model of services, declared artifacts
//! and ghost nodes for what is used or referenced but not declared. Pure over the topology.
use super::service_elasticsearch_findings::{index_parts, search_artifact_covers};
use archguard_core::{SearchArtifact, ServiceTopology};
use std::collections::{BTreeSet, HashMap, HashSet};

const MAX_LABEL: usize = 32;

/// `kind` is `service` or the artifact kind; `artifact` indexes `elasticsearch.artifacts`,
/// `service` indexes `services`; ghosts have neither and `declared = false`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchNode {
    pub kind: String,
    pub label: String,
    pub title: String,
    pub scope: String,
    pub artifact: Option<usize>,
    pub service: Option<usize>,
    pub warn: bool,
    pub declared: bool,
}

/// `relation` is `write`, `read`, `unknown`, `reference` or `covers`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchEdge {
    pub from: usize,
    pub to: usize,
    pub relation: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchGraph {
    pub nodes: Vec<SearchNode>,
    pub edges: Vec<SearchEdge>,
}

struct Build<'a> {
    t: &'a ServiceTopology,
    /// Node index of artifact 0: services come first.
    base: usize,
    nodes: Vec<SearchNode>,
    edges: BTreeSet<(usize, usize, &'static str)>,
    /// (kind, name) -> artifact nodes, so resolving a name is one lookup, not a scan.
    by_name: HashMap<(&'a str, &'a str), Vec<usize>>,
    ghosts: HashMap<(String, String), usize>,
}

pub fn search_graph(t: &ServiceTopology) -> SearchGraph {
    let mut b = Build {
        t,
        base: 0,
        nodes: Vec::new(),
        edges: BTreeSet::new(),
        by_name: HashMap::new(),
        ghosts: HashMap::new(),
    };
    let services = b.add_services();
    b.base = b.nodes.len();
    b.add_artifacts();
    b.usage_edges(&services);
    b.reference_edges();
    b.cover_edges();
    b.mark_warnings();
    relabel(&mut b.nodes);
    SearchGraph {
        nodes: b.nodes,
        edges: b
            .edges
            .into_iter()
            .map(|(from, to, relation)| SearchEdge { from, to, relation })
            .collect(),
    }
}

fn node(kind: &str, label: &str, title: &str, scope: &str) -> SearchNode {
    SearchNode {
        kind: kind.into(),
        label: label.into(),
        title: title.into(),
        scope: scope.into(),
        artifact: None,
        service: None,
        warn: false,
        declared: true,
    }
}

impl<'a> Build<'a> {
    /// A node per service named in a usage, in first-use order; returns name -> node.
    fn add_services(&mut self) -> HashMap<&'a str, usize> {
        let mut seen = HashMap::new();
        for u in &self.t.elasticsearch.usages {
            if seen.contains_key(u.service.as_str()) {
                continue;
            }
            let mut n = node("service", &u.service, &u.service, "");
            n.service = self
                .t
                .services
                .iter()
                .position(|s| s.name == u.service || s.id == u.service);
            seen.insert(u.service.as_str(), self.nodes.len());
            self.nodes.push(n);
        }
        seen
    }

    fn add_artifacts(&mut self) {
        for (i, a) in self.t.elasticsearch.artifacts.iter().enumerate() {
            let title = a.name.as_deref().unwrap_or(&a.address);
            let mut n = node(&a.kind, &base_label(a), title, &a.scope);
            n.artifact = Some(i);
            if let Some(name) = a.name.as_deref() {
                self.by_name
                    .entry((a.kind.as_str(), name))
                    .or_default()
                    .push(self.nodes.len());
            }
            self.nodes.push(n);
        }
    }

    /// One ghost per (kind, name), shared by every use and reference of that name.
    fn ghost(&mut self, kind: &str, name: &str) -> usize {
        let key = (kind.to_string(), name.to_string());
        if let Some(&i) = self.ghosts.get(&key) {
            return i;
        }
        let mut n = node(kind, name, name, "");
        n.warn = true;
        n.declared = false;
        self.nodes.push(n);
        self.ghosts.insert(key, self.nodes.len() - 1);
        self.nodes.len() - 1
    }

    fn artifact_of(&self, node: usize) -> &'a SearchArtifact {
        &self.t.elasticsearch.artifacts[node - self.base]
    }

    /// Declared indices and aliases that cover one name of an index list, or one ghost.
    fn index_targets(&mut self, part: &str) -> Vec<usize> {
        let found: Vec<usize> = ["index", "alias"]
            .iter()
            .filter_map(|k| self.by_name.get(&(*k, part)))
            .flatten()
            .copied()
            .filter(|&n| search_artifact_covers(self.artifact_of(n), part))
            .collect();
        if found.is_empty() {
            vec![self.ghost("index", part)]
        } else {
            found
        }
    }

    fn usage_edges(&mut self, services: &HashMap<&'a str, usize>) {
        for u in &self.t.elasticsearch.usages {
            let s = services[u.service.as_str()];
            for part in index_parts(&u.index) {
                for ix in self.index_targets(part) {
                    let edge = match u.access.as_str() {
                        "write" => (s, ix, "write"),
                        "read" => (ix, s, "read"),
                        _ => (s, ix, "unknown"),
                    };
                    self.edges.insert(edge);
                }
            }
        }
    }

    fn reference_edges(&mut self) {
        let base = self
            .nodes
            .iter()
            .take_while(|n| n.artifact.is_none())
            .count();
        for (i, a) in self.t.elasticsearch.artifacts.iter().enumerate() {
            for r in &a.references {
                let hits = self.by_name.get(&(r.kind.as_str(), r.name.as_str()));
                let to = hits
                    .cloned()
                    .unwrap_or_else(|| vec![self.ghost(&r.kind, &r.name)]);
                self.edges
                    .extend(to.into_iter().map(|t| (base + i, t, "reference")));
            }
        }
    }

    /// Pattern holders against every declared index and alias. shortcut: patterns have no name
    /// index, so this is patterns x indices; upgrade when corpora reach thousands of indices.
    fn cover_edges(&mut self) {
        let base = self
            .nodes
            .iter()
            .take_while(|n| n.artifact.is_none())
            .count();
        let arts = &self.t.elasticsearch.artifacts;
        let targets: Vec<(usize, &str)> = arts
            .iter()
            .enumerate()
            .filter(|(_, a)| matches!(a.kind.as_str(), "index" | "alias"))
            .filter_map(|(i, a)| Some((base + i, a.name.as_deref()?)))
            .collect();
        for (i, a) in arts
            .iter()
            .enumerate()
            .filter(|(_, a)| !a.patterns.is_empty())
        {
            for &(to, name) in &targets {
                if search_artifact_covers(a, name) {
                    self.edges.insert((base + i, to, "covers"));
                }
            }
        }
    }

    /// The subject rule of a finding: name, pattern or referenced name (as the desktop list).
    fn mark_warnings(&mut self) {
        let subjects: HashSet<&str> = self
            .t
            .elasticsearch
            .findings
            .iter()
            .map(|f| f.subject.as_str())
            .collect();
        for n in &mut self.nodes {
            let Some(i) = n.artifact else { continue };
            let a = &self.t.elasticsearch.artifacts[i];
            let names = a
                .name
                .iter()
                .chain(&a.patterns)
                .chain(a.references.iter().map(|r| &r.name));
            n.warn = names.into_iter().any(|x| subjects.contains(x.as_str()));
        }
    }
}

/// The name, else the resource name of the Terraform address: after the last
/// `elasticstack_*.` type, without `module.<m>.` prefixes, `[key]` kept.
fn base_label(a: &SearchArtifact) -> String {
    if let Some(name) = &a.name {
        return name.clone();
    }
    let (head, key) = a
        .address
        .split_at(a.address.find('[').unwrap_or(a.address.len()));
    let resource = match head.rfind("elasticstack_") {
        Some(i) => head[i..].split_once('.').map_or(head, |(_, r)| r),
        None => strip_modules(head),
    };
    format!("{resource}{key}")
}

fn strip_modules(mut s: &str) -> &str {
    while let Some(rest) = s.strip_prefix("module.") {
        match rest.split_once('.') {
            Some((_, r)) => s = r,
            None => break,
        }
    }
    s
}

/// Equal labels of different scopes get ` (<scope>)`, then labels are cut to `MAX_LABEL`.
fn relabel(nodes: &mut [SearchNode]) {
    let mut scopes: HashMap<String, HashSet<String>> = HashMap::new();
    for n in nodes.iter() {
        scopes
            .entry(n.label.clone())
            .or_default()
            .insert(n.scope.clone());
    }
    for n in nodes.iter_mut() {
        if scopes[&n.label].len() > 1 && !n.scope.is_empty() {
            n.label = format!("{} ({})", n.label, n.scope);
        }
        if n.label.chars().count() > MAX_LABEL {
            n.label = n.label.chars().take(MAX_LABEL - 1).chain(['…']).collect();
        }
    }
}
