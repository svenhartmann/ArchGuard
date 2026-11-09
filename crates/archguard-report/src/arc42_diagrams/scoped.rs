//! Package map levels (ADR 0006): the packages inside one package, for the
//! ids listed in `[arc42] shown_diagrams` (`<map>@<package>`), as the
//! readers show them when a package is opened.

use archguard_core::{
    Arc42Config, CallAnalysis, Diagram, DiagramElement, DiagramKind, DiagramRelation, DocLang,
    ElementKind, Graph, Node, NodeId, NodeKind, RelationKind,
};
use std::collections::{BTreeMap, HashMap};

use super::cyclic;
use crate::arc42_blocks::Arc42Inputs;

/// Boxes per level: eleven packages and a remainder box.
const BOXES: usize = 12;

pub fn diagrams(input: &Arc42Inputs<'_>) -> Vec<Diagram> {
    let config = &input.config.arc42;
    config
        .shown_diagrams
        .iter()
        .filter_map(|id| level(input.lang, input.source_graph, input.calls, id))
        .collect()
}

fn level(l: DocLang, graph: &Graph, calls: &CallAnalysis, id: &str) -> Option<Diagram> {
    let (map, scope) = Arc42Config::level_of(id)?;
    let root = graph
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Module && n.name == scope)?;
    let (chapter, title, note) = match map {
        "building-blocks" => (
            5,
            l.pick("Package overview", "Paketübersicht"),
            l.pick(
                "Source: source connections between the packages in",
                "Quelle: Quellverbindungen zwischen den Paketen in",
            ),
        ),
        "runtime-calls" => (
            6,
            l.pick("Static calls", "Statische Aufrufe"),
            l.pick(
                "Source: compiler-resolved call sites between the packages in",
                "Quelle: compileraufgelöste Aufrufstellen zwischen den Paketen in",
            ),
        ),
        "cycles" => (
            11,
            l.pick("Cyclic package relations", "Zyklische Paketbeziehungen"),
            l.pick(
                "Source: cycles between the packages in",
                "Quelle: Zyklen zwischen den Paketen in",
            ),
        ),
        _ => return None,
    };
    let boxes = Boxes::new(l, graph, root);
    let counts = if map == "runtime-calls" {
        boxes.calls(graph, calls)
    } else {
        boxes.edges(graph)
    };
    let kind = if map == "runtime-calls" {
        RelationKind::Call
    } else {
        RelationKind::Dependency
    };
    let mut d = Diagram {
        id: id.into(),
        chapter,
        title: format!("{title} · {scope}"),
        kind: DiagramKind::Packages,
        note: format!(
            "{note} {scope}; {}",
            l.pick(
                "files directly in the package as a box of their own.",
                "Dateien direkt im Paket als eigener Kasten."
            )
        ),
        unit: if map == "runtime-calls" {
            l.pick("call sites", "Aufrufstellen")
        } else {
            l.pick("source connections", "Quellverbindungen")
        }
        .into(),
        elements: boxes.elements,
        relations: counts
            .into_iter()
            .map(|((a, b), count)| relation(a, b, kind, count))
            .collect(),
        shown: true,
    };
    cyclic::mark(&mut d);
    if map == "cycles" {
        d.relations.retain(|r| r.cycle);
        d.elements.retain(|e| e.cycle);
    }
    Some(d)
}

fn relation(a: usize, b: usize, kind: RelationKind, count: usize) -> DiagramRelation {
    DiagramRelation {
        from: format!("e{a}"),
        to: format!("e{b}"),
        kind,
        count,
        cycle: false,
        label: None,
        error: false,
        caller: None,
        reply: false,
    }
}

/// The boxes of one level and which box every node below the package
/// falls into.
struct Boxes {
    elements: Vec<DiagramElement>,
    owner: HashMap<NodeId, usize>,
}

impl Boxes {
    fn new(l: DocLang, graph: &Graph, root: &Node) -> Self {
        let by_id: HashMap<NodeId, &Node> = graph.nodes.iter().map(|n| (n.id, n)).collect();
        let mut children: Vec<&Node> = graph
            .nodes
            .iter()
            .filter(|n| n.parent == Some(root.id) && n.kind == NodeKind::Module)
            .collect();
        children.sort_by(|a, b| a.name.cmp(&b.name));
        let element = |i: usize, name: String, kind| DiagramElement {
            id: format!("e{i}"),
            name,
            kind,
            technology: None,
            detail: None,
            group: None,
            cycle: false,
        };
        let shown = children.len().min(BOXES - 1);
        let short = |n: &Node| {
            n.name
                .strip_prefix(&format!("{}/", root.name))
                .unwrap_or(&n.name)
                .to_string()
        };
        let mut elements: Vec<DiagramElement> = children[..shown]
            .iter()
            .enumerate()
            .map(|(i, n)| element(i, short(n), ElementKind::Package))
            .collect();
        if children.len() > shown {
            let more = l.pick("{} more packages", "Weitere {} Pakete");
            let name = crate::arc42_text::fill(more, &[&(children.len() - shown)]);
            elements.push(element(shown, name, ElementKind::Remainder));
        }
        let direct = elements.len();
        let index: HashMap<NodeId, usize> = children
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i.min(shown)))
            .collect();
        let mut owner = HashMap::new();
        for n in &graph.nodes {
            if let Some(top) = below(&by_id, n, root.id) {
                owner.insert(n.id, index.get(&top.id).copied().unwrap_or(direct));
            }
        }
        if owner.values().any(|&o| o == direct) {
            elements.push(element(
                direct,
                format!(
                    "{} / {}",
                    root.name,
                    l.pick("direct files", "direkte Dateien")
                ),
                ElementKind::Remainder,
            ));
        }
        Boxes { elements, owner }
    }

    fn edges(&self, graph: &Graph) -> BTreeMap<(usize, usize), usize> {
        self.count(graph.edges.iter().map(|e| (e.from, e.to)))
    }

    fn calls(&self, graph: &Graph, calls: &CallAnalysis) -> BTreeMap<(usize, usize), usize> {
        let by_name: HashMap<&str, NodeId> = graph
            .nodes
            .iter()
            .map(|n| (n.name.as_str(), n.id))
            .collect();
        let class: HashMap<&str, NodeId> = calls
            .classes
            .iter()
            .filter_map(|c| Some((c.id.as_str(), *by_name.get(c.graph_node.as_str())?)))
            .collect();
        self.count(
            calls
                .calls
                .iter()
                .filter(|c| c.resolution == "project")
                .filter_map(|c| {
                    Some((
                        *class.get(c.from_class.as_str())?,
                        *class.get(c.to_class.as_deref()?)?,
                    ))
                }),
        )
    }

    fn count(
        &self,
        pairs: impl Iterator<Item = (NodeId, NodeId)>,
    ) -> BTreeMap<(usize, usize), usize> {
        let mut out = BTreeMap::new();
        for (from, to) in pairs {
            if let (Some(&a), Some(&b)) = (self.owner.get(&from), self.owner.get(&to))
                && a != b
            {
                *out.entry((a, b)).or_default() += 1;
            }
        }
        out
    }
}

/// The ancestor of `n` directly below `root` (or `n` itself), when `n`
/// lies inside `root`.
fn below<'a>(by_id: &HashMap<NodeId, &'a Node>, n: &'a Node, root: NodeId) -> Option<&'a Node> {
    let mut at = n;
    loop {
        match at.parent {
            Some(p) if p == root => return Some(at),
            Some(p) => at = by_id.get(&p)?,
            None => return None,
        }
    }
}
