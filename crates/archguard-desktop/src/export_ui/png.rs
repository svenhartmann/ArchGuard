//! The PNG of a pane: its rectangle in the window, as `ui/app.slint`
//! places the panes, cropped from the rendered window.

use crate::export::Rect;

/// Pane gap and minimum width (`gap`, `min-pane` of the body in
/// `ui/app.slint`).
const GAP: f32 = 5.0;
const MIN_PANE: f32 = 380.0;

/// The panes' area and layout, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub width: f32,
    pub height: f32,
    /// The body's top edge (below documents, header and bars).
    pub top: f32,
    pub split: bool,
    /// The left pane's share of the width.
    pub ratio: f32,
}

/// `pane`'s rectangle as `x, y, width, height` in logical pixels.
pub fn pane_area(b: Body, pane: usize) -> (f32, f32, f32, f32) {
    let height = (b.height - b.top).max(0.0);
    if !b.split {
        return (0.0, b.top, b.width, height);
    }
    let left = (b.width * b.ratio)
        .min(b.width - MIN_PANE - GAP)
        .max(MIN_PANE);
    if pane == 0 {
        (0.0, b.top, left, height)
    } else {
        (left + GAP, b.top, (b.width - left - GAP).max(0.0), height)
    }
}

/// `area` in physical pixels at `scale`, within a `max_w` × `max_h`
/// image.
pub fn physical(area: (f32, f32, f32, f32), scale: f32, max_w: u32, max_h: u32) -> Rect {
    let px = |v: f32| (v * scale).round().max(0.0) as u32;
    let (x, y) = (px(area.0).min(max_w), px(area.1).min(max_h));
    Rect {
        x,
        y,
        width: px(area.2).min(max_w - x),
        height: px(area.3).min(max_h - y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(split: bool, ratio: f32) -> Body {
        Body {
            width: 1480.0,
            height: 920.0,
            top: 120.0,
            split,
            ratio,
        }
    }

    /// One pane fills the body; two split it at the ratio with the gap
    /// between, each at least 380 px, as `ui/app.slint` places them.
    #[test]
    fn panes_are_placed_as_in_the_window() {
        assert_eq!(pane_area(body(false, 0.5), 0), (0.0, 120.0, 1480.0, 800.0));
        assert_eq!(pane_area(body(true, 0.5), 0), (0.0, 120.0, 740.0, 800.0));
        assert_eq!(pane_area(body(true, 0.5), 1), (745.0, 120.0, 735.0, 800.0));
        assert_eq!(pane_area(body(true, 0.1), 0).2, 380.0);
        assert_eq!(pane_area(body(true, 0.95), 1).2, 380.0);
    }

    /// Logical × scale, kept inside the window image.
    #[test]
    fn the_area_scales_to_physical_pixels() {
        let r = physical((745.0, 120.0, 735.0, 800.0), 2.0, 2960, 1840);
        assert_eq!((r.x, r.y, r.width, r.height), (1490, 240, 1470, 1600));
        let clipped = physical((745.0, 120.0, 900.0, 900.0), 2.0, 2960, 1840);
        assert_eq!((clipped.width, clipped.height), (1470, 1600));
    }
}
