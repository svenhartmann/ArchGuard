//! The call tree: language → packages → types/modules → functions/methods,
//! plus groups for external and unresolved targets (viewer_calls_tree.js).

use std::collections::{HashMap, HashSet};

use super::model::{Index, Labels, Relation, json_list};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Language,
    Package,
    Class,
    Method,
    Group,
    /// A connection to external or unresolved targets.
    Leaf,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub key: String,
    pub kind: Kind,
    pub id: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub count: usize,
    pub depth: usize,
    /// Display label; `None` sorts by id (language and group nodes).
    pub label: Option<String>,
    /// Package path of a package node.
    pub package: String,
    /// Relation key of a leaf.
    pub relation: Option<String>,
}

#[derive(Default)]
pub struct CallTree {
    pub nodes: Vec<Node>,
    by_key: HashMap<String, usize>,
    pub roots: Vec<usize>,
    /// Sites without a complete method identity.
    pub missing_methods: usize,
}

/// `callTreeKey`.
pub fn key(kind: &str, id: &str) -> String {
    json_list([kind, id])
}

impl CallTree {
    pub fn find(&self, key: &str) -> Option<usize> {
        self.by_key.get(key).copied()
    }

    fn add(&mut self, kind: &str, k: Kind, id: &str, parent: Option<usize>) -> usize {
        let key = key(kind, id);
        if let Some(&n) = self.by_key.get(&key) {
            return n;
        }
        let n = self.nodes.len();
        self.nodes.push(Node {
            key: key.clone(),
            kind: k,
            id: id.to_string(),
            parent,
            children: Vec::new(),
            count: 0,
            depth: parent.map_or(0, |p| self.nodes[p].depth + 1),
            label: None,
            package: String::new(),
            relation: None,
        });
        self.by_key.insert(key, n);
        match parent {
            Some(p) => self.nodes[p].children.push(n),
            None => self.roots.push(n),
        }
        n
    }

    fn add_class(&mut self, idx: &Index, id: &str) -> Option<usize> {
        let cls = idx.class(Some(id))?;
        if let Some(n) = self.find(&key("class", id)) {
            return Some(n);
        }
        let language = idx.language(cls);
        let mut parent = self.add("language", Kind::Language, &language, None);
        let mut path = String::new();
        for part in cls.package.split('/').filter(|p| !p.is_empty()) {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(part);
            parent = self.package(&language, &path, part, parent);
        }
        if cls.package.is_empty() {
            parent = self.package(&language, "", "", parent);
        }
        let prefix = format!("{}.", cls.package.replace('/', "."));
        let label = cls
            .name
            .strip_prefix(&prefix)
            .unwrap_or(&cls.name)
            .to_string();
        let n = self.add("class", Kind::Class, id, Some(parent));
        self.nodes[n].label = Some(label);
        Some(n)
    }

    fn package(&mut self, language: &str, path: &str, label: &str, parent: usize) -> usize {
        let n = self.add(
            "package",
            Kind::Package,
            &json_list([language, path]),
            Some(parent),
        );
        self.nodes[n].package = path.to_string();
        self.nodes[n].label = Some(label.to_string());
        n
    }

    /// Open every ancestor of a node (`callTreeOpenPath`).
    pub fn open_path(&self, key: &str, expanded: &mut HashSet<String>) {
        let mut node = self.find(key);
        while let Some(n) = node {
            if !self.nodes[n].children.is_empty() {
                expanded.insert(self.nodes[n].key.clone());
            }
            node = self.nodes[n].parent;
        }
    }

    /// The rows of the expanded tree, depth first (`callTreeRows`).
    pub fn visible(&self, expanded: &HashSet<String>) -> Vec<usize> {
        let mut rows = Vec::new();
        let mut stack: Vec<usize> = self.roots.iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            rows.push(n);
            if expanded.contains(&self.nodes[n].key) {
                stack.extend(self.nodes[n].children.iter().rev());
            }
        }
        rows
    }

    /// The rows shown within `limit`: reserved nodes and their visible
    /// ancestors first, then the tree from the top.
    pub fn shown(&self, visible: &[usize], reserved: &[String], limit: usize) -> HashSet<usize> {
        let on: HashSet<usize> = visible.iter().copied().collect();
        let mut shown = HashSet::new();
        for k in reserved {
            let mut node = self.find(k);
            while let Some(n) = node {
                if on.contains(&n) {
                    shown.insert(n);
                }
                node = self.nodes[n].parent;
            }
        }
        for &n in visible {
            if shown.len() >= limit {
                break;
            }
            shown.insert(n);
        }
        shown
    }

    pub fn has_children(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(|n| !n.children.is_empty())
    }
}

fn add_site(t: &mut CallTree, idx: &Index, call: usize) {
    let c = &idx.a.calls[call];
    let from = t.add_class(idx, &c.from_class);
    let to = c.to_class.as_deref().and_then(|id| t.add_class(idx, id));
    for n in [from, to.filter(|&to| Some(to) != from)]
        .into_iter()
        .flatten()
    {
        t.nodes[n].count += 1;
    }
    let mut seen: Vec<&str> = Vec::new();
    for (id, owner) in [(&c.from_method, from), (&c.to_method, to)] {
        let (Some(id), Some(owner)) = (id.as_deref(), owner) else {
            continue;
        };
        let Some(m) = idx.method(Some(id)) else {
            continue;
        };
        if m.class_id != t.nodes[owner].id || seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let n = t.add("method", Kind::Method, id, Some(owner));
        t.nodes[n].label = Some(m.signature.clone());
        t.nodes[n].count += 1;
    }
    if idx.method(c.from_method.as_deref()).is_none()
        || (c.resolution == "project" && idx.method(c.to_method.as_deref()).is_none())
    {
        t.missing_methods += 1;
    }
}

/// `buildCallTree`.
pub fn build(rels: &[Relation], idx: &Index, l: &Labels) -> CallTree {
    let mut t = CallTree::default();
    for r in rels {
        for &call in &r.calls {
            add_site(&mut t, idx, call);
        }
        if r.resolution != "project" {
            let group = t.add("group", Kind::Group, &r.resolution, None);
            let n = t.add(&r.resolution, Kind::Leaf, &r.key, Some(group));
            let to = match &r.to {
                Some(to) => idx.class_label(Some(to), l),
                None => r.label.clone(),
            };
            let node = &mut t.nodes[n];
            node.label = Some(format!("{} → {}", idx.class_label(Some(&r.from), l), to));
            node.relation = Some(r.key.clone());
            node.count = r.calls.len();
        }
    }
    let sort_key = |n: &Node| {
        (
            n.label.clone().unwrap_or_else(|| n.id.clone()),
            n.key.clone(),
        )
    };
    let keys: Vec<(String, String)> = t.nodes.iter().map(sort_key).collect();
    t.roots.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
    for node in &mut t.nodes {
        node.children.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::super::model::{Filter, Resolution, fixture, relations};
    use super::*;

    fn tree() -> (Index, CallTree) {
        let idx = Index::new(fixture::analysis());
        let f = Filter {
            resolution: Resolution::All,
            ..Default::default()
        };
        let rels = relations(&idx, &f, &Labels::default());
        let t = build(&rels, &idx, &Labels::default());
        (idx, t)
    }

    #[test]
    fn builds_language_package_class_method_levels() {
        let (_, t) = tree();
        let roots: Vec<&str> = t.roots.iter().map(|&r| t.nodes[r].id.as_str()).collect();
        // "group" sorts by its id "unresolved", after "java".
        assert_eq!(roots, vec!["java", "unresolved"]);
        let java = &t.nodes[t.roots[0]];
        let pkgs: Vec<&str> = java
            .children
            .iter()
            .map(|&c| t.nodes[c].label.as_deref().unwrap())
            .collect();
        assert_eq!(pkgs, vec!["blue", "green", "red"]);
        let blue = t.find(&key("class", "blue.Blue")).unwrap();
        assert_eq!(t.nodes[blue].label.as_deref(), Some("Blue"));
        assert_eq!(t.nodes[blue].depth, 2);
        // Blue → Red (2 sites) and Red → Blue.
        assert_eq!(t.nodes[blue].count, 3);
        let name = t.find(&key("method", "blue.Blue#name()")).unwrap();
        assert_eq!(t.nodes[name].count, 3);
        // red.Red#<init>() is not declared.
        assert_eq!(t.missing_methods, 1);
        let leaf = t.nodes.iter().find(|n| n.kind == Kind::Leaf).unwrap();
        assert_eq!(leaf.label.as_deref(), Some("blue.Util → dyn.call"));
        assert_eq!(leaf.count, 1);
    }

    #[test]
    fn rows_follow_the_expanded_nodes_and_reserve_selection() {
        let (_, t) = tree();
        let mut open = HashSet::new();
        assert_eq!(t.visible(&open).len(), 2);
        t.open_path(&key("method", "red.Red#describe()"), &mut open);
        let rows = t.visible(&open);
        let labels: Vec<String> = rows
            .iter()
            .map(|&r| t.nodes[r].label.clone().unwrap_or(t.nodes[r].id.clone()))
            .collect();
        assert_eq!(
            labels,
            vec![
                "java",
                "blue",
                "green",
                "red",
                "Red",
                "describe()",
                "unresolved"
            ]
        );
        let shown = t.shown(&rows, &[key("method", "red.Red#describe()")], 2);
        // describe() and its three visible ancestors beat the limit.
        assert_eq!(shown.len(), 4);
    }
}
