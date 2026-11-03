//! Graph construction (T04): the [`GraphBuilder`] that interns module
//! paths into the containment tree of Kap. 5 and finalizes it into a
//! canonical [`Graph`].
//!
//! Insertion order never leaks into the result: [`GraphBuilder::build`]
//! sorts nodes by name, reassigns ids, and remaps every edge and parent
//! pointer, so the same facts fed in any order produce a byte-identical
//! [`Graph`] (Kap. 12).

use std::collections::BTreeMap;

use crate::model::{ArchguardError, Edge, Graph, Node, NodeId, NodeKind, Ref};

/// `schema_version` emitted by [`GraphBuilder::build`] (Kap. 5).
const SCHEMA_VERSION: u32 = 1;

/// Incremental builder for the canonical [`Graph`] (Kap. 5).
///
/// [`Self::ensure_path`] interns a file node together with its whole
/// ancestor chain as module nodes, [`Self::ensure_pseudo`] adds the
/// `…/*` package pseudo-file used for symbolic imports, and
/// [`Self::edge`] records file→file dependencies together with their
/// source-evidence [`Ref`]s. [`Self::build`] freezes everything into a
/// deterministic [`Graph`].
///
/// The node name is the unique interning key: asking twice for the same
/// path always yields the same [`NodeId`]. If a name already interned as
/// a module is later claimed by a file path (or vice versa) the existing
/// node is reused — the builder never holds two nodes with one name.
#[derive(Debug, Default)]
pub struct GraphBuilder {
    /// Nodes in insertion order; a node's id is its index in this vector.
    nodes: Vec<Node>,
    /// Interning index: node name → id. `BTreeMap` keeps lookups and
    /// iteration deterministic (Kap. 12).
    by_name: BTreeMap<String, NodeId>,
    /// Pending edges keyed by `(from, to)`; repeated calls merge their
    /// refs into one edge, exact duplicate refs are dropped.
    edges: BTreeMap<(u32, u32), Vec<Ref>>,
}

impl GraphBuilder {
    /// A new, empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern `module_path` as a file node and create/intern its whole
    /// ancestor chain as module nodes with `parent` links, so
    /// `"a/b/c.ts"` yields the modules `a` and `a/b` plus the file
    /// `a/b/c.ts`. The same path always returns the same id; `lang` is
    /// recorded on the file node only and never rewrites an already
    /// interned node.
    pub fn ensure_path(&mut self, module_path: &str, lang: Option<&str>) -> NodeId {
        let parent = module_path
            .rfind('/')
            .map(|slash| self.ensure_module(&module_path[..slash]));
        self.intern(
            module_path,
            NodeKind::File,
            parent,
            lang.map(str::to_string),
        )
    }

    /// Intern the package pseudo-file `<module_path>/*` (used for symbolic
    /// imports) under the module `module_path`, creating that module and
    /// all its ancestors as needed. Interning behaves as in
    /// [`Self::ensure_path`].
    pub fn ensure_pseudo(&mut self, module_path: &str) -> NodeId {
        self.ensure_path(&format!("{module_path}/*"), None)
    }

    /// Record a dependency `from → to` proven by the source reference `r`.
    ///
    /// Both endpoints must be existing file nodes, otherwise
    /// [`ArchguardError::Analysis`] is returned and nothing is recorded.
    /// Repeated calls with the same `(from, to)` merge into a single
    /// edge; exact duplicate refs are dropped. Self-edges are silently
    /// ignored.
    pub fn edge(&mut self, from: NodeId, to: NodeId, r: Ref) -> Result<(), ArchguardError> {
        for id in [from, to] {
            let kind = self
                .nodes
                .get(id.0 as usize)
                .map(|n| n.kind)
                .ok_or_else(|| ArchguardError::Analysis(format!("unknown node id {}", id.0)))?;
            if kind != NodeKind::File {
                return Err(ArchguardError::Analysis(format!(
                    "edge endpoint {} is not a file node",
                    id.0
                )));
            }
        }
        if from == to {
            return Ok(());
        }
        let refs = self.edges.entry((from.0, to.0)).or_default();
        if !refs.contains(&r) {
            refs.push(r);
        }
        Ok(())
    }

    /// Consume the builder into the canonical [`Graph`]: nodes are sorted
    /// by name and get fresh ids `0..n`, parent pointers and edge
    /// endpoints are remapped to the new ids, edges are sorted by
    /// `(from, to)` and the refs of every edge by `(file, line)`.
    pub fn build(self, root: String) -> Graph {
        let Self {
            mut nodes,
            edges,
            by_name: _,
        } = self;
        nodes.sort_by(|a, b| a.name.cmp(&b.name));

        // Old id (interning order) → new id (name order). Old ids are
        // dense `0..n` because `intern` assigns them consecutively.
        let mut remap: Vec<NodeId> = vec![NodeId(0); nodes.len()];
        for (index, node) in nodes.iter_mut().enumerate() {
            let new_id = NodeId(index as u32);
            remap[node.id.0 as usize] = new_id;
            node.id = new_id;
        }
        for node in &mut nodes {
            node.parent = node.parent.map(|old| remap[old.0 as usize]);
        }

        let mut out_edges: Vec<Edge> = edges
            .into_iter()
            .map(|((from, to), mut refs)| {
                refs.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
                // F12: the deduplicated sample is the complete evidence
                // here (the builder never truncates), so the total is
                // exactly its length.
                let count = refs.len() as u64;
                Edge {
                    from: remap[from as usize],
                    to: remap[to as usize],
                    refs,
                    count: Some(count),
                }
            })
            .collect();
        out_edges.sort_by_key(|e| (e.from, e.to));

        Graph {
            semantic: None,
            review_kpis: None,
            schema_version: SCHEMA_VERSION,
            root,
            nodes,
            edges: out_edges,
        }
    }

    /// Intern the module node for `path` plus every ancestor module,
    /// nearest ancestor first, so each node's `parent` is already interned.
    fn ensure_module(&mut self, path: &str) -> NodeId {
        if let Some(&id) = self.by_name.get(path) {
            return id;
        }
        let parent = path
            .rfind('/')
            .map(|slash| self.ensure_module(&path[..slash]));
        self.intern(path, NodeKind::Module, parent, None)
    }

    /// Intern a single node under `name`. The first request wins: a name
    /// that is already interned returns its established id untouched.
    fn intern(
        &mut self,
        name: &str,
        kind: NodeKind,
        parent: Option<NodeId>,
        lang: Option<String>,
    ) -> NodeId {
        if let Some(&id) = self.by_name.get(name) {
            return id;
        }
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node {
            id,
            name: name.to_string(),
            kind,
            parent,
            lang,
        });
        self.by_name.insert(name.to_string(), id);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(line: u32) -> Ref {
        Ref {
            file: "src/a/b/c.ts".to_string(),
            line,
            raw: format!("import x from \"./m{line}\";"),
        }
    }

    fn node<'a>(builder: &'a GraphBuilder, name: &str) -> &'a Node {
        let id = *builder
            .by_name
            .get(name)
            .unwrap_or_else(|| panic!("test: no node {name}"));
        &builder.nodes[id.0 as usize]
    }

    #[test]
    fn ensure_path_creates_whole_ancestor_chain() {
        let mut builder = GraphBuilder::new();
        let file = builder.ensure_path("a/b/c.ts", Some("ts"));

        assert_eq!(builder.nodes.len(), 3);
        assert_eq!(builder.by_name.get("a"), Some(&NodeId(0)));
        assert_eq!(builder.by_name.get("a/b"), Some(&NodeId(1)));
        assert_eq!(file, NodeId(2));

        let root = node(&builder, "a");
        assert_eq!(root.kind, NodeKind::Module);
        assert_eq!(root.parent, None);
        assert_eq!(root.lang, None);

        let mid = node(&builder, "a/b");
        assert_eq!(mid.kind, NodeKind::Module);
        assert_eq!(mid.parent, Some(NodeId(0)));

        let file_node = node(&builder, "a/b/c.ts");
        assert_eq!(file_node.kind, NodeKind::File);
        assert_eq!(file_node.parent, Some(NodeId(1)));
        assert_eq!(file_node.lang.as_deref(), Some("ts"));
    }

    #[test]
    fn ensure_path_without_directory_has_no_parent() {
        let mut builder = GraphBuilder::new();
        let file = builder.ensure_path("c.ts", Some("ts"));
        assert_eq!(builder.nodes.len(), 1);
        assert_eq!(builder.nodes[file.0 as usize].kind, NodeKind::File);
        assert_eq!(builder.nodes[file.0 as usize].parent, None);
    }

    #[test]
    fn ensure_path_interns_repeated_paths_and_shared_ancestors() {
        let mut builder = GraphBuilder::new();
        let first = builder.ensure_path("a/b/c.ts", Some("ts"));
        let again = builder.ensure_path("a/b/c.ts", None);
        assert_eq!(first, again);
        assert_eq!(builder.nodes.len(), 3);
        // Interning returns the established node untouched: lang stays.
        assert_eq!(node(&builder, "a/b/c.ts").lang.as_deref(), Some("ts"));

        // A sibling file reuses both ancestor modules.
        let sibling = builder.ensure_path("a/b/d.ts", Some("ts"));
        assert_eq!(builder.nodes.len(), 4);
        assert_eq!(
            builder.nodes[sibling.0 as usize].parent,
            builder.nodes[first.0 as usize].parent
        );
    }

    #[test]
    fn ensure_pseudo_interns_package_pseudo_file() {
        let mut builder = GraphBuilder::new();
        let pseudo = builder.ensure_pseudo("a/b");
        let again = builder.ensure_pseudo("a/b");
        assert_eq!(pseudo, again);

        assert_eq!(builder.nodes.len(), 3);
        let pseudo_node = node(&builder, "a/b/*");
        assert_eq!(pseudo_node.kind, NodeKind::File);
        assert_eq!(pseudo_node.lang, None);
        assert_eq!(pseudo_node.id, pseudo);
        let module = node(&builder, "a/b");
        assert_eq!(pseudo_node.parent, Some(module.id));
        assert_eq!(module.kind, NodeKind::Module);
        assert_eq!(module.parent, Some(node(&builder, "a").id));
    }

    #[test]
    fn edge_merges_refs_dedups_and_sorts_them() {
        let mut builder = GraphBuilder::new();
        let from = builder.ensure_path("a/b/c.ts", Some("ts"));
        let to = builder.ensure_path("d/e.ts", Some("ts"));

        builder.edge(from, to, r(10)).expect("valid edge");
        builder.edge(from, to, r(3)).expect("valid edge");
        builder.edge(from, to, r(10)).expect("duplicate is fine");
        assert_eq!(builder.edges.get(&(from.0, to.0)).map(Vec::len), Some(2));

        let graph = builder.build("/abs/projekt".to_string());
        assert_eq!(graph.edges.len(), 1);
        // Name order: "a" 0, "a/b" 1, "a/b/c.ts" 2, "d" 3, "d/e.ts" 4.
        assert_eq!(graph.edges[0].from, NodeId(2));
        assert_eq!(graph.edges[0].to, NodeId(4));
        let lines: Vec<u32> = graph.edges[0].refs.iter().map(|x| x.line).collect();
        assert_eq!(lines, [3, 10]);
        // F12: the total counts the deduplicated refs (the duplicate
        // r(10) does not inflate it) and matches the weight accessor.
        assert_eq!(graph.edges[0].count, Some(2));
        assert_eq!(graph.edges[0].weight(), 2);
    }

    #[test]
    fn edge_rejects_non_file_and_unknown_endpoints() {
        let mut builder = GraphBuilder::new();
        let file = builder.ensure_path("a/b/c.ts", Some("ts"));
        let module = builder.by_name["a"];

        let err = builder.edge(module, file, r(1));
        assert!(matches!(err, Err(ArchguardError::Analysis(_))));
        assert!(
            err.unwrap_err()
                .to_string()
                .contains("edge endpoint 0 is not a file node")
        );
        assert!(matches!(
            builder.edge(file, module, r(1)),
            Err(ArchguardError::Analysis(_))
        ));
        assert!(matches!(
            builder.edge(NodeId(99), file, r(1)),
            Err(ArchguardError::Analysis(_))
        ));
        // Rejected calls record nothing.
        assert!(builder.edges.is_empty());
    }

    #[test]
    fn edge_drops_self_edges_silently() {
        let mut builder = GraphBuilder::new();
        let file = builder.ensure_path("a/b/c.ts", Some("ts"));
        builder
            .edge(file, file, r(1))
            .expect("self edge is dropped");
        assert!(builder.edges.is_empty());
        let graph = builder.build("/abs".to_string());
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn build_sorts_edges_by_from_then_to() {
        let mut builder = GraphBuilder::new();
        let x = builder.ensure_path("x.ts", Some("ts"));
        let y = builder.ensure_path("y.ts", Some("ts"));
        let a = builder.ensure_path("a/b.ts", Some("ts"));
        builder.edge(x, y, r(1)).expect("valid edge");
        builder.edge(x, a, r(1)).expect("valid edge");

        let graph = builder.build("/abs".to_string());
        // Name order: "a" (0), "a/b.ts" (1), "x.ts" (2), "y.ts" (3).
        assert_eq!(graph.edges[0].from, NodeId(2));
        assert_eq!(graph.edges[0].to, NodeId(1));
        assert_eq!(graph.edges[1].from, NodeId(2));
        assert_eq!(graph.edges[1].to, NodeId(3));
    }

    #[test]
    fn build_sorts_nodes_reassigns_ids_and_remaps_parents() {
        let mut builder = GraphBuilder::new();
        // Insertion order deliberately differs from name order.
        builder.ensure_path("domain/money.ts", Some("ts"));
        builder.ensure_pseudo("app");
        let invoice = builder.ensure_path("app/billing/invoice.ts", Some("ts"));
        let money = builder.by_name["domain/money.ts"];
        builder
            .edge(invoice, money, r(4))
            .expect("test: valid edge");

        let graph = builder.build("/abs/projekt".to_string());
        assert_eq!(graph.schema_version, 1);
        assert_eq!(graph.root, "/abs/projekt");

        let names: Vec<&str> = graph.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "app",
                "app/*",
                "app/billing",
                "app/billing/invoice.ts",
                "domain",
                "domain/money.ts",
            ]
        );
        assert!(
            graph
                .nodes
                .iter()
                .enumerate()
                .all(|(i, n)| n.id == NodeId(i as u32))
        );

        // Parent pointers survived the id reassignment: names still line up.
        let invoice_id = graph
            .nodes
            .iter()
            .find(|n| n.name == "app/billing/invoice.ts")
            .expect("test: invoice exists")
            .id;
        assert_eq!(graph.node_name(invoice_id), Some("app/billing/invoice.ts"));
        let ancestor_names: Vec<&str> = graph
            .ancestors_of(invoice_id)
            .iter()
            .filter_map(|a| graph.node_name(*a))
            .collect();
        assert_eq!(ancestor_names, ["app/billing", "app"]);
        let root = graph
            .nodes
            .iter()
            .find(|n| n.name == "app")
            .expect("test: app exists");
        assert_eq!(root.parent, None);
        let pseudo = graph
            .nodes
            .iter()
            .find(|n| n.name == "app/*")
            .expect("test: pseudo exists");
        assert_eq!(
            graph.node_name(pseudo.parent.expect("test: has parent")),
            Some("app")
        );

        // Edge endpoints point at the renamed nodes.
        let money_id = graph
            .nodes
            .iter()
            .find(|n| n.name == "domain/money.ts")
            .expect("test: money exists")
            .id;
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].from, invoice_id);
        assert_eq!(graph.edges[0].to, money_id);
        assert_eq!(graph.out_edges(invoice_id).count(), 1);
    }
}
