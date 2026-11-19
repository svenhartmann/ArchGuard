//! Repair plans for one cycle: up to [`MAX_PLANS`] sets of dependencies
//! whose removal breaks the cycles among the selected members, a port of
//! the former web viewer's `planCycleRepairs` (cycle_planner.js).
//!
//! The members' containment subtrees form the scope; every graph edge
//! between two members' subtrees is bundled into one scope edge carrying
//! its evidence (affected files, reference total, whether the total is a
//! lower bound). The search branches on one deterministic witness cycle at
//! a time: every feedback-edge set has to remove an edge of each cycle.
//! Greedy seeds give complete alternatives early, a bounded best-first
//! search ranked by (affected files, references, cuts, key) follows.
//! `max_examined` bounds the number of evaluated cut sets; when it is
//! reached the result says `search.complete = false`.
//!
//! The plans are hypotheses for source changes, not refactorings: each
//! [`SimOp::CutEdge`] still needs a code change. Deterministic (Kap. 12).

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use archguard_core::{ArchguardError, Edge, Graph, Node, NodeId, NodeKind, Ref};
use serde::Serialize;

use crate::scc::{Scc, find_sccs};
use crate::simulate::SimOp;

/// The viewer's default search budget.
pub const DEFAULT_MAX_EXAMINED: usize = 2048;
/// Upper bound of the search budget.
const MAX_EXAMINED: usize = 8192;
/// Plans returned at most.
pub const MAX_PLANS: usize = 3;
/// Evidence records listed per plan.
const MAX_EVIDENCE: usize = 30;

/// SCC figures of a graph.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RepairStats {
    pub groups: usize,
    pub cyclic_nodes: usize,
    pub cyclic_edges: usize,
    pub largest_scc: usize,
}

/// How far the search went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RepairSearch {
    /// Every candidate was examined; `false` when the budget ran out.
    pub complete: bool,
    pub examined: usize,
    pub limit: usize,
}

/// One set of dependencies to remove.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepairPlan {
    /// The cut scope edges' indices, comma-separated.
    pub key: String,
    /// One cut per removed dependency, between scope members.
    pub ops: Vec<SimOp>,
    /// References behind each op, in `ops` order (not in the viewer).
    pub op_references: Vec<u64>,
    /// The scope has no cycle left.
    pub complete: bool,
    /// Scope figures after the cuts.
    pub after: RepairStats,
    /// Figures of the whole source graph after the cuts.
    pub source_after: RepairStats,
    pub affected_files: Vec<String>,
    pub reference_count: u64,
    /// Some removed edge has no exact total.
    pub reference_lower_bound: bool,
    /// The first [`MAX_EVIDENCE`] distinct references, by file, line, raw.
    pub evidence: Vec<Ref>,
    pub evidence_truncated: bool,
    /// Source graph edges removed.
    pub removed_edges: usize,
}

/// The plans for one cycle.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepairPlans {
    /// The members, deduplicated and sorted.
    pub scope_names: Vec<String>,
    pub before: RepairStats,
    pub source_before: RepairStats,
    pub plans: Vec<RepairPlan>,
    /// The ranking used a cyclic scope edge without an exact total.
    pub ranking_lower_bound: bool,
    pub search: RepairSearch,
}

/// Up to [`MAX_PLANS`] plans that break the cycles among `members` of
/// `graph`, best first. `max_examined` is clamped to `1..=8192`. Fails
/// when a member is unknown or lies inside another member.
pub fn plan_cycle_repairs(
    graph: &Graph,
    members: &[String],
    max_examined: usize,
) -> Result<RepairPlans, ArchguardError> {
    let limit = max_examined.clamp(1, MAX_EXAMINED);
    let scope_names: Vec<String> = members
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let owner = owners(graph, &scope_names)?;
    let edges = bundles(graph, &owner);
    let mut search = Search {
        scope: &scope_names,
        edges: &edges,
        limit,
        examined: 0,
        evaluated: BTreeMap::new(),
        solutions: BTreeMap::new(),
    };
    let all: Vec<usize> = (0..edges.len()).collect();
    let sccs = search.sccs(&all);
    let before = stats(&sccs);
    let mut result = RepairPlans {
        before,
        source_before: source_stats(graph, &BTreeSet::new()),
        plans: Vec::new(),
        ranking_lower_bound: false,
        search: RepairSearch {
            complete: before.cyclic_nodes == 0,
            examined: 0,
            limit,
        },
        scope_names: scope_names.clone(),
    };
    if before.cyclic_nodes == 0 {
        return Ok(result);
    }
    let cyclic: BTreeSet<(usize, usize)> = sccs
        .iter()
        .flat_map(|c| c.edges.iter().map(|(a, b)| (a.0 as usize, b.0 as usize)))
        .collect();
    result.ranking_lower_bound = edges
        .iter()
        .any(|e| e.lower_bound && cyclic.contains(&(e.from, e.to)));
    let initial = search
        .evaluate(search.state_for(Vec::new()))
        .expect("the budget is at least 1");
    search.greedy(&initial);
    result.search.complete = search.best_first(initial);
    result.search.examined = search.examined;
    result.plans = search
        .ranked(&before)
        .iter()
        .map(|state| plan(graph, &scope_names, &edges, state))
        .collect();
    Ok(result)
}

/// Each node in a member's subtree → the member's index in `scope`.
fn owners(graph: &Graph, scope: &[String]) -> Result<BTreeMap<NodeId, usize>, ArchguardError> {
    let mut children: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for node in &graph.nodes {
        if let Some(parent) = node.parent {
            children.entry(parent).or_default().push(node.id);
        }
    }
    let mut owner = BTreeMap::new();
    for (index, name) in scope.iter().enumerate() {
        let node = graph
            .nodes
            .iter()
            .rfind(|n| &n.name == name)
            .ok_or_else(|| ArchguardError::Analysis(format!("Cycle member not found: {name}")))?;
        let mut pending = vec![node.id];
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            if owner.insert(id, index).is_some() {
                let message = format!("Cycle member scopes overlap: {name}");
                return Err(ArchguardError::Analysis(message));
            }
            pending.extend(children.get(&id).into_iter().flatten());
        }
    }
    Ok(owner)
}

/// The graph edges between two members' subtrees, one bundle per member
/// pair, by (from, to).
fn bundles(graph: &Graph, owner: &BTreeMap<NodeId, usize>) -> Vec<Bundle> {
    let mut pairs: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (index, edge) in graph.edges.iter().enumerate() {
        if let (Some(&from), Some(&to)) = (owner.get(&edge.from), owner.get(&edge.to))
            && from != to
        {
            pairs.entry((from, to)).or_default().push(index);
        }
    }
    pairs
        .into_iter()
        .map(|((from, to), sources)| {
            let edges = || sources.iter().map(|&i| &graph.edges[i]);
            let files: BTreeSet<String> = edges().flat_map(|e| source_files(graph, e)).collect();
            Bundle {
                from,
                to,
                files: files.into_iter().collect(),
                references: edges().map(Edge::weight).sum(),
                lower_bound: edges().any(|e| e.count.is_none()),
                sources,
            }
        })
        .collect()
}

/// The files an edge's references sit in; the importer without references.
fn source_files(graph: &Graph, edge: &Edge) -> Vec<String> {
    if edge.refs.is_empty() {
        return vec![graph.node_name(edge.from).unwrap_or("").to_string()];
    }
    edge.refs.iter().map(|r| r.file.clone()).collect()
}

/// Figures of the components `sccs`.
fn stats(sccs: &[Scc]) -> RepairStats {
    RepairStats {
        groups: sccs.len(),
        cyclic_nodes: sccs.iter().map(|c| c.node_ids.len()).sum(),
        cyclic_edges: sccs.iter().map(|c| c.edges.len()).sum(),
        largest_scc: sccs.iter().map(|c| c.node_ids.len()).max().unwrap_or(0),
    }
}

/// Figures of `graph` without the edges at `removed`.
fn source_stats(graph: &Graph, removed: &BTreeSet<usize>) -> RepairStats {
    let edges = graph.edges.iter().enumerate();
    let without = Graph {
        schema_version: graph.schema_version,
        root: String::new(),
        nodes: graph.nodes.clone(),
        edges: edges
            .filter(|(i, _)| !removed.contains(i))
            .map(|(_, e)| Edge {
                from: e.from,
                to: e.to,
                refs: Vec::new(),
                count: None,
            })
            .collect(),
        semantic: None,
        review_kpis: None,
    };
    stats(&find_sccs(&without))
}

/// The output for a ranked `state`.
fn plan(graph: &Graph, scope: &[String], edges: &[Bundle], state: &State) -> RepairPlan {
    let removed: BTreeSet<usize> = state
        .cuts
        .iter()
        .flat_map(|&i| edges[i].sources.iter().copied())
        .collect();
    let removed_edges = || removed.iter().map(|&i| &graph.edges[i]);
    let evidence: BTreeSet<(&str, u32, &str)> = removed_edges()
        .flat_map(|e| &e.refs)
        .map(|r| (r.file.as_str(), r.line, r.raw.as_str()))
        .collect();
    let evidence_truncated = evidence.len() > MAX_EVIDENCE
        || removed_edges().any(|e| e.count.is_none_or(|c| c > e.refs.len() as u64));
    RepairPlan {
        key: state.key.clone(),
        ops: state
            .cuts
            .iter()
            .map(|&i| SimOp::CutEdge {
                from: scope[edges[i].from].clone(),
                to: scope[edges[i].to].clone(),
            })
            .collect(),
        op_references: state.cuts.iter().map(|&i| edges[i].references).collect(),
        complete: state.after.cyclic_nodes == 0,
        after: state.after,
        source_after: source_stats(graph, &removed),
        affected_files: state.files.clone(),
        reference_count: state.references,
        reference_lower_bound: state.cuts.iter().any(|&i| edges[i].lower_bound),
        evidence: evidence
            .into_iter()
            .take(MAX_EVIDENCE)
            .map(|(file, line, raw)| Ref {
                file: file.into(),
                line,
                raw: raw.into(),
            })
            .collect(),
        evidence_truncated,
        removed_edges: removed.len(),
    }
}

/// The graph edges from member `from`'s subtree to member `to`'s.
struct Bundle {
    from: usize,
    to: usize,
    /// Indices into the graph's edges.
    sources: Vec<usize>,
    files: Vec<String>,
    references: u64,
    lower_bound: bool,
}

/// A set of cut bundles; `after` and `witness` are set once evaluated.
#[derive(Clone)]
struct State {
    /// Bundle indices, ascending.
    cuts: Vec<usize>,
    key: String,
    files: Vec<String>,
    references: u64,
    after: RepairStats,
    /// Bundles of one remaining cycle; empty when none is left.
    witness: Vec<usize>,
}

/// Cheapest first: affected files, references, cuts, key (as text, as the
/// viewer compared it).
fn compare_cost(a: &State, b: &State) -> Ordering {
    (a.files.len(), a.references, a.cuts.len(), &a.key).cmp(&(
        b.files.len(),
        b.references,
        b.cuts.len(),
        &b.key,
    ))
}

/// `cuts` plus `index`.
fn with(cuts: &[usize], index: usize) -> Vec<usize> {
    let mut out = cuts.to_vec();
    out.push(index);
    out
}

struct Search<'a> {
    scope: &'a [String],
    edges: &'a [Bundle],
    limit: usize,
    examined: usize,
    evaluated: BTreeMap<String, State>,
    solutions: BTreeMap<String, State>,
}

impl Search<'_> {
    fn state_for(&self, mut cuts: Vec<usize>) -> State {
        cuts.sort_unstable();
        let files: BTreeSet<&String> = cuts.iter().flat_map(|&i| &self.edges[i].files).collect();
        State {
            key: cuts
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(","),
            files: files.into_iter().cloned().collect(),
            references: cuts.iter().map(|&i| self.edges[i].references).sum(),
            cuts,
            after: RepairStats::default(),
            witness: Vec::new(),
        }
    }

    /// Cyclic components of the scope with only the bundles `remaining`.
    fn sccs(&self, remaining: &[usize]) -> Vec<Scc> {
        let node = |(id, name): (usize, &String)| Node {
            id: NodeId(id as u32),
            name: name.clone(),
            kind: NodeKind::Module,
            parent: None,
            lang: None,
        };
        let edge = |&i: &usize| Edge {
            from: NodeId(self.edges[i].from as u32),
            to: NodeId(self.edges[i].to as u32),
            refs: Vec::new(),
            count: None,
        };
        find_sccs(&Graph {
            schema_version: 1,
            root: String::new(),
            nodes: self.scope.iter().enumerate().map(node).collect(),
            edges: remaining.iter().map(edge).collect(),
            semantic: None,
            review_kpis: None,
        })
    }

    /// `state` evaluated, memoized by key; `None` once the budget is spent.
    fn evaluate(&mut self, mut state: State) -> Option<State> {
        if let Some(done) = self.evaluated.get(&state.key) {
            return Some(done.clone());
        }
        if self.examined >= self.limit {
            return None;
        }
        self.examined += 1;
        let remaining: Vec<usize> = (0..self.edges.len())
            .filter(|i| state.cuts.binary_search(i).is_err())
            .collect();
        let sccs = self.sccs(&remaining);
        state.after = stats(&sccs);
        state.witness = sccs
            .first()
            .map(|c| self.witness(c, &remaining))
            .unwrap_or_default();
        self.evaluated.insert(state.key.clone(), state.clone());
        if state.after.cyclic_nodes == 0 && !state.cuts.is_empty() {
            self.solutions.insert(state.key.clone(), state.clone());
        }
        Some(state)
    }

    /// One cycle of `component` over `remaining`: its first internal
    /// bundle closed by the shortest path back (breadth first).
    fn witness(&self, component: &Scc, remaining: &[usize]) -> Vec<usize> {
        let ids: BTreeSet<usize> = component.node_ids.iter().map(|n| n.0 as usize).collect();
        let edges = self.edges;
        let first = *remaining
            .iter()
            .find(|&&i| ids.contains(&edges[i].from) && ids.contains(&edges[i].to))
            .expect("a cyclic component has an internal edge");
        let (start, goal) = (edges[first].to, edges[first].from);
        let mut previous: BTreeMap<usize, Option<usize>> = BTreeMap::from([(start, None)]);
        let mut queue = vec![start];
        let mut at = 0;
        while at < queue.len() && !previous.contains_key(&goal) {
            for &i in remaining {
                let e = &edges[i];
                if e.from == queue[at] && ids.contains(&e.to) && !previous.contains_key(&e.to) {
                    previous.insert(e.to, Some(i));
                    queue.push(e.to);
                }
            }
            at += 1;
        }
        let mut path = vec![first];
        let mut cursor = goal;
        while cursor != start {
            let i = previous[&cursor].expect("the path is connected");
            path.push(i);
            cursor = edges[i].from;
        }
        path
    }

    /// Greedy descents from the cheapest first cuts, each pruned of cuts
    /// that another SCC pass proves redundant.
    fn greedy(&mut self, initial: &State) {
        let mut seeds: Vec<State> = initial
            .witness
            .iter()
            .map(|&i| self.state_for(vec![i]))
            .collect();
        seeds.sort_by(compare_cost);
        for seed in seeds.into_iter().take(MAX_PLANS) {
            let Some(found) = self.descend(seed) else {
                break;
            };
            self.prune(found);
        }
    }

    fn descend(&mut self, seed: State) -> Option<State> {
        let mut current = self.evaluate(seed)?;
        while !current.witness.is_empty() {
            let next = current
                .witness
                .iter()
                .map(|&i| self.state_for(with(&current.cuts, i)))
                .min_by(compare_cost)
                .expect("a witness is not empty");
            current = self.evaluate(next)?;
        }
        Some(current)
    }

    fn prune(&mut self, mut current: State) {
        for index in current.cuts.clone().into_iter().rev() {
            let fewer = current
                .cuts
                .iter()
                .copied()
                .filter(|&i| i != index)
                .collect();
            let Some(smaller) = self.evaluate(self.state_for(fewer)) else {
                break;
            };
            if smaller.after.cyclic_nodes == 0 {
                self.solutions.remove(&current.key);
                current = smaller;
            }
        }
    }

    /// Bounded best-first search over witness branches; `true` when it
    /// ran out of candidates rather than budget or queue space.
    fn best_first(&mut self, initial: State) -> bool {
        let cap = self.limit * 4;
        let mut queued = BTreeSet::from([initial.key.clone()]);
        let mut queue = vec![initial];
        let mut truncated = false;
        while !queue.is_empty() {
            queue.sort_by(compare_cost);
            let Some(current) = self.evaluate(queue.remove(0)) else {
                truncated = true;
                break;
            };
            for &i in &current.witness {
                let candidate = self.state_for(with(&current.cuts, i));
                if queued.insert(candidate.key.clone()) {
                    queue.push(candidate);
                }
            }
            if queue.len() > cap {
                queue.sort_by(compare_cost);
                queue.truncate(cap);
                truncated = true;
            }
        }
        !truncated && queue.is_empty()
    }

    /// The best [`MAX_PLANS`] solutions without a smaller subset among
    /// them; without solutions the evaluated sets that remove the most.
    fn ranked(&self, before: &RepairStats) -> Vec<State> {
        let found: Vec<&State> = self.solutions.values().collect();
        let subset = |o: &State, p: &State| o.cuts.iter().all(|i| p.cuts.contains(i));
        let mut out: Vec<State> = found
            .iter()
            .filter(|p| {
                !found
                    .iter()
                    .any(|o| o.cuts.len() < p.cuts.len() && subset(o, p))
            })
            .map(|p| (*p).clone())
            .collect();
        out.sort_by(compare_cost);
        if out.is_empty() {
            out = self
                .evaluated
                .values()
                .filter(|p| !p.cuts.is_empty() && p.after.cyclic_edges < before.cyclic_edges)
                .cloned()
                .collect();
            out.sort_by(|a, b| {
                let rest = |s: &State| (s.after.cyclic_nodes, s.after.cyclic_edges);
                rest(a).cmp(&rest(b)).then_with(|| compare_cost(a, b))
            });
        }
        out.truncate(MAX_PLANS);
        out
    }
}
