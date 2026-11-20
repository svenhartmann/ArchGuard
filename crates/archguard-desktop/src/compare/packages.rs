//! One side of a comparison at package level: every file grouped into its
//! nearest package (module node), edges between packages with their merged
//! source evidence, and the packages in a cycle. The desktop form of the
//! web viewer's `packageDiffGraph` and `annotateDiffCycles`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use archguard_engine::core::{Graph, NodeKind, Ref};

/// The package of files directly at the project root.
pub const ROOT: &str = "/";

#[derive(Debug, Default)]
pub struct Packages {
    /// Package name → its files.
    pub files: BTreeMap<String, BTreeSet<String>>,
    /// File name → its package.
    pub owner: HashMap<String, String>,
    /// `(from, to)` package edge → its evidence, sorted and deduplicated.
    pub edges: BTreeMap<(String, String), Vec<Ref>>,
    /// Package → its cycle (strongly connected group of two or more).
    cycle: HashMap<String, usize>,
}

impl Packages {
    pub fn of(graph: &Graph) -> Self {
        let by_id: HashMap<u32, _> = graph.nodes.iter().map(|n| (n.id.0, n)).collect();
        let mut p = Packages::default();
        let mut owner_of = HashMap::new();
        for file in graph.nodes.iter().filter(|n| n.kind == NodeKind::File) {
            let mut parent = file.parent.and_then(|id| by_id.get(&id.0));
            // The nearest module ancestor; containment is a tree, the bound
            // only guards against a malformed graph.
            for _ in 0..graph.nodes.len() {
                match parent {
                    Some(n) if n.kind != NodeKind::Module => {
                        parent = n.parent.and_then(|id| by_id.get(&id.0));
                    }
                    _ => break,
                }
            }
            let package = parent.map_or(ROOT, |n| n.name.as_str()).to_string();
            owner_of.insert(file.id.0, package.clone());
            p.owner.insert(file.name.clone(), package.clone());
            p.files
                .entry(package)
                .or_default()
                .insert(file.name.clone());
        }
        for e in &graph.edges {
            let (Some(from), Some(to)) = (owner_of.get(&e.from.0), owner_of.get(&e.to.0)) else {
                continue;
            };
            if from != to {
                let refs = p.edges.entry((from.clone(), to.clone())).or_default();
                refs.extend(e.refs.iter().cloned());
            }
        }
        for refs in p.edges.values_mut() {
            refs.sort_by(|a, b| (&a.file, a.line, &a.raw).cmp(&(&b.file, b.line, &b.raw)));
            refs.dedup();
        }
        p.cycle = cycles(&p);
        p
    }

    pub fn in_cycle(&self, package: &str) -> bool {
        self.cycle.contains_key(package)
    }

    /// Whether the edge lies in a cycle: both ends in the same group.
    pub fn edge_in_cycle(&self, from: &str, to: &str) -> bool {
        matches!((self.cycle.get(from), self.cycle.get(to)), (Some(a), Some(b)) if a == b)
    }
}

/// Package → cycle number, for groups of two or more packages.
fn cycles(p: &Packages) -> HashMap<String, usize> {
    let names: Vec<&String> = p.files.keys().collect();
    let index: HashMap<&String, usize> = names.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let edges = p
        .edges
        .keys()
        .filter_map(|(a, b)| Some((*index.get(a)?, *index.get(b)?)));
    crate::tree_order::scc(names.len(), edges)
        .into_iter()
        .filter(|group| group.len() > 1)
        .enumerate()
        .flat_map(|(c, group)| group.into_iter().map(move |i| (i, c)))
        .map(|(i, c)| (names[i].clone(), c))
        .collect()
}
