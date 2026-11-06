//! The detail column: call sites of the selected connection grouped by
//! caller and target method (renderCallEvidence) and the refactoring
//! candidates (callSuggestions in viewer.js).

use std::collections::{BTreeSet, HashMap};

use archguard_engine::core::MethodCall;

use super::cycles::Group;
use super::model::{Index, Labels, Relation, json_list};

/// Sites of one caller → target method pair.
#[derive(Debug, PartialEq, Eq)]
pub struct MethodGroup {
    pub key: String,
    /// Shown sites (within the site limit).
    pub calls: Vec<usize>,
    pub total: usize,
}

/// `callMethodPairKey`.
pub fn pair_key(c: &MethodCall) -> String {
    let to = c
        .to_method
        .as_deref()
        .filter(|m| !m.is_empty())
        .unwrap_or(&c.target_label);
    json_list([c.from_method.as_deref().unwrap_or(""), to])
}

/// Group the first `limit` sites by method pair; totals count all sites.
pub fn method_groups(r: &Relation, idx: &Index, limit: usize) -> Vec<MethodGroup> {
    let mut groups: Vec<MethodGroup> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for (n, &i) in r.calls.iter().enumerate() {
        let key = pair_key(&idx.a.calls[i]);
        let g = *at.entry(key.clone()).or_insert_with(|| {
            groups.push(MethodGroup {
                key,
                calls: Vec::new(),
                total: 0,
            });
            groups.len() - 1
        });
        groups[g].total += 1;
        if n < limit {
            groups[g].calls.push(i);
        }
    }
    groups.retain(|g| !g.calls.is_empty());
    groups
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proposal {
    Interface,
    Adapter,
    Move,
}

/// A review plan; the window words it from these values.
#[derive(Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub kind: Proposal,
    pub source: String,
    pub target: String,
    /// Interface: the called operations; Move: the target package.
    pub methods: String,
    pub cycle_size: usize,
    pub target_calls: usize,
    pub outgoing: usize,
    pub own_calls: usize,
    pub same_file: usize,
    /// Unresolved outgoing sites of the caller, else 0.
    pub unresolved: usize,
    /// Partial coverage note (only without unresolved sites).
    pub partial: bool,
    /// `file:line → target`, distinct, in site order.
    pub evidence: Vec<String>,
}

fn evidence(calls: &[&MethodCall], idx: &Index, l: &Labels) -> Vec<String> {
    let mut seen = BTreeSet::new();
    calls
        .iter()
        .map(|c| format!("{}:{} → {}", c.file, c.line, idx.target_label(c, true, l)))
        .filter(|e| seen.insert(e.clone()))
        .collect()
}

fn plain(kind: Proposal, source: &str, target: &str) -> Suggestion {
    Suggestion {
        kind,
        source: source.into(),
        target: target.into(),
        methods: String::new(),
        cycle_size: 0,
        target_calls: 0,
        outgoing: 0,
        own_calls: 0,
        same_file: 0,
        unresolved: 0,
        partial: false,
        evidence: Vec::new(),
    }
}

fn cycle_proposal(r: &Relation, idx: &Index, cycles: &[Group], l: &Labels) -> Option<Suggestion> {
    let (from, to) = (idx.class(Some(&r.from))?, idx.class(r.to.as_deref())?);
    let project = |c: &&MethodCall| {
        c.resolution == "project"
            && c.from_class == from.id
            && c.to_class.as_deref() == Some(&to.id)
    };
    let sites: Vec<&MethodCall> = idx.a.calls.iter().filter(project).collect();
    let cycle = cycles
        .iter()
        .find(|g| g.members.contains(&from.id) && g.members.contains(&to.id))?;
    let instance: Vec<&MethodCall> = sites
        .iter()
        .copied()
        .filter(|c| {
            c.dispatch == "instance"
                && !idx
                    .method(c.to_method.as_deref())
                    .is_some_and(|m| m.is_static)
        })
        .collect();
    let methods: BTreeSet<String> = instance
        .iter()
        .map(|c| idx.method_label(c.to_method.as_deref(), l))
        .collect();
    let distinct = from.kind != "module" && from.graph_node != to.graph_node;
    if distinct && to.kind == "class" && !methods.is_empty() {
        return Some(Suggestion {
            methods: methods.into_iter().collect::<Vec<_>>().join(", "),
            cycle_size: cycle.members.len(),
            evidence: evidence(&instance, idx, l),
            ..plain(Proposal::Interface, &from.name, &to.name)
        });
    }
    let static_use = sites
        .iter()
        .any(|c| c.dispatch == "static" || c.dispatch == "constructor");
    (distinct && to.kind != "module" && static_use).then(|| Suggestion {
        evidence: evidence(&sites, idx, l),
        ..plain(Proposal::Adapter, &from.name, &to.name)
    })
}

fn move_proposal(r: &Relation, idx: &Index, l: &Labels) -> Option<Suggestion> {
    let (from, to) = (idx.class(Some(&r.from))?, idx.class(r.to.as_deref())?);
    let outgoing: Vec<&MethodCall> = idx
        .a
        .calls
        .iter()
        .filter(|c| {
            c.resolution == "project"
                && c.from_class == from.id
                && c.to_class.as_deref() != Some(&from.id)
        })
        .collect();
    let package_of = |c: &&MethodCall| idx.class(c.to_class.as_deref()).map(|k| k.package.clone());
    let target: Vec<&MethodCall> = outgoing
        .iter()
        .copied()
        .filter(|c| package_of(c).as_deref() == Some(&to.package))
        .collect();
    let own = outgoing
        .iter()
        .filter(|c| package_of(c).as_deref() == Some(&from.package))
        .count();
    let n = target.len();
    let applies = !from.id.contains('@')
        && from.package != to.package
        && n >= 2
        && (n as f64) > outgoing.len() as f64 / 2.0
        && n > own;
    applies.then(|| Suggestion {
        methods: to.package.replace('/', "."),
        target_calls: n,
        outgoing: outgoing.len(),
        own_calls: own,
        same_file: idx
            .a
            .classes
            .iter()
            .filter(|c| c.graph_node == from.graph_node)
            .count(),
        evidence: evidence(&target, idx, l),
        ..plain(Proposal::Move, &from.name, &to.name)
    })
}

/// `callSuggestions`: interface, adapter/factory and move candidates.
pub fn suggestions(r: &Relation, idx: &Index, cycles: &[Group], l: &Labels) -> Vec<Suggestion> {
    if r.resolution != "project" || r.to.as_deref() == Some(&r.from) {
        return Vec::new();
    }
    let mut out: Vec<Suggestion> = cycle_proposal(r, idx, cycles, l)
        .into_iter()
        .chain(move_proposal(r, idx, l))
        .collect();
    let unresolved = idx
        .a
        .calls
        .iter()
        .filter(|c| c.from_class == r.from && c.resolution == "unresolved")
        .count();
    for s in &mut out {
        s.unresolved = unresolved;
        s.partial = unresolved == 0 && idx.a.status == "partial";
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::cycles;
    use super::super::model::{Filter, fixture, relations};
    use super::*;

    fn setup() -> (Index, Vec<Relation>) {
        let idx = Index::new(fixture::analysis());
        let rels = relations(&idx, &Filter::default(), &Labels::default());
        (idx, rels)
    }

    #[test]
    fn groups_sites_by_method_pair_within_the_limit() {
        let (idx, rels) = setup();
        let blue_red = &rels[0];
        let groups = method_groups(blue_red, &idx, 100);
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0].key,
            r#"["blue.Blue#name()","red.Red#describe()"]"#
        );
        assert_eq!(method_groups(blue_red, &idx, 1).len(), 1);
        assert_eq!(method_groups(blue_red, &idx, 1)[0].total, 1);
    }

    #[test]
    fn a_cycle_edge_with_instance_calls_suggests_an_interface() {
        let (idx, rels) = setup();
        let m = cycles::compute(&idx);
        let s = suggestions(&rels[0], &idx, &m.classes, &Labels::default());
        // Both Blue → Red sites target package red: also a move candidate.
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].kind, Proposal::Move);
        assert_eq!(s[0].kind, Proposal::Interface);
        assert_eq!(s[0].methods, "describe()");
        assert_eq!(s[0].cycle_size, 2);
        assert_eq!(s[0].evidence, vec!["blue/Blue.java:7 → red.Red#describe()"]);
        // Util → Leaf: no cycle, one call only: no candidate.
        let util = rels.iter().find(|r| r.from == "blue.Util").unwrap();
        assert!(suggestions(util, &idx, &m.classes, &Labels::default()).is_empty());
    }

    #[test]
    fn calls_into_another_package_suggest_a_move() {
        let mut a = fixture::analysis();
        let extra = fixture::call("blue.Util#hint()", Some("green.Leaf#text()"), "project", 20);
        a.calls.push(extra);
        let idx = Index::new(a);
        let rels = relations(&idx, &Filter::default(), &Labels::default());
        let util = rels.iter().find(|r| r.from == "blue.Util").unwrap();
        let s = suggestions(util, &idx, &[], &Labels::default());
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].kind, Proposal::Move);
        assert_eq!(
            (s[0].target_calls, s[0].outgoing, s[0].own_calls),
            (2, 2, 0)
        );
        assert_eq!(s[0].methods, "green");
        assert_eq!(s[0].unresolved, 1);
    }
}
