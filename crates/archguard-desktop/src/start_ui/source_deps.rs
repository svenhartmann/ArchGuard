//! The details' "Source dependencies" (`renderWorldSourceDetails` in
//! `viewer_world.js`): the original source graph's dependencies crossing
//! the selection, Used by and Depends on, each with its references.

use std::collections::{HashMap, HashSet};

use archguard_engine::core::{Graph, NodeId};

/// References shown per neighbour.
const REFS: usize = 3;

/// One dependency across the selection's border.
#[derive(Debug, Clone, PartialEq)]
pub struct Neighbor {
    pub node: String,
    pub weight: u64,
    /// `file:line raw`, the first few.
    pub refs: Vec<String>,
}

/// The dependencies of the selection's node and everything below it.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceDeps {
    pub name: String,
    pub incoming: Vec<Neighbor>,
    pub outgoing: Vec<Neighbor>,
}

/// The node `root` and every node below it.
fn subtree(g: &Graph, root: NodeId) -> HashSet<NodeId> {
    let parent: HashMap<NodeId, Option<NodeId>> =
        g.nodes.iter().map(|n| (n.id, n.parent)).collect();
    let inside = |mut id: NodeId| loop {
        if id == root {
            return true;
        }
        match parent.get(&id).copied().flatten() {
            Some(p) => id = p,
            None => return false,
        }
    };
    g.nodes
        .iter()
        .map(|n| n.id)
        .filter(|&id| inside(id))
        .collect()
}

/// The dependencies of `name` in `g` (`None`: no such node).
pub fn of(g: &Graph, name: &str) -> Option<SourceDeps> {
    let source = g.nodes.iter().find(|n| n.name == name)?;
    let ids = subtree(g, source.id);
    let names: HashMap<NodeId, &str> = g.nodes.iter().map(|n| (n.id, n.name.as_str())).collect();
    let neighbor = |other: NodeId, e: &archguard_engine::core::Edge| Neighbor {
        node: names.get(&other).copied().unwrap_or_default().to_string(),
        weight: e.weight(),
        refs: e
            .refs
            .iter()
            .take(REFS)
            .map(|r| format!("{}:{} {}", r.file, r.line, r.raw.trim()))
            .collect(),
    };
    let crossing = |inward: bool| {
        let mut rows: Vec<Neighbor> = g
            .edges
            .iter()
            .filter(|e| ids.contains(&e.to) == inward && ids.contains(&e.from) != inward)
            .map(|e| neighbor(if inward { e.from } else { e.to }, e))
            .collect();
        rows.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.node.cmp(&b.node)));
        rows
    };
    Some(SourceDeps {
        name: source.name.clone(),
        incoming: crossing(true),
        outgoing: crossing(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Edge, Node, NodeKind, Ref};

    fn node(id: u32, name: &str, parent: Option<u32>) -> Node {
        Node {
            id: NodeId(id),
            name: name.into(),
            kind: if parent.is_none() {
                NodeKind::Module
            } else {
                NodeKind::File
            },
            parent: parent.map(NodeId),
            lang: None,
        }
    }

    fn edge(from: u32, to: u32, line: u32) -> Edge {
        Edge {
            from: NodeId(from),
            to: NodeId(to),
            refs: vec![Ref {
                file: format!("f{from}.py"),
                line,
                raw: " import x ".into(),
            }],
            count: Some(2),
        }
    }

    /// Edges inside the package are not counted; those crossing it are
    /// Used by or Depends on with their references.
    #[test]
    fn dependencies_cross_the_selection() {
        let mut g: Graph =
            serde_json::from_str(r#"{"schema_version":1,"root":".","nodes":[],"edges":[]}"#)
                .expect("an empty graph");
        g.nodes = vec![
            node(1, "app", None),
            node(2, "app/a.py", Some(1)),
            node(3, "app/b.py", Some(1)),
            node(4, "lib", None),
            node(5, "lib/c.py", Some(4)),
        ];
        g.edges = vec![edge(2, 3, 1), edge(2, 5, 7), edge(5, 3, 9)];
        let d = of(&g, "app").expect("app");
        assert_eq!(d.outgoing.len(), 1);
        assert_eq!(d.outgoing[0].node, "lib/c.py");
        assert_eq!(d.outgoing[0].refs, ["f2.py:7 import x"]);
        assert_eq!(
            d.incoming
                .iter()
                .map(|n| n.node.as_str())
                .collect::<Vec<_>>(),
            ["lib/c.py"]
        );
        assert_eq!(d.incoming[0].weight, 2);
        let file = of(&g, "app/a.py").expect("a file");
        assert_eq!((file.incoming.len(), file.outgoing.len()), (0, 2));
        assert_eq!(of(&g, "nope"), None);
    }
}
