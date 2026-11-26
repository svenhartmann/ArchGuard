//! Diagram graph (boxes, edges, neighborhood, cluster), ticket 04.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use archguard_engine::core::{SearchArtifact, SearchIndexDetail, ServiceTopology};
use archguard_engine::{
    SearchAnalysisPart, SearchEdge, SearchGraph, SearchNode, search_analysis_parts,
};

use super::types::{Key, Switches};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BoxKind {
    Service,
    Index,
    Template,
    Alias,
    /// Every column-0 kind except services (pipeline, ILM, transform, watch, ...).
    Processing,
    Analyzer,
    Normalizer,
    Tokenizer,
    Filter,
    CharFilter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DBox {
    pub key: Key,
    pub kind: BoxKind,
    pub ghost: bool,
    /// `SearchNode.warn`: engine findings only.
    pub warn: bool,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeStyle {
    Write,
    Read,
    Unknown,
    Uses,
    Alias,
    Covers,
    Pipeline,
    Lifecycle,
    Reference,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DEdge {
    pub from: Key,
    pub to: Key,
    pub style: EdgeStyle,
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagram {
    pub boxes: Vec<DBox>,
    pub edges: Vec<DEdge>,
}

/// Search the graph for the neighbors, then lay out boxes and edges; a selected `Key` out of
/// range gives an empty diagram.
pub fn diagram(
    t: &ServiceTopology,
    g: &SearchGraph,
    selected: &Key,
    cluster: Option<&[Key]>,
    sw: &Switches,
) -> Diagram {
    let cx = Cx::new(t, g, sw);
    let (mut nodes, parts) = match cluster {
        Some(keys) => cx.cluster_set(keys),
        None => cx.neighborhood(selected),
    };
    nodes.retain(|&i| sw.services || g.nodes[i].kind != "service");
    cx.assemble(&nodes, &parts, selected)
}

struct Cx<'a> {
    t: &'a ServiceTopology,
    g: &'a SearchGraph,
    /// Declared analysis parts per graph node (empty with the analysis switch off).
    parts: Vec<Vec<SearchAnalysisPart>>,
}

type Edge = (Key, Key, EdgeStyle, String);

fn part_key(owner: usize, kind: &str, name: &str) -> Key {
    Key::Part {
        owner,
        kind: kind.into(),
        name: name.into(),
    }
}

fn part_box_kind(kind: &str) -> BoxKind {
    match kind {
        "analyzer" => BoxKind::Analyzer,
        "normalizer" => BoxKind::Normalizer,
        "tokenizer" => BoxKind::Tokenizer,
        "char_filter" => BoxKind::CharFilter,
        _ => BoxKind::Filter,
    }
}

fn node_box_kind(kind: &str) -> BoxKind {
    match kind {
        "service" => BoxKind::Service,
        "index" => BoxKind::Index,
        "alias" => BoxKind::Alias,
        "index_template" | "component_template" => BoxKind::Template,
        _ => BoxKind::Processing,
    }
}

/// The declared chain members of a part; built-in, undeclared and inline ones make no box.
fn used(p: &SearchAnalysisPart) -> Vec<(&'static str, &str)> {
    let tok = p.tokenizer.iter().map(|c| ("tokenizer", c));
    let filters = p.filters.iter().map(|c| ("filter", c));
    let chars = p.char_filters.iter().map(|c| ("char_filter", c));
    tok.chain(filters)
        .chain(chars)
        .filter(|(_, c)| c.class == "declared")
        .map(|(stage, c)| (stage, c.name.as_str()))
        .collect()
}

impl<'a> Cx<'a> {
    fn new(t: &'a ServiceTopology, g: &'a SearchGraph, sw: &Switches) -> Self {
        let parts = g
            .nodes
            .iter()
            .map(|n| match (sw.analysis, Self::detail_of(t, n)) {
                (true, Some(d)) => search_analysis_parts(d),
                _ => Vec::new(),
            })
            .collect();
        Self { t, g, parts }
    }

    fn artifact(&self, n: &SearchNode) -> Option<&'a SearchArtifact> {
        n.artifact
            .and_then(|i| self.t.elasticsearch.artifacts.get(i))
    }

    fn detail_of(t: &'a ServiceTopology, n: &SearchNode) -> Option<&'a SearchIndexDetail> {
        n.artifact
            .and_then(|i| t.elasticsearch.artifacts.get(i))
            .and_then(|a| a.detail.as_ref())
    }

    fn find_part(&self, owner: usize, kind: &str, name: &str) -> Option<&SearchAnalysisPart> {
        let ps = self.parts.get(owner)?;
        ps.iter().find(|p| p.kind == kind && p.name == name)
    }

    fn all_parts(&self, owner: usize) -> impl Iterator<Item = Key> + '_ {
        let ps = self.parts.get(owner).into_iter().flatten();
        ps.map(move |p| part_key(owner, &p.kind, &p.name))
    }

    fn adjacent(&self, i: usize) -> Vec<usize> {
        let ends = |e: &SearchEdge| match (e.from == i, e.to == i) {
            (true, _) => Some(e.to),
            (_, true) => Some(e.from),
            _ => None,
        };
        self.g.edges.iter().filter_map(ends).collect()
    }

    /// The selection, its direct neighbors and the services behind neighboring aliases.
    fn neighbors(&self, i: usize) -> BTreeSet<usize> {
        let mut set: BTreeSet<usize> = self.adjacent(i).into_iter().collect();
        let is = |n: usize, kind: &str| self.g.nodes[n].kind == kind;
        let behind: Vec<usize> = set
            .iter()
            .filter(|&&n| is(n, "alias"))
            .flat_map(|&a| self.adjacent(a))
            .filter(|&n| is(n, "service"))
            .collect();
        set.extend(behind);
        set.insert(i);
        set
    }

    fn neighborhood(&self, sel: &Key) -> (BTreeSet<usize>, BTreeSet<Key>) {
        match sel {
            Key::Node(i) if *i < self.g.nodes.len() => {
                (self.neighbors(*i), self.all_parts(*i).collect())
            }
            Key::Part { owner, kind, name } => {
                let parts = self.related(*owner, kind, name);
                (BTreeSet::from([*owner]), parts)
            }
            Key::Node(_) => Default::default(),
        }
    }

    /// The part, the parts it uses and the parts that use it, all of one owner.
    fn related(&self, owner: usize, kind: &str, name: &str) -> BTreeSet<Key> {
        let Some(sel) = self.find_part(owner, kind, name) else {
            return BTreeSet::new();
        };
        let mut out = BTreeSet::from([part_key(owner, kind, name)]);
        out.extend(used(sel).into_iter().map(|(k, n)| part_key(owner, k, n)));
        for p in &self.parts[owner] {
            if used(p).contains(&(kind, name)) {
                out.insert(part_key(owner, &p.kind, &p.name));
            }
        }
        out
    }

    fn cluster_set(&self, keys: &[Key]) -> (BTreeSet<usize>, BTreeSet<Key>) {
        let nodes: BTreeSet<usize> = keys
            .iter()
            .filter_map(|k| match k {
                Key::Node(i) if *i < self.g.nodes.len() => Some(*i),
                _ => None,
            })
            .collect();
        let mut parts: BTreeSet<Key> = nodes.iter().flat_map(|&i| self.all_parts(i)).collect();
        let exists = |k: &&Key| match k {
            Key::Part { owner, kind, name } => self.find_part(*owner, kind, name).is_some(),
            Key::Node(_) => false,
        };
        parts.extend(keys.iter().filter(exists).cloned());
        (nodes, parts)
    }

    /// Part keys of equal (kind, name, definition) map to the first of them.
    fn canon(&self, parts: &BTreeSet<Key>) -> BTreeMap<Key, Key> {
        let mut first: HashMap<(&str, &str, &str), &Key> = HashMap::new();
        let mut map = BTreeMap::new();
        for k in parts {
            let Key::Part { owner, kind, name } = k else {
                continue;
            };
            let Some(p) = self.find_part(*owner, kind, name) else {
                continue;
            };
            let id = (kind.as_str(), name.as_str(), p.definition.as_str());
            map.insert(k.clone(), (*first.entry(id).or_insert(k)).clone());
        }
        map
    }

    fn assemble(&self, nodes: &BTreeSet<usize>, parts: &BTreeSet<Key>, sel: &Key) -> Diagram {
        let canon = self.canon(parts);
        let mut boxes: Vec<DBox> = nodes.iter().map(|&i| self.node_box(i, sel)).collect();
        let unique: BTreeSet<&Key> = canon.values().collect();
        boxes.extend(unique.into_iter().map(|k| part_box(k, sel)));
        boxes.sort_by_key(|b| !b.selected);
        let mut edges = self.graph_edges(nodes);
        edges.extend(self.part_edges(nodes, &canon));
        let edges: BTreeSet<Edge> = edges.into_iter().collect();
        let edges = edges
            .into_iter()
            .map(|(from, to, style, label)| DEdge {
                from,
                to,
                style,
                label,
            })
            .collect();
        Diagram { boxes, edges }
    }

    fn node_box(&self, i: usize, sel: &Key) -> DBox {
        let n = &self.g.nodes[i];
        DBox {
            key: Key::Node(i),
            kind: node_box_kind(&n.kind),
            ghost: !n.declared,
            warn: n.warn,
            selected: Key::Node(i) == *sel,
        }
    }

    /// `index -> pipeline` for the `default_pipeline`/`final_pipeline` settings naming a declared
    /// pipeline.
    fn pipeline_edges(&self, nodes: &BTreeSet<usize>) -> Vec<(usize, usize, &'static str)> {
        let mut out = Vec::new();
        for &i in nodes.iter().filter(|&&i| self.g.nodes[i].kind == "index") {
            let Some(d) = Self::detail_of(self.t, &self.g.nodes[i]) else {
                continue;
            };
            for key in ["default_pipeline", "final_pipeline"] {
                let Some(name) = d.settings.get(key) else {
                    continue;
                };
                let hit = |j: &usize| {
                    let n = &self.g.nodes[*j];
                    let a = self.artifact(n);
                    n.declared
                        && n.kind == "ingest_pipeline"
                        && a.is_some_and(|a| a.name.as_deref() == Some(name))
                };
                out.extend(nodes.iter().copied().filter(hit).map(|j| (i, j, key)));
            }
        }
        out
    }

    fn graph_edges(&self, nodes: &BTreeSet<usize>) -> Vec<Edge> {
        let pipes = self.pipeline_edges(nodes);
        let mut out: Vec<Edge> = pipes
            .iter()
            .map(|&(i, j, key)| (Key::Node(i), Key::Node(j), EdgeStyle::Pipeline, key.into()))
            .collect();
        let inside = |e: &&SearchEdge| nodes.contains(&e.from) && nodes.contains(&e.to);
        for e in self.g.edges.iter().filter(inside) {
            let (a, b) = (&self.g.nodes[e.from], &self.g.nodes[e.to]);
            let (from, to) = (Key::Node(e.from), Key::Node(e.to));
            let edge = match e.relation {
                "write" => (from, to, EdgeStyle::Write, ""),
                "unknown" => (from, to, EdgeStyle::Unknown, ""),
                "read" => (to, from, EdgeStyle::Read, ""),
                "covers" => (from, to, EdgeStyle::Covers, "«covers»"),
                _ if a.kind == "alias" => (from, to, EdgeStyle::Alias, ""),
                _ if b.kind == "ilm_policy" => (from, to, EdgeStyle::Lifecycle, "lifecycle"),
                _ if pipes.iter().any(|p| (p.0, p.1) == (e.from, e.to)) => continue,
                _ => (from, to, EdgeStyle::Reference, ""),
            };
            out.push((edge.0, edge.1, edge.2, edge.3.into()));
        }
        out
    }

    /// `uses`: owner -> analyzer/normalizer named by a field or `default`; part -> declared chain.
    fn part_edges(&self, nodes: &BTreeSet<usize>, canon: &BTreeMap<Key, Key>) -> Vec<Edge> {
        let mut out = Vec::new();
        let mut uses = |from: &Key, to: Key| {
            if let (Some(a), Some(b)) = (canon.get(from).or(Some(from)), canon.get(&to)) {
                out.push((a.clone(), b.clone(), EdgeStyle::Uses, String::new()));
            }
        };
        for &i in nodes {
            let fields =
                Self::detail_of(self.t, &self.g.nodes[i]).map_or(&[][..], |d| &d.fields[..]);
            let analyzers = fields
                .iter()
                .flat_map(|f| [&f.analyzer, &f.search_analyzer]);
            let named = analyzers.flatten().map(|n| ("analyzer", n.as_str()));
            let norms = fields
                .iter()
                .filter_map(|f| f.normalizer.as_deref())
                .map(|n| ("normalizer", n));
            for (kind, name) in named.chain(norms).chain([("analyzer", "default")]) {
                uses(&Key::Node(i), part_key(i, kind, name));
            }
        }
        for k in canon.keys() {
            let Key::Part { owner, kind, name } = k else {
                continue;
            };
            let Some(p) = self.find_part(*owner, kind, name) else {
                continue;
            };
            for (stage, member) in used(p) {
                uses(k, part_key(*owner, stage, member));
            }
        }
        out
    }
}

fn part_box(k: &Key, sel: &Key) -> DBox {
    let kind = match k {
        Key::Part { kind, .. } => part_box_kind(kind),
        Key::Node(_) => BoxKind::Filter,
    };
    DBox {
        key: k.clone(),
        kind,
        ghost: false,
        warn: false,
        selected: k == sel,
    }
}
