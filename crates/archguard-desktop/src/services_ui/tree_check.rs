//! Checks of the Services tab in the software renderer: the topology tree
//! creates only the rows near the visible band, also for a large tree,
//! and the toolbar fits a narrow split pane.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use slint::platform::WindowEvent;
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { ServicesTree, SvcRow } from "ui/services_tree.slint";
    import { ServicesView, ServicesData, ServicesModel } from "ui/services_view.slint";
    export { SvcRow, ServicesData, ServicesModel }
    export component Tree inherits Window {
        in property <[SvcRow]> rows;
        in property <float> canvas-h;
        ServicesTree { rows: root.rows; canvas-w: 400; canvas-h: root.canvas-h; }
    }
    export component Page inherits Window {
        ServicesView {}
    }
}

const W: usize = 400;
const H: usize = 600;
/// Row pitch of the layout (`layout.rs`).
const PITCH: f32 = 30.0;

struct Pixels(Rc<MinimalSoftwareWindow>);

impl slint::platform::Platform for Pixels {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn window(w: usize, h: usize) -> Rc<MinimalSoftwareWindow> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(w as u32, h as u32));
    slint::platform::set_platform(Box::new(Pixels(window.clone()))).expect("platform");
    window
}

fn frame(window: &MinimalSoftwareWindow, w: usize, h: usize) -> Vec<PremultipliedRgbaColor> {
    let mut buf = vec![PremultipliedRgbaColor::default(); w * h];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, w);
    });
    buf
}

/// A list model that records which rows the view reads.
struct Counting {
    rows: VecModel<SvcRow>,
    read: RefCell<BTreeSet<usize>>,
}

impl Model for Counting {
    type Data = SvcRow;
    fn row_count(&self) -> usize {
        self.rows.row_count()
    }
    fn row_data(&self, row: usize) -> Option<SvcRow> {
        self.read.borrow_mut().insert(row);
        self.rows.row_data(row)
    }
    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        self.rows.model_tracker()
    }
}

fn row(i: usize) -> SvcRow {
    SvcRow {
        name: format!("service-{i}").into(),
        service: i as i32,
        left: 24.0,
        y: 38.0 + i as f32 * PITCH,
        w: 200.0,
        h: 28.0,
        label_width: 100.0,
        relation_x: 300.0,
        compact: true,
        incoming: 1,
        outgoing: 2,
        ..Default::default()
    }
}

fn scroll(window: &MinimalSoftwareWindow, delta_y: f32) {
    window.dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(100.0, 300.0),
    });
    window.dispatch_event(WindowEvent::PointerScrolled {
        position: LogicalPosition::new(100.0, 300.0),
        delta_x: 0.0,
        delta_y,
    });
}

/// Whether a row label is drawn left of the relation axis at height `y`.
fn label_at(buf: &[PremultipliedRgbaColor], y: usize) -> bool {
    let bg = buf[y * W + W - 2];
    (50..200).any(|x| {
        let p = buf[y * W + x];
        (p.red, p.green, p.blue) != (bg.red, bg.green, bg.blue)
    })
}

#[test]
fn a_large_tree_creates_only_the_rows_near_the_view() {
    let window = window(W, H);
    let tree = Tree::new().expect("component");
    let n = 5000;
    let rows = Rc::new(Counting {
        rows: VecModel::from((0..n).map(row).collect::<Vec<_>>()),
        read: RefCell::default(),
    });
    tree.set_rows(ModelRc::from(rows.clone()));
    tree.set_canvas_h(38.0 + n as f32 * PITCH + 38.0);
    tree.show().expect("shown");
    let buf = frame(&window, W, H);
    // The visible band and one screen height above and below it.
    let near = 3 * H / PITCH as usize + 4;
    let read = std::mem::take(&mut *rows.read.borrow_mut());
    assert!(read.len() <= near, "{} of {n} rows read", read.len());
    assert!(label_at(&buf, 38), "the first row is drawn");
    // A row changed in place (`sync.rs`) shows its new data.
    let corner = |buf: &[PremultipliedRgbaColor]| buf[28 * W + 30];
    let plain = corner(&buf);
    rows.rows.set_row_data(
        0,
        SvcRow {
            selected: true,
            ..row(0)
        },
    );
    let lit = corner(&frame(&window, W, H));
    assert_ne!(
        (lit.red, lit.green, lit.blue),
        (plain.red, plain.green, plain.blue),
        "selection drawn"
    );
    rows.read.borrow_mut().clear();
    // Far down the tree the rows there are created, each step only a band.
    let mut last = BTreeSet::new();
    for _ in 0..40 {
        scroll(&window, -2000.0);
        frame(&window, W, H);
        last = std::mem::take(&mut *rows.read.borrow_mut());
        assert!(last.len() <= near, "{} rows read in one step", last.len());
    }
    assert!(last.iter().any(|&r| r > 1000), "scrolled rows: {last:?}");
    let buf = frame(&window, W, H);
    assert!((0..H).any(|y| label_at(&buf, y)), "rows drawn");
}

const PW: usize = 380;
const PH: usize = 700;

/// The narrow pane's toolbar: no control runs past the right edge.
#[test]
fn the_toolbar_fits_a_narrow_pane() {
    let window = window(PW, PH);
    let page = Page::new().expect("component");
    page.show().expect("shown");
    page.global::<ServicesData>().set_model(ServicesModel {
        state: 1,
        rows: ModelRc::new(VecModel::from((0..4).map(row).collect::<Vec<_>>())),
        width: 360.0,
        height: 200.0,
        zoom: 1.0,
        ..Default::default()
    });
    let buf = frame(&window, PW, PH);
    let bg = buf[(PH - 4) * PW + 4];
    let same = |p: PremultipliedRgbaColor| (p.red, p.green, p.blue) == (bg.red, bg.green, bg.blue);
    // The toolbar lies between the two full-width lines below the summary.
    let lines: Vec<usize> = (0..PH)
        .filter(|&y| [2, PW / 2, PW - 2].iter().all(|&x| !same(buf[y * PW + x])))
        .collect();
    assert!(lines.len() >= 2, "separators at {lines:?}");
    let (top, bottom) = (lines[0] + 1, lines[lines.len() - 1]);
    assert!(
        bottom > top + 60,
        "the toolbar wraps into rows: {top}..{bottom}"
    );
    let ink = (top..bottom)
        .flat_map(|y| (PW - 12..PW).map(move |x| (x, y)))
        .find(|&(x, y)| !same(buf[y * PW + x]));
    assert_eq!(ink, None, "a control drawn into the right padding");
}
