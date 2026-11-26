#![cfg_attr(not(test), allow(dead_code))]
// Unused until ticket 12.
//! Column layout and edge paths, ticket 08.

use std::collections::{BTreeMap, HashMap};

use super::boxes::{BoxContent, Chip, Compartment};
use super::diagram::{BoxKind, DEdge, Diagram, EdgeStyle};
use super::types::Key;
use crate::diagram::{HeadShape, Point, curve_stroke, head_path, line_stroke};

/// A positioned box. `comps` are the compartment tops (relative to the box); `chips` are
/// `(x, y, w)` of all chips in compartment order, relative to their compartment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placed {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub comps: Vec<f32>,
    pub chips: Vec<(f32, f32, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedEdge {
    pub line: String,
    pub head: String,
    pub shape: HeadShape,
    pub dashed: bool,
    pub style: EdgeStyle,
    pub label: String,
    pub lx: f32,
    pub ly: f32,
}

/// `boxes` is index-aligned with `Diagram::boxes`, `edges` follows `Diagram::edges`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub boxes: Vec<Placed>,
    pub edges: Vec<PlacedEdge>,
    pub width: f32,
    pub height: f32,
}

const COL_W: [f32; 4] = [230.0, 300.0, 270.0, 190.0];
const MARGIN: f32 = 20.0;
const GAP_X: f32 = 80.0;
const GAP_Y: f32 = 24.0;
const ROW: f32 = 16.0;
const HEAD: f32 = 3.0 * ROW;
const CHIP_ROW: f32 = 18.0;
const PAD: f32 = 8.0;

/// 0 services and processing, 1 aliases/templates/indices, 2 analyzers/normalizers,
/// 3 tokenizers/filters/char filters.
fn column(k: BoxKind) -> usize {
    match k {
        BoxKind::Service | BoxKind::Processing => 0,
        BoxKind::Alias | BoxKind::Template | BoxKind::Index => 1,
        BoxKind::Analyzer | BoxKind::Normalizer => 2,
        _ => 3,
    }
}

/// Chips of one compartment wrapped at the box width; returns the number of chip rows.
fn place_chips(chips: &[Chip], w: f32, top: f32, out: &mut Vec<(f32, f32, f32)>) -> f32 {
    let (mut x, mut row) = (PAD, 0.0);
    for c in chips {
        let cw = c.text.chars().count() as f32 * 7.0 + 16.0;
        if x > PAD && x + cw > w - PAD {
            (x, row) = (PAD, row + 1.0);
        }
        out.push((x, top + 2.0 + row * CHIP_ROW, cw));
        x += cw + 4.0;
    }
    if chips.is_empty() { 0.0 } else { row + 1.0 }
}

/// Size, compartment tops and chip positions of a box; its position is set later.
fn measure(c: Option<&BoxContent>, w: f32) -> Placed {
    let mut p = Placed {
        w,
        h: HEAD,
        ..Default::default()
    };
    let comps: &[Compartment] = c.map_or(&[], |c| &c.compartments);
    for k in comps {
        p.comps.push(p.h);
        let top = ROW * (1 + k.lines.len()) as f32;
        let rows = place_chips(&k.chips, w, top, &mut p.chips);
        p.h += top + rows * CHIP_ROW;
    }
    p
}

/// Aliases and templates directly above the first index they point to; groups ordered by their
/// first member (the selection comes first in the diagram).
fn order_column1(kinds: &[BoxKind], col: &[usize], nbrs: &[Vec<usize>]) -> Vec<usize> {
    let (mut slot, mut groups): (HashMap<usize, usize>, Vec<Vec<usize>>) = Default::default();
    for i in (0..col.len()).filter(|&i| col[i] == 1) {
        let target = nbrs[i].iter().find(|&&j| kinds[j] == BoxKind::Index);
        let leader = if kinds[i] == BoxKind::Index {
            i
        } else {
            *target.unwrap_or(&i)
        };
        let g = *slot.entry(leader).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[g].push(i);
    }
    for g in &mut groups {
        g.sort_by_key(|&i| kinds[i] == BoxKind::Index);
    }
    groups.concat()
}

/// Where the owner's text first names the building block (shortcut: substring match, so a
/// name inside a longer name can sort early; upgrade when the engine reports field order).
fn use_pos(owner: Option<&BoxContent>, name: &str) -> usize {
    let lines = owner
        .into_iter()
        .flat_map(|c| &c.compartments)
        .flat_map(|k| &k.lines);
    lines
        .into_iter()
        .position(|l| {
            [&l.text, &l.kind_text, &l.note]
                .iter()
                .any(|t| t.contains(name))
        })
        .unwrap_or(usize::MAX)
}

/// Column 2 in the order of first use by the (already ordered) column 1.
fn order_column2(
    order1: &[usize],
    col: &[usize],
    nbrs: &[Vec<usize>],
    (contents, names): (&[BoxContent], &[&str]),
) -> Vec<usize> {
    let mut seen = vec![false; col.len()];
    let mut out = Vec::new();
    for &i in order1 {
        let mut ns: Vec<usize> = nbrs[i].iter().copied().filter(|&j| col[j] == 2).collect();
        ns.sort_by_key(|&j| use_pos(contents.get(i), names[j]));
        for j in ns
            .into_iter()
            .filter(|&j| !std::mem::replace(&mut seen[j], true))
        {
            out.push(j);
        }
    }
    out.extend((0..col.len()).filter(|&j| col[j] == 2 && !seen[j]));
    out
}

/// Stack boxes top to bottom, each at its wanted `y` unless that overlaps the box above; an
/// infinite wish means "just below the previous".
fn stack(order: &[(usize, f32)], boxes: &mut [Placed]) {
    let mut cur = MARGIN;
    for &(i, want) in order {
        let y = if want.is_finite() { cur.max(want) } else { cur };
        boxes[i].y = y;
        cur = y + boxes[i].h + GAP_Y;
    }
}

/// The boxes of `order` at the height of their topmost neighbour in column `from`; equal
/// heights keep the order given.
fn near(order: &[usize], from: usize, col: &[usize], nbrs: &[Vec<usize>], boxes: &mut [Placed]) {
    let top = |i: usize| {
        nbrs[i]
            .iter()
            .filter(|&&j| col[j] == from)
            .map(|&j| boxes[j].y)
            .fold(f32::INFINITY, f32::min)
    };
    let mut wishes: Vec<(usize, f32)> = order.iter().map(|&i| (i, top(i))).collect();
    wishes.sort_by(|a, b| a.1.total_cmp(&b.1));
    stack(&wishes, boxes);
}

/// Boxes of column `c`, tokenizers before filters before char filters.
fn column_by_kind(c: usize, col: &[usize], kinds: &[BoxKind]) -> Vec<usize> {
    let mut v: Vec<usize> = (0..col.len()).filter(|&i| col[i] == c).collect();
    v.sort_by_key(|&i| kinds[i]);
    v
}

/// Ports at the header's middle: right to left between columns, bottom to top in one column.
fn ports(a: &Placed, b: &Placed, same: bool) -> (Point, Point) {
    if same {
        let (down, up) = if a.y <= b.y { (a, b) } else { (b, a) };
        let (p, q) = (
            (down.x + down.w / 2.0, down.y + down.h),
            (up.x + up.w / 2.0, up.y),
        );
        return if a.y <= b.y { (p, q) } else { (q, p) };
    }
    let (ay, by) = (a.y + HEAD / 2.0, b.y + HEAD / 2.0);
    if a.x < b.x {
        ((a.x + a.w, ay), (b.x, by))
    } else {
        ((a.x, ay), (b.x + b.w, by))
    }
}

fn place_edge(e: &DEdge, a: &Placed, b: &Placed, same: bool) -> PlacedEdge {
    let (p, q) = ports(a, b, same);
    let (shape, dashed) = match e.style {
        EdgeStyle::Write | EdgeStyle::Read => (HeadShape::Filled, false),
        EdgeStyle::Covers => (HeadShape::Hollow, true),
        EdgeStyle::Alias => (HeadShape::Diamond, false),
        EdgeStyle::Uses => (HeadShape::Open, true),
        _ => (HeadShape::Open, false),
    };
    let st = if same {
        line_stroke(p, q, dashed)
    } else {
        curve_stroke(p, q, dashed)
    };
    let end = if shape == HeadShape::Diamond {
        HeadShape::Open
    } else {
        shape
    };
    let mut head = head_path(end, st.tail, q);
    if shape == HeadShape::Diamond {
        // From the target: the body then extends from the start point toward it.
        head = format!("{} {head}", head_path(HeadShape::Diamond, q, p));
    }
    let shape = if shape == HeadShape::Hollow || shape == HeadShape::Diamond {
        shape
    } else {
        end
    };
    PlacedEdge {
        line: st.path,
        head,
        shape,
        dashed,
        style: e.style,
        label: e.label.clone(),
        lx: st.mid.0,
        ly: st.mid.1,
    }
}

/// Box indices at both ends of each edge (`None` if an end is not in the diagram), and the
/// neighbours of each box in edge order.
type Ends = Vec<Option<(usize, usize)>>;
fn neighbours(d: &Diagram) -> (Ends, Vec<Vec<usize>>) {
    let idx: BTreeMap<_, _> = d
        .boxes
        .iter()
        .enumerate()
        .map(|(i, b)| (&b.key, i))
        .collect();
    let ends: Ends = d
        .edges
        .iter()
        .map(|e| idx.get(&e.from).zip(idx.get(&e.to)).map(|(&a, &b)| (a, b)))
        .collect();
    let mut nbrs = vec![Vec::new(); d.boxes.len()];
    for &(a, b) in ends.iter().flatten() {
        nbrs[a].push(b);
        nbrs[b].push(a);
    }
    (ends, nbrs)
}

/// Positions, sizes, chip positions and edge paths in four columns. `contents` is index-aligned
/// with `d.boxes`.
pub fn layout(d: &Diagram, contents: &[BoxContent]) -> Scene {
    let kinds: Vec<BoxKind> = d.boxes.iter().map(|b| b.kind).collect();
    let col: Vec<usize> = kinds.iter().map(|&k| column(k)).collect();
    let mut boxes: Vec<Placed> = (0..kinds.len())
        .map(|i| measure(contents.get(i), COL_W[col[i]]))
        .collect();
    let (ends, nbrs) = neighbours(d);
    let mut x = MARGIN;
    let xs: Vec<f32> = COL_W
        .iter()
        .map(|w| {
            let at = x;
            x += w + GAP_X;
            at
        })
        .collect();
    for (p, &c) in boxes.iter_mut().zip(&col) {
        p.x = xs[c];
    }
    // A part's `BoxContent::name` is its owner's label, so take the name from the key.
    let names: Vec<&str> = d
        .boxes
        .iter()
        .map(|b| match &b.key {
            Key::Part { name, .. } => name.as_str(),
            Key::Node(_) => "",
        })
        .collect();
    let order1 = order_column1(&kinds, &col, &nbrs);
    let order2 = order_column2(&order1, &col, &nbrs, (contents, &names));
    let wishes: Vec<(usize, f32)> = order1.iter().map(|&i| (i, f32::INFINITY)).collect();
    stack(&wishes, &mut boxes);
    near(&order2, 1, &col, &nbrs, &mut boxes);
    near(&column_by_kind(3, &col, &kinds), 2, &col, &nbrs, &mut boxes);
    near(&column_by_kind(0, &col, &kinds), 1, &col, &nbrs, &mut boxes);
    let edges = d
        .edges
        .iter()
        .zip(&ends)
        .filter_map(|(e, end)| {
            let &(a, b) = end.as_ref()?;
            Some(place_edge(e, &boxes[a], &boxes[b], col[a] == col[b]))
        })
        .collect();
    let bottom = boxes.iter().map(|p| p.y + p.h).fold(0.0, f32::max);
    let (width, height) = if boxes.is_empty() {
        (0.0, 0.0)
    } else {
        (xs[3] + COL_W[3] + MARGIN, bottom + MARGIN)
    };
    Scene {
        boxes,
        edges,
        width,
        height,
    }
}
