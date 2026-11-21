//! Why two packages depend on each other: a port of the web viewer's
//! `packageCycleExplanation` (`viewer_detail_graphs.js`, last in 46b5462).
//!
//! For a package pair with file edges in both directions it picks one
//! recorded two-edge example (a file of one package using a file of the
//! other, which uses a file of the first back), counts the connections per
//! direction by source path (production / tests / mixed / unknown) and
//! counts the file-level cycle groups inside both packages. Nothing is
//! inferred: every edge shown is a recorded graph edge.

use std::collections::{HashMap, HashSet};

use archguard_core::{Edge, Graph, NodeId, NodeKind, Ref};

use crate::scc::find_sccs;

/// Which source set a connection comes from, by path convention only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceScope {
    Production,
    Test,
    /// Evidence from both production and test paths.
    Mixed,
    /// Other paths, missing references or an incomplete reference sample.
    Unknown,
}

impl SourceScope {
    /// Example choice: production first, tests last.
    fn priority(self) -> u8 {
        match self {
            SourceScope::Production => 0,
            SourceScope::Unknown => 1,
            SourceScope::Mixed => 2,
            SourceScope::Test => 3,
        }
    }
}

/// Distinct file pairs of one direction per [`SourceScope`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeCounts {
    pub production: usize,
    pub test: usize,
    pub mixed: usize,
    pub unknown: usize,
}

impl ScopeCounts {
    fn add(&mut self, scope: SourceScope) {
        *match scope {
            SourceScope::Production => &mut self.production,
            SourceScope::Test => &mut self.test,
            SourceScope::Mixed => &mut self.mixed,
            SourceScope::Unknown => &mut self.unknown,
        } += 1;
    }
}

/// One package direction: its distinct file pairs and their counts.
#[derive(Debug, Clone, PartialEq)]
pub struct CycleDirection {
    pub from: String,
    pub to: String,
    pub counts: ScopeCounts,
    /// File edges `from` → `to`, one per file pair, references merged.
    pub edges: Vec<Edge>,
}

/// One recorded file edge of the example, by file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessEdge {
    pub from: String,
    pub to: String,
    pub refs: Vec<Ref>,
}

/// The example: two edges, one per direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleWitness {
    /// The second edge starts at the file the first one reaches.
    pub connected: bool,
    /// The second edge returns to the file the first one starts at.
    pub closed: bool,
    /// The package the example starts in and the one it passes through.
    pub start: String,
    pub middle: String,
    /// The files in path order, each once.
    pub files: Vec<String>,
    pub edges: Vec<WitnessEdge>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackageCycleExplanation {
    /// `from` → `to`, then `to` → `from`.
    pub directions: [CycleDirection; 2],
    pub witness: CycleWitness,
    /// Cyclic file groups among the files of both packages.
    pub file_cycles: usize,
}

/// The source set of one edge's evidence. Only a complete reference sample
/// (`count` equals the number of `refs`) can establish a category.
pub fn source_scope(edge: &Edge) -> SourceScope {
    if edge.refs.is_empty() || edge.count != Some(edge.refs.len() as u64) {
        return SourceScope::Unknown;
    }
    let kinds: HashSet<SourceScope> = edge.refs.iter().map(|r| path_scope(&r.file)).collect();
    if kinds.contains(&SourceScope::Unknown) {
        SourceScope::Unknown
    } else if kinds.len() == 1 {
        *kinds.iter().next().expect("one kind")
    } else {
        SourceScope::Mixed
    }
}

/// A Gradle/Maven source root (`src/main`, `src/test`, …, or one at the
/// top) decides; without one, an explicit test directory means tests.
fn path_scope(file: &str) -> SourceScope {
    let file = file.replace('\\', "/");
    let parts: Vec<&str> = file
        .strip_prefix("./")
        .unwrap_or(&file)
        .split('/')
        .collect();
    let (mut production, mut test) = (false, false);
    for i in 0..parts.len().saturating_sub(1) {
        if i == 0 || parts[i - 1] == "src" {
            match parts[i] {
                "main" => production = true,
                "test" | "testFixtures" | "integrationTest" | "androidTest" => test = true,
                _ => {}
            }
        }
    }
    match (production, test) {
        (true, false) => SourceScope::Production,
        (false, true) => SourceScope::Test,
        (true, true) => SourceScope::Unknown,
        (false, false)
            if parts
                .iter()
                .any(|p| matches!(*p, "test" | "tests" | "__tests__" | "spec" | "specs")) =>
        {
            SourceScope::Test
        }
        (false, false) => SourceScope::Unknown,
    }
}

/// `root` and every node below it.
fn subtree(graph: &Graph, root: NodeId) -> HashSet<NodeId> {
    let mut children: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for n in &graph.nodes {
        if let Some(p) = n.parent {
            children.entry(p).or_default().push(n.id);
        }
    }
    let (mut out, mut pending) = (HashSet::new(), vec![root]);
    while let Some(n) = pending.pop() {
        if out.insert(n) {
            pending.extend(children.get(&n).into_iter().flatten().copied());
        }
    }
    out
}

/// One edge per file pair, in first-seen order; a missing count stays
/// missing, references are concatenated.
fn unique(edges: impl Iterator<Item = Edge>) -> Vec<Edge> {
    let mut out: Vec<Edge> = Vec::new();
    let mut index: HashMap<(NodeId, NodeId), usize> = HashMap::new();
    for e in edges {
        match index.get(&(e.from, e.to)) {
            Some(&i) => {
                let old = &mut out[i];
                old.count = old.count.zip(e.count).map(|(a, b)| a + b);
                old.refs.extend(e.refs);
            }
            None => {
                index.insert((e.from, e.to), out.len());
                out.push(e);
            }
        }
    }
    out
}

/// Name lookups of the graph.
struct Names<'a> {
    nodes: HashMap<NodeId, (&'a str, NodeKind)>,
}

impl<'a> Names<'a> {
    fn of(graph: &'a Graph) -> Self {
        Names {
            nodes: graph
                .nodes
                .iter()
                .map(|n| (n.id, (n.name.as_str(), n.kind)))
                .collect(),
        }
    }
    fn name(&self, id: NodeId) -> &'a str {
        self.nodes.get(&id).map_or("", |n| n.0)
    }
    /// A source file, not an external wildcard (`lib/*`).
    fn real(&self, id: NodeId) -> bool {
        self.nodes
            .get(&id)
            .is_some_and(|&(name, kind)| kind == NodeKind::File && !name.ends_with("/*"))
    }
    fn real_edge(&self, e: &Edge) -> bool {
        self.real(e.from) && self.real(e.to)
    }
    /// Example order: production evidence first, then by file names.
    fn order(&self, e: &Edge) -> (u8, &'a str, &'a str) {
        (
            source_scope(e).priority(),
            self.name(e.from),
            self.name(e.to),
        )
    }
}

fn direction(names: &Names, from: &str, to: &str, edges: Vec<Edge>) -> CycleDirection {
    let edges = unique(edges.into_iter());
    let mut counts = ScopeCounts::default();
    for e in &edges {
        counts.add(if names.real_edge(e) {
            source_scope(e)
        } else {
            SourceScope::Unknown
        });
    }
    CycleDirection {
        from: from.into(),
        to: to.into(),
        counts,
        edges,
    }
}

/// The best (score, key) example `first` → `second` with `first` in
/// `forward` and `second` in `back`, starting where `first` ends.
type Score = (u8, i64, usize, String);

fn best_in<'e>(
    names: &Names,
    incoming: &HashMap<NodeId, i64>,
    forward: &'e [Edge],
    back: &'e [Edge],
) -> Option<(Score, &'e Edge, &'e Edge)> {
    let mut returns: HashMap<NodeId, &Edge> = HashMap::new();
    for e in back.iter().filter(|e| names.real_edge(e)) {
        let slot = returns.entry(e.from).or_insert(e);
        if names.order(e) < names.order(slot) {
            *slot = e;
        }
    }
    let mut best: Option<(Score, &Edge, &Edge)> = None;
    for first in forward.iter().filter(|e| names.real_edge(e)) {
        let Some(&second) = returns.get(&first.to) else {
            continue;
        };
        let score = (
            source_scope(first).priority() + source_scope(second).priority(),
            -incoming.get(&first.from).copied().unwrap_or(0),
            names.name(first.to).split('/').count(),
            [first.from, first.to, second.to]
                .map(|id| names.name(id))
                .join("\n"),
        );
        if best.as_ref().is_none_or(|b| score < b.0) {
            best = Some((score, first, second));
        }
    }
    best
}

fn witness_edge(names: &Names, e: &Edge) -> WitnessEdge {
    WitnessEdge {
        from: names.name(e.from).into(),
        to: names.name(e.to).into(),
        refs: e.refs.clone(),
    }
}

fn witness(names: &Names, graph: &Graph, directions: &[CycleDirection; 2]) -> CycleWitness {
    let mut incoming: HashMap<NodeId, i64> = HashMap::new();
    for e in graph
        .edges
        .iter()
        .filter(|e| source_scope(e) == SourceScope::Production)
    {
        *incoming.entry(e.to).or_default() += 1;
    }
    let [a, b] = directions;
    let found = [(0, &a.edges, &b.edges), (1, &b.edges, &a.edges)]
        .into_iter()
        .filter_map(|(i, f, r)| best_in(names, &incoming, f, r).map(|b| (b, i)))
        .min_by(|x, y| x.0.0.cmp(&y.0.0).then(x.1.cmp(&y.1)));
    let Some(((_, first, second), i)) = found else {
        // Separate examples: the first real edge of each direction.
        let edges: Vec<&Edge> = directions
            .iter()
            .filter_map(|d| {
                d.edges
                    .iter()
                    .filter(|e| names.real_edge(e))
                    .min_by_key(|e| names.order(e))
            })
            .collect();
        return CycleWitness {
            connected: false,
            closed: false,
            start: a.from.clone(),
            middle: a.to.clone(),
            files: distinct(edges.iter().flat_map(|e| [e.from, e.to]), names),
            edges: edges.into_iter().map(|e| witness_edge(names, e)).collect(),
        };
    };
    CycleWitness {
        connected: true,
        closed: first.from == second.to,
        start: directions[i].from.clone(),
        middle: directions[i].to.clone(),
        files: distinct([first.from, first.to, second.to].into_iter(), names),
        edges: vec![witness_edge(names, first), witness_edge(names, second)],
    }
}

fn distinct(ids: impl Iterator<Item = NodeId>, names: &Names) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.filter(|id| seen.insert(*id))
        .map(|id| names.name(id).to_string())
        .collect()
}

/// Cyclic file groups among the files below `inside`.
fn file_cycles(graph: &Graph, inside: &HashSet<NodeId>) -> usize {
    let nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::File && inside.contains(&n.id))
        .cloned()
        .collect();
    let files: HashSet<NodeId> = nodes.iter().map(|n| n.id).collect();
    let sub = Graph {
        schema_version: graph.schema_version,
        root: graph.root.clone(),
        edges: graph
            .edges
            .iter()
            .filter(|e| files.contains(&e.from) && files.contains(&e.to))
            .cloned()
            .collect(),
        nodes,
        semantic: None,
        review_kpis: None,
    };
    find_sccs(&sub).len()
}

/// The explanation for packages `from` and `to` (node names); `None` when
/// either is missing or the file edges do not run both ways.
pub fn explain_package_cycle(
    graph: &Graph,
    from: &str,
    to: &str,
) -> Option<PackageCycleExplanation> {
    let id = |name: &str| graph.nodes.iter().find(|n| n.name == name).map(|n| n.id);
    let (left, right) = (subtree(graph, id(from)?), subtree(graph, id(to)?));
    let between = |a: &HashSet<NodeId>, b: &HashSet<NodeId>| -> Vec<Edge> {
        graph
            .edges
            .iter()
            .filter(|e| a.contains(&e.from) && b.contains(&e.to))
            .cloned()
            .collect()
    };
    let (forward, reverse) = (between(&left, &right), between(&right, &left));
    if forward.is_empty() || reverse.is_empty() {
        return None;
    }
    let names = Names::of(graph);
    let directions = [
        direction(&names, from, to, forward),
        direction(&names, to, from, reverse),
    ];
    let witness = witness(&names, graph, &directions);
    let inside: HashSet<NodeId> = left.union(&right).copied().collect();
    Some(PackageCycleExplanation {
        file_cycles: file_cycles(graph, &inside),
        directions,
        witness,
    })
}
