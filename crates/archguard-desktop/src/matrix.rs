//! The Matrix tab: source hierarchy on the rows, stable target packages
//! on the columns. Port of the web viewer's `buildTreeMatrix` and
//! `treeMatrixRows` (`viewer.js`).

use std::collections::{BTreeMap, BTreeSet};

use crate::tree::Tree;

/// What a column collects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Everything below this package.
    Package(usize),
    /// Files directly in the scope root.
    Direct,
    /// Targets outside the scope.
    Outside,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub target: Target,
    /// Full name (shown as tooltip and in the jump list).
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cell {
    pub count: u64,
    /// False when an edge lacks an exact count (shown as "≥").
    pub exact: bool,
    /// Indices into the full graph's edges.
    pub edges: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    /// `C1`, `C2`, … in SCC order.
    pub id: String,
    pub members: Vec<usize>,
    /// One closed example path, first node repeated at the end.
    pub path: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    /// Top rows of the scope.
    pub roots: Vec<usize>,
    pub columns: Vec<Column>,
    /// Row node → column index → cell. Package rows sum their descendants.
    pub cells: BTreeMap<usize, BTreeMap<usize, Cell>>,
    /// Nodes that belong to a column (the rows the matrix can show).
    pub included: BTreeSet<usize>,
    pub files: usize,
    pub total: u64,
    pub total_exact: bool,
    pub cycles: Vec<Cycle>,
    /// Node → indices of the file cycles below it.
    pub badges: BTreeMap<usize, BTreeSet<usize>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatrixRow {
    pub node: usize,
    pub depth: usize,
}

impl Matrix {
    /// `scope` `None` is the project overview; `project` names the root.
    pub fn build(tree: &Tree, scope: Option<usize>, project: &str) -> Matrix {
        let scope = scope.filter(|&s| s < tree.nodes.len());
        let roots = overview_roots(tree, scope);
        let packages: Vec<usize> = roots
            .iter()
            .copied()
            .filter(|&r| !tree.nodes[r].file)
            .collect();
        let direct: BTreeSet<usize> = roots
            .iter()
            .copied()
            .filter(|&r| tree.nodes[r].file)
            .collect();
        let mut columns: Vec<Column> = packages
            .iter()
            .map(|&p| Column {
                target: Target::Package(p),
                name: tree.nodes[p].name.clone(),
            })
            .collect();
        if !direct.is_empty() {
            let base = scope.map_or(project.to_string(), |s| tree.nodes[s].name.clone());
            columns.push(Column {
                target: Target::Direct,
                name: base,
            });
        }
        let column_of: BTreeMap<usize, usize> =
            packages.iter().enumerate().map(|(i, &p)| (p, i)).collect();
        let direct_col = packages.len();
        // Owner column of every node inside the scope.
        let owner: BTreeMap<usize, usize> = (0..tree.nodes.len())
            .filter_map(|n| {
                chain(tree, n)
                    .find_map(|a| column_of.get(&a).copied())
                    .or_else(|| direct.contains(&n).then_some(direct_col))
                    .map(|c| (n, c))
            })
            .collect();
        let included: BTreeSet<usize> = owner.keys().copied().collect();
        let mut m = Matrix {
            roots,
            columns,
            cells: BTreeMap::new(),
            files: included.iter().filter(|&&n| tree.nodes[n].file).count(),
            included,
            total: 0,
            total_exact: true,
            cycles: Vec::new(),
            badges: BTreeMap::new(),
        };
        m.fill_cells(tree, &owner);
        m.find_cycles(tree);
        m
    }

    fn fill_cells(&mut self, tree: &Tree, owner: &BTreeMap<usize, usize>) {
        let mut outside = None;
        for (from, to, e) in tree.edges() {
            if !self.included.contains(&from) {
                continue;
            }
            let col = match owner.get(&to) {
                Some(&c) => c,
                None => *outside.get_or_insert_with(|| {
                    self.columns.push(Column {
                        target: Target::Outside,
                        name: String::new(),
                    });
                    self.columns.len() - 1
                }),
            };
            let edge = &tree.full.edges[e];
            let (w, exact) = (edge.weight(), edge.count.is_some());
            self.total += w;
            self.total_exact &= exact;
            for id in chain(tree, from) {
                if !self.included.contains(&id) {
                    break;
                }
                let cell = self
                    .cells
                    .entry(id)
                    .or_default()
                    .entry(col)
                    .or_insert(Cell {
                        count: 0,
                        exact: true,
                        edges: Vec::new(),
                    });
                cell.count += w;
                cell.exact &= exact;
                cell.edges.push(e);
            }
        }
    }

    /// File cycles inside the scope, with one closed example path each.
    fn find_cycles(&mut self, tree: &Tree) {
        let files: Vec<usize> = self
            .included
            .iter()
            .copied()
            .filter(|&n| tree.nodes[n].file)
            .collect();
        let index: BTreeMap<usize, usize> =
            files.iter().enumerate().map(|(i, &f)| (f, i)).collect();
        let mut adj: BTreeMap<usize, Vec<usize>> = files.iter().map(|&f| (f, Vec::new())).collect();
        let mut g = petgraph::graph::DiGraph::<usize, ()>::new();
        let ids: Vec<_> = files.iter().map(|&f| g.add_node(f)).collect();
        for (from, to, _) in tree.edges() {
            if let (Some(&a), Some(&b)) = (index.get(&from), index.get(&to)) {
                g.add_edge(ids[a], ids[b], ());
                adj.get_mut(&from).map(|v| v.push(to));
            }
        }
        let mut groups: Vec<Vec<usize>> = petgraph::algo::tarjan_scc(&g)
            .into_iter()
            .map(|c| {
                let mut m: Vec<usize> = c.into_iter().map(|i| g[i]).collect();
                m.sort_by(|a, b| tree.nodes[*a].name.cmp(&tree.nodes[*b].name));
                m
            })
            .filter(|m| m.len() > 1 || adj[&m[0]].contains(&m[0]))
            .collect();
        groups.sort_by(|a, b| tree.nodes[a[0]].name.cmp(&tree.nodes[b[0]].name));
        for (i, members) in groups.into_iter().enumerate() {
            let path = example_path(&members, &adj);
            for &member in &members {
                for a in chain(tree, member) {
                    if !self.included.contains(&a) {
                        break;
                    }
                    self.badges.entry(a).or_default().insert(i);
                }
            }
            self.cycles.push(Cycle {
                id: format!("C{}", i + 1),
                members,
                path,
            });
        }
    }

    /// Visible rows: the roots and the children of `expanded` rows. A
    /// search, or `only` the chosen `cycle`, keeps the matching rows and
    /// their ancestors and opens them all; rows of `cycle` come first.
    pub fn rows(
        &self,
        tree: &Tree,
        expanded: &BTreeSet<usize>,
        query: &str,
        cycle: Option<usize>,
        only: bool,
    ) -> Vec<MatrixRow> {
        let q = query.trim().to_lowercase();
        let cycle_filter = cycle.filter(|_| only);
        let filtered = !q.is_empty() || cycle_filter.is_some();
        let mut keep = BTreeSet::new();
        if filtered {
            for &id in &self.included {
                let hit = (q.is_empty() || tree.nodes[id].name.to_lowercase().contains(&q))
                    && cycle_filter
                        .is_none_or(|c| self.badges.get(&id).is_some_and(|b| b.contains(&c)));
                if hit {
                    keep.extend(chain(tree, id).filter(|a| self.included.contains(a)));
                }
            }
        }
        let shown = |list: &[usize]| -> Vec<usize> {
            let mut v: Vec<usize> = list
                .iter()
                .copied()
                .filter(|n| !filtered || keep.contains(n))
                .collect();
            if let Some(c) = cycle {
                // Rows of the chosen cycle first, as the web's `prioritise`.
                v.sort_by_key(|n| !self.badges.get(n).is_some_and(|b| b.contains(&c)));
            }
            v
        };
        let mut rows = Vec::new();
        let mut stack: Vec<MatrixRow> = shown(&self.roots)
            .into_iter()
            .rev()
            .map(|node| MatrixRow { node, depth: 0 })
            .collect();
        while let Some(row) = stack.pop() {
            rows.push(row);
            if filtered || expanded.contains(&row.node) {
                let kids = shown(&tree.nodes[row.node].children);
                stack.extend(kids.into_iter().rev().map(|node| MatrixRow {
                    node,
                    depth: row.depth + 1,
                }));
            }
        }
        rows
    }
}

/// `n` and its ancestors, innermost first.
fn chain(tree: &Tree, n: usize) -> impl Iterator<Item = usize> + '_ {
    std::iter::successors(Some(n), move |&c| tree.nodes[c].parent)
}

/// Top rows: the scope's children; in the overview each root descends
/// through single-package chains, and a package with subpackages is
/// replaced by its children.
fn overview_roots(tree: &Tree, scope: Option<usize>) -> Vec<usize> {
    let children = |p: Option<usize>| -> &[usize] {
        match p {
            None => &tree.roots,
            Some(n) => &tree.nodes[n].children,
        }
    };
    let is_package = |n: usize| !tree.nodes[n].file;
    let roots = children(scope).to_vec();
    if scope.is_some() {
        return roots;
    }
    let mut out = Vec::new();
    for mut root in roots {
        while let [only] = children(Some(root)) {
            if !is_package(root) || !is_package(*only) {
                break;
            }
            root = *only;
        }
        let kids = children(Some(root));
        if is_package(root) && kids.iter().any(|&k| is_package(k)) {
            out.extend_from_slice(kids);
        } else {
            out.push(root);
        }
    }
    out
}

/// A closed walk through `members`: from the first member to one of its
/// successors inside the group, then breadth-first back to the start.
fn example_path(members: &[usize], adj: &BTreeMap<usize, Vec<usize>>) -> Vec<usize> {
    let set: BTreeSet<usize> = members.iter().copied().collect();
    let start = members[0];
    let next = adj[&start]
        .iter()
        .copied()
        .find(|&n| set.contains(&n) && n != start)
        .unwrap_or(start);
    if next == start {
        return vec![start, start];
    }
    let mut parent: BTreeMap<usize, Option<usize>> = BTreeMap::from([(next, None)]);
    let mut queue = std::collections::VecDeque::from([next]);
    while let Some(cur) = queue.pop_front() {
        if parent.contains_key(&start) {
            break;
        }
        for &t in &adj[&cur] {
            if set.contains(&t) && !parent.contains_key(&t) {
                parent.insert(t, Some(cur));
                queue.push_back(t);
            }
        }
    }
    let mut tail = vec![start];
    let mut cur = parent.get(&start).copied().flatten();
    while let Some(c) = cur {
        tail.push(c);
        cur = parent.get(&c).copied().flatten();
    }
    tail.reverse();
    std::iter::once(start).chain(tail).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Edge, Graph, Node, NodeId, NodeKind};

    /// lib/src/{a.rs, b.rs}, app/{ui/{v.rs}, main.rs}, top.rs; a→b, b→a,
    /// v.rs→a, main.rs→top.rs.
    fn tree() -> Tree {
        let node = |id: u32, name: &str, kind, parent: Option<u32>| Node {
            id: NodeId(id),
            name: name.into(),
            kind,
            parent: parent.map(NodeId),
            lang: None,
        };
        let edge = |from: u32, to: u32, count: u64| Edge {
            from: NodeId(from),
            to: NodeId(to),
            refs: Vec::new(),
            count: Some(count),
        };
        use NodeKind::{File, Module};
        Tree::from_graph(&Graph {
            schema_version: 1,
            root: ".".into(),
            nodes: vec![
                node(0, "lib", Module, None),
                node(1, "lib/src", Module, Some(0)),
                node(2, "lib/src/a.rs", File, Some(1)),
                node(3, "lib/src/b.rs", File, Some(1)),
                node(4, "app", Module, None),
                node(5, "app/ui", Module, Some(4)),
                node(6, "app/ui/v.rs", File, Some(5)),
                node(7, "app/main.rs", File, Some(4)),
                node(8, "top.rs", File, None),
            ],
            edges: vec![edge(2, 3, 2), edge(3, 2, 1), edge(6, 2, 4), edge(7, 8, 1)],
            semantic: None,
            review_kpis: None,
        })
    }

    fn names(m: &Matrix) -> Vec<String> {
        m.columns.iter().map(|c| c.name.clone()).collect()
    }

    #[test]
    fn overview_descends_unary_chains_and_splits_packages_with_subpackages() {
        let t = tree();
        let m = Matrix::build(&t, None, "proj");
        // lib → lib/src (unary, files only) stays one root; app has the
        // subpackage ui, so its children become roots; top.rs is direct.
        let roots: Vec<&str> = m.roots.iter().map(|&r| t.nodes[r].name.as_str()).collect();
        assert_eq!(roots, ["app/ui", "app/main.rs", "lib/src", "top.rs"]);
        assert_eq!(
            names(&m),
            ["app/ui", "lib/src", "proj"],
            "packages, then the direct-files bucket"
        );
    }

    #[test]
    fn cells_sum_along_the_source_chain() {
        let t = tree();
        let m = Matrix::build(&t, None, "proj");
        let lib = 1; // column of lib/src
        let v = 6; // app/ui/v.rs
        assert_eq!(m.cells[&v][&lib].count, 4);
        assert_eq!(m.cells[&5][&lib].count, 4, "app/ui sums its file");
        // lib/src → lib/src: internal dependencies stay visible.
        assert_eq!(m.cells[&1][&lib].count, 3);
        assert_eq!(
            m.cells[&7][&2].count, 1,
            "main.rs → top.rs in the direct bucket"
        );
        assert_eq!((m.files, m.total), (5, 8));
        assert!(m.total_exact);
    }

    #[test]
    fn file_cycles_get_ids_paths_and_badges() {
        let t = tree();
        let m = Matrix::build(&t, None, "proj");
        assert_eq!(m.cycles.len(), 1);
        let c = &m.cycles[0];
        assert_eq!(c.id, "C1");
        assert_eq!(c.members.len(), 2);
        assert_eq!(c.path.first(), c.path.last());
        assert!(m.badges[&1].contains(&0), "lib/src carries the badge");
    }

    #[test]
    fn rows_open_expanded_packages_and_filters_keep_ancestors() {
        let t = tree();
        let m = Matrix::build(&t, None, "proj");
        let names = |rows: Vec<MatrixRow>| -> Vec<String> {
            rows.iter()
                .map(|r| format!("{}{}", " ".repeat(r.depth), t.nodes[r.node].name))
                .collect()
        };
        assert_eq!(
            names(m.rows(&t, &BTreeSet::new(), "", None, false)).len(),
            4
        );
        let open = names(m.rows(&t, &BTreeSet::from([1]), "", None, false));
        assert_eq!(
            open[2..6],
            ["lib/src", " lib/src/a.rs", " lib/src/b.rs", "top.rs"]
        );
        let found = names(m.rows(&t, &BTreeSet::new(), "V.RS", None, false));
        assert_eq!(found, ["app/ui", " app/ui/v.rs"]);
        let cyc = names(m.rows(&t, &BTreeSet::new(), "", Some(0), true));
        assert!(m.columns[2].target == Target::Direct);
        assert_eq!(cyc, ["lib/src", " lib/src/a.rs", " lib/src/b.rs"]);
    }

    #[test]
    fn a_scope_uses_its_children_and_an_outside_column() {
        let t = tree();
        let m = Matrix::build(&t, Some(4), "proj");
        let roots: Vec<&str> = m.roots.iter().map(|&r| t.nodes[r].name.as_str()).collect();
        assert_eq!(roots, ["app/ui", "app/main.rs"]);
        let last = m.columns.last().unwrap();
        assert_eq!(
            last.target,
            Target::Outside,
            "v.rs → lib and main.rs → top.rs leave app"
        );
    }
}
