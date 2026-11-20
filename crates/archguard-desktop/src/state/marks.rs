//! Evidence marked in the Graph from elsewhere (Services → pattern
//! candidates, the web viewer's `focusPattern`): source files with their
//! packages, and the dependencies between file pairs, on whatever rows
//! stand for them.

use std::collections::BTreeSet;

use crate::tree::{Row, Tree, TreeView};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks {
    /// The marked files and every package above them.
    nodes: BTreeSet<usize>,
    /// Marked dependencies as file node pairs.
    pairs: Vec<(usize, usize)>,
}

impl Marks {
    /// Marks for the nodes named `files` and the dependencies named
    /// `pairs` in `tree`; names `tree` does not have are left out.
    pub fn new(tree: &Tree, files: &[String], pairs: &[(String, String)]) -> Self {
        let node = |name: &str| tree.nodes.iter().position(|n| n.name == name);
        let mut nodes = BTreeSet::new();
        for mut n in files.iter().filter_map(|f| node(f)) {
            while nodes.insert(n) {
                match tree.nodes[n].parent {
                    Some(p) => n = p,
                    None => break,
                }
            }
        }
        let pairs = pairs
            .iter()
            .filter_map(|(a, b)| Some((node(a)?, node(b)?)))
            .collect();
        Marks { nodes, pairs }
    }

    /// The row stands for a marked file or one of its packages.
    pub fn row(&self, row: &Row) -> bool {
        row.chain.iter().any(|n| self.nodes.contains(n))
    }

    /// `(from row, to row)` of every link that carries a marked dependency.
    pub fn links(&self, tree: &Tree, view: &TreeView) -> BTreeSet<(usize, usize)> {
        if self.pairs.is_empty() {
            return BTreeSet::new();
        }
        let mut row_of = vec![usize::MAX; tree.nodes.len()];
        for (r, row) in view.rows.iter().enumerate() {
            row_of[row.node] = r;
        }
        // The row of `n` or of the nearest package above it.
        let row = |mut n: usize| loop {
            if row_of[n] != usize::MAX {
                return row_of[n];
            }
            match tree.nodes[n].parent {
                Some(p) => n = p,
                None => return usize::MAX,
            }
        };
        self.pairs
            .iter()
            .map(|&(a, b)| (row(a), row(b)))
            .filter(|&(a, b)| a != b && a != usize::MAX && b != usize::MAX)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{GraphBuilder, Ref};

    /// a/{x.rs, y.rs}, b/z.rs; x→z, y→z.
    fn tree() -> Tree {
        let mut b = GraphBuilder::new();
        for (from, to) in [("a/x.rs", "b/z.rs"), ("a/y.rs", "b/z.rs")] {
            let (f, t) = (b.ensure_path(from, None), b.ensure_path(to, None));
            let r = Ref {
                file: from.into(),
                line: 1,
                raw: "use".into(),
            };
            b.edge(f, t, r).unwrap();
        }
        Tree::from_graph(&b.build("/unused".into()))
    }

    /// `a/x.rs → b/z.rs` marked: open, it is the files' link; closed, the
    /// packages' link and the packages are marked.
    #[test]
    fn marks_follow_the_rows_that_stand_for_them() {
        let tree = tree();
        let name = |n: usize| tree.nodes[n].name.clone();
        let m = Marks::new(
            &tree,
            &["a/x.rs".into(), "b/z.rs".into(), "gone.rs".into()],
            &[("a/x.rs".into(), "b/z.rs".into())],
        );
        let closed = tree.view(&BTreeSet::new());
        let marked: Vec<String> = closed
            .rows
            .iter()
            .filter(|r| m.row(r))
            .map(|r| name(r.node))
            .collect();
        assert_eq!(marked, ["a", "b"]);
        let row = |v: &TreeView, n: &str| v.rows.iter().position(|r| name(r.node) == n).unwrap();
        assert_eq!(
            m.links(&tree, &closed),
            BTreeSet::from([(row(&closed, "a"), row(&closed, "b"))])
        );
        let open = tree.view(&(0..tree.nodes.len()).collect());
        assert!(!m.row(&open.rows[row(&open, "a/y.rs")]));
        assert_eq!(
            m.links(&tree, &open),
            BTreeSet::from([(row(&open, "a/x.rs"), row(&open, "b/z.rs"))])
        );
        assert!(Marks::default().links(&tree, &open).is_empty());
    }
}
