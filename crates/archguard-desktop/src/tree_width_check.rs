//! Pixel check of the Graph tree (`ui/tree_view.slint`) at the pane's
//! width: in a narrow split pane the lanes and the tree shrink, so both
//! lanes' ports and the right lane's heading stay in view instead of the
//! canvas being cut at the right edge. At full width the lanes keep their
//! 280 px, as before.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { TreeView, TreeRow } from "ui/tree_view.slint";
    import { Theme } from "ui/theme.slint";
    export { TreeRow }
    export component Tree inherits Window {
        background: Theme.bg;
        in property <[TreeRow]> rows;
        TreeView { rows: root.rows; }
    }
}

const H: usize = 400;
/// A row below the zoom controls, which cover the first rows' left lane.
const ROW: i32 = 4;
/// Centre line of that row (`TreeGeo`: top 34, row height 30, card 26).
const ROW_Y: usize = 34 + 30 * ROW as usize + 13;
/// Port centres are 22 from the tree's edges; a port is 12 wide.
const PORT_GAP: usize = 22;

struct Pixels(Rc<MinimalSoftwareWindow>);

impl slint::platform::Platform for Pixels {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn rows() -> ModelRc<TreeRow> {
    let row = |i: i32| TreeRow {
        row: i,
        node: i,
        label: format!("archguard-package-with-a-long-name-{i}").into(),
        expandable: true,
        in_count: 12,
        out_count: 7,
        ..Default::default()
    };
    ModelRc::new(VecModel::from((0..6).map(row).collect::<Vec<_>>()))
}

/// The tree rendered in a window `w` wide.
fn render(w: usize) -> Vec<PremultipliedRgbaColor> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(w as u32, H as u32));
    slint::platform::set_platform(Box::new(Pixels(window.clone()))).expect("platform");
    let tree = Tree::new().expect("component");
    tree.set_rows(rows());
    tree.show().expect("shown");
    let mut buf = vec![PremultipliedRgbaColor::default(); w * H];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, w);
    });
    buf
}

fn ink(buf: &[PremultipliedRgbaColor], w: usize, x: usize, y: usize) -> bool {
    let (a, b) = (buf[y * w + x], buf[(H - 2) * w + w / 2]);
    (a.red, a.green, a.blue) != (b.red, b.green, b.blue)
}

/// Runs (first, last) of drawn pixels along the row's centre line; gaps of
/// up to 4 px are bridged, so a port's ring and dot make one run.
fn runs(buf: &[PremultipliedRgbaColor], w: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for x in (0..w).filter(|&x| ink(buf, w, x, ROW_Y)) {
        match out.last_mut() {
            Some(run) if x <= run.1 + 5 => run.1 = x,
            _ => out.push((x, x)),
        }
    }
    out
}

fn is_port(run: (usize, usize)) -> bool {
    (9..=15).contains(&(run.1 - run.0 + 1))
}

/// Left port, row card and right port along the row, all inside the
/// window; returns them.
fn lane_parts(buf: &[PremultipliedRgbaColor], w: usize) -> [(usize, usize); 3] {
    let r = runs(buf, w);
    assert_eq!(r.len(), 3, "left port, row card, right port in view: {r:?}");
    assert!(is_port(r[0]) && is_port(r[2]), "ports at both ends: {r:?}");
    assert!(r[2].1 < w - 1, "right port inside the window: {r:?}");
    [r[0], r[1], r[2]]
}

#[test]
fn a_narrow_pane_shows_both_lanes_and_their_headings() {
    let w = 380;
    let buf = render(w);
    let [left, card, right] = lane_parts(&buf, w);
    // Ports keep their distance to the row card.
    assert!(
        card.0 - (left.0 + left.1) / 2 >= PORT_GAP - 1,
        "{left:?} {card:?}"
    );
    assert!(
        (right.0 + right.1) / 2 - card.1 >= PORT_GAP - 1,
        "{card:?} {right:?}"
    );
    // The right lane's heading is drawn in that lane and ends before the edge.
    let buf = &buf;
    let heading: Vec<usize> = (0..28)
        .flat_map(|y| (right.0..w).filter(move |&x| ink(buf, w, x, y)))
        .collect();
    assert!(!heading.is_empty(), "right lane heading in view");
    assert!(
        heading.iter().all(|&x| x < w - 2),
        "heading not cut at the edge"
    );
}

#[test]
fn a_wide_pane_keeps_the_full_lanes() {
    let w = 1280;
    let buf = render(w);
    let [left, card, right] = lane_parts(&buf, w);
    let centre = |r: (usize, usize)| (r.0 + r.1) as f32 / 2.0;
    assert!(
        (centre(left) - 258.0).abs() <= 1.0,
        "left port at 258: {left:?}"
    );
    assert!(
        (card.0 as f32 - 280.0).abs() <= 1.0,
        "tree from 280: {card:?}"
    );
    assert!(
        (card.1 as f32 - 1000.0).abs() <= 1.0,
        "tree to 1000: {card:?}"
    );
    assert!(
        (centre(right) - 1022.0).abs() <= 1.0,
        "right port at 1022: {right:?}"
    );
}
