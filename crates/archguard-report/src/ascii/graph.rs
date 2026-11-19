//! A box graph in the terminal: boxes in rows top-down by dependency
//! direction ([`super::layers`]), edges as orthogonal lines between rows,
//! each turn on its own track, arrowheads at the used side.

use std::collections::{BTreeSet, HashMap};

use archguard_core::{Diagram, DiagramElement, DiagramRelation, RelationKind};

use super::canvas::{BLUE, Canvas, DIM, E, MAGENTA, RED, Style, W, YELLOW, fit};
use super::layers::{Edge, Layout, Segment, break_cycles, ends, longest_path, route, wrap};

/// Columns between boxes of a row.
const GAP: usize = 2;
/// Box height: border, name, info, border.
const HEIGHT: usize = 4;
/// The widest text in a box.
const INNER: usize = 28;

/// Where everything is drawn.
struct Geometry {
    x: Vec<usize>,
    w: Vec<usize>,
    /// Top row of each layout row.
    y: Vec<usize>,
    /// Per segment: column at the upper and at the lower node, track.
    ports: Vec<(usize, usize, Option<usize>)>,
    /// Per segment: the upper node's bottom row, the lower node's top row.
    spans: Vec<(usize, usize)>,
}

/// `d` as boxes and lines; relations a box to itself, and counts without
/// room on the picture, are listed below it.
pub fn draw(d: &Diagram, style: Style) -> String {
    let index: HashMap<&str, usize> = d
        .elements
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.as_str(), i))
        .collect();
    let mut notes = Vec::new();
    let mut pairs = Vec::new();
    let mut rels = Vec::new();
    for r in &d.relations {
        match (index.get(r.from.as_str()), index.get(r.to.as_str())) {
            (Some(&a), Some(&b)) if a != b => {
                pairs.push((a, b));
                rels.push(r);
            }
            (Some(&a), Some(_)) => notes.push(format!(
                "{} → itself: {}",
                d.elements[a].name,
                label(r, false)
            )),
            _ => {}
        }
    }
    let edges = break_cycles(d.elements.len(), &pairs);
    let texts: Vec<(String, String)> = d.elements.iter().map(|e| texts(e, style)).collect();
    let widths: Vec<usize> = texts
        .iter()
        .map(|(a, b)| width(a).max(width(b)) + 4)
        .collect();
    let layout = fit_layout(&edges, &widths, style.width);
    let geo = geometry(&layout, &widths);
    let mut canvas = Canvas::new(style);
    for (v, e) in d.elements.iter().enumerate() {
        draw_box(&mut canvas, &geo, &layout, v, (&texts[v], e.cycle));
    }
    for (i, s) in layout.segments.iter().enumerate() {
        let e = &edges[s.edge];
        draw_segment(
            &mut canvas,
            &geo,
            &layout,
            (i, s),
            (e, color(rels[s.edge], e)),
        );
    }
    for (i, s) in layout.segments.iter().enumerate() {
        let (e, r) = (&edges[s.edge], rels[s.edge]);
        let (a, b) = ends(e);
        let labelled = if e.up { s.upper == a } else { s.lower == b };
        let text = label(r, e.up);
        if labelled && !put_label(&mut canvas, &geo, i, e, &text) {
            let name = |v: usize| d.elements[v].name.as_str();
            notes.push(format!("{} → {}: {text}", name(e.from), name(e.to)));
        }
    }
    let mut out = canvas.render();
    for n in notes {
        out.push_str(&n);
        out.push('\n');
    }
    out
}

fn width(s: &str) -> usize {
    s.chars().count()
}

/// Name line (`!` for a cycle) and info line of a box.
fn texts(e: &DiagramElement, style: Style) -> (String, String) {
    let name = if e.cycle {
        format!("! {}", e.name)
    } else {
        e.name.clone()
    };
    let info: Vec<&str> = [&e.group, &e.technology, &e.detail]
        .into_iter()
        .filter_map(|t| t.as_deref())
        .collect();
    let info = if info.is_empty() {
        kind_name(&e.kind)
    } else {
        info.join(" · ")
    };
    let inner = INNER.min(style.width.saturating_sub(4)).max(1);
    (
        fit(&name, inner, style.unicode),
        fit(&info, inner, style.unicode),
    )
}

/// The serde name of a kind (`package`, `http`).
fn kind_name<T: serde::Serialize>(kind: &T) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The text at an arrow: `!` for a cycle, the kind unless it is a source
/// dependency or call, the count.
fn label(r: &DiagramRelation, up: bool) -> String {
    let mut out = String::new();
    if r.cycle || up {
        out.push('!');
    }
    if !matches!(r.kind, RelationKind::Dependency | RelationKind::Call) {
        out.push_str(&kind_name(&r.kind));
        out.push(' ');
    }
    out.push_str(&r.count.to_string());
    out
}

fn color(r: &DiagramRelation, e: &Edge) -> u8 {
    match r.kind {
        _ if r.cycle || e.up || r.error => RED,
        RelationKind::Http => BLUE,
        RelationKind::Grpc => MAGENTA,
        RelationKind::Event => YELLOW,
        _ => 0,
    }
}

/// The layout whose widest row fits `max`, rows narrowed until it does or
/// a row holds a single box.
fn fit_layout(edges: &[Edge], widths: &[usize], max: usize) -> Layout {
    let layer = longest_path(widths.len(), edges);
    let widest = widths.iter().copied().max().unwrap_or(0);
    let mut budget = max;
    loop {
        let layout = route(wrap(&layer, widths, GAP, budget), widths.len(), edges);
        let all = all_widths(&layout, widths);
        let total = layout
            .rows
            .iter()
            .map(|row| row_width(row, &all))
            .max()
            .unwrap_or(0);
        if total <= max || budget <= widest {
            return layout;
        }
        budget = budget.saturating_sub(total - max).max(widest);
    }
}

/// Box widths, then 1 per dummy.
fn all_widths(layout: &Layout, widths: &[usize]) -> Vec<usize> {
    let dummies = layout.segments.len() + layout.real;
    let mut all = widths.to_vec();
    all.resize(dummies, 1);
    all
}

fn row_width(row: &[usize], w: &[usize]) -> usize {
    row.iter().map(|&v| w[v]).sum::<usize>() + GAP * row.len().saturating_sub(1)
}

/// Columns of every node (rows centred), ports, tracks and row tops.
fn geometry(layout: &Layout, widths: &[usize]) -> Geometry {
    let w = all_widths(layout, widths);
    let total = layout
        .rows
        .iter()
        .map(|r| row_width(r, &w))
        .max()
        .unwrap_or(0);
    let mut x = vec![0; w.len()];
    for row in &layout.rows {
        let mut at = (total - row_width(row, &w)) / 2;
        for &v in row {
            x[v] = at;
            at += w[v] + GAP;
        }
    }
    let mut geo = Geometry {
        x,
        w,
        y: Vec::new(),
        ports: Vec::new(),
        spans: Vec::new(),
    };
    geo.ports = ports(layout, &geo);
    let mut top = 0;
    for r in 0..layout.rows.len() {
        geo.y.push(top);
        let tracks = (geo.ports.iter().zip(&layout.segments))
            .filter(|(_, s)| row_of(layout, s.upper) == r)
            .filter_map(|(p, _)| p.2)
            .max()
            .map_or(0, |t| t + 1);
        top += HEIGHT + tracks + 2;
    }
    geo.spans = (layout.segments.iter())
        .map(|s| {
            let r = row_of(layout, s.upper);
            (geo.y[r] + HEIGHT - 1, geo.y[r + 1])
        })
        .collect();
    geo
}

fn row_of(layout: &Layout, v: usize) -> usize {
    layout.rows.iter().position(|r| r.contains(&v)).unwrap_or(0)
}

/// Column `k` of `m` spread over a node's inner width.
fn spread(x: usize, w: usize, k: usize, m: usize) -> usize {
    if w <= 2 {
        x
    } else {
        x + 1 + (w - 2) * (2 * k + 1) / (2 * m)
    }
}

/// Port columns and tracks of every segment: ports spread over the box
/// edge in the order of the other ends; a lower port moves off a column
/// an upper port of the same gap uses, so lines never run into each other.
fn ports(layout: &Layout, geo: &Geometry) -> Vec<(usize, usize, Option<usize>)> {
    let segs = &layout.segments;
    let centre = |v: usize| geo.x[v] * 2 + geo.w[v];
    let column = |v: usize, i: usize, out: bool| {
        let mut mine: Vec<usize> = (0..segs.len())
            .filter(|&j| {
                if out {
                    segs[j].upper == v
                } else {
                    segs[j].lower == v
                }
            })
            .collect();
        mine.sort_by_key(|&j| (centre(if out { segs[j].lower } else { segs[j].upper }), j));
        let k = mine.iter().position(|&j| j == i).unwrap_or(0);
        spread(geo.x[v], geo.w[v], k, mine.len())
    };
    let mut cols: Vec<(usize, usize)> = (0..segs.len())
        .map(|i| {
            (
                column(segs[i].upper, i, true),
                column(segs[i].lower, i, false),
            )
        })
        .collect();
    for r in 0..layout.rows.len() {
        let gap: Vec<usize> = (0..segs.len())
            .filter(|&i| row_of(layout, segs[i].upper) == r)
            .collect();
        let uppers: BTreeSet<usize> = gap.iter().map(|&i| cols[i].0).collect();
        for &i in &gap {
            let (a, b) = cols[i];
            if a != b && uppers.contains(&b) {
                let taken: BTreeSet<usize> = gap.iter().map(|&j| cols[j].1).collect();
                cols[i].1 = moved(geo, segs[i].lower, b, &uppers, &taken);
            }
        }
    }
    let tracks = tracks(layout, &cols);
    cols.into_iter()
        .zip(tracks)
        .map(|((a, b), t)| (a, b, t))
        .collect()
}

/// Lower port `b` of node `v` moved to the nearest inner column neither an
/// upper port nor another lower port uses; `b` without one.
fn moved(
    geo: &Geometry,
    v: usize,
    b: usize,
    uppers: &BTreeSet<usize>,
    taken: &BTreeSet<usize>,
) -> usize {
    let inside = geo.x[v] + 1..(geo.x[v] + geo.w[v]).saturating_sub(1);
    (1..geo.w[v])
        .flat_map(|d| [b + d, b.wrapping_sub(d)])
        .find(|c| inside.contains(c) && !uppers.contains(c) && !taken.contains(c))
        .unwrap_or(b)
}

/// A track per turning segment, the upper ones first for runs that start
/// farther out (leftward: farther left), so a line turning later does not
/// cross one turning earlier; runs share a track with a blank column
/// between them.
fn tracks(layout: &Layout, cols: &[(usize, usize)]) -> Vec<Option<usize>> {
    let segs = &layout.segments;
    let mut out = vec![None; segs.len()];
    for r in 0..layout.rows.len() {
        let mut turns: Vec<usize> = (0..segs.len())
            .filter(|&i| row_of(layout, segs[i].upper) == r && cols[i].0 != cols[i].1)
            .collect();
        turns.sort_by_key(|&i| {
            let (a, b) = cols[i];
            if b < a {
                (0, a, i)
            } else {
                (1, usize::MAX - a, i)
            }
        });
        let mut used: Vec<Vec<(usize, usize)>> = Vec::new();
        for i in turns {
            let (lo, hi) = (cols[i].0.min(cols[i].1), cols[i].0.max(cols[i].1));
            let apart = |t: &Vec<(usize, usize)>| t.iter().all(|&(l, h)| hi + 1 < l || h + 1 < lo);
            let t = used.iter().position(apart).unwrap_or_else(|| {
                used.push(Vec::new());
                used.len() - 1
            });
            used[t].push((lo, hi));
            out[i] = Some(t);
        }
    }
    out
}

fn draw_box(
    canvas: &mut Canvas,
    geo: &Geometry,
    layout: &Layout,
    v: usize,
    (text, cycle): (&(String, String), bool),
) {
    let (x, y) = (geo.x[v], geo.y[row_of(layout, v)]);
    canvas.frame(x, y, geo.w[v], HEIGHT, if cycle { RED } else { 0 });
    canvas.text(x + 2, y + 1, &text.0, 0);
    canvas.text(x + 2, y + 2, &text.1, DIM);
}

/// One segment: down from the upper node, along its track, down into the
/// lower node; an arrowhead where the edge ends at a box.
fn draw_segment(
    canvas: &mut Canvas,
    geo: &Geometry,
    layout: &Layout,
    (i, s): (usize, &Segment),
    (e, color): (&Edge, u8),
) {
    let real = |v: usize| v < layout.real;
    let (a, b, track) = geo.ports[i];
    let (bottom, below) = geo.spans[i];
    for v in [s.upper, s.lower].into_iter().filter(|&v| !real(v)) {
        let top = geo.y[row_of(layout, v)];
        canvas.vline(geo.x[v], top, top + HEIGHT - 1, color);
    }
    let arrow = real(s.lower) && !e.up;
    let end = if arrow { below - 1 } else { below };
    match track {
        Some(t) => {
            let ty = bottom + 2 + t;
            canvas.vline(a, bottom, ty, color);
            canvas.hline(ty, a, b, color);
            canvas.vline(b, ty, end, color);
        }
        None => canvas.vline(a, bottom, end, color),
    }
    let unicode = canvas.style.unicode;
    if arrow {
        canvas.put(b, below - 1, if unicode { '▼' } else { 'v' }, color);
    }
    if real(s.upper) && e.up {
        canvas.put(a, bottom + 1, if unicode { '▲' } else { '^' }, color);
    }
}

/// The label on the track, else beside the arrowhead; false without room.
fn put_label(canvas: &mut Canvas, geo: &Geometry, i: usize, e: &Edge, text: &str) -> bool {
    let (a, b, track) = geo.ports[i];
    let (bottom, below) = geo.spans[i];
    let len = width(text);
    let color = if e.up { RED } else { 0 };
    if let Some(t) = track {
        let ty = bottom + 2 + t;
        let (lo, hi) = (a.min(b), a.max(b));
        let x = lo + 1 + (hi - lo).saturating_sub(1 + len) / 2;
        if hi - lo > len + 2 && (x..x + len).all(|c| canvas.only(c, ty, E | W)) {
            canvas.text(x, ty, text, color);
            return true;
        }
    }
    // Beside the vertical run at the arrowhead, nearest to the head first.
    let (col, rows) = match (e.up, track) {
        (true, Some(t)) => (a, (bottom + 1..bottom + 2 + t).collect::<Vec<_>>()),
        (true, None) => (a, (bottom + 1..below).collect()),
        (false, Some(t)) => (b, (bottom + 3 + t..below).rev().collect()),
        (false, None) => (b, (bottom + 1..below).rev().collect()),
    };
    for row in rows {
        for x in [Some(col + 2), col.checked_sub(len + 1)]
            .into_iter()
            .flatten()
        {
            if (x.saturating_sub(1)..=x + len).all(|c| canvas.free(c, row)) {
                canvas.text(x, row, text, color);
                return true;
            }
        }
    }
    false
}
