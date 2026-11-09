//! Pixel check of the Start tab in a split pane of about 500 px: the
//! header controls, the legend, the status figures and the footer's
//! search and area picker wrap or stack instead of running past the
//! card's right edge, where the card clips them.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { StartView, StartData } from "ui/start_view.slint";
    export { StartData }
    export component Page inherits Window {
        StartView {}
    }
}

const W: usize = 500;
const H: usize = 820;
/// Strips of the card's padding next to its border: page padding 18,
/// card border 1, card padding 24 (the World's controls keep 12 px).
const LEFT: std::ops::Range<usize> = 21..29;
const RIGHT: std::ops::Range<usize> = W - 29..W - 21;

struct Pixels(Rc<MinimalSoftwareWindow>);

impl slint::platform::Platform for Pixels {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn frame(window: &MinimalSoftwareWindow) -> Vec<PremultipliedRgbaColor> {
    let mut buf = vec![PremultipliedRgbaColor::default(); W * H];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, W);
    });
    buf
}

fn texts(items: &[&str]) -> ModelRc<slint::SharedString> {
    ModelRc::new(VecModel::from(
        items.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
    ))
}

/// The architecture World of a repository with a long name, as in the
/// split pane of the audit.
fn model(lens: i32, kpi: i32, services: bool) -> StartModel {
    let crumb = |label: &str, last: bool, services: bool| StartCrumb {
        label: label.into(),
        last,
        services,
    };
    StartModel {
        ready: true,
        lens,
        kpi,
        crumbs: ModelRc::new(VecModel::from(vec![
            crumb("", false, true),
            crumb("agent-a834101c366a70d0f", true, false),
        ])),
        can_back: true,
        cycle_groups: KpiItem {
            label: "area cycle groups".into(),
            value: "0".into(),
            ..Default::default()
        },
        areas: texts(&["Choose an area…", "crates/archguard-desktop"]),
        count: 1271,
        tiles: 400,
        services: StartServices {
            available: true,
            shown: services,
            deployments: texts(&["All", "checkout", "fulfillment"]),
            complete: true,
            islands: 14,
            local: 10,
            connections: 15,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The rows (y) where something is drawn into the card's right padding
/// but not into its left one: content cut off at the right edge. The
/// World and the rules span both strips alike.
fn cut_off(buf: &[PremultipliedRgbaColor]) -> Vec<usize> {
    let bg = buf[30 * W + W - 30];
    let ink = |y: usize, xs: std::ops::Range<usize>| {
        xs.into_iter().any(|x| {
            let p = buf[y * W + x];
            // The World's gradient differs by a shade between the sides.
            u16::from(p.red.abs_diff(bg.red))
                + u16::from(p.green.abs_diff(bg.green))
                + u16::from(p.blue.abs_diff(bg.blue))
                > 8
        })
    };
    (24..H - 24)
        .filter(|&y| ink(y, RIGHT) && !ink(y, LEFT))
        .collect()
}

#[test]
fn the_start_card_fits_a_narrow_pane() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(W as u32, H as u32));
    slint::platform::set_platform(Box::new(Pixels(window.clone()))).expect("platform");
    let page = Page::new().expect("component");
    page.show().expect("shown");
    let cases = [
        ("problems", model(0, 0, false)),
        ("explore NOC", model(1, 4, false)),
        ("test gaps", model(2, 0, false)),
        ("services", model(0, 0, true)),
    ];
    for (name, m) in cases {
        page.global::<StartData>().set_model(m);
        let buf = frame(&window);
        let rows = cut_off(&buf);
        assert!(rows.is_empty(), "{name}: cut off at y {rows:?}");
    }
}
