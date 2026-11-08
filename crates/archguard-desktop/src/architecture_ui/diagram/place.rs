//! Coordinates of a layered package map: cards and bend points get x
//! positions near their neighbours, edges leave the bottom of the upper
//! card and enter the top of the lower one as curves, counts sit beside
//! the curve without overlapping.

use std::collections::HashMap;

use super::layered::{Chain, Grid, neighbours};

pub const CARD_W: f32 = 204.0;
pub const CARD_H: f32 = 98.0;
const BEND_W: f32 = 12.0;
const CARD_GAP: f32 = 48.0;
const BEND_GAP: f32 = 14.0;
const ROW_GAP: f32 = 86.0;
const MARGIN: f32 = 30.0;
/// Above the first row: the package tab and labels.
const TOP: f32 = 44.0;
const PLACE_ROUNDS: usize = 40;
/// Label box used to keep counts apart.
const LABEL_W: f32 = 20.0;
const LABEL_H: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Card {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// SVG path of the line; empty for an edge that cannot be drawn.
    pub path: String,
    /// SVG path of the arrow head at the target.
    pub head: String,
    /// Anchor of the count label (horizontal centre, baseline).
    pub label: (f32, f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub cards: Vec<Card>,
    pub routes: Vec<Route>,
}

pub fn layout(g: &Grid) -> Layout {
    let x = xs(g);
    let width_of = |s: usize| if s < g.nodes { CARD_W } else { BEND_W };
    let top = |s: usize| TOP + g.row_of[s] as f32 * (CARD_H + ROW_GAP);
    let cards: Vec<Card> = (0..g.nodes)
        .map(|i| Card {
            x: x[i],
            y: top(i),
            w: CARD_W,
            h: CARD_H,
        })
        .collect();
    let ports = ports(g, &x);
    let mut routes: Vec<Route> = g
        .chains
        .iter()
        .enumerate()
        .map(|(i, c)| route(c, i, &x, &ports, &|s| (top(s), width_of(s))))
        .collect();
    spread_labels(&mut routes);
    let right = (0..g.row_of.len())
        .map(|s| x[s] + width_of(s))
        .fold(0.0, f32::max);
    let bottom = (0..g.row_of.len())
        .map(|s| top(s) + CARD_H)
        .fold(0.0, f32::max);
    Layout {
        width: (right + MARGIN).max(300.0),
        height: (bottom + MARGIN).max(230.0),
        cards,
        routes,
    }
}

fn gap(left: usize, right: usize, nodes: usize) -> f32 {
    if left < nodes || right < nodes {
        if left < nodes && right < nodes {
            CARD_GAP
        } else {
            BEND_GAP + 10.0
        }
    } else {
        BEND_GAP
    }
}

/// Left edges of all slots: rows are packed, then moved towards the mean
/// centre of their neighbours while keeping order and gaps.
fn xs(g: &Grid) -> Vec<f32> {
    let w = |s: usize| if s < g.nodes { CARD_W } else { BEND_W };
    let mut x = vec![0.0f32; g.row_of.len()];
    for row in &g.rows {
        let wanted = vec![0.0; row.len()];
        place_row(row, &wanted, &mut x, g.nodes);
    }
    let (above, below) = neighbours(g);
    for round in 0..PLACE_ROUNDS {
        let rows: Box<dyn Iterator<Item = &Vec<usize>>> = if round % 2 == 0 {
            Box::new(g.rows.iter())
        } else {
            Box::new(g.rows.iter().rev())
        };
        for row in rows {
            let wanted: Vec<f32> = row
                .iter()
                .map(|&s| {
                    let near: Vec<f32> = above[s]
                        .iter()
                        .chain(&below[s])
                        .map(|&n| x[n] + w(n) / 2.0)
                        .collect();
                    if near.is_empty() {
                        x[s]
                    } else {
                        near.iter().sum::<f32>() / near.len() as f32 - w(s) / 2.0
                    }
                })
                .collect();
            place_row(row, &wanted, &mut x, g.nodes);
        }
    }
    tuck_lonely(g, &mut x, &above, &below);
    let shift = MARGIN - x.iter().copied().fold(f32::INFINITY, f32::min);
    x.iter().map(|v| (v + shift).round()).collect()
}

/// Cards without edges go into free space of their row inside the width
/// the connected slots need anyway: left of them first, then right; only
/// what does not fit widens the map.
fn tuck_lonely(g: &Grid, x: &mut [f32], above: &[Vec<usize>], below: &[Vec<usize>]) {
    let w = |s: usize| if s < g.nodes { CARD_W } else { BEND_W };
    let lonely = |s: usize| s < g.nodes && above[s].is_empty() && below[s].is_empty();
    let linked: Vec<usize> = (0..g.row_of.len()).filter(|&s| !lonely(s)).collect();
    let Some(lo) = linked.iter().map(|&s| x[s]).reduce(f32::min) else {
        return;
    };
    for row in &g.rows {
        let (mut quiet, mut busy): (Vec<usize>, Vec<usize>) = row.iter().partition(|&&s| lonely(s));
        if quiet.is_empty() {
            continue;
        }
        busy.sort_by(|a, b| x[*a].total_cmp(&x[*b]));
        // A row of lonely cards only starts at the left edge.
        let mut right = lo - CARD_GAP;
        if let (Some(&first), Some(&last)) = (busy.first(), busy.last()) {
            let mut left = x[first];
            while let Some(&s) = quiet.first() {
                let at = left - CARD_GAP - w(s);
                if at < lo {
                    break;
                }
                (x[s], left) = (at, at);
                quiet.remove(0);
            }
            right = x[last] + w(last);
        }
        for s in quiet {
            x[s] = right + CARD_GAP;
            right = x[s] + w(s);
        }
    }
}

/// Positions closest to `wanted` in order: the mean of a left-to-right and
/// a right-to-left packing, both of which keep the gaps.
fn place_row(row: &[usize], wanted: &[f32], x: &mut [f32], nodes: usize) {
    let w = |s: usize| if s < nodes { CARD_W } else { BEND_W };
    let n = row.len();
    let mut left = wanted.to_vec();
    for i in 1..n {
        let min = left[i - 1] + w(row[i - 1]) + gap(row[i - 1], row[i], nodes);
        left[i] = left[i].max(min);
    }
    let mut right = wanted.to_vec();
    for i in (0..n.saturating_sub(1)).rev() {
        let max = right[i + 1] - w(row[i]) - gap(row[i], row[i + 1], nodes);
        right[i] = right[i].min(max);
    }
    for i in 0..n {
        x[row[i]] = (left[i] + right[i]) / 2.0;
    }
}

/// Port x of every chain end on a card: `(chain, true)` the top end (on the
/// card's bottom side), `(chain, false)` the bottom end (on its top side).
/// Ports on one side are spread over the card, ordered by where they go.
fn ports(g: &Grid, x: &[f32]) -> HashMap<(usize, bool), f32> {
    let centre = |s: usize| x[s] + (if s < g.nodes { CARD_W } else { BEND_W }) / 2.0;
    // Per card side: (centre of the other end, chain, which chain end).
    type Side = Vec<(f32, usize, bool)>;
    let mut sides: HashMap<(usize, bool), Side> = HashMap::new();
    for (i, c) in g.chains.iter().enumerate() {
        let (Some(&first), Some(&last)) = (c.slots.first(), c.slots.last()) else {
            continue;
        };
        let (next, prev) = (c.slots[1], c.slots[c.slots.len() - 2]);
        sides
            .entry((first, false))
            .or_default()
            .push((centre(next), i, true));
        sides
            .entry((last, true))
            .or_default()
            .push((centre(prev), i, false));
    }
    let mut out = HashMap::new();
    for ((card, _), mut list) in sides {
        list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let n = list.len() as f32;
        let span = (CARD_W * 0.7).min(n * 18.0);
        for (k, &(_, chain, end)) in list.iter().enumerate() {
            let offset = if n > 1.0 {
                span * (k as f32 / (n - 1.0) - 0.5)
            } else {
                0.0
            };
            out.insert((chain, end), (centre(card) + offset).round());
        }
    }
    out
}

fn route(
    c: &Chain,
    i: usize,
    x: &[f32],
    ports: &HashMap<(usize, bool), f32>,
    slot: &dyn Fn(usize) -> (f32, f32),
) -> Route {
    if c.slots.len() < 2 {
        return Route {
            path: String::new(),
            head: String::new(),
            label: (0.0, 0.0),
        };
    }
    let (first, last) = (c.slots[0], c.slots[c.slots.len() - 1]);
    let mut pts = vec![(ports[&(i, true)], slot(first).0 + CARD_H)];
    for &b in &c.slots[1..c.slots.len() - 1] {
        let (top, w) = slot(b);
        let mid = (x[b] + w / 2.0).round();
        pts.push((mid, top));
        pts.push((mid, top + CARD_H));
    }
    pts.push((ports[&(i, false)], slot(last).0));
    if c.up {
        pts.reverse();
    }
    let (tip, back) = (pts[pts.len() - 1], pts[pts.len() - 2]);
    let dir = if back.1 < tip.1 { -9.0 } else { 9.0 };
    let head = format!(
        "M {} {} L {} {} L {} {} Z",
        tip.0,
        tip.1,
        tip.0 - 4.5,
        tip.1 + dir,
        tip.0 + 4.5,
        tip.1 + dir
    );
    let label = (
        ((pts[0].0 + pts[1].0) / 2.0 + 8.0).round(),
        ((pts[0].1 + pts[1].1) / 2.0 + 4.0).round(),
    );
    Route {
        path: path(&pts),
        head,
        label,
    }
}

/// Straight lines inside a row, vertical-tangent curves between rows.
fn path(pts: &[(f32, f32)]) -> String {
    let mut out = format!("M {} {}", pts[0].0, pts[0].1);
    for w in pts.windows(2) {
        let ((x1, y1), (x2, y2)) = (w[0], w[1]);
        if x1 == x2 && (y2 - y1).abs() <= CARD_H + 0.5 {
            out.push_str(&format!(" L {x2} {y2}"));
        } else {
            let my = (y1 + y2) / 2.0;
            out.push_str(&format!(" C {x1} {my} {x2} {my} {x2} {y2}"));
        }
    }
    out
}

/// Moves labels right until no two overlap.
fn spread_labels(routes: &mut [Route]) {
    let mut order: Vec<usize> = (0..routes.len())
        .filter(|&i| !routes[i].path.is_empty())
        .collect();
    order.sort_by(|&a, &b| {
        let (la, lb) = (routes[a].label, routes[b].label);
        la.1.total_cmp(&lb.1).then(la.0.total_cmp(&lb.0))
    });
    let mut placed: Vec<(f32, f32)> = Vec::new();
    for i in order {
        let (mut lx, ly) = routes[i].label;
        while let Some(p) = placed
            .iter()
            .find(|p| (p.0 - lx).abs() < LABEL_W && (p.1 - ly).abs() < LABEL_H)
        {
            lx = p.0 + LABEL_W;
        }
        routes[i].label = (lx, ly);
        placed.push((lx, ly));
    }
}
