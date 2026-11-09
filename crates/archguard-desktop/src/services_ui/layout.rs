//! Port of the web viewer's `packageTreeLayout` as the Services tab calls
//! it (`viewer_tree_layout.js`, no `tracks`, no `order`): a forest of
//! groups and services, siblings in dependency construction order, and
//! relation arcs on one axis to the right of the rows.

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BinaryHeap};

/// One tree node in input order; `id` is the web's `src` key.
pub struct Node {
    pub id: String,
    pub name: String,
    pub parent: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// A relation between two nodes; `key` orders links as the web's
/// `serviceKey` string, `side` forces a side ("In/Out" routing).
pub struct Link {
    pub from: usize,
    pub to: usize,
    pub key: String,
    pub side: Option<Side>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// A placed row: node index, depth and centre.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub node: usize,
    pub indent: f32,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// Index into the input links.
    pub link: usize,
    pub side: Side,
    pub points: [Point; 6],
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub rows: Vec<Row>,
    /// Row width and height, the relation axis and the canvas size.
    pub w: f32,
    pub h: f32,
    /// Rows narrower than the web's minimum show bare counts.
    pub compact: bool,
    pub relation_x: f32,
    pub width: f32,
    pub height: f32,
    /// Hierarchy connectors (child row → points of its rail).
    pub hierarchy: Vec<[Point; 3]>,
    /// One route per link whose ends are both placed, in key order.
    pub routes: Vec<Route>,
}

const LEFT: f32 = 24.0;
const BASE_GUTTER: f32 = 194.0;
/// Narrow pane: the smallest gutter and row; arcs reach at least
/// `MIN_REACH` from the axis (in a wide pane they always do).
const NARROW_GUTTER: f32 = 56.0;
const NARROW_ROW: f32 = 160.0;
const MIN_REACH: f32 = 28.0;

/// Lay out `nodes` and `links` for a viewport `width` (0 = unknown).
pub fn layout(nodes: &[Node], links: &[Link], width: f32) -> Layout {
    if nodes.is_empty() {
        return Layout::default();
    }
    let cmp = |a: &usize, b: &usize| compare(nodes, *a, *b);
    let mut children: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
    for (i, n) in nodes.iter().enumerate() {
        children.entry(n.parent).or_default().push(i);
    }
    for siblings in children.values_mut() {
        siblings.sort_by(cmp);
    }
    let mut deps: Vec<usize> = (0..links.len()).collect();
    deps.sort_by(|&a, &b| links[a].key.cmp(&links[b].key));
    let grouped = group_edges(nodes, links, &deps);
    for (parent, siblings) in children.iter_mut() {
        let pairs = grouped.get(parent).map(Vec::as_slice).unwrap_or(&[]);
        let mut sorted = siblings.clone();
        sorted.sort_by(|&a, &b| compare(nodes, a, b));
        let rank: BTreeMap<usize, usize> =
            sorted.iter().enumerate().map(|(i, &n)| (n, i)).collect();
        *siblings = crate::tree_order::level_order(&sorted, pairs, &rank);
    }
    let ordered = preorder(&children);
    place(nodes, links, &deps, &ordered, width)
}

/// The web's `compareId` for nodes of one kind: name, then id.
fn compare(nodes: &[Node], a: usize, b: usize) -> Ordering {
    (nodes[a].name.as_str(), nodes[a].id.as_str()).cmp(&(nodes[b].name.as_str(), &nodes[b].id))
}

/// Path from the outermost ancestor down to `n`.
fn ancestry(nodes: &[Node], n: usize) -> Vec<usize> {
    let mut out = vec![n];
    let mut cur = n;
    while let Some(p) = nodes[cur].parent {
        if out.contains(&p) {
            break;
        }
        out.push(p);
        cur = p;
    }
    out.reverse();
    out
}

/// Each link orders the two branches below its lowest common ancestor.
fn group_edges(
    nodes: &[Node],
    links: &[Link],
    deps: &[usize],
) -> BTreeMap<Option<usize>, Vec<(usize, usize)>> {
    let mut out: BTreeMap<Option<usize>, Vec<(usize, usize)>> = BTreeMap::new();
    for &d in deps {
        let (from, to) = (links[d].from, links[d].to);
        let (pf, pt) = (ancestry(nodes, from), ancestry(nodes, to));
        let i = pf.iter().zip(&pt).take_while(|(a, b)| a == b).count();
        let (key, a, b) = if from == to {
            (nodes[from].parent, from, from)
        } else if i == pf.len() || i == pt.len() {
            continue;
        } else {
            (if i > 0 { Some(pf[i - 1]) } else { None }, pf[i], pt[i])
        };
        out.entry(key).or_default().push((a, b));
    }
    out
}

/// Depth-first order of the forest with each node's depth.
fn preorder(children: &BTreeMap<Option<usize>, Vec<usize>>) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut pending: Vec<(usize, usize)> = children
        .get(&None)
        .map(|c| c.iter().rev().map(|&n| (n, 0)).collect())
        .unwrap_or_default();
    while let Some((n, depth)) = pending.pop() {
        if out.iter().any(|&(m, _)| m == n) {
            continue;
        }
        out.push((n, depth));
        if let Some(c) = children.get(&Some(n)) {
            pending.extend(c.iter().rev().map(|&m| (m, depth + 1)));
        }
    }
    out
}

/// Row width and gutter for a viewport `width`. From the web's minimum on
/// as the web; below it (a split pane) the gutters shrink first, then the
/// rows, so the tree fits the pane instead of scrolling sideways.
fn widths(min_label: f32, width: f32) -> (f32, f32) {
    let minimum = min_label + LEFT * 3.0 + BASE_GUTTER * 2.0;
    if width > 0.0 && width < minimum {
        let gutter = ((width - LEFT * 3.0 - min_label) / 2.0).max(NARROW_GUTTER);
        let w = (width - LEFT * 3.0 - 2.0 * gutter).max(NARROW_ROW);
        return (w, gutter);
    }
    let extra = (width - minimum).max(0.0);
    let label_extra = (700.0 - min_label).max(0.0).min(extra * 0.4);
    (
        min_label + label_extra,
        BASE_GUTTER + (extra - label_extra) / 2.0,
    )
}

/// Row geometry, lanes and routes for the ordered rows.
fn place(
    nodes: &[Node],
    links: &[Link],
    deps: &[usize],
    ordered: &[(usize, usize)],
    width: f32,
) -> Layout {
    let max_depth = ordered.iter().map(|&(_, d)| d).max().unwrap_or(0) as f32;
    let min_label = 400f32.max(300.0 + max_depth * 18.0);
    let (w, gutter) = widths(min_label, width);
    let relation_x = LEFT + w + 24.0 + gutter;
    let rows: Vec<Row> = ordered
        .iter()
        .enumerate()
        .map(|(i, &(node, depth))| Row {
            node,
            indent: depth as f32 * 18.0,
            x: LEFT + w / 2.0,
            y: 38.0 + i as f32 * 30.0,
        })
        .collect();
    let mut at = vec![None; nodes.len()];
    for r in &rows {
        at[r.node] = Some(r);
    }
    let hierarchy = rows
        .iter()
        .filter_map(|r| {
            let p = at[nodes[r.node].parent?]?;
            let rail = LEFT + p.indent + 9.0;
            Some([
                Point {
                    x: rail,
                    y: p.y + 8.0,
                },
                Point { x: rail, y: r.y },
                Point {
                    x: LEFT + r.indent + 7.0,
                    y: r.y,
                },
            ])
        })
        .collect();
    let routes = routes(links, deps, &at, relation_x, gutter);
    let height = rows.last().map_or(0.0, |r| r.y + 14.0 + 24.0);
    Layout {
        rows,
        w,
        h: 28.0,
        compact: w < min_label,
        relation_x,
        width: relation_x + gutter + 24.0,
        height,
        hierarchy,
        routes,
    }
}

struct Interval {
    link: usize,
    source_y: f32,
    target_y: f32,
    start: f32,
    end: f32,
    lane: usize,
}

fn routes(
    links: &[Link],
    deps: &[usize],
    at: &[Option<&Row>],
    relation_x: f32,
    gutter: f32,
) -> Vec<Route> {
    let mut intervals: Vec<Interval> = deps
        .iter()
        .filter_map(|&d| {
            let (from, to) = (at[links[d].from]?, at[links[d].to]?);
            let (s, t) = (from.y + 3.0, to.y - 3.0);
            Some(Interval {
                link: d,
                source_y: s,
                target_y: t,
                start: s.min(t),
                end: s.max(t),
                lane: 0,
            })
        })
        .collect();
    intervals.sort_by(|a, b| {
        (a.start, a.end)
            .partial_cmp(&(b.start, b.end))
            .unwrap_or(Ordering::Equal)
            .then_with(|| links[a.link].key.cmp(&links[b.link].key))
    });
    let lanes = assign_lanes(&mut intervals);
    let mut out: Vec<Route> = intervals
        .iter()
        .map(|iv| route(&links[iv.link], iv, lanes, relation_x, gutter))
        .collect();
    out.sort_by(|a, b| links[a.link].key.cmp(&links[b.link].key));
    out
}

/// The lowest free lane per interval (`packageTreeAssignLanes`).
fn assign_lanes(intervals: &mut [Interval]) -> usize {
    // Heap keys as bits keep f32 ends totally ordered (ends are positive).
    let mut active: BinaryHeap<Reverse<(u32, usize)>> = BinaryHeap::new();
    let mut free: BinaryHeap<Reverse<usize>> = BinaryHeap::new();
    let mut count = 0;
    for iv in intervals.iter_mut() {
        while let Some(&Reverse((end, lane))) = active.peek() {
            if f32::from_bits(end) + 2.0 >= iv.start {
                break;
            }
            active.pop();
            free.push(Reverse(lane));
        }
        iv.lane = free.pop().map_or_else(
            || {
                count += 1;
                count - 1
            },
            |Reverse(l)| l,
        );
        active.push(Reverse((iv.end.to_bits(), iv.lane)));
    }
    count
}

fn route(link: &Link, iv: &Interval, lanes: usize, axis: f32, gutter: f32) -> Route {
    let side = link
        .side
        .unwrap_or(if iv.source_y - 3.0 < iv.target_y + 3.0 {
            Side::Left
        } else {
            Side::Right
        });
    let sign = if side == Side::Left { -1.0 } else { 1.0 };
    let spread = if lanes > 1 {
        iv.lane as f32 / (lanes - 1) as f32
    } else {
        0.0
    };
    let span = (iv.target_y - iv.source_y).abs();
    let reach = (((48.0 + span.sqrt() * 5.0).min(166.0) + spread * 28.0) * (gutter / BASE_GUTTER))
        .max(MIN_REACH);
    let p = |dx: f32, y: f32| Point {
        x: axis + sign * dx,
        y,
    };
    let (s, t) = (iv.source_y, iv.target_y);
    Route {
        link: iv.link,
        side,
        points: [
            p(8.0, s),
            p(20.0, s),
            p(reach, s),
            p(reach, t),
            p(20.0, t),
            p(8.0, t),
        ],
    }
}
