//! Package name pins over the world: the placement loop of `worldDraw`.
//! A pin goes 65 px above its plate's centre, or the first free slot of a
//! small search grid around it; pins that find no slot stay hidden.

/// A screen rectangle by centre and size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn overlaps(&self, o: &Rect) -> bool {
        (o.x - self.x).abs() < (o.w + self.w) / 2.0 + 6.0
            && (o.y - self.y).abs() < (o.h + self.h) / 2.0 + 6.0
    }
}

/// Height of a pin (two text lines with padding) in pixels.
pub const PIN_H: f64 = 50.0;

/// Estimated pin width for its two lines; at most 180 px as in the web.
pub fn width(name: &str, sub: &str, module: bool) -> f64 {
    let text = (name.chars().count() as f64 * 7.8).max(sub.chars().count() as f64 * 6.2);
    (text + 26.0 + if module { 18.0 } else { 0.0 }).min(180.0)
}

/// `Math.max(lo, Math.min(hi, v))`: `lo` wins when the canvas is too small.
fn fit(v: f64, lo: f64, hi: f64) -> f64 {
    lo.max(hi.min(v))
}

/// Place pins in priority order. `anchors` are plate centres on screen,
/// `sizes` the pin widths; `occupied` starts with the floating controls.
pub fn place(
    anchors: &[(f64, f64)],
    widths: &[f64],
    canvas: (f64, f64),
    mut occupied: Vec<Rect>,
) -> Vec<Option<Rect>> {
    let (w, h) = canvas;
    anchors
        .iter()
        .zip(widths)
        .map(|(&(px, py), &bw)| {
            if !(0.0..=w).contains(&px) || !(0.0..=h).contains(&py) {
                return None;
            }
            let slot = slots().find_map(|(dx, dy)| {
                let r = Rect {
                    x: fit(px + dx * (bw + 8.0), bw / 2.0 + 8.0, w - bw / 2.0 - 8.0),
                    y: fit(
                        py - 65.0 + dy * (PIN_H + 8.0),
                        PIN_H / 2.0 + 8.0,
                        h - PIN_H / 2.0 - 8.0,
                    ),
                    w: bw,
                    h: PIN_H,
                };
                (!occupied.iter().any(|o| o.overlaps(&r))).then_some(r)
            });
            if let Some(r) = slot {
                occupied.push(r);
            }
            slot
        })
        .collect()
}

fn slots() -> impl Iterator<Item = (f64, f64)> {
    [0.0, -1.0, 1.0, -2.0, 2.0, -3.0, 3.0, -4.0, 4.0]
        .into_iter()
        .flat_map(|dy| [0.0, -1.0, 1.0].into_iter().map(move |dx| (dx, dy)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_take_the_first_free_slot() {
        let anchors = [(500.0, 400.0), (500.0, 400.0), (-5.0, 10.0)];
        let placed = place(
            &anchors,
            &[100.0, 100.0, 100.0],
            (1000.0, 800.0),
            Vec::new(),
        );
        assert_eq!(
            placed[0],
            Some(Rect {
                x: 500.0,
                y: 335.0,
                w: 100.0,
                h: PIN_H
            })
        );
        // Same anchor: the slot left of the first one (dx -1, dy 0).
        assert_eq!(placed[1].map(|r| (r.x, r.y)), Some((392.0, 335.0)));
        // Off-canvas anchors get no pin.
        assert_eq!(placed[2], None);
    }

    #[test]
    fn controls_block_slots_and_pins_stay_inside() {
        let blocker = Rect {
            x: 500.0,
            y: 335.0,
            w: 400.0,
            h: 60.0,
        };
        let placed = place(&[(500.0, 400.0)], &[100.0], (1000.0, 800.0), vec![blocker]);
        let r = placed[0].unwrap();
        assert!(!blocker.overlaps(&r));
        assert_eq!((r.x, r.y), (500.0, 335.0 - 58.0 * 2.0));
        let edge = place(&[(2.0, 2.0)], &[100.0], (1000.0, 800.0), Vec::new());
        assert_eq!(edge[0].map(|r| (r.x, r.y)), Some((58.0, 33.0)));
        assert_eq!(width("x", "", true), 7.8 + 26.0 + 18.0);
        assert_eq!(width(&"n".repeat(40), "", false), 180.0);
    }
}
