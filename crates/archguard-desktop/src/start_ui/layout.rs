//! Ground plan of the world: the port of `worldLayout` in
//! `viewer_world_model.js`. Areas are rows of square plates; each plate
//! holds one surface (cell) per entity, or per batch of entities when a
//! scope has more than 750 of them.

/// One area's rectangle on the ground grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Plate {
    /// Index into the scope's areas.
    pub area: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One surface: a grid square with a peak of height `z`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub x: i64,
    pub y: i64,
    pub z: f64,
    /// Highest measured value of the members, `None` when none is measured.
    pub value: Option<f64>,
    /// Index into the scope's areas.
    pub area: usize,
    /// Indices into the world's entities.
    pub entities: Vec<usize>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub plates: Vec<Plate>,
    pub cells: Vec<Cell>,
}

/// An area as the layout needs it: its name and members, already sorted by
/// entity id.
pub struct Region<'a> {
    pub name: &'a str,
    pub entities: &'a [usize],
}

/// The default plate height: log-scaled maximum method complexity.
pub fn complexity_height(value: Option<f64>) -> f64 {
    value.map_or(0.08, |v| 0.08 + (1.0 + v).log2() * 0.36)
}

/// Plate height for a class metric, normalised to the highest value shown.
pub fn metric_height(value: Option<f64>, view_max: f64) -> f64 {
    value.map_or(0.08, |v| {
        0.08 + (1.0 + v).log2() / (1.0 + view_max.max(1.0)).log2() * 2.0
    })
}

/// Lay out the regions in name order, wrapping rows at a width that keeps
/// the plan roughly square. `value` gives an entity's metric, `height` the
/// peak height for a cell value.
pub fn layout(
    regions: &[Region<'_>],
    value: impl Fn(usize) -> Option<f64>,
    height: impl Fn(Option<f64>) -> f64,
) -> Layout {
    let mut order: Vec<usize> = (0..regions.len()).collect();
    order.sort_by(|&a, &b| regions[a].name.cmp(regions[b].name));
    let total: usize = regions.iter().map(|r| r.entities.len()).sum();
    let batch = total.div_ceil(750).max(1);
    let width = ((total.min(750) as f64).sqrt() * 1.4).max(8.0);
    let mut out = Layout::default();
    let (mut x, mut y, mut row_height) = (0i64, 0i64, 0i64);
    for area in order {
        let members = regions[area].entities;
        let n = members.len().div_ceil(batch) as i64;
        let cols = ((n as f64).sqrt().ceil() as i64).max(1);
        let rows = (n + cols - 1) / cols;
        if x != 0 && (x + cols) as f64 > width {
            x = 0;
            y += row_height + 2;
            row_height = 0;
        }
        out.plates.push(Plate {
            area,
            x: x as f64,
            y: y as f64,
            w: cols as f64,
            h: rows.max(1) as f64,
        });
        for (i, chunk) in members.chunks(batch).enumerate() {
            let i = i as i64;
            let cell_value = chunk
                .iter()
                .filter_map(|&e| value(e).filter(|v| v.is_finite()))
                .reduce(f64::max);
            out.cells.push(Cell {
                x: x + i % cols,
                y: y + i / cols,
                z: height(cell_value),
                value: cell_value,
                area,
                entities: chunk.to_vec(),
            });
        }
        x += cols + 2;
        row_height = row_height.max(rows);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_of(e: usize) -> Option<f64> {
        (!e.is_multiple_of(3)).then_some(e as f64)
    }

    /// Mirrors `worldLayout` on a small scope: name order, square plates,
    /// a two-unit gap and one cell per entity.
    #[test]
    fn plates_follow_name_order_with_square_grids() {
        let b: Vec<usize> = (0..5).collect();
        let a: Vec<usize> = (5..7).collect();
        let regions = [
            Region {
                name: "pkg/b",
                entities: &b,
            },
            Region {
                name: "pkg/a",
                entities: &a,
            },
        ];
        let l = layout(&regions, value_of, complexity_height);
        assert_eq!(l.plates.len(), 2);
        // "pkg/a" first: 2 entities -> 2 columns, 1 row.
        assert_eq!(
            l.plates[0],
            Plate {
                area: 1,
                x: 0.0,
                y: 0.0,
                w: 2.0,
                h: 1.0
            }
        );
        // "pkg/b": 5 entities -> 3 columns, 2 rows, after a gap of 2.
        assert_eq!(
            l.plates[1],
            Plate {
                area: 0,
                x: 4.0,
                y: 0.0,
                w: 3.0,
                h: 2.0
            }
        );
        assert_eq!(l.cells.len(), 7);
        let last = l.cells.last().unwrap();
        assert_eq!((last.x, last.y, last.area), (5, 1, 0));
        // Entity 0 is unmeasured: the default height, no value.
        let first_b = &l.cells[2];
        assert_eq!(first_b.entities, vec![0]);
        assert_eq!(first_b.value, None);
        assert!((first_b.z - 0.08).abs() < 1e-12);
        let e1 = &l.cells[3];
        assert!((e1.z - (0.08 + 2f64.log2() * 0.36)).abs() < 1e-12);
    }

    /// More than 750 entities share surfaces; a cell keeps its highest value.
    #[test]
    fn large_scopes_group_entities_into_batches() {
        let many: Vec<usize> = (0..1600).collect();
        let regions = [Region {
            name: "x",
            entities: &many,
        }];
        let l = layout(&regions, |e| Some(e as f64), complexity_height);
        // ceil(1600 / 750) = 3 entities per cell.
        assert_eq!(l.cells.len(), 534);
        assert_eq!(l.cells[0].entities, vec![0, 1, 2]);
        assert_eq!(l.cells[0].value, Some(2.0));
        assert_eq!(l.cells.last().unwrap().entities, vec![1599]);
    }

    /// Rows wrap once the next plate would pass the plan width (at least 8).
    #[test]
    fn rows_wrap_at_the_plan_width() {
        let five: Vec<Vec<usize>> = (0..4).map(|i| vec![i * 2, i * 2 + 1]).collect();
        let names = ["a", "b", "c", "d"];
        let regions: Vec<Region<'_>> = names
            .iter()
            .zip(&five)
            .map(|(n, e)| Region {
                name: n,
                entities: e,
            })
            .collect();
        let l = layout(&regions, |_| None, complexity_height);
        let xs: Vec<(f64, f64)> = l.plates.iter().map(|p| (p.x, p.y)).collect();
        // Width 8: plates at 0 and 4 fit, the third (4 + 4 + 2 > 8) wraps.
        assert_eq!(xs, vec![(0.0, 0.0), (4.0, 0.0), (0.0, 3.0), (4.0, 3.0)]);
    }

    #[test]
    fn metric_heights_normalise_to_the_view_maximum() {
        assert!((metric_height(Some(15.0), 15.0) - 2.08).abs() < 1e-12);
        assert!((metric_height(None, 15.0) - 0.08).abs() < 1e-12);
        // A maximum below one counts as one.
        assert!((metric_height(Some(0.0), 0.0) - 0.08).abs() < 1e-12);
    }
}
