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
    /// Comes from a project of a set other than the primary one.
    pub foreign: bool,
    /// Projects of a set the node comes from (an artifact's origins, a foreign service's project);
    /// empty outside a set and for the primary project's services.
    pub projects: Vec<String>,
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
    relabel(&mut b.nodes, &t.elasticsearch.artifacts);
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
        foreign: false,
        projects: Vec::new(),
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
            let at = self
                .t
                .services
                .iter()
                .position(|s| s.name == u.service || s.id == u.service);
            // The label is the name the services view shows; the id stays in the title.
            let label = at.map_or(u.service.as_str(), |i| self.t.services[i].name.as_str());
            // A set prefixes foreign services `<project>:`; the title says the project, the label stays short.
            let m = &self.t.elasticsearch;
            let project = u.service.split_once(':').map(|(p, _)| p).filter(|p| {
                m.primary().is_some_and(|x| x != *p) && m.projects.iter().any(|q| q == p)
            });
            let (label, title) = match project {
                // Not in this project's service list: the id is `<file>::<name>`, show the name.
                Some(p) => {
                    let rest = &label[p.len() + 1..];
                    (
                        rest.rsplit("::").next().unwrap_or(rest),
                        format!("{} · {p}", u.service),
                    )
                }
                None => (label, u.service.clone()),
            };
            let mut n = node("service", label, &title, "");
            n.foreign = project.is_some();
            n.projects = project.map(String::from).into_iter().collect();
            n.service = at;
            seen.insert(u.service.as_str(), self.nodes.len());
            self.nodes.push(n);
        }
        seen
    }

    fn add_artifacts(&mut self) {
        for (i, a) in self.t.elasticsearch.artifacts.iter().enumerate() {
            let mut title = a.name.as_deref().unwrap_or(&a.address).to_string();
            if !a.origins.is_empty() {
                title = format!("{title} · {}", a.origins.join(", "));
            }
            let mut n = node(&a.kind, &base_label(a), &title, &a.scope);
            n.foreign = self.t.elasticsearch.primary().is_some_and(|p| a.foreign(p));
            n.projects = a.origins.clone();
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
    match a.address.rfind("elasticstack_") {
        Some(i) => a.address[i..]
            .split_once('.')
            .map_or(&a.address[i..], |(_, r)| r)
            .into(),
        None => strip_modules(&a.address).into(),
    }
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

/// `s` cut to `max` characters, the last one `…` when something was cut.
fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max.saturating_sub(1)).chain(['…']).collect()
}

/// The shortest tail of `/`-segments that tells the scopes of one label apart.
fn scope_tails(scopes: &HashSet<String>) -> HashMap<String, String> {
    let depth = scopes
        .iter()
        .map(|s| s.split('/').count())
        .max()
        .unwrap_or(1);
    let tail = |s: &str, k: usize| {
        let parts: Vec<&str> = s.split('/').collect();
        parts[parts.len().saturating_sub(k)..].join("/")
    };
    let k = (1..=depth)
        .find(|&k| {
            scopes
                .iter()
                .map(|s| tail(s, k))
                .collect::<HashSet<_>>()
                .len()
                == scopes.len()
        })
        .unwrap_or(depth);
    scopes.iter().map(|s| (s.clone(), tail(s, k))).collect()
}

/// `module.a.module.b["k"]` of `module.a.module.b["k"].<type>.<name>`: the module instance, else empty.
fn instance_of(address: &str) -> String {
    let at = address.rfind("elasticstack_").unwrap_or(address.len());
    let head = &address[..at];
    let head = if at < address.len() {
        head
    } else {
        &address[..address.len() - strip_modules(address).len()]
    };
    head.trim_end_matches('.').into()
}

/// The first unused candidate: name kept at 16, 8, 4, 2 characters at least, each cut at the head,
/// then at the middle (two long names often differ at either end); `#<n>` is the last resort.
fn fit_label(used: &mut HashSet<String>, name: &str, parts: &[(&str, &str, String)]) -> String {
    let mut label = String::new();
    for floor in [16, 8, 4, 2] {
        let suffix = suffix(name.chars().count(), floor, parts);
        let room = MAX_LABEL - suffix.chars().count();
        for cut in [shorten(name, room), shorten_scope(name, room)] {
            label = format!("{cut}{suffix}");
            if !used.contains(&label) {
                used.insert(label.clone());
                return label;
            }
        }
    }
    unique(used, label)
}

/// `module.a` stays; nested `module.a.module.b` reads `a.b`.
fn display_instance(inst: &str) -> String {
    if inst.matches("module.").count() > 1 {
        inst.replace("module.", "")
    } else {
        inst.into()
    }
}

/// `<type>.<name>[key]` of the Terraform address, the last thing that tells two resources apart.
fn resource_of(n: &SearchNode, artifacts: &[SearchArtifact]) -> String {
    let Some(i) = n.artifact else {
        return String::new();
    };
    let a = &artifacts[i].address;
    a[a.rfind("elasticstack_")
        .map_or(0, |i| i + "elasticstack_".len())..]
        .into()
}

/// The name part of ` (<folder tail>)`, ` · <instance>` and ` [<kind>]`, shrunk (longest first)
/// until the name keeps `min(name, floor)` of the 32 characters.
fn suffix(name_len: usize, floor: usize, parts: &[(&str, &str, String)]) -> String {
    let budget = MAX_LABEL - name_len.min(floor);
    let mut lens: Vec<usize> = parts.iter().map(|p| p.2.chars().count()).collect();
    let overhead: usize = parts
        .iter()
        .map(|p| p.0.chars().count() + p.1.chars().count())
        .sum();
    while lens.iter().sum::<usize>() + overhead > budget {
        match lens.iter_mut().filter(|l| **l > 4).max() {
            Some(l) => *l -= 1,
            None => break,
        }
    }
    let cut = |(o, c, t): &(&str, &str, String), n| format!("{o}{}{c}", shorten_scope(t, n));
    parts.iter().zip(lens).map(|(p, n)| cut(p, n)).collect()
}

/// Labels unique per graph and within `MAX_LABEL`: equal labels of different roots get
/// ` (<folder tail>)`, of different module instances in one root ` · <instance>`, of different
/// kinds in one instance ` [<kind>]` (the name is cut, never the suffix); `#<n>` is the last resort.
fn relabel(nodes: &mut [SearchNode], artifacts: &[SearchArtifact]) {
    let scope_of = |n: &SearchNode| {
        n.scope
            .strip_prefix("terraform:")
            .unwrap_or(&n.scope)
            .to_string()
    };
    let instance = |n: &SearchNode| {
        n.artifact
            .map_or(String::new(), |i| instance_of(&artifacts[i].address))
    };
    let mut scopes: HashMap<String, HashSet<String>> = HashMap::new();
    let mut by_scope: HashMap<(String, String), HashSet<String>> = HashMap::new();
    let mut by_inst: HashMap<(String, String, String), HashSet<String>> = HashMap::new();
    let mut by_kind: HashMap<(String, String, String, String), HashSet<String>> = HashMap::new();
    for n in nodes.iter() {
        let (label, scope, inst) = (n.label.clone(), scope_of(n), instance(n));
        scopes
            .entry(label.clone())
            .or_default()
            .insert(scope.clone());
        by_scope
            .entry((label.clone(), scope.clone()))
            .or_default()
            .insert(inst.clone());
        by_inst
            .entry((label.clone(), scope.clone(), inst.clone()))
            .or_default()
            .insert(n.kind.clone());
        by_kind
            .entry((label, scope, inst, n.kind.clone()))
            .or_default()
            .insert(resource_of(n, artifacts));
    }
    let tails: HashMap<_, _> = scopes
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(k, v)| (k.clone(), scope_tails(v)))
        .collect();
    let mut used = HashSet::new();
    for n in nodes.iter_mut() {
        let (scope, inst) = (scope_of(n), instance(n));
        let mut parts = Vec::new();
        if let Some(t) = tails.get(&n.label).filter(|_| !scope.is_empty()) {
            parts.push((" (", ")", t[&scope].clone()));
        }
        if !inst.is_empty() && by_scope[&(n.label.clone(), scope.clone())].len() > 1 {
            parts.push((" · ", "", display_instance(&inst)));
        }
        let key = (n.label.clone(), scope, inst);
        if by_inst[&key].len() > 1 {
            parts.push((" [", "]", n.kind.clone()));
        }
        if by_kind[&(key.0, key.1, key.2, n.kind.clone())].len() > 1 {
            parts.push((" {", "}", resource_of(n, artifacts)));
        }
        n.label = fit_label(&mut used, &n.label, &parts);
    }
}

/// At most `max` characters, both ends kept (what tells paths apart can sit at either end).
fn shorten_scope(s: &str, max: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= max {
        return s.into();
    }
    let tail = (max - 1) / 2;
    let head = max - 1 - tail;
    c[..head]
        .iter()
        .chain(['…'].iter())
        .chain(&c[c.len() - tail..])
        .collect()
}

fn unique(used: &mut HashSet<String>, label: String) -> String {
    let mut out = label.clone();
    let mut k = 2;
    while !used.insert(out.clone()) {
        let tag = format!(" #{k}");
        out = format!("{}{tag}", shorten(&label, MAX_LABEL - tag.chars().count()));
        k += 1;
    }
    out
}
