//! Orthogonal arrow routes between the grid cards: every arrow leaves its
//! card through a port of its own, runs through the gaps between grid cells
//! on a lane of its own and ends with its head at the target's border.
//!
//! A gap ("channel") is the free strip between two card rows (horizontal)
//! or two card columns (vertical). Lanes come from interval colouring per
//! channel (sort by start, reuse the lane that ended first), so parallel
//! runs never share a line. Cost: O(n log n) in the number of arrows.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use super::draw::{label_at, path, simplify};
use super::grid::{Arrow, CARD_H, CARD_W, Network, TAB_H, stroke_width};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

/// A channel: horizontal between rows `k-1` and `k`, vertical between
/// columns `j-1` and `j` (0 and the count are the outer margins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Chan {
    H(usize),
    V(usize),
}

/// The channels an arrow runs through, in order.
#[derive(Debug, Clone, Copy)]
enum Plan {
    /// Neighbours in a row: side to side through the column gap.
    Beside(usize),
    /// Through one row gap.
    Across(usize),
    /// Row gap, column gap, row gap.
    Around(usize, usize, usize),
}

struct Leg {
    from: (usize, Side),
    to: (usize, Side),
    plan: Plan,
}

/// Gap between the cards and its free band (the part where a lane leaves
/// room for the arrow head on both sides).
const STUB: f32 = 22.0;
const LANE: f32 = 12.0;
/// Start and end port of a leg.
type Ends = [(f32, f32); 2];
/// Port requests per card side: (sort key, leg, 0 start / 1 end).
type SidePorts = HashMap<(usize, Side), Vec<(f32, usize, usize)>>;
/// Arrow heads stop this far before the border.
const TIP_GAP: f32 = 2.0;

fn cell(net: &Network, i: usize) -> (usize, usize) {
    (i % net.cols, i / net.cols)
}

/// The column gap a long route takes from column `ca` toward `cb`.
fn column_gap(net: &Network, ca: usize, cb: usize) -> usize {
    match cb.cmp(&ca) {
        std::cmp::Ordering::Greater => ca + 1,
        std::cmp::Ordering::Less => ca,
        // Same column: the gap to the right, or left at the outer edge.
        std::cmp::Ordering::Equal if ca + 1 < net.cols || ca == 0 => ca + 1,
        std::cmp::Ordering::Equal => ca,
    }
}

/// Side and row gap where a route leaves a card in row `ra` toward row
/// `rb`. Same row: below, unless that is the outer margin and a gap above
/// exists.
fn exit(net: &Network, ra: usize, rb: usize) -> (Side, usize) {
    let rows = net.cards.len().div_ceil(net.cols);
    let down = rb > ra || (rb == ra && (ra + 1 < rows || ra == 0));
    if down {
        (Side::Bottom, ra + 1)
    } else {
        (Side::Top, ra)
    }
}

fn plan(net: &Network, a: usize, b: usize) -> Leg {
    let ((ca, ra), (cb, rb)) = (cell(net, a), cell(net, b));
    let leg = |sa: Side, sb: Side, plan: Plan| Leg {
        from: (a, sa),
        to: (b, sb),
        plan,
    };
    if ra == rb && ca.abs_diff(cb) == 1 {
        let (sa, sb) = if cb > ca {
            (Side::Right, Side::Left)
        } else {
            (Side::Left, Side::Right)
        };
        return leg(sa, sb, Plan::Beside(ca.max(cb)));
    }
    let (sa, ka) = exit(net, ra, rb);
    if rb == ra {
        return leg(sa, sa, Plan::Across(ka));
    }
    let (sb, kb) = if rb > ra {
        (Side::Top, rb)
    } else {
        (Side::Bottom, rb + 1)
    };
    if rb.abs_diff(ra) == 1 {
        return leg(sa, sb, Plan::Across(ka));
    }
    leg(sa, sb, Plan::Around(ka, column_gap(net, ca, cb), kb))
}

/// Free band of a channel: between the neighbouring cards, minus room
/// for the arrow heads (outer margins have no card on their far side).
fn bounds(net: &Network, c: Chan) -> (f32, f32) {
    let (k, edges, size, total) = match c {
        Chan::H(k) => (k, &net.ys, CARD_H, net.height),
        Chan::V(j) => (j, &net.xs, CARD_W, net.width),
    };
    let (pad_lo, pad_hi) = pads(k, edges.len());
    let lo = if k == 0 { 0.0 } else { edges[k - 1] + size };
    let hi = edges.get(k).copied().unwrap_or(total);
    (lo + pad_lo, hi - pad_hi)
}

fn pads(k: usize, count: usize) -> (f32, f32) {
    let pad = |outer: bool| if outer { 8.0 } else { STUB };
    (pad(k == 0), pad(k >= count))
}

fn centre(net: &Network, c: Chan) -> f32 {
    let (lo, hi) = bounds(net, c);
    (lo + hi) / 2.0
}

/// Range of port positions along a card side, and the border coordinate.
fn side_span(net: &Network, i: usize, s: Side) -> ((f32, f32), f32) {
    let c = &net.cards[i];
    let body = c.y + TAB_H;
    match s {
        // Top ports right of the package tab, ending on the body below it;
        // bottom ports left of them, so that the stubs of two cards facing
        // each other across a row gap never share a line.
        Side::Top => ((c.x + CARD_W * 0.6, c.x + CARD_W * 0.92), body),
        Side::Bottom => ((c.x + CARD_W * 0.08, c.x + CARD_W * 0.52), c.y + CARD_H),
        Side::Left => ((body + 14.0, c.y + CARD_H - 14.0), c.x),
        Side::Right => ((body + 14.0, c.y + CARD_H - 14.0), c.x + CARD_W),
    }
}

/// Coordinate of the route's next bend seen from an end: ports along a
/// side are sorted by it, so neighbouring routes do not cross at the card.
fn toward(net: &Network, leg: &Leg, start: bool) -> f32 {
    let other = if start { leg.to.0 } else { leg.from.0 };
    let c = &net.cards[other];
    match leg.plan {
        Plan::Beside(_) => c.y + CARD_H / 2.0,
        Plan::Across(_) => c.x + CARD_W / 2.0,
        Plan::Around(_, j, _) => centre(net, Chan::V(j)),
    }
}

/// Port points (on the border) of every leg's start and end.
fn ports(net: &Network, legs: &[Leg]) -> Vec<Ends> {
    let mut sides: SidePorts = HashMap::new();
    for (n, leg) in legs.iter().enumerate() {
        for (end, at) in [(0, leg.from), (1, leg.to)] {
            let key = toward(net, leg, end == 0);
            sides.entry(at).or_default().push((key, n, end));
        }
    }
    let mut out = vec![[(0.0, 0.0); 2]; legs.len()];
    for ((card, side), mut list) in sides {
        list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let ((lo, hi), border) = side_span(net, card, side);
        let count = list.len() as f32;
        for (k, &(_, n, end)) in list.iter().enumerate() {
            let along = lo + (hi - lo) * (k as f32 + 1.0) / (count + 1.0);
            out[n][end] = match side {
                Side::Top | Side::Bottom => (along, border),
                Side::Left | Side::Right => (border, along),
            };
        }
    }
    out
}

/// The channel runs of a leg with their span along the channel.
fn runs(net: &Network, leg: &Leg, p: Ends) -> Vec<(Chan, f32, f32)> {
    let span = |a: f32, b: f32| (a.min(b), a.max(b));
    let with = |c: Chan, (lo, hi): (f32, f32)| (c, lo, hi);
    match leg.plan {
        Plan::Beside(j) => vec![with(Chan::V(j), span(p[0].1, p[1].1))],
        Plan::Across(k) => vec![with(Chan::H(k), span(p[0].0, p[1].0))],
        Plan::Around(ka, j, kb) => {
            let x = centre(net, Chan::V(j));
            vec![
                with(Chan::H(ka), span(p[0].0, x)),
                with(
                    Chan::V(j),
                    span(centre(net, Chan::H(ka)), centre(net, Chan::H(kb))),
                ),
                with(Chan::H(kb), span(x, p[1].0)),
            ]
        }
    }
}

type Runs = Vec<Vec<(Chan, f32, f32)>>;
/// Runs per channel: (start, end, leg, run of the leg), sorted by start.
type ByChan = HashMap<Chan, Vec<(f32, f32, usize, usize)>>;

fn by_chan(runs: &Runs) -> ByChan {
    let mut out: ByChan = HashMap::new();
    for (n, list) in runs.iter().enumerate() {
        for (r, &(c, lo, hi)) in list.iter().enumerate() {
            out.entry(c).or_default().push((lo, hi, n, r));
        }
    }
    for list in out.values_mut() {
        list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
    }
    out
}

/// Interval colouring of sorted runs: a lane per run and the lane count.
/// Runs closer than a lane apart count as overlapping.
fn colour(list: &[(f32, f32, usize, usize)]) -> (Vec<usize>, usize) {
    let mut ends: BinaryHeap<Reverse<(i64, usize)>> = BinaryHeap::new();
    let mut lane_of = Vec::with_capacity(list.len());
    for &(lo, hi, ..) in list {
        let (lo, hi) = ((lo - LANE) as i64, (hi + LANE) as i64);
        let lane = match ends.peek() {
            Some(&Reverse((end, lane))) if end < lo => {
                ends.pop();
                lane
            }
            _ => ends.len(),
        };
        ends.push(Reverse((hi, lane)));
        lane_of.push(lane);
    }
    (lane_of, ends.len())
}

/// Lane coordinate per run, the lanes centred in the channel's free band.
fn lanes(net: &Network, runs: &Runs) -> Vec<Vec<f32>> {
    let mut out: Vec<Vec<f32>> = runs.iter().map(|l| vec![0.0; l.len()]).collect();
    for (chan, list) in by_chan(runs) {
        let (lane_of, count) = colour(&list);
        let (lo, hi) = bounds(net, chan);
        let count = count.max(1) as f32;
        let step = if count > 1.0 {
            LANE.min((hi - lo) / (count - 1.0))
        } else {
            0.0
        };
        let mid = (lo + hi) / 2.0;
        for (&(_, _, n, r), lane) in list.iter().zip(lane_of) {
            out[n][r] = mid + (lane as f32 - (count - 1.0) / 2.0) * step;
        }
    }
    out
}

fn legs(net: &Network, links: &[(usize, usize, u64)]) -> (Vec<Leg>, Vec<Ends>, Runs) {
    let legs: Vec<Leg> = links.iter().map(|&(a, b, _)| plan(net, a, b)).collect();
    let ports = ports(net, &legs);
    let runs = legs
        .iter()
        .zip(&ports)
        .map(|(l, &p)| runs(net, l, p))
        .collect();
    (legs, ports, runs)
}

/// Gap widths (column gaps, row gaps; margins first and last) that give
/// every lane of `links` a full lane width on `net`'s grid.
pub(super) fn gaps(net: &Network, links: &[(usize, usize, u64)]) -> (Vec<f32>, Vec<f32>) {
    let mut cols = vec![0.0; net.xs.len() + 1];
    let mut rows = vec![0.0; net.ys.len() + 1];
    for (chan, list) in by_chan(&legs(net, links).2) {
        let (_, count) = colour(&list);
        let (k, gaps) = match chan {
            Chan::H(k) => (k, &mut rows),
            Chan::V(j) => (j, &mut cols),
        };
        let (pad_lo, pad_hi) = pads(k, gaps.len() - 1);
        gaps[k] = (count.saturating_sub(1)) as f32 * LANE + pad_lo + pad_hi + 4.0;
    }
    (cols, rows)
}

/// Corner points of a leg, from the start port to the arrow tip.
fn corners(leg: &Leg, p: Ends, lane: &[f32]) -> Vec<(f32, f32)> {
    let (a, b) = (p[0], p[1]);
    let tip = match leg.to.1 {
        Side::Top => (b.0, b.1 - TIP_GAP),
        Side::Bottom => (b.0, b.1 + TIP_GAP),
        Side::Left => (b.0 - TIP_GAP, b.1),
        Side::Right => (b.0 + TIP_GAP, b.1),
    };
    match leg.plan {
        Plan::Beside(_) => vec![a, (lane[0], a.1), (lane[0], b.1), tip],
        Plan::Across(_) => vec![a, (a.0, lane[0]), (b.0, lane[0]), tip],
        Plan::Around(..) => vec![
            a,
            (a.0, lane[0]),
            (lane[1], lane[0]),
            (lane[1], lane[2]),
            (b.0, lane[2]),
            tip,
        ],
    }
}

/// Arrows for `links` (from card, to card, reference count), in order.
/// Links of a card to itself get an empty path.
pub fn route(net: &Network, links: &[(usize, usize, u64)]) -> Vec<Arrow> {
    let (legs, ports, runs) = legs(net, links);
    let lanes = lanes(net, &runs);
    links
        .iter()
        .enumerate()
        .map(|(n, &(a, b, count))| {
            let width = stroke_width(count);
            if a == b {
                return Arrow {
                    path: String::new(),
                    width,
                    label: (0.0, 0.0),
                };
            }
            let points = simplify(corners(&legs[n], ports[n], &lanes[n]));
            Arrow {
                path: path(&points, width),
                width,
                label: label_at(&points),
            }
        })
        .collect()
}
