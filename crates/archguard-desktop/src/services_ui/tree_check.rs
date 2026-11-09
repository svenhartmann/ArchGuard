//! Checks of the Services tab in the software renderer: the toolbar fits
//! a narrow split pane.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { SvcRow } from "ui/services_tree.slint";
    import { ServicesView, ServicesData, ServicesModel } from "ui/services_view.slint";
    export { SvcRow, ServicesData, ServicesModel }
    export component Page inherits Window {
        ServicesView {}
    }
}

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

fn row(i: usize) -> SvcRow {
    SvcRow {
        name: format!("service-{i}").into(),
        service: i as i32,
        left: 24.0,
        y: 38.0 + i as f32 * 30.0,
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
