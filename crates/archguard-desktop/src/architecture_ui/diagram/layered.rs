//! Rows and order of a layered package map, after Graphviz `dot`, which the
//! web viewer uses: cycles are broken by turning back edges upwards, every
//! card gets a row below the cards it is used by, long edges get bend
//! points in the rows they cross, and barycentre sweeps reduce crossings.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Cards per row; further cards move one row down.
pub const ROW_CARDS: usize = 4;
const SWEEPS: usize = 12;

/// Rows of slots. Slot `i < nodes` is card `i`; the others are bend points.
#[derive(Debug, Clone)]
pub struct Grid {
    pub nodes: usize,
    /// Slot ids per row, left to right.
    pub rows: Vec<Vec<usize>>,
    pub row_of: Vec<usize>,
    /// Per input edge: its slots top to bottom, empty for a loop.
    pub chains: Vec<Chain>,
}

#[derive(Debug, Clone, Default)]
pub struct Chain {
    pub slots: Vec<usize>,
    /// The edge points upwards (a broken cycle).
    pub up: bool,
}

pub fn grid(nodes: usize, edges: &[(usize, usize)]) -> Grid {
    let valid: Vec<bool> = edges
        .iter()
        .map(|&(a, b)| a < nodes && b < nodes && a != b)
        .collect();
    let up = back_edges(nodes, edges, &valid);
    let down: Vec<Option<(usize, usize)>> = edges
        .iter()
        .enumerate()
        .map(|(i, &(a, b))| valid[i].then_some(if up[i] { (b, a) } else { (a, b) }))
        .collect();
    let rank = ranks(nodes, &down);
    let mut row_of = rank;
    let chains = down
        .iter()
        .zip(&up)
        .map(|(d, &up)| d.map_or_else(Chain::default, |(a, b)| chain(a, b, up, &mut row_of)))
        .collect();
    let mut g = Grid {
        nodes,
        rows: Vec::new(),
        row_of,
        chains,
    };
    g.rows = order(&g);
    g
}

/// Edges that close a cycle in a depth-first search started at the sources.
fn back_edges(nodes: usize, edges: &[(usize, usize)], valid: &[bool]) -> Vec<bool> {
    let mut out: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nodes];
    let mut has_in = vec![false; nodes];
    for (i, &(a, b)) in edges.iter().enumerate().filter(|(i, _)| valid[*i]) {
        out[a].push((b, i));
        has_in[b] = true;
    }
    // 0 new, 1 on the stack, 2 done.
    let mut state = vec![0u8; nodes];
    let mut back = vec![false; edges.len()];
    let roots = (0..nodes).filter(|&n| !has_in[n]).chain(0..nodes);
    for root in roots {
        if state[root] != 0 {
            continue;
        }
        state[root] = 1;
        let mut stack = vec![(root, 0usize)];
        while let Some((n, next)) = stack.last_mut() {
            let n = *n;
            let Some(&(to, e)) = out[n].get(*next) else {
                state[n] = 2;
                stack.pop();
                continue;
            };
            *next += 1;
            match state[to] {
                0 => {
                    state[to] = 1;
                    stack.push((to, 0));
                }
                1 => back[e] = true,
                _ => {}
            }
        }
    }
    back
}

/// Longest-path rows in topological order; a full row pushes a card down.
fn ranks(nodes: usize, down: &[Option<(usize, usize)>]) -> Vec<usize> {
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    for &(a, b) in down.iter().flatten() {
        preds[b].push(a);
        succs[a].push(b);
    }
    let lonely = |n: usize| preds[n].is_empty() && succs[n].is_empty();
    let mut waiting: Vec<usize> = preds.iter().map(Vec::len).collect();
    let mut ready: BinaryHeap<Reverse<(bool, usize)>> = (0..nodes)
        .filter(|&n| waiting[n] == 0)
        .map(|n| Reverse((lonely(n), n)))
        .collect();
    let mut rank = vec![0usize; nodes];
    let mut filled: Vec<usize> = Vec::new();
    while let Some(Reverse((_, n))) = ready.pop() {
        let mut r = preds[n].iter().map(|&p| rank[p] + 1).max().unwrap_or(0);
        while filled.get(r).is_some_and(|&f| f >= ROW_CARDS) {
            r += 1;
        }
        if filled.len() <= r {
            filled.resize(r + 1, 0);
        }
        filled[r] += 1;
        rank[n] = r;
        for &s in &succs[n] {
            waiting[s] -= 1;
            if waiting[s] == 0 {
                ready.push(Reverse((lonely(s), s)));
            }
        }
    }
    rank
}

/// Slots from `a` down to `b`, with a new bend point in every row between.
fn chain(a: usize, b: usize, up: bool, row_of: &mut Vec<usize>) -> Chain {
    let mut slots = vec![a];
    for r in row_of[a] + 1..row_of[b] {
        slots.push(row_of.len());
        row_of.push(r);
    }
    slots.push(b);
    Chain { slots, up }
}

/// Neighbours of every slot in the row above and below.
pub fn neighbours(g: &Grid) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let mut above = vec![Vec::new(); g.row_of.len()];
    let mut below = vec![Vec::new(); g.row_of.len()];
    for c in &g.chains {
        for w in c.slots.windows(2) {
            below[w[0]].push(w[1]);
            above[w[1]].push(w[0]);
        }
    }
    (above, below)
}

/// Barycentre sweeps down and up; keeps the order with the fewest crossings.
fn order(g: &Grid) -> Vec<Vec<usize>> {
    let height = g.row_of.iter().max().map_or(0, |r| r + 1);
    let mut rows = vec![Vec::new(); height];
    for (slot, &r) in g.row_of.iter().enumerate() {
        rows[r].push(slot);
    }
    let (above, below) = neighbours(g);
    let mut pos = positions(&rows, g.row_of.len());
    let mut best = (crossings(&rows, &below, &pos), rows.clone());
    for _ in 0..SWEEPS {
        for row in rows.iter_mut().skip(1) {
            sort_row(row, &above, &mut pos);
        }
        for row in rows.iter_mut().rev().skip(1) {
            sort_row(row, &below, &mut pos);
        }
        let c = crossings(&rows, &below, &pos);
        if c < best.0 {
            best = (c, rows.clone());
        }
    }
    best.1
}

fn positions(rows: &[Vec<usize>], slots: usize) -> Vec<usize> {
    let mut pos = vec![0; slots];
    for row in rows {
        for (i, &s) in row.iter().enumerate() {
            pos[s] = i;
        }
    }
    pos
}

fn sort_row(row: &mut [usize], by: &[Vec<usize>], pos: &mut [usize]) {
    let key = |s: usize, pos: &[usize]| -> f32 {
        if by[s].is_empty() {
            pos[s] as f32
        } else {
            by[s].iter().map(|&n| pos[n] as f32).sum::<f32>() / by[s].len() as f32
        }
    };
    let keys: Vec<(f32, usize)> = row.iter().map(|&s| (key(s, pos), s)).collect();
    let mut sorted = keys;
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (i, (_, s)) in sorted.into_iter().enumerate() {
        row[i] = s;
        pos[s] = i;
    }
}

fn crossings(rows: &[Vec<usize>], below: &[Vec<usize>], pos: &[usize]) -> usize {
    rows.iter()
        .map(|row| {
            let segs: Vec<(usize, usize)> = row
                .iter()
                .flat_map(|&s| below[s].iter().map(move |&t| (pos[s], pos[t])))
                .collect();
            let mut n = 0;
            for (i, a) in segs.iter().enumerate() {
                n += segs[i + 1..]
                    .iter()
                    .filter(|b| (a.0 < b.0 && a.1 > b.1) || (a.0 > b.0 && a.1 < b.1))
                    .count();
            }
            n
        })
        .sum()
}
