//! Render tests for the class view (`es_class.slint`), ticket 12: three columns, details place.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { EsClassView, EsClassData, EsClassModel } from "../archguard-desktop-ui/ui/es_class.slint";
    import { EsClassGroup, EsClassRow } from "../archguard-desktop-ui/ui/es_class_list.slint";
    import { EsClassBox } from "../archguard-desktop-ui/ui/es_class_canvas.slint";
    import { EsDetails, EsDetailHead } from "../archguard-desktop-ui/ui/es_class_details.slint";
    export { EsClassData, EsClassModel, EsClassGroup, EsClassRow, EsClassBox, EsDetails, EsDetailHead }
    export component Page inherits Window {
        EsClassView {}
    }
}

const H: usize = 700;

struct Pixels(Rc<MinimalSoftwareWindow>);

impl slint::platform::Platform for Pixels {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn window(w: u32) -> Rc<MinimalSoftwareWindow> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(w, H as u32));
    slint::platform::set_platform(Box::new(Pixels(window.clone()))).expect("platform");
    window
}

fn frame(window: &MinimalSoftwareWindow, w: usize) -> Vec<PremultipliedRgbaColor> {
    let mut buf = vec![PremultipliedRgbaColor::default(); w * H];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, w);
    });
    buf
}

/// Pixels of the rectangle that differ between two frames.
fn diff(
    a: &[PremultipliedRgbaColor],
    b: &[PremultipliedRgbaColor],
    w: usize,
    x: (usize, usize),
    y: (usize, usize),
) -> usize {
    let at = |p: &PremultipliedRgbaColor| (p.red, p.green, p.blue, p.alpha);
    (y.0..y.1)
        .flat_map(|r| (x.0..x.1).map(move |c| r * w + c))
        .filter(|&i| at(&a[i]) != at(&b[i]))
        .count()
}

fn model(full: bool) -> EsClassModel {
    let mut m = EsClassModel {
        state: 1,
        switches: ModelRc::new(VecModel::from(vec![true, true, true, false])),
        zoom: 1.0,
        canvas_w: 400.0,
        canvas_h: 300.0,
        ..Default::default()
    };
    if full {
        let row = EsClassRow {
            label: "orders".into(),
            index: 0,
            y: 30.0,
            ..Default::default()
        };
        let group = EsClassGroup {
            title: 1,
            rows: ModelRc::new(VecModel::from(vec![row])),
            ..Default::default()
        };
        m.groups = ModelRc::new(VecModel::from(vec![group]));
        let b = EsClassBox {
            x: 20.0,
            y: 20.0,
            w: 200.0,
            h: 120.0,
            name: "orders".into(),
            stereotype: "index".into(),
            ..Default::default()
        };
        m.boxes = ModelRc::new(VecModel::from(vec![b]));
        m.details = EsDetails {
            head: EsDetailHead {
                title: "orders".into(),
                ..Default::default()
            },
            ..Default::default()
        };
    }
    m
}

type Frame = Vec<PremultipliedRgbaColor>;

/// The view with the full model and the same view with an empty one (the reference).
fn shown(w: usize) -> (Frame, Frame) {
    let window = window(w as u32);
    let page = Page::new().expect("component");
    page.show().expect("shown");
    let data = page.global::<EsClassData>();
    data.set_model(model(false));
    let empty = frame(&window, w);
    data.set_model(model(true));
    (frame(&window, w), empty)
}

#[test]
fn three_columns_are_drawn_and_the_details_sit_right_on_a_wide_view() {
    let w = 1400;
    let (drawn, empty) = shown(w);
    let list = diff(&drawn, &empty, w, (0, 280), (100, 400));
    let canvas = diff(&drawn, &empty, w, (300, 700), (120, 450));
    let details = diff(&drawn, &empty, w, (w - 410, w - 200), (60, 300));
    assert!(list > 50, "the list shows its rows: {list}");
    assert!(canvas > 50, "the canvas shows the box: {canvas}");
    assert!(details > 50, "the details sit at the right: {details}");
}

/// The full model against the same one without details: only the details differ.
fn details_only(w: usize) -> (Frame, Frame) {
    let window = window(w as u32);
    let page = Page::new().expect("component");
    page.show().expect("shown");
    let data = page.global::<EsClassData>();
    let mut bare = model(true);
    bare.details = EsDetails::default();
    data.set_model(bare);
    let without = frame(&window, w);
    data.set_model(model(true));
    (frame(&window, w), without)
}

#[test]
fn a_narrow_view_puts_the_details_below_the_canvas() {
    let (drawn, without) = details_only(800);
    let top = diff(&drawn, &without, 800, (280, 800), (0, 330));
    let below = diff(&drawn, &without, 800, (280, 800), (330, 700));
    assert!(below > 50, "the details lie under the canvas: {below}");
    assert_eq!(top, 0, "nothing of them beside the canvas");
}

#[test]
fn the_canvas_asks_for_a_fit_when_the_tick_changes() {
    let window = window(1400);
    let page = Page::new().expect("component");
    page.show().expect("shown");
    let data = page.global::<EsClassData>();
    let asked = Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = asked.clone();
    data.on_fit(move |w, h| log.borrow_mut().push((w, h)));
    data.set_model(model(true));
    frame(&window, 1400);
    asked.borrow_mut().clear();
    let mut next = model(true);
    next.fit_tick = 1;
    data.set_model(next);
    frame(&window, 1400);
    let asked = asked.borrow();
    assert_eq!(asked.len(), 1, "one fit per tick: {asked:?}");
    assert!(
        asked[0].0 > 300.0 && asked[0].1 > 200.0,
        "the canvas's size: {asked:?}"
    );
}

#[test]
fn class_texts_are_translated() {
    let slint = include_str!("../../../../archguard-desktop-ui/ui/es_class.slint");
    let texts: Vec<&str> = slint
        .split("@tr(\"")
        .skip(1)
        .map(|r| r.split('"').next().unwrap_or_default())
        .collect();
    assert!(!texts.is_empty(), "the view has translatable texts");
    let po = crate::tab_support::texts_check::po_de();
    let missing: Vec<_> = texts
        .iter()
        .filter(|t| po.get(**t).is_none_or(|g| g.is_empty()))
        .collect();
    assert!(missing.is_empty(), "no German text for {missing:?}");
}
