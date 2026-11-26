//! Tests for the Slint list, ticket 10.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { EsClassList, EsClassGroup, EsClassRow } from "../archguard-desktop-ui/ui/es_class_list.slint";
    export { EsClassGroup, EsClassRow }
    export component Page inherits Window {
        in property <[EsClassGroup]> groups;
        in property <int> count;
        in-out property <int> picked: -1;
        out property <length> scroll-y: list.scroll-y;
        callback picked-row(int);
        callback chosen-kind(int);
        list := EsClassList {
            groups: root.groups;
            count: root.count;
            select(i) => { root.picked = i; root.picked-row(i); }
            choose-kind(i) => { root.chosen-kind(i); }
        }
    }
}

const W: usize = 300;
const H: usize = 400;
/// The row pitch Rust computes `EsClassRow::y` with.
const GROUP_H: f32 = 30.0;
const ROW_H: f32 = 26.0;

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
        y: GROUP_H + ROW_H * index as f32,
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
    // The right part of the list: counters add ink there.
    let band = ((150, 280), (50, H));
    page.set_groups(groups(vec![group(0, vec![row("a", 2, 0, false, 0)])]));
    let writes = frame(&window);
    page.set_groups(groups(vec![group(0, vec![row("a", 0, 3, false, 0)])]));
    let reads = frame(&window);
    assert!(!same(&none, &writes) && !same(&none, &reads) && !same(&writes, &reads));
    assert!(ink(&writes, band.0, band.1) > ink(&none, band.0, band.1));
}

fn press(window: &MinimalSoftwareWindow, key: Key) {
    let text: slint::SharedString = key.into();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

fn click(window: &MinimalSoftwareWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

/// A list of `n` rows in one group; a quick click on the first row focuses the list (the
/// scroll view's flickable holds the press back, so the click picks no row).
fn focused(
    n: i32,
) -> (
    Rc<MinimalSoftwareWindow>,
    Page,
    Rc<std::cell::RefCell<Vec<i32>>>,
) {
    let rows = (0..n).map(|i| row("r", 0, 0, false, i)).collect();
    let (window, page) = shown(vec![group(0, rows)]);
    page.set_count(n);
    let log = Rc::new(std::cell::RefCell::new(Vec::new()));
    let l = log.clone();
    page.on_picked_row(move |i| l.borrow_mut().push(i));
    window.dispatch_event(WindowEvent::WindowActiveChanged(true));
    frame(&window);
    // Search row (10 + 28 + 8) and the group header (30) lie above the first row.
    click(&window, 100.0, 76.0 + ROW_H / 2.0);
    (window, page, log)
}

#[test]
fn arrow_keys_step_through_the_rows_and_stop_at_the_ends() {
    let (window, _page, log) = focused(3);
    for k in [
        Key::DownArrow,
        Key::DownArrow,
        Key::DownArrow,
        Key::DownArrow,
        Key::UpArrow,
        Key::UpArrow,
        Key::UpArrow,
    ] {
        press(&window, k);
    }
    // The list keeps its own cursor (the owner's model need not follow) and stops at both ends.
    assert_eq!(*log.borrow(), [0, 1, 2, 2, 1, 0, 0]);
}

#[test]
fn the_view_follows_the_cursor_down_and_back_up() {
    let (window, page, _log) = focused(30);
    assert_eq!(page.get_scroll_y(), 0.0);
    for _ in 0..20 {
        press(&window, Key::DownArrow);
    }
    frame(&window);
    let down = page.get_scroll_y();
    // Row 20 ends at 30 + 21 * 26 = 576 px; the pane shows less than 400.
    assert!(down < -150.0, "the view scrolled down: {down}");
    for _ in 0..20 {
        press(&window, Key::UpArrow);
    }
    frame(&window);
    assert_eq!(
        page.get_scroll_y(),
        0.0,
        "the view is back at the top, group header shown"
    );
}

#[test]
fn choosing_a_kind_reports_its_index() {
    let (window, page) = shown(vec![]);
    let log = Rc::new(std::cell::RefCell::new(Vec::new()));
    let l = log.clone();
    page.on_chosen_kind(move |i| l.borrow_mut().push(i));
    window.dispatch_event(WindowEvent::WindowActiveChanged(true));
    let closed = frame(&window);
    // The combo sits right of the search field (x 115..270, y 10..38) and opens its popup below.
    click(&window, 200.0, 24.0);
    assert!(!same(&closed, &frame(&window)), "the popup opens");
    // Sweep the popup top to bottom, opening it again before each click. The popup's list is a
    // flickable: it forwards a press to a row only after a short delay, so the click is held.
    for y in (44..=215).step_by(5) {
        let position = LogicalPosition::new(200.0, y as f32);
        window.dispatch_event(WindowEvent::PointerMoved { position });
        window.dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        std::thread::sleep(std::time::Duration::from_millis(150));
        slint::platform::update_timers_and_animations();
        window.dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
        frame(&window);
        click(&window, 200.0, 24.0);
        frame(&window);
    }
    let mut chosen = log.borrow().clone();
    chosen.dedup();
    assert_eq!(
        chosen,
        [0, 1, 2, 3, 4, 5, 6],
        "each entry reports its index, in order"
    );
}
