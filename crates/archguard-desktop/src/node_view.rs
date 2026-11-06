//! What the File view shows of one tree node, a file or a package:
//! independent of Slint; `file_ui.rs` shows it.
//!
//! Dependencies are the analysed file-level edges that cross the node's
//! border, grouped by the file on the other end, heaviest first, each
//! with its source evidence (`file:line  code`) as the Graph's details
//! and Dependencies display show it.

use std::collections::BTreeMap;

use archguard_engine::core::Ref;

use crate::tree::Tree;

/// Evidence lines kept per dependency, as the tree's links keep them.
const MAX_REFS: usize = 10;

/// The file on the other end of a node's dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeDep {
    pub node: usize,
    /// Dependency evidence records.
    pub weight: u64,
    /// `file:line  code`, sorted and deduplicated, at most [`MAX_REFS`],
    /// then `… +n` for the rest.
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeView {
    pub node: usize,
    /// `(files below, direct child packages)`; zero for a file.
    pub files: usize,
    pub packages: usize,
    pub outgoing: Vec<NodeDep>,
    pub incoming: Vec<NodeDep>,
}

/// Whether `n` is `root` or below it.
fn inside(tree: &Tree, root: usize, mut n: usize) -> bool {
    loop {
        if n == root {
            return true;
        }
        match tree.nodes[n].parent {
            Some(p) => n = p,
            None => return false,
        }
    }
}

/// The view of `node`; `None` for a node the tree does not have.
pub fn node_view(tree: &Tree, node: usize) -> Option<NodeView> {
    tree.nodes.get(node)?;
    let (mut out, mut inc): (Group, Group) = (BTreeMap::new(), BTreeMap::new());
    for (from, to, e) in tree.edges() {
        let edge = &tree.full.edges[e];
        let entry = match (inside(tree, node, from), inside(tree, node, to)) {
            (true, false) => out.entry(to).or_default(),
            (false, true) => inc.entry(from).or_default(),
            _ => continue,
        };
        entry.0 += edge.weight();
        entry.1.extend(edge.refs.iter().cloned());
    }
    let (files, packages) = if tree.nodes[node].file {
        (0, 0)
    } else {
        tree.counts(node)
    };
    Some(NodeView {
        node,
        files,
        packages,
        outgoing: deps(tree, out),
        incoming: deps(tree, inc),
    })
}

/// Weight and references per node on the other end.
type Group = BTreeMap<usize, (u64, Vec<Ref>)>;

fn deps(tree: &Tree, group: Group) -> Vec<NodeDep> {
    let mut list: Vec<NodeDep> = group
        .into_iter()
        .map(|(node, (weight, refs))| NodeDep {
            node,
            weight,
            evidence: evidence(refs),
        })
        .collect();
    list.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then_with(|| tree.nodes[a.node].name.cmp(&tree.nodes[b.node].name))
    });
    list
}

fn evidence(mut refs: Vec<Ref>) -> Vec<String> {
    refs.sort_by(|a, b| (&a.file, a.line, &a.raw).cmp(&(&b.file, b.line, &b.raw)));
    refs.dedup();
    let more = refs.len().saturating_sub(MAX_REFS);
    let mut lines: Vec<String> = refs
        .iter()
        .take(MAX_REFS)
        .map(|r| format!("{}:{}  {}", r.file, r.line, r.raw.trim()))
        .collect();
    if more > 0 {
        lines.push(format!("… +{more}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Edge, Graph, Node, NodeId, NodeKind};

    /// app/{ui/view.rs, main.rs}, core/lib.rs; view.rs → lib.rs twice
    /// (two references), lib.rs → view.rs, main.rs → view.rs.
    fn tree() -> Tree {
        let node = |id: u32, name: &str, kind, parent: Option<u32>| Node {
            id: NodeId(id),
            name: name.into(),
            kind,
            parent: parent.map(NodeId),
            lang: None,
        };
        let r = |file: &str, line: u32| Ref {
            file: file.into(),
            line,
            raw: " use x; ".into(),
        };
        let edge = |from: u32, to: u32, refs: Vec<Ref>| Edge {
            from: NodeId(from),
            to: NodeId(to),
            count: Some(refs.len() as u64),
            refs,
        };
        use NodeKind::{File, Module};
        Tree::from_graph(&Graph {
            schema_version: 1,
            root: ".".into(),
            nodes: vec![
                node(0, "app", Module, None),
                node(1, "app/ui", Module, Some(0)),
                node(2, "app/ui/view.rs", File, Some(1)),
                node(3, "app/main.rs", File, Some(0)),
                node(4, "core", Module, None),
                node(5, "core/lib.rs", File, Some(4)),
            ],
            edges: vec![
                edge(2, 5, vec![r("app/ui/view.rs", 9), r("app/ui/view.rs", 3)]),
                edge(5, 2, vec![r("core/lib.rs", 1)]),
                edge(3, 2, vec![r("app/main.rs", 2)]),
            ],
            semantic: None,
            review_kpis: None,
        })
    }

    #[test]
    fn a_file_lists_its_dependencies_with_evidence() {
        let t = tree();
        let v = node_view(&t, 2).expect("view.rs");
        assert_eq!(
            v.outgoing,
            [NodeDep {
                node: 5,
                weight: 2,
                evidence: vec![
                    "app/ui/view.rs:3  use x;".into(),
                    "app/ui/view.rs:9  use x;".into()
                ],
            }]
        );
        let users: Vec<(usize, u64)> = v.incoming.iter().map(|d| (d.node, d.weight)).collect();
        assert_eq!(users, [(3, 1), (5, 1)], "same weight: by name");
        assert_eq!((v.files, v.packages), (0, 0));
    }

    /// A package's dependencies cross its border; those inside it are
    /// not listed.
    #[test]
    fn a_package_lists_what_crosses_its_border() {
        let t = tree();
        let app = node_view(&t, 0).expect("app");
        let out: Vec<usize> = app.outgoing.iter().map(|d| d.node).collect();
        let inc: Vec<usize> = app.incoming.iter().map(|d| d.node).collect();
        assert_eq!(
            (out, inc),
            (vec![5], vec![5]),
            "main.rs → view.rs is inside"
        );
        assert_eq!((app.files, app.packages), (2, 1));
        assert_eq!(node_view(&t, 99), None);
    }

    #[test]
    fn evidence_is_capped_with_the_rest_counted() {
        let refs = (1..=12)
            .map(|line| Ref {
                file: "a.rs".into(),
                line,
                raw: "x".into(),
            })
            .collect();
        let lines = evidence(refs);
        assert_eq!(lines.len(), MAX_REFS + 1);
        assert_eq!(lines.last().map(String::as_str), Some("… +2"));
    }
}
