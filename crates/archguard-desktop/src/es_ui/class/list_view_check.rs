//! Tests for the Slint list, ticket 10.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { EsClassList, EsClassGroup, EsClassRow } from "../archguard-desktop-ui/ui/es_class_list.slint";
    export { EsClassGroup, EsClassRow }
    export component Page inherits Window {
        in property <[EsClassGroup]> groups;
        in-out property <int> picked: -1;
        callback picked-row(int);
        list := EsClassList {
            groups: root.groups;
            select(i) => { root.picked = i; root.picked-row(i); }
        }
    }
}

const W: usize = 300;
const H: usize = 400;

struct Pixels(Rc<MinimalSoftwareWindow>);

impl slint::platform::Platform for Pixels {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn window() -> Rc<MinimalSoftwareWindow> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(W as u32, H as u32));
    slint::platform::set_platform(Box::new(Pixels(window.clone()))).expect("platform");
    window
}

fn frame(window: &MinimalSoftwareWindow) -> Vec<PremultipliedRgbaColor> {
    let mut buf = vec![PremultipliedRgbaColor::default(); W * H];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, W);
    });
    buf
}

fn key(p: &PremultipliedRgbaColor) -> (u8, u8, u8, u8) {
    (p.red, p.green, p.blue, p.alpha)
}

fn same(a: &[PremultipliedRgbaColor], b: &[PremultipliedRgbaColor]) -> bool {
    a.iter().map(key).eq(b.iter().map(key))
}

/// Pixels in the rectangle that differ from the pixel at its top left corner.
fn ink(buf: &[PremultipliedRgbaColor], x: (usize, usize), y: (usize, usize)) -> usize {
    let bg = key(&buf[y.0 * W + x.0]);
    (y.0..y.1)
        .flat_map(|r| (x.0..x.1).map(move |c| (r, c)))
        .filter(|&(r, c)| key(&buf[r * W + c]) != bg)
        .count()
}

fn row(label: &str, writes: i32, reads: i32, selected: bool, index: i32) -> EsClassRow {
    EsClassRow {
        label: label.into(),
        side: "".into(),
        writes,
        reads,
        warn: false,
        selected,
        index,
    }
}

fn group(title: i32, rows: Vec<EsClassRow>) -> EsClassGroup {
    EsClassGroup {
        title,
        root: "".into(),
        rows: ModelRc::new(VecModel::from(rows)),
    }
}

fn groups(g: Vec<EsClassGroup>) -> ModelRc<EsClassGroup> {
    ModelRc::new(VecModel::from(g))
}

fn shown(g: Vec<EsClassGroup>) -> (Rc<MinimalSoftwareWindow>, Page) {
    let window = window();
    let page = Page::new().expect("component");
    page.show().expect("shown");
    page.set_groups(groups(g));
    (window, page)
}

#[test]
fn list_texts_are_translated() {
    let slint = include_str!("../../../../archguard-desktop-ui/ui/es_class_list.slint");
    let texts: Vec<&str> = slint
        .split("@tr(\"")
        .skip(1)
        .map(|r| r.split('"').next().unwrap_or_default())
        .collect();
    assert!(!texts.is_empty(), "the list has translatable texts");
    let po = crate::tab_support::texts_check::po_de();
    let missing: Vec<_> = texts
        .iter()
        .filter(|t| po.get(**t).is_none_or(|g| g.is_empty()))
        .collect();
    assert!(missing.is_empty(), "no German text for {missing:?}");
}

#[test]
fn an_empty_list_shows_a_hint_and_rows_replace_it() {
    let (window, page) = shown(vec![]);
    let empty = frame(&window);
    assert!(
        ink(&empty, (0, W), (H / 3, H / 2)) > 0,
        "the hint sits below the search field"
    );
    page.set_groups(groups(vec![group(0, vec![row("katalog", 0, 0, false, 0)])]));
    assert!(!same(&empty, &frame(&window)), "rows replace the hint");
}

#[test]
fn a_group_header_shows_its_row_count() {
    let (window, page) = shown(vec![group(0, vec![row("a", 0, 0, false, 0)])]);
    let one = frame(&window);
    // Same first row, one more row below it: the header (top band) changes only by its count.
    page.set_groups(groups(vec![group(
        0,
        vec![row("a", 0, 0, false, 0), row("b", 0, 0, false, 1)],
    )]));
    let two = frame(&window);
    // The count sits at the header's right edge, right below the search row.
    assert_ne!(
        ink(&one, (230, 280), (60, 90)),
        ink(&two, (230, 280), (60, 90)),
        "the count changes with the rows"
    );
}

#[test]
fn the_selected_row_is_highlighted() {
    let (window, page) = shown(vec![group(0, vec![row("a", 0, 0, false, 0)])]);
    let plain = frame(&window);
    page.set_groups(groups(vec![group(0, vec![row("a", 0, 0, true, 0)])]));
    assert!(!same(&plain, &frame(&window)), "the row is highlighted");
}

#[test]
fn counters_show_only_above_zero() {
    let (window, page) = shown(vec![group(0, vec![row("a", 0, 0, false, 0)])]);
    let none = frame(&window);
    // Right half of the first row: nothing without counters.
    let band = ((200, 280), (95, 120));
    page.set_groups(groups(vec![group(0, vec![row("a", 2, 0, false, 0)])]));
    let writes = frame(&window);
    page.set_groups(groups(vec![group(0, vec![row("a", 0, 3, false, 0)])]));
    let reads = frame(&window);
    assert!(!same(&none, &writes) && !same(&none, &reads) && !same(&writes, &reads));
    assert!(ink(&writes, band.0, band.1) > ink(&none, band.0, band.1));
}
