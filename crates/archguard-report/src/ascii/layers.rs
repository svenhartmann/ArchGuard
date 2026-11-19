//! The layering of a box graph: cycles broken, longest-path layers, wide
//! layers wrapped, long edges split by dummy nodes, layers ordered.

/// An edge between node positions, `up` when drawn against its direction
/// to break a cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub up: bool,
}

/// One drawn piece of an edge between two neighbouring rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// Node in the upper row, node in the lower row.
    pub upper: usize,
    pub lower: usize,
    /// Index into the edges.
    pub edge: usize,
}

/// Rows of nodes top-down; nodes from `real` on are dummies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub rows: Vec<Vec<usize>>,
    pub real: usize,
    pub segments: Vec<Segment>,
}

/// Each `(from, to)` with `up` set where a depth-first walk in node order
/// meets a node still on its path: removing those leaves no cycle.
pub fn break_cycles(n: usize, pairs: &[(usize, usize)]) -> Vec<Edge> {
    let mut adj = vec![Vec::new(); n];
    for (i, &(a, _)) in pairs.iter().enumerate() {
        adj[a].push(i);
    }
    // 0: unseen, 1: on the path, 2: done.
    let mut state = vec![0u8; n];
    let mut up = vec![false; pairs.len()];
    for start in 0..n {
        if state[start] != 0 {
            continue;
        }
        state[start] = 1;
        let mut stack = vec![(start, 0usize)];
        while let Some(top) = stack.last_mut() {
            let v = top.0;
            let Some(&e) = adj[v].get(top.1) else {
                state[v] = 2;
                stack.pop();
                continue;
            };
            top.1 += 1;
            let w = pairs[e].1;
            match state[w] {
                0 => {
                    state[w] = 1;
                    stack.push((w, 0));
                }
                1 => up[e] = true,
                _ => {}
            }
        }
    }
    pairs
        .iter()
        .zip(up)
        .map(|(&(from, to), up)| Edge { from, to, up })
        .collect()
}

/// The upper and lower end of an edge as drawn.
pub fn ends(e: &Edge) -> (usize, usize) {
    if e.up { (e.to, e.from) } else { (e.from, e.to) }
}

/// Longest-path layer of each node over the edges as drawn.
pub fn longest_path(n: usize, edges: &[Edge]) -> Vec<usize> {
    let mut layer = vec![0; n];
    // At most n rounds: every round fixes one more node of the longest path.
    for _ in 0..n {
        let mut changed = false;
        for (a, b) in edges.iter().map(ends) {
            if layer[b] < layer[a] + 1 {
                layer[b] = layer[a] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    layer
}

/// Nodes per row: each layer in node order, cut into rows whose boxes
/// (`widths`, `gap` between them) fit `budget`.
pub fn wrap(layer: &[usize], widths: &[usize], gap: usize, budget: usize) -> Vec<Vec<usize>> {
    let depth = layer.iter().max().map_or(0, |m| m + 1);
    let mut rows = Vec::new();
    for l in 0..depth {
        let mut row: Vec<usize> = Vec::new();
        let mut used = 0;
        for v in (0..layer.len()).filter(|&v| layer[v] == l) {
            let need = widths[v] + if row.is_empty() { 0 } else { gap };
            if !row.is_empty() && used + need > budget {
                rows.push(std::mem::take(&mut row));
                used = 0;
            }
            used += widths[v] + if row.is_empty() { 0 } else { gap };
            row.push(v);
        }
        rows.push(row);
    }
    rows
}

/// The rows with a dummy node wherever an edge passes a row, and the
/// segments of every edge; then each row ordered by its upper neighbours.
pub fn route(rows: Vec<Vec<usize>>, real: usize, edges: &[Edge]) -> Layout {
    let mut row_of = vec![0; real];
    for (r, row) in rows.iter().enumerate() {
        row.iter().for_each(|&v| row_of[v] = r);
    }
    let mut layout = Layout {
        rows,
        real,
        segments: Vec::new(),
    };
    let mut next = real;
    for (i, e) in edges.iter().enumerate() {
        let (a, b) = ends(e);
        let mut upper = a;
        for r in row_of[a] + 1..row_of[b] {
            layout.rows[r].push(next);
            layout.segments.push(Segment {
                upper,
                lower: next,
                edge: i,
            });
            upper = next;
            next += 1;
        }
        layout.segments.push(Segment {
            upper,
            lower: b,
            edge: i,
        });
    }
    order(&mut layout);
    layout
}

/// One top-down sweep: each node sorted by the mean position of its upper
/// neighbours (none: its own position), stable.
fn order(layout: &mut Layout) {
    for r in 1..layout.rows.len() {
        let above = &layout.rows[r - 1];
        let pos = |v: usize| above.iter().position(|&u| u == v);
        let row = &layout.rows[r];
        let keys: Vec<(usize, usize)> = row
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let ups: Vec<usize> = layout
                    .segments
                    .iter()
                    .filter(|s| s.lower == v)
                    .filter_map(|s| pos(s.upper))
                    .collect();
                // Scaled mean, so a neighbour's position and an own index compare.
                let key = if ups.is_empty() {
                    i * 100
                } else {
                    ups.iter().sum::<usize>() * 100 / ups.len()
                };
                (key, v)
            })
            .collect();
        let mut sorted = keys;
        sorted.sort_by_key(|&(k, _)| k);
        layout.rows[r] = sorted.into_iter().map(|(_, v)| v).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cycle_is_broken_at_the_edge_back_into_the_walk() {
        let edges = break_cycles(3, &[(0, 1), (1, 2), (2, 0)]);
        assert_eq!(
            edges.iter().map(|e| e.up).collect::<Vec<_>>(),
            [false, false, true]
        );
        assert_eq!(longest_path(3, &edges), [0, 1, 2]);
    }

    #[test]
    fn a_long_edge_gets_a_dummy_per_row_it_passes() {
        let edges = break_cycles(3, &[(0, 1), (1, 2), (0, 2)]);
        let layer = longest_path(3, &edges);
        let layout = route(wrap(&layer, &[5, 5, 5], 2, 80), 3, &edges);
        assert_eq!(layout.rows, [vec![0], vec![1, 3], vec![2]]);
        assert_eq!(layout.segments.len(), 4);
    }

    #[test]
    fn a_wide_layer_wraps_into_rows() {
        let rows = wrap(&[0, 1, 1, 1], &[4, 10, 10, 10], 2, 23);
        assert_eq!(rows, [vec![0], vec![1, 2], vec![3]]);
    }
}
