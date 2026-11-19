//! arc42 chapter 5: which packages become building blocks, what they hold,
//! which layer and project they belong to and how they use each other.
//! The Markdown table around it stays in `arc42.rs`.

use archguard_core::{
    CallAnalysis, Config, DocLang, GoalAssessment, Graph, Node, NodeId, NodeKind, Report,
    ServiceProject, ServiceTopology,
};
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::BTreeMap;

/// What a building block is for, gathered by the engine from files the
/// report crate cannot read or match itself.
#[derive(Debug, Clone, Default)]
pub struct Arc42Blocks {
    /// Source-graph node -> index into `config.layers` (first matching
    /// declared layer).
    pub layers: BTreeMap<NodeId, usize>,
    /// The same with derived layers too, for the layer diagram.
    pub all_layers: BTreeMap<NodeId, usize>,
    /// What the source starts and addresses, for the context diagram.
    pub context: Arc42Context,
    /// Manifest project id -> its declared description.
    pub descriptions: BTreeMap<String, String>,
    /// The project's recorded runtime (ADR 0004), for chapter 6.
    pub runtime: Option<archguard_core::RuntimeProfile>,
}

/// Programs the source starts and hosts it addresses over HTTP, with the
/// number of files each appears in (gathered by the engine).
#[derive(Debug, Clone, Default)]
pub struct Arc42Context {
    pub programs: Vec<(String, usize)>,
    pub hosts: Vec<(String, usize)>,
}

/// Everything the arc42 chapters are written from.
pub struct Arc42Inputs<'a> {
    /// Original containment tree and source edges, before view aggregation.
    pub source_graph: &'a Graph,
    pub view: &'a Graph,
    pub report: &'a Report,
    pub config: &'a Config,
    pub calls: &'a CallAnalysis,
    pub services: &'a ServiceTopology,
    /// The same normalized evidence models that the desktop and MCP read.
    pub insights: &'a Value,
    /// Logical compiler paths -> unambiguous physical project-relative paths.
    pub source_paths: &'a BTreeMap<String, String>,
    pub blocks: &'a Arc42Blocks,
    /// One assessment per declared goal (ADR 0003).
    pub goals: &'a [GoalAssessment],
    /// The language of the generated text (ADR 0008).
    pub lang: DocLang,
}

type Children<'a> = BTreeMap<Option<NodeId>, Vec<&'a Node>>;

/// Building blocks in reading order (`order`); every vector is indexed by
/// block, and "Files without a package" is the last block when present.
pub(crate) struct Blocks<'a> {
    /// The package node of each real block.
    frontier: Vec<&'a Node>,
    pub labels: Vec<String>,
    pub files: Vec<usize>,
    layer_names: Vec<Option<&'a str>>,
    descriptions: Vec<Option<&'a str>>,
    /// The manifest project id owning each block.
    projects: Vec<Option<&'a str>>,
    pub order: Vec<usize>,
    /// Source-graph node -> block.
    owner: BTreeMap<NodeId, usize>,
    uses: BTreeMap<(usize, usize), usize>,
    /// Source connections whose both ends lie in blocks.
    pub covered: usize,
    /// Of those, the ones inside one block.
    pub internal: usize,
}

struct Groups<'a> {
    owner: BTreeMap<NodeId, usize>,
    files: Vec<usize>,
    sample: Vec<Option<&'a str>>,
}

impl<'a> Blocks<'a> {
    pub fn new(
        graph: &'a Graph,
        config: &'a Config,
        services: &'a ServiceTopology,
        source_paths: &BTreeMap<String, String>,
        facts: &'a Arc42Blocks,
        lang: DocLang,
    ) -> Self {
        let frontier = frontier(graph);
        let mut names: Vec<String> = frontier.iter().map(|n| n.name.clone()).collect();
        if graph.nodes.iter().any(loose_file) {
            names.push(
                lang.pick("Files without a package", "Dateien ohne Paket")
                    .into(),
            );
        }
        let groups = group(graph, &frontier, names.len());
        let owners = owners(&groups.sample, frontier.len(), source_paths, services);
        let labels = labels(&names, &owners);
        let layers: Vec<Option<usize>> = (0..names.len())
            .map(|i| {
                frontier
                    .get(i)
                    .and_then(|n| facts.layers.get(&n.id))
                    .copied()
            })
            .collect();
        let order = order(&layers, &groups.files, &labels);
        let mut blocks = Blocks {
            layer_names: layers
                .iter()
                .map(|l| {
                    l.and_then(|l| config.layers.get(l))
                        .map(|l| l.name.as_str())
                })
                .collect(),
            projects: owners.iter().map(|o| o.map(|p| p.id.as_str())).collect(),
            descriptions: owners
                .iter()
                .map(|o| {
                    o.and_then(|p| facts.descriptions.get(&p.id))
                        .map(String::as_str)
                })
                .collect(),
            frontier,
            labels,
            files: groups.files,
            order,
            owner: BTreeMap::new(),
            uses: BTreeMap::new(),
            covered: 0,
            internal: 0,
        };
        blocks.count_edges(graph, &groups.owner);
        blocks.owner = groups.owner;
        blocks
    }

    pub fn describe(&self, i: usize) -> Option<&'a str> {
        self.descriptions[i]
    }

    /// The manifest project id owning block `i`.
    pub fn project(&self, i: usize) -> Option<&'a str> {
        self.projects.get(i).copied().flatten()
    }

    pub fn layer_name(&self, i: usize) -> Option<&'a str> {
        self.layer_names[i]
    }

    fn count_edges(&mut self, graph: &Graph, owner: &BTreeMap<NodeId, usize>) {
        for edge in &graph.edges {
            let (Some(&a), Some(&b)) = (owner.get(&edge.from), owner.get(&edge.to)) else {
                continue;
            };
            self.covered += 1;
            if a == b {
                self.internal += 1;
            } else {
                *self.uses.entry((a, b)).or_default() += 1;
            }
        }
    }

    /// Labels of the blocks `i` uses, the most used first.
    pub fn used(&self, i: usize) -> Vec<&str> {
        let mut used: Vec<_> = self
            .uses
            .iter()
            .filter(|((a, _), _)| *a == i)
            .map(|((_, b), n)| (*n, *b))
            .collect();
        used.sort_by_key(|&(n, b)| (Reverse(n), self.labels[b].clone()));
        used.into_iter()
            .map(|(_, b)| self.labels[b].as_str())
            .collect()
    }

    /// The building block (index into `labels`) of every call-model class.
    pub fn class_blocks<'c>(
        &self,
        graph: &Graph,
        calls: &'c CallAnalysis,
    ) -> BTreeMap<&'c str, usize> {
        let by_name: BTreeMap<&str, NodeId> = graph
            .nodes
            .iter()
            .map(|n| (n.name.as_str(), n.id))
            .collect();
        calls
            .classes
            .iter()
            .filter_map(|c| {
                let block = self.owner.get(by_name.get(c.graph_node.as_str())?)?;
                Some((c.id.as_str(), *block))
            })
            .collect()
    }

    /// Per block: its layer in `all` (index into `config.layers`); the
    /// remainder of loose files has none.
    pub fn block_layers(&self, all: &BTreeMap<NodeId, usize>) -> Vec<Option<usize>> {
        (0..self.labels.len())
            .map(|i| self.frontier.get(i).and_then(|n| all.get(&n.id)).copied())
            .collect()
    }

    /// Source connections between blocks (not boxes), by block pair.
    pub fn uses(&self) -> &BTreeMap<(usize, usize), usize> {
        &self.uses
    }

    /// The open question for packages whose manifest declares no responsibility.
    pub fn missing_question(&self, l: DocLang) -> Option<String> {
        let missing: Vec<&str> = self
            .order
            .iter()
            .filter(|&&i| i < self.frontier.len() && self.descriptions[i].is_none())
            .map(|&i| self.labels[i].as_str())
            .collect();
        if missing.is_empty() {
            return None;
        }
        let mut list = missing[..missing.len().min(10)].join(", ");
        if missing.len() > 10 {
            let more = l.pick(" and {} more", " und {} weitere");
            list += &crate::arc42_text::fill(more, &[&(missing.len() - 10)]);
        }
        let text = l.pick(
            "Add a responsibility for: {}. Simplest as `description` in the manifest; it then appears here.",
            "Verantwortung ergänzen für: {}. Am einfachsten als `description` im Manifest; sie erscheint dann hier.",
        );
        Some(crate::arc42_text::fill(text, &[&list]))
    }
}

fn loose_file(node: &Node) -> bool {
    node.kind == NodeKind::File && node.parent.is_none()
}

/// The packages that become blocks: below each top-level module (after
/// skipping single-module chains) its sub-packages, plus the module itself
/// when it has no sub-packages or holds files of its own.
fn frontier(graph: &Graph) -> Vec<&Node> {
    let mut children: Children = BTreeMap::new();
    for node in &graph.nodes {
        children.entry(node.parent).or_default().push(node);
    }
    let mut frontier = Vec::new();
    for &root in children.get(&None).into_iter().flatten() {
        if root.kind != NodeKind::Module {
            continue;
        }
        let root = chain_end(&children, root);
        let below = kids(&children, root.id);
        let packages = below.iter().filter(|n| n.kind == NodeKind::Module);
        let before = frontier.len();
        frontier.extend(packages.copied());
        if frontier.len() == before || below.iter().any(|n| n.kind == NodeKind::File) {
            frontier.push(root);
        }
    }
    frontier
}

fn kids<'m, 'a>(children: &'m Children<'a>, id: NodeId) -> &'m [&'a Node] {
    children
        .get(&Some(id))
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// The first module below `root` with more than one child or a non-module child.
fn chain_end<'a>(children: &Children<'a>, mut root: &'a Node) -> &'a Node {
    loop {
        match kids(children, root.id) {
            [only] if only.kind == NodeKind::Module => root = only,
            _ => return root,
        }
    }
}

/// Each node's block, and per block its file count and first file.
fn group<'a>(graph: &'a Graph, frontier: &[&Node], count: usize) -> Groups<'a> {
    let by_id: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id, n)).collect();
    let index: BTreeMap<_, _> = frontier
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    let mut groups = Groups {
        owner: BTreeMap::new(),
        files: vec![0; count],
        sample: vec![None; count],
    };
    for node in &graph.nodes {
        let block = if loose_file(node) {
            Some(frontier.len())
        } else {
            enclosing(&by_id, &index, node.id, graph.nodes.len())
        };
        let Some(i) = block else { continue };
        groups.owner.insert(node.id, i);
        if node.kind == NodeKind::File {
            groups.files[i] += 1;
            groups.sample[i].get_or_insert(node.name.as_str());
        }
    }
    groups
}

/// The block of the nearest enclosing package node, if any.
fn enclosing(
    by_id: &BTreeMap<NodeId, &Node>,
    index: &BTreeMap<NodeId, usize>,
    id: NodeId,
    bound: usize,
) -> Option<usize> {
    let mut current = Some(id);
    // Canonical graphs are trees; the bound also protects legacy hand-built reports.
    for _ in 0..bound {
        let id = current?;
        if let Some(&i) = index.get(&id) {
            return Some(i);
        }
        current = by_id.get(&id).and_then(|n| n.parent);
    }
    None
}

/// A block belongs to the deepest manifest project containing one of its files.
/// "Files without a package" stays a pseudo block, never a project.
fn owners<'a>(
    sample: &[Option<&str>],
    blocks: usize,
    source_paths: &BTreeMap<String, String>,
    services: &'a ServiceTopology,
) -> Vec<Option<&'a ServiceProject>> {
    sample
        .iter()
        .enumerate()
        .map(|(i, logical)| {
            let logical = (*logical).filter(|_| i < blocks)?;
            let physical = source_paths.get(logical).map_or(logical, String::as_str);
            services
                .projects
                .iter()
                .filter(|p| p.path == "." || physical.starts_with(&format!("{}/", p.path)))
                .max_by_key(|p| if p.path == "." { 0 } else { p.path.len() })
        })
        .collect()
}

/// A block owned alone by a project is named after it, else after its package.
fn labels(names: &[String], owners: &[Option<&ServiceProject>]) -> Vec<String> {
    let alone = |id: &str| {
        owners
            .iter()
            .filter(|o| o.is_some_and(|o| o.id == id))
            .count()
            == 1
    };
    names
        .iter()
        .zip(owners)
        .map(|(name, owner)| match owner {
            Some(p) if alone(&p.id) => p.name.clone(),
            _ => name.clone(),
        })
        .collect()
}

/// Consumers before providers: the highest declared layer first, unlayered
/// blocks last, then by file count and label.
fn order(layers: &[Option<usize>], files: &[usize], labels: &[String]) -> Vec<usize> {
    let mut order: Vec<_> = (0..labels.len()).collect();
    order.sort_by_key(|&i| {
        (
            Reverse(layers[i].map_or(-1, |l| l as i64)),
            Reverse(files[i]),
            labels[i].clone(),
        )
    });
    order
}
