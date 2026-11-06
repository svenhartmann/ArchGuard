//! Cyclic call groups: strongly connected components of resolved project
//! calls between types/modules and between functions/methods, each with one
//! closed example path (viewer_call_cycles_model.js, prepareCallCycles in
//! viewer_call_cycles_ui.js).

use std::collections::{BTreeMap, HashMap, VecDeque};

use super::model::{ElementKind, Filter, Index, Labels, Relation, json_list, json_str};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// Call indices, ordered by source position.
    pub calls: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: String,
    pub kind: ElementKind,
    /// Sorted member ids.
    pub members: Vec<String>,
    pub edges: Vec<Edge>,
    /// Closed path: the first member, …, the first member again.
    pub witness: Vec<String>,
    pub call_sites: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CycleModel {
    pub classes: Vec<Group>,
    pub methods: Vec<Group>,
    pub unresolved_targets: usize,
    pub missing_methods: usize,
}

impl CycleModel {
    pub fn groups(&self, kind: ElementKind) -> &[Group] {
        match kind {
            ElementKind::Class => &self.classes,
            ElementKind::Method => &self.methods,
        }
    }
}

type Edges = BTreeMap<(String, String), Vec<usize>>;

fn add(edges: &mut Edges, from: &str, to: &str, call: usize) {
    edges
        .entry((from.to_string(), to.to_string()))
        .or_default()
        .push(call);
}

/// `computeCallCycles` over the complete snapshot.
pub fn compute(idx: &Index) -> CycleModel {
    let (mut class_edges, mut method_edges) = (Edges::new(), Edges::new());
    let mut model = CycleModel::default();
    for (i, c) in idx.a.calls.iter().enumerate() {
        model.unresolved_targets += usize::from(c.resolution == "unresolved");
        if c.resolution != "project" {
            continue;
        }
        let from = idx
            .class(Some(&c.from_class))
            .map(|_| c.from_class.as_str());
        let to = c
            .to_class
            .as_deref()
            .filter(|t| idx.class(Some(t)).is_some());
        if let (Some(f), Some(t)) = (from, to)
            && f != t
        {
            add(&mut class_edges, f, t, i);
        }
        match (idx.owned(&c.from_method, from), idx.owned(&c.to_method, to)) {
            (Some(f), Some(t)) => add(&mut method_edges, f, t, i),
            _ => model.missing_methods += 1,
        }
    }
    model.classes = groups(ElementKind::Class, class_edges, idx);
    model.methods = groups(ElementKind::Method, method_edges, idx);
    model
}

fn site_order(idx: &Index, a: usize, b: usize) -> std::cmp::Ordering {
    let (x, y) = (&idx.a.calls[a], &idx.a.calls[b]);
    (
        &x.file,
        x.line,
        x.column,
        &x.code,
        &x.dispatch,
        &x.from_class,
    )
        .cmp(&(
            &y.file,
            y.line,
            y.column,
            &y.code,
            &y.dispatch,
            &y.from_class,
        ))
        .then_with(|| {
            (
                &x.from_method,
                &x.resolution,
                &x.target_label,
                &x.to_class,
                &x.to_method,
            )
                .cmp(&(
                    &y.from_method,
                    &y.resolution,
                    &y.target_label,
                    &y.to_class,
                    &y.to_method,
                ))
        })
}

fn groups(kind: ElementKind, edges: Edges, idx: &Index) -> Vec<Group> {
    let mut ids: Vec<&String> = edges.keys().flat_map(|(f, t)| [f, t]).collect();
    ids.sort();
    ids.dedup();
    let node: HashMap<&String, usize> = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let links = edges.keys().map(|(f, t)| (node[f], node[t]));
    let mut result: Vec<Group> = crate::tree_order::scc(ids.len(), links)
        .into_iter()
        .filter(|c| c.len() > 1 || edges.contains_key(&(ids[c[0]].clone(), ids[c[0]].clone())))
        .map(|c| {
            let mut members: Vec<String> = c.into_iter().map(|n| ids[n].clone()).collect();
            members.sort();
            let kind_name = if kind == ElementKind::Class {
                "class"
            } else {
                "method"
            };
            Group {
                id: format!(
                    "[{},{}]",
                    json_str(kind_name),
                    json_list(members.iter().map(String::as_str))
                ),
                kind,
                members,
                edges: Vec::new(),
                witness: Vec::new(),
                call_sites: 0,
            }
        })
        .collect();
    let mut owner = HashMap::new();
    for (g, group) in result.iter().enumerate() {
        for m in &group.members {
            owner.insert(m.clone(), g);
        }
    }
    // BTreeMap order is (from, to): the web's edge order inside a group.
    for ((from, to), mut calls) in edges {
        let Some(&g) = owner.get(&from) else { continue };
        if owner.get(&to) != Some(&g) {
            continue;
        }
        calls.sort_by(|&a, &b| site_order(idx, a, b));
        result[g].call_sites += calls.len();
        result[g].edges.push(Edge { from, to, calls });
    }
    for g in &mut result {
        g.witness = witness(g);
    }
    result.sort_by(|a, b| a.members[0].cmp(&b.members[0]));
    result
}

/// One closed path through the group, by breadth-first search from the
/// first member (prefers a step to another member over a self-loop).
fn witness(g: &Group) -> Vec<String> {
    let start = g.members[0].clone();
    if g.members.len() == 1 {
        return vec![start.clone(), start];
    }
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &g.edges {
        adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
    }
    let succ = |id: &str| adj.get(id).cloned().unwrap_or_default();
    let Some(next) = succ(&start).into_iter().find(|id| *id != start) else {
        return vec![start.clone(), start];
    };
    let mut parents: HashMap<&str, Option<&str>> = HashMap::from([(next, None)]);
    let mut queue = VecDeque::from([next]);
    while let Some(cur) = queue.pop_front() {
        if parents.contains_key(start.as_str()) {
            break;
        }
        for t in succ(cur) {
            if parents.contains_key(t) {
                continue;
            }
            parents.insert(t, Some(cur));
            queue.push_back(t);
            if t == start {
                break;
            }
        }
    }
    let mut path = Vec::new();
    let mut id = Some(start.as_str());
    while let Some(cur) = id {
        path.push(cur.to_string());
        id = parents.get(cur).copied().flatten();
    }
    path.reverse();
    std::iter::once(start).chain(path).collect()
}

/// The groups a filter selects: whole groups with one matching member.
pub fn matching(groups: &[Group], idx: &Index, f: &Filter, l: &Labels) -> Vec<usize> {
    let q = f.query.trim().to_lowercase();
    groups
        .iter()
        .enumerate()
        .filter(|(_, g)| {
            g.members.iter().any(|id| {
                let class_id = match g.kind {
                    ElementKind::Method => idx.method(Some(id)).map(|m| m.class_id.as_str()),
                    ElementKind::Class => Some(id.as_str()),
                };
                let Some(cls) = idx.class(class_id) else {
                    return false;
                };
                idx.in_language(cls, f.language.as_deref())
                    && f.package.as_ref().is_none_or(|p| cls.package == *p)
                    && (q.is_empty()
                        || format!(
                            "{} {} {}",
                            idx.qualified(g.kind, id, l),
                            cls.package,
                            cls.file
                        )
                        .to_lowercase()
                        .contains(&q))
            })
        })
        .map(|(i, _)| i)
        .collect()
}

/// The group's internal connections as relations.
pub fn relations(g: &Group, idx: &Index, l: &Labels) -> Vec<Relation> {
    let kind = if g.kind == ElementKind::Class {
        "class"
    } else {
        "method"
    };
    let owner = |id: &str| match g.kind {
        ElementKind::Class => id.to_string(),
        ElementKind::Method => idx
            .method(Some(id))
            .map(|m| m.class_id.clone())
            .unwrap_or_default(),
    };
    g.edges
        .iter()
        .map(|e| Relation {
            key: json_list(["cycle", kind, &e.from, &e.to]),
            from: owner(&e.from),
            to: Some(owner(&e.to)),
            label: idx.qualified(g.kind, &e.to, l),
            resolution: "project".into(),
            calls: e.calls.clone(),
            cycle: Some((e.from.clone(), e.to.clone())),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::model::fixture;
    use super::*;

    #[test]
    fn finds_class_and_method_groups_with_a_closed_path() {
        let idx = Index::new(fixture::analysis());
        let m = compute(&idx);
        assert_eq!(m.classes.len(), 1);
        let g = &m.classes[0];
        assert_eq!(g.members, vec!["blue.Blue", "red.Red"]);
        assert_eq!(g.edges.len(), 2);
        assert_eq!(g.call_sites, 3);
        assert_eq!(g.witness, vec!["blue.Blue", "red.Red", "blue.Blue"]);
        assert_eq!(g.id, r#"["class",["blue.Blue","red.Red"]]"#);
        // name() ⇄ describe(), and name() calls itself: one group.
        assert_eq!(m.methods.len(), 1);
        assert_eq!(m.methods[0].members.len(), 2);
        assert_eq!(m.methods[0].edges.len(), 3);
        assert_eq!(m.unresolved_targets, 1);
        // <init> is undeclared; the unresolved site is not a project call.
        assert_eq!(m.missing_methods, 1);
    }

    #[test]
    fn a_self_call_alone_is_a_recursion_group() {
        let mut a = fixture::analysis();
        a.calls.retain(|c| c.line == 12);
        let m = compute(&Index::new(a));
        assert!(m.classes.is_empty());
        assert_eq!(m.methods.len(), 1);
        assert_eq!(m.methods[0].witness, vec!["blue.Blue#name()"; 2]);
    }

    #[test]
    fn filters_select_whole_groups() {
        let idx = Index::new(fixture::analysis());
        let m = compute(&idx);
        let l = Labels::default();
        let red = Filter {
            package: Some("red".into()),
            ..Default::default()
        };
        assert_eq!(matching(&m.classes, &idx, &red, &l).len(), 1);
        let green = Filter {
            package: Some("green".into()),
            ..Default::default()
        };
        assert!(matching(&m.classes, &idx, &green, &l).is_empty());
        let rels = relations(&m.classes[0], &idx, &l);
        assert_eq!(rels[0].from, "blue.Blue");
        assert_eq!(rels[0].calls.len(), 2);
    }
}
