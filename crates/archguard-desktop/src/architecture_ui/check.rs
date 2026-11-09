//! Pixel check of the Architecture tab in a narrow split pane: the page
//! fits the pane. The download buttons flow into further rows, long texts
//! wrap and the diagrams scale or scroll inside their own boxes, so the
//! chapter panel keeps its right border in view instead of being widened
//! beyond the pane, where it would be clipped and scroll sideways. And
//! the panel is as tall as its wrapped blocks: none is drawn below it.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::{ComponentHandle, ModelRc, PhysicalSize, SharedString, StyledText, VecModel};

slint::slint! {
    import { ArchitectureView, ArchitectureData } from "ui/architecture_view.slint";
    export { ArchitectureData }
    export component Page inherits Window {
        width: 380px;
        height: 3000px;
        ArchitectureView { width: 100%; height: 100%; }
    }
}

const W: usize = 380;
const H: usize = 3000;
/// The page's right padding (`Theme.page-pad`).
const PAD: usize = 16;
const LONG: &str = "A long sentence about the building blocks of this project that has to wrap in a narrow pane, as the German text does.";

struct Pixels(Rc<MinimalSoftwareWindow>);

impl slint::platform::Platform for Pixels {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn rows<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows))
}

fn level(scope: i32, level: i32, label: &str) -> ArcLevelItem {
    ArcLevelItem {
        scope,
        level,
        label: label.into(),
        current: level == 3,
    }
}

fn card(index: i32, x: f32) -> ArcCard {
    ArcCard {
        index,
        x,
        y: 20.0,
        w: 220.0,
        h: 90.0,
        name1: "archguard-engine".into(),
        files: 12,
        ..Default::default()
    }
}

/// A package map on level 3, wider than the pane, with a selection.
fn map() -> ArcMap {
    ArcMap {
        kind: 0,
        level: 3,
        levels: rows(vec![
            level(-1, 1, "Project overview"),
            level(1, 2, "archguard-engine"),
        ]),
        crumbs: rows(vec![
            level(-1, 1, "Project overview"),
            level(1, 2, "archguard-engine"),
            level(2, 3, "archguard-engine/src/report"),
        ]),
        scopes: rows(vec![SharedString::from("Project overview")]),
        total: 1732,
        internal: 1350,
        cycles: KpiItem {
            value: "0".into(),
            label: "Package cycles".into(),
            ..Default::default()
        },
        cards: rows(vec![card(0, 20.0), card(1, 600.0)]),
        lines: rows(vec![ArcLine {
            path: "M 240 60 L 600 60".into(),
            count: 3,
            ..Default::default()
        }]),
        dw: 840.0,
        dh: 140.0,
        selected_name: "archguard-engine/src/report/arc42".into(),
        selected_path: "crates/archguard-engine/src/report/arc42".into(),
        selected_action: 1,
        relation_total: 1,
        ..Default::default()
    }
}

fn figure() -> ArcFigure {
    ArcFigure {
        id: "context".into(),
        title: "Context diagram of the project and its neighbours".into(),
        note: LONG.into(),
        shown: true,
        cards: rows(vec![card(0, 20.0), card(1, 1000.0)]),
        dw: 1240.0,
        dh: 140.0,
        legend: LONG.into(),
        ..Default::default()
    }
}

fn block(kind: i32, text: &str) -> ArcBlock {
    ArcBlock {
        kind,
        level: 2,
        plain: text.into(),
        text: StyledText::from_plain_text(text),
        ..Default::default()
    }
}

fn model() -> ArchitectureModel {
    ArchitectureModel {
        state: 1,
        chapters: rows(
            (1..=12)
                .map(|n| ArcChapter {
                    number: n,
                    label: format!("{n:02}").into(),
                    title: "Chapter".into(),
                })
                .collect(),
        ),
        chapter: 5,
        chapter_title: "5. Building block view".into(),
        chapter_note: LONG.into(),
        // Heading, paragraphs, list items and code, each wrapping.
        blocks: rows([0, 1, 1, 2, 2, 5].iter().map(|&k| block(k, LONG)).collect()),
        map: map(),
        map_id: "packages".into(),
        map_title: "Packages".into(),
        map_shown: true,
        figures: rows(vec![figure()]),
        saved: "/a/very/long/path/to/the/project/folder/arc42-05.md".into(),
        has_model: true,
        ..Default::default()
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

fn same(a: &PremultipliedRgbaColor, b: &PremultipliedRgbaColor) -> bool {
    (a.red, a.green, a.blue) == (b.red, b.green, b.blue)
}

/// Asserts that nothing is drawn in the page's right padding.
fn fits(buf: &[PremultipliedRgbaColor]) {
    let bg = buf[(H - 2) * W + W - 2];
    let ink = (0..H)
        .flat_map(|y| (W - PAD + 2..W).map(move |x| (x, y)))
        .find(|&(x, y)| !same(&buf[y * W + x], &bg));
    assert_eq!(ink, None, "drawn in the page's right padding");
}

/// The rows (first, last) of the chapter panel's right border: the
/// longest run of one colour in the column left of the page's right
/// padding that is neither the background nor the colour left of it (a
/// panel widened beyond the pane shows its fill and content there).
fn right_border(buf: &[PremultipliedRgbaColor]) -> (usize, usize) {
    let x = W - PAD - 1;
    let bg = buf[(H - 2) * W + W - 2];
    let line = |y: usize| {
        let p = &buf[y * W + x];
        !same(p, &bg) && !same(p, &buf[y * W + x - 1])
    };
    let mut best = (0, 0);
    let mut start = 0;
    for y in 1..H {
        if !line(y) || !same(&buf[(y - 1) * W + x], &buf[y * W + x]) {
            start = y;
        } else if y - start > best.1 - best.0 {
            best = (start, y);
        }
    }
    best
}

/// `ARCH_CHECK_PNG=<file>` saves the frame, to look at the layout.
fn save(buf: &[PremultipliedRgbaColor]) {
    let Ok(path) = std::env::var("ARCH_CHECK_PNG") else {
        return;
    };
    let file = std::fs::File::create(path).expect("png file");
    let mut enc = png::Encoder::new(file, W as u32, H as u32);
    enc.set_color(png::ColorType::Rgba);
    let data: Vec<u8> = buf
        .iter()
        .flat_map(|p| [p.red, p.green, p.blue, 255])
        .collect();
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&data))
        .expect("png");
}

fn render(model: ArchitectureModel) -> Vec<PremultipliedRgbaColor> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(W as u32, H as u32));
    slint::platform::set_platform(Box::new(Pixels(window.clone()))).expect("platform");
    let page = Page::new().expect("component");
    page.show().expect("shown");
    page.global::<ArchitectureData>().set_model(model);
    frame(&window)
}

/// Asserts that nothing is drawn in the gap between the chapter panel's
/// bottom and the line below it (the layout's 18 px spacing). `bottom`
/// is where the right border ends, above the 12 px corner.
fn encloses_its_blocks(buf: &[PremultipliedRgbaColor], bottom: usize) {
    let bg = buf[(H - 2) * W + W - 2];
    let ink = (bottom + 15..bottom + 27)
        .flat_map(|y| (90..W - PAD - 2).map(move |x| (x, y)))
        .find(|&(x, y)| !same(&buf[y * W + x], &bg));
    assert_eq!(ink, None, "drawn below the panel's bottom at {bottom}");
}

/// The chapter panel ends at the pane's right edge, where its border is
/// drawn top to bottom: the content did not widen it. Its blocks, which
/// wrap into many lines there, stay inside it.
#[test]
fn the_page_fits_a_narrow_pane() {
    let buf = render(model());
    save(&buf);
    fits(&buf);
    let (top, bottom) = right_border(&buf);
    assert!(
        bottom - top > 1000,
        "the panel's right border, rows {top}..{bottom}"
    );
    encloses_its_blocks(&buf, bottom);
}
