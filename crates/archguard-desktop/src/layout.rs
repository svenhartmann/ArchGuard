//! Layered layout of the module dependency graph, independent of Slint.
//!
//! Dependents sit above their dependencies: layer 0 holds the modules no
//! other module depends on, and every acyclic edge points downwards.
//! Modules of one strongly connected component share a layer; their
//! edges are marked `cyclic` and drawn as arcs below the row.

use std::collections::BTreeMap;

pub const NODE_W: f32 = 196.0;
pub const NODE_H: f32 = 68.0;
const H_GAP: f32 = 44.0;
const V_GAP: f32 = 120.0;
const MARGIN: f32 = 60.0;

/// One dependency `from` → `to` between node indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dep {
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedNode {
    pub x: f32,
    pub y: f32,
    pub layer: usize,
    /// Index of the node's strongly connected component.
    pub component: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedEdge {
    pub from: usize,
    pub to: usize,
    /// SVG path commands in canvas coordinates.
    pub path: String,
    /// Both ends lie in the same strongly connected component.
    pub cyclic: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Same order as the input names.
    pub nodes: Vec<PlacedNode>,
    /// Same order as the input deps; self loops are dropped.
    pub edges: Vec<PlacedEdge>,
    pub width: f32,
    pub height: f32,
}

/// Lay out `names.len()` nodes. `names` only break ties, so equal input
/// always gives the same layout.
pub fn layout(names: &[String], deps: &[Dep]) -> Layout {
    let n = names.len();
    let deps: Vec<Dep> = deps
        .iter()
        .copied()
        .filter(|d| d.from != d.to && d.from < n && d.to < n)
        .collect();
    let mut succ = vec![Vec::new(); n];
    for d in &deps {
        succ[d.from].push(d.to);
    }
    let component = strongly_connected(&succ);
    let comp_count = component.iter().copied().max().map_or(0, |m| m + 1);
    let comp_layer = longest_path_layers(comp_count, &component, &deps);
    let layer: Vec<usize> = component.iter().map(|&c| comp_layer[c]).collect();
    let rows = order_rows(names, &deps, &layer, &component);

    // A layer wider than `per_line` wraps onto several visual lines.
    let per_line = (n as f32).sqrt().ceil().max(4.0) as usize;
    let lines: Vec<(usize, &[usize])> = rows
        .iter()
        .enumerate()
        .flat_map(|(l, row)| row.chunks(per_line).map(move |c| (l, c)))
        .collect();
    let row_width = |k: usize| k as f32 * NODE_W + k.saturating_sub(1) as f32 * H_GAP;
    let widest = lines
        .iter()
        .map(|(_, r)| row_width(r.len()))
        .fold(0.0, f32::max);
    let mut nodes = vec![
        PlacedNode {
            x: 0.0,
            y: 0.0,
            layer: 0,
            component: 0,
        };
        n
    ];
    for (line, &(l, row)) in lines.iter().enumerate() {
        let left = MARGIN + (widest - row_width(row.len())) / 2.0;
        for (i, &v) in row.iter().enumerate() {
            nodes[v] = PlacedNode {
                x: left + i as f32 * (NODE_W + H_GAP),
                y: MARGIN + line as f32 * (NODE_H + V_GAP),
                layer: l,
                component: component[v],
            };
        }
    }
    let edges = deps
        .iter()
        .map(|d| {
            let cyclic = component[d.from] == component[d.to];
            PlacedEdge {
                from: d.from,
                to: d.to,
                path: edge_path(&nodes[d.from], &nodes[d.to], cyclic),
                cyclic,
            }
        })
        .collect();
    let height = if lines.is_empty() {
        0.0
    } else {
        2.0 * MARGIN + lines.len() as f32 * NODE_H + (lines.len() - 1) as f32 * V_GAP
    };
    Layout {
        nodes,
        edges,
        width: if n == 0 { 0.0 } else { 2.0 * MARGIN + widest },
        height,
    }
}

/// Component index per node (Tarjan via petgraph).
fn strongly_connected(succ: &[Vec<usize>]) -> Vec<usize> {
    let mut g = petgraph::graph::DiGraph::<(), ()>::with_capacity(succ.len(), 0);
    let ids: Vec<_> = (0..succ.len()).map(|_| g.add_node(())).collect();
    for (from, targets) in succ.iter().enumerate() {
        for &to in targets {
            g.add_edge(ids[from], ids[to], ());
        }
    }
    let mut component = vec![0; succ.len()];
    for (c, members) in petgraph::algo::tarjan_scc(&g).into_iter().enumerate() {
        for v in members {
            component[v.index()] = c;
        }
    }
    component
}

/// Longest path from the sources of the condensation (Kahn order).
fn longest_path_layers(comp_count: usize, component: &[usize], deps: &[Dep]) -> Vec<usize> {
    let mut succ = vec![Vec::new(); comp_count];
    let mut indegree = vec![0usize; comp_count];
    for d in deps {
        let (a, b) = (component[d.from], component[d.to]);
        if a != b {
            succ[a].push(b);
            indegree[b] += 1;
        }
    }
    let mut layer = vec![0; comp_count];
    let mut ready: Vec<usize> = (0..comp_count).filter(|&c| indegree[c] == 0).collect();
    while let Some(c) = ready.pop() {
        for &s in &succ[c] {
            layer[s] = layer[s].max(layer[c] + 1);
            indegree[s] -= 1;
            if indegree[s] == 0 {
                ready.push(s);
            }
        }
    }
    layer
}

/// Rows top to bottom. The first row sorts by name; every later row by
/// the mean relative position of its parents in the rows above, so
/// edges cross less. Members of one cycle stay next to each other.
fn order_rows(
    names: &[String],
    deps: &[Dep],
    layer: &[usize],
    component: &[usize],
) -> Vec<Vec<usize>> {
    let depth = layer.iter().copied().max().map_or(0, |m| m + 1);
    let mut members: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (v, &l) in layer.iter().enumerate() {
        members.entry(l).or_default().push(v);
    }
    let mut position = vec![0.5f32; layer.len()];
    let mut rows = Vec::with_capacity(depth);
    for l in 0..depth {
        let row = members.remove(&l).unwrap_or_default();
        let center = |v: usize| {
            let parents: Vec<f32> = deps
                .iter()
                .filter(|d| d.to == v && layer[d.from] < l)
                .map(|d| position[d.from])
                .collect();
            if parents.is_empty() {
                f32::INFINITY
            } else {
                parents.iter().sum::<f32>() / parents.len() as f32
            }
        };
        let mut keyed: Vec<(f32, usize)> = row.iter().map(|&v| (center(v), v)).collect();
        // A cycle sorts by the mean centre of its members.
        let mut comp_center: BTreeMap<usize, (f32, usize)> = BTreeMap::new();
        for &(c, v) in &keyed {
            let e = comp_center.entry(component[v]).or_insert((0.0, 0));
            e.0 += if c.is_finite() { c } else { 2.0 };
            e.1 += 1;
        }
        for k in &mut keyed {
            let (sum, count) = comp_center[&component[k.1]];
            k.0 = sum / count as f32;
        }
        keyed.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(component[a.1].cmp(&component[b.1]))
                .then(names[a.1].cmp(&names[b.1]))
        });
        let ordered: Vec<usize> = keyed.into_iter().map(|(_, v)| v).collect();
        for (i, &v) in ordered.iter().enumerate() {
            position[v] = (i as f32 + 0.5) / ordered.len() as f32;
        }
        rows.push(ordered);
    }
    rows
}

fn edge_path(from: &PlacedNode, to: &PlacedNode, cyclic: bool) -> String {
    if cyclic {
        // Arc below the shared row; the offsets keep a→b and b→a apart.
        // Members of one cycle may sit on different wrapped lines.
        let (sx, tx) = (from.x + NODE_W / 2.0 + 14.0, to.x + NODE_W / 2.0 - 14.0);
        let (sy, ty) = (from.y + NODE_H, to.y + NODE_H);
        let dip = (36.0 + 0.18 * (tx - sx).abs()).min(V_GAP * 0.8);
        let low = sy.max(ty) + dip;
        return format!(
            "M {sx:.1} {sy:.1} C {sx:.1} {low:.1} {tx:.1} {low:.1} {tx:.1} {ty:.1} {}",
            arrow_head(tx, ty, 1.0)
        );
    }
    let (sx, sy) = (from.x + NODE_W / 2.0, from.y + NODE_H);
    let (tx, ty) = (to.x + NODE_W / 2.0, to.y);
    let dy = (ty - sy) / 2.0;
    format!(
        "M {sx:.1} {sy:.1} C {sx:.1} {:.1} {tx:.1} {:.1} {tx:.1} {ty:.1} {}",
        sy + dy,
        ty - dy,
        arrow_head(tx, ty, -1.0)
    )
}

/// Open arrow head with its tip at (`x`, `y`); `dir` −1 points down
/// (the shaft comes from above), +1 points up.
fn arrow_head(x: f32, y: f32, dir: f32) -> String {
    let back = y + dir * 9.0;
    format!(
        "M {:.1} {back:.1} L {x:.1} {y:.1} L {:.1} {back:.1}",
        x - 5.5,
        x + 5.5
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn chain_goes_downwards_one_layer_per_step() {
        let l = layout(
            &names(&["a", "b", "c"]),
            &[Dep { from: 0, to: 1 }, Dep { from: 1, to: 2 }],
        );
        let layers: Vec<usize> = l.nodes.iter().map(|n| n.layer).collect();
        assert_eq!(layers, vec![0, 1, 2]);
        assert!(l.nodes[0].y < l.nodes[1].y && l.nodes[1].y < l.nodes[2].y);
        assert!(l.edges.iter().all(|e| !e.cyclic));
    }

    #[test]
    fn longest_path_decides_the_layer() {
        // a→b→c and a→c: c must sit below b, not next to it.
        let l = layout(
            &names(&["a", "b", "c"]),
            &[
                Dep { from: 0, to: 1 },
                Dep { from: 1, to: 2 },
                Dep { from: 0, to: 2 },
            ],
        );
        assert_eq!(l.nodes[2].layer, 2);
    }

    #[test]
    fn cycle_members_share_a_layer_and_edges_are_cyclic() {
        let l = layout(
            &names(&["a", "b", "c"]),
            &[
                Dep { from: 0, to: 1 },
                Dep { from: 1, to: 0 },
                Dep { from: 1, to: 2 },
            ],
        );
        assert_eq!(l.nodes[0].layer, l.nodes[1].layer);
        assert_eq!(l.nodes[0].component, l.nodes[1].component);
        assert_eq!(l.nodes[2].layer, l.nodes[0].layer + 1);
        let cyclic: Vec<bool> = l.edges.iter().map(|e| e.cyclic).collect();
        assert_eq!(cyclic, vec![true, true, false]);
    }

    #[test]
    fn row_order_follows_the_parents() {
        // Names sort the lower row as c, d, but d's parent a stands left
        // of c's parent b, so d must come first.
        let l = layout(
            &names(&["a", "b", "c", "d"]),
            &[Dep { from: 0, to: 3 }, Dep { from: 1, to: 2 }],
        );
        assert!(l.nodes[0].x < l.nodes[1].x);
        assert!(l.nodes[3].x < l.nodes[2].x);
    }

    #[test]
    fn nodes_do_not_overlap_and_fit_the_canvas() {
        let n = names(&["a", "b", "c", "d", "e"]);
        let l = layout(&n, &[Dep { from: 0, to: 4 }]);
        for (i, a) in l.nodes.iter().enumerate() {
            assert!(a.x >= 0.0 && a.y >= 0.0);
            assert!(a.x + NODE_W <= l.width && a.y + NODE_H <= l.height);
            for b in &l.nodes[i + 1..] {
                let apart = a.x + NODE_W <= b.x
                    || b.x + NODE_W <= a.x
                    || a.y + NODE_H <= b.y
                    || b.y + NODE_H <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn self_loops_are_dropped_and_paths_are_svg() {
        let l = layout(
            &names(&["a", "b"]),
            &[Dep { from: 0, to: 0 }, Dep { from: 0, to: 1 }],
        );
        assert_eq!(l.edges.len(), 1);
        assert!(l.edges[0].path.starts_with("M "));
        assert!(l.edges[0].path.contains(" C "));
    }

    #[test]
    fn wide_layers_wrap_into_several_lines() {
        let n: Vec<String> = (0..15).map(|i| format!("m{i:02}")).collect();
        let l = layout(&n, &[]);
        let mut ys: Vec<i32> = l.nodes.iter().map(|p| p.y as i32).collect();
        ys.sort();
        ys.dedup();
        assert!(
            ys.len() >= 3,
            "15 loose modules use several lines, got {ys:?}"
        );
        assert!(l.width < l.height * 2.5, "canvas stays near screen format");
        assert!(l.nodes.iter().all(|p| p.layer == 0));
    }

    #[test]
    fn empty_graph_has_an_empty_canvas() {
        let l = layout(&[], &[]);
        assert!(l.nodes.is_empty() && l.edges.is_empty());
    }
}
