//! Port of the web viewer's `packageTreeConnectionsOrder`
//! (`viewer_tree_layout.js`): reorder whole sibling subtrees, never single
//! descendants across parents, so that tree links cross less and stay
//! short. The search is bounded and deterministic.

use std::collections::{BTreeMap, BTreeSet};

/// Dependencies evaluated at most; larger scopes use an evenly spaced
/// sample (the budget bounds work, not the links eventually drawn).
const SAMPLE: usize = 256;

/// Siblings per parent (`None` = the forest root), already in their
/// initial (alphabetical) order. Every listed node is visible.
pub type Children = BTreeMap<Option<usize>, Vec<usize>>;

/// The web's full "By dependencies" order: construction levels first,
/// then the bounded crossing search. `children` starts alphabetical;
/// `rank` is each node's alphabetical position (the web's `compareId`),
/// `parent` the visible parent of a node.
pub fn order(
    children: &mut Children,
    deps: &[(usize, usize)],
    rank: &BTreeMap<usize, usize>,
    parent: &BTreeMap<usize, usize>,
) {
    let alphabetical = children.clone();
    construction_order(children, deps, rank, parent);
    reduce_crossings(children, &alphabetical, deps, rank);
}

/// Branch path of `n` from its outermost visible ancestor down to `n`.
fn path(n: usize, parent: &BTreeMap<usize, usize>) -> Vec<usize> {
    let mut out = vec![n];
    let mut cur = n;
    while let Some(&p) = parent.get(&cur) {
        if out.contains(&p) {
            break;
        }
        out.push(p);
        cur = p;
    }
    out.reverse();
    out
}

/// Each dependency orders the two branches below its lowest common
/// ancestor (`a`, `b` under `parent`); nested pairs are skipped.
fn group_edges(
    deps: &[(usize, usize)],
    parent: &BTreeMap<usize, usize>,
) -> BTreeMap<Option<usize>, Vec<(usize, usize)>> {
    let mut out: BTreeMap<Option<usize>, Vec<(usize, usize)>> = BTreeMap::new();
    for &(from, to) in deps {
        let (pf, pt) = (path(from, parent), path(to, parent));
        let i = pf.iter().zip(&pt).take_while(|(a, b)| a == b).count();
        let (key, a, b) = if from == to {
            (parent.get(&from).copied(), from, from)
        } else if i == pf.len() || i == pt.len() {
            continue;
        } else {
            (if i > 0 { Some(pf[i - 1]) } else { None }, pf[i], pt[i])
        };
        out.entry(key).or_default().push((a, b));
    }
    out
}

/// Siblings in dependency construction order: strongly connected groups
/// (Kosaraju), dependents before their dependencies by longest path,
/// ties alphabetical — the web viewer's sibling order before searching.
fn construction_order(
    children: &mut Children,
    deps: &[(usize, usize)],
    rank: &BTreeMap<usize, usize>,
    parent: &BTreeMap<usize, usize>,
) {
    let edges = group_edges(deps, parent);
    for (key, siblings) in children.iter_mut() {
        let pairs = edges.get(key).map(Vec::as_slice).unwrap_or(&[]);
        *siblings = level_order(siblings, pairs, rank);
    }
}

fn level_order(
    siblings: &[usize],
    pairs: &[(usize, usize)],
    rank: &BTreeMap<usize, usize>,
) -> Vec<usize> {
    let member: BTreeSet<usize> = siblings.iter().copied().collect();
    let mut adjacent: BTreeMap<usize, BTreeSet<usize>> =
        siblings.iter().map(|&s| (s, BTreeSet::new())).collect();
    let mut reverse = adjacent.clone();
    for &(a, b) in pairs {
        if member.contains(&a) && member.contains(&b) {
            adjacent.get_mut(&a).map(|s| s.insert(b));
            reverse.get_mut(&b).map(|s| s.insert(a));
        }
    }
    let by_rank = |set: &BTreeSet<usize>| {
        let mut v: Vec<usize> = set.iter().copied().collect();
        v.sort_by_key(|n| rank[n]);
        v
    };
    let finish = kosaraju_finish(siblings, &adjacent, &by_rank);
    let (groups, assigned) = kosaraju_groups(&finish, &reverse, rank);
    let mut outgoing: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); groups.len()];
    let mut incoming = vec![0usize; groups.len()];
    for (&a, targets) in &adjacent {
        for b in targets {
            let (ga, gb) = (assigned[&a], assigned[b]);
            if ga != gb && outgoing[ga].insert(gb) {
                incoming[gb] += 1;
            }
        }
    }
    let first = |g: usize| rank[&groups[g][0]];
    let mut ready: Vec<usize> = (0..groups.len()).filter(|&g| incoming[g] == 0).collect();
    let mut ordered = Vec::new();
    while !ready.is_empty() {
        ready.sort_by_key(|&g| first(g));
        let g = ready.remove(0);
        ordered.push(g);
        for &next in &outgoing[g] {
            incoming[next] -= 1;
            if incoming[next] == 0 {
                ready.push(next);
            }
        }
    }
    let mut levels = vec![0usize; groups.len()];
    for &g in ordered.iter().rev() {
        for &next in &outgoing[g] {
            levels[g] = levels[g].max(levels[next] + 1);
        }
    }
    ordered.sort_by(|&a, &b| levels[b].cmp(&levels[a]).then(first(a).cmp(&first(b))));
    ordered
        .into_iter()
        .flat_map(|g| groups[g].clone())
        .collect()
}

/// First Kosaraju pass: finish order of an iterative DFS, neighbours in
/// alphabetical order.
fn kosaraju_finish(
    siblings: &[usize],
    adjacent: &BTreeMap<usize, BTreeSet<usize>>,
    by_rank: &dyn Fn(&BTreeSet<usize>) -> Vec<usize>,
) -> Vec<usize> {
    let mut visited = BTreeSet::new();
    let mut finish = Vec::new();
    for &start in siblings {
        if !visited.insert(start) {
            continue;
        }
        let mut stack = vec![(start, by_rank(&adjacent[&start]), 0usize)];
        while let Some((id, neighbors, next)) = stack.last_mut() {
            if *next == neighbors.len() {
                finish.push(*id);
                stack.pop();
                continue;
            }
            let n = neighbors[*next];
            *next += 1;
            if visited.insert(n) {
                stack.push((n, by_rank(&adjacent[&n]), 0));
            }
        }
    }
    finish
}

/// Second Kosaraju pass over the reverse graph; members sorted.
fn kosaraju_groups(
    finish: &[usize],
    reverse: &BTreeMap<usize, BTreeSet<usize>>,
    rank: &BTreeMap<usize, usize>,
) -> (Vec<Vec<usize>>, BTreeMap<usize, usize>) {
    let mut assigned = BTreeMap::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for &start in finish.iter().rev() {
        if assigned.contains_key(&start) {
            continue;
        }
        let gid = groups.len();
        let (mut group, mut pending) = (Vec::new(), vec![start]);
        assigned.insert(start, gid);
        while let Some(id) = pending.pop() {
            group.push(id);
            for &next in &reverse[&id] {
                if let std::collections::btree_map::Entry::Vacant(e) = assigned.entry(next) {
                    e.insert(gid);
                    pending.push(next);
                }
            }
        }
        group.sort_by_key(|n| rank[n]);
        groups.push(group);
    }
    (groups, assigned)
}

/// The bounded crossing search; keeps the alphabetical order instead
/// when that already scores better than the construction order.
fn reduce_crossings(
    children: &mut Children,
    alphabetical: &Children,
    deps: &[(usize, usize)],
    rank: &BTreeMap<usize, usize>,
) {
    if deps.is_empty() || !children.values().any(|s| s.len() > 1) {
        return;
    }
    let sample: Vec<(usize, usize)> = if deps.len() <= SAMPLE {
        deps.to_vec()
    } else {
        (0..SAMPLE).map(|i| deps[i * deps.len() / SAMPLE]).collect()
    };
    let scorer = Scorer::new(&sample, rank);
    let mut best = scorer.score(children);
    let alpha = scorer.score(alphabetical);
    if alpha < best {
        best = alpha;
        *children = alphabetical.clone();
    }
    let mut groups: Vec<Option<usize>> = children
        .iter()
        .filter(|(_, s)| s.len() > 1)
        .map(|(&p, _)| p)
        .collect();
    groups.sort_by_key(|p| rank[&children[p][0]]);
    let mut budget = (250_000 / rank.len().max(1)).clamp(32, 512);
    for _ in 0..4 {
        if budget == 0 {
            break;
        }
        let improved = pass(children, &groups, &scorer, &mut best, &mut budget);
        if !improved {
            break;
        }
    }
}

/// One improvement pass: try swapping siblings `gap` apart (gap < 9) and
/// keep a swap only when it lowers the score.
fn pass(
    children: &mut Children,
    groups: &[Option<usize>],
    scorer: &Scorer,
    best: &mut u64,
    budget: &mut usize,
) -> bool {
    let mut improved = false;
    for parent in groups {
        let len = children[parent].len();
        for gap in 1..len.min(9) {
            for i in 0..len - gap {
                if *budget == 0 {
                    return improved;
                }
                swap(children, parent, i, i + gap);
                let candidate = scorer.score(children);
                *budget -= 1;
                if candidate < *best {
                    *best = candidate;
                    improved = true;
                } else {
                    swap(children, parent, i, i + gap);
                }
            }
        }
    }
    improved
}

fn swap(children: &mut Children, parent: &Option<usize>, i: usize, j: usize) {
    if let Some(s) = children.get_mut(parent) {
        s.swap(i, j);
    }
}

/// Preorder of the forest under the current sibling orders.
pub fn preorder(children: &Children) -> Vec<usize> {
    let mut out = Vec::new();
    let mut pending: Vec<usize> = children
        .get(&None)
        .map(|r| r.iter().rev().copied().collect())
        .unwrap_or_default();
    while let Some(n) = pending.pop() {
        out.push(n);
        if let Some(c) = children.get(&Some(n)) {
            pending.extend(c.iter().rev().copied());
        }
    }
    out
}

struct Scorer {
    /// Endpoints of the sampled links, as node ids.
    endpoints: Vec<(usize, usize)>,
    /// Index pairs of sampled links with four distinct endpoints.
    pairs: Vec<(usize, usize)>,
    scale: u64,
}

impl Scorer {
    fn new(sample: &[(usize, usize)], rank: &BTreeMap<usize, usize>) -> Self {
        let mut pairs = Vec::new();
        for i in 0..sample.len() {
            for j in i + 1..sample.len() {
                let ((a, b), (c, d)) = (sample[i], sample[j]);
                if a != c && a != d && b != c && b != d {
                    pairs.push((i, j));
                }
            }
        }
        Scorer {
            endpoints: sample.to_vec(),
            pairs,
            scale: (rank.len() * sample.len() + 1) as u64,
        }
    }

    /// Crossings dominate, total link length breaks ties.
    fn score(&self, children: &Children) -> u64 {
        let position: BTreeMap<usize, i64> = preorder(children)
            .into_iter()
            .enumerate()
            .map(|(i, n)| (n, i as i64))
            .collect();
        let span = |(a, b): (usize, usize)| {
            let (x, y) = (position[&a], position[&b]);
            (x.min(y), x.max(y))
        };
        let crossings = self
            .pairs
            .iter()
            .filter(|&&(i, j)| {
                let ((lo, hi), (olo, ohi)) = (span(self.endpoints[i]), span(self.endpoints[j]));
                (lo < olo && olo < hi && hi < ohi) || (olo < lo && lo < ohi && ohi < hi)
            })
            .count() as u64;
        let length: u64 = self
            .endpoints
            .iter()
            .map(|&e| {
                let (lo, hi) = span(e);
                (hi - lo) as u64
            })
            .sum();
        crossings * self.scale + length
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preorder_follows_sibling_lists() {
        let mut c = Children::new();
        c.insert(None, vec![0, 3]);
        c.insert(Some(0), vec![1, 2]);
        assert_eq!(preorder(&c), [0, 1, 2, 3]);
    }

    #[test]
    fn a_swap_that_removes_a_crossing_is_kept() {
        // Roots 0..4; links 0→2 and 1→3 cross; swapping 1 and 2 fixes it.
        let mut c = Children::new();
        c.insert(None, vec![0, 1, 2, 3]);
        let rank: BTreeMap<usize, usize> = (0..4).map(|i| (i, i)).collect();
        order(&mut c, &[(0, 2), (1, 3)], &rank, &BTreeMap::new());
        let p = preorder(&c);
        let pos = |n| p.iter().position(|&x| x == n).unwrap();
        let (a, b) = ((pos(0), pos(2)), (pos(1), pos(3)));
        let cross = |(lo, hi): (usize, usize), (olo, ohi): (usize, usize)| {
            let (lo, hi) = (lo.min(hi), lo.max(hi));
            let (olo, ohi) = (olo.min(ohi), olo.max(ohi));
            (lo < olo && olo < hi && hi < ohi) || (olo < lo && lo < ohi && ohi < hi)
        };
        assert!(!cross(a, b), "{p:?}");
    }

    #[test]
    fn nothing_to_do_without_links() {
        let mut c = Children::new();
        c.insert(None, vec![2, 0, 1]);
        let rank: BTreeMap<usize, usize> = [(2, 0), (0, 1), (1, 2)].into_iter().collect();
        order(&mut c, &[], &rank, &BTreeMap::new());
        assert_eq!(c[&None], [2, 0, 1]);
    }
}
