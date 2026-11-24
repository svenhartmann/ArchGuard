//! Details → Elasticsearch in the software renderer, and its German texts.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { ServicesView, ServicesData, ServicesModel, SvcSearch, SvcSearchKind, SvcSearchRow } from "../archguard-desktop-ui/ui/services_view.slint";
    export { ServicesData, ServicesModel, SvcSearch, SvcSearchKind, SvcSearchRow }
    export component Page inherits Window {
        ServicesView { details-open: true; }
    }
}

const W: usize = 400;
const H: usize = 900;

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

/// The frames show the same picture (the pixel type has no `PartialEq`).
fn same(a: &[PremultipliedRgbaColor], b: &[PremultipliedRgbaColor]) -> bool {
    let key = |p: &PremultipliedRgbaColor| (p.red, p.green, p.blue, p.alpha);
    a.iter().map(key).eq(b.iter().map(key))
}

/// The texts of `@tr("…")` in the section's file.
fn tr_texts(slint: &str) -> Vec<String> {
    slint
        .split("@tr(\"")
        .skip(1)
        .map(|rest| {
            let mut out = String::new();
            let mut chars = rest.chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => out.extend(chars.next()),
                    c => out.push(c),
                }
            }
            out
        })
        .collect()
}

fn row(i: i32, title: &str) -> SvcSearchRow {
    SvcSearchRow {
        index: i,
        title: title.into(),
        warn: i == 0,
        open: false,
    }
}

fn kind(kind: &str, titles: &[&str]) -> SvcSearchKind {
    SvcSearchKind {
        kind: kind.into(),
        count: titles.len() as i32,
        rows: ModelRc::new(VecModel::from(
            titles
                .iter()
                .enumerate()
                .map(|(i, t)| row(i as i32, t))
                .collect::<Vec<_>>(),
        )),
    }
}

fn model(search: SvcSearch) -> ServicesModel {
    ServicesModel {
        state: 1,
        search,
        zoom: 1.0,
        ..Default::default()
    }
}

fn two_kinds() -> SvcSearch {
    SvcSearch {
        total: 3,
        kinds: ModelRc::new(VecModel::from(vec![
            kind(
                "elasticstack_elasticsearch_index",
                &[
                    "orders-a-very-long-generic-index-name-that-has-to-wrap-inside-a-narrow-pane-without-spaces",
                ],
            ),
            kind(
                "elasticstack_kibana_data_view",
                &["orders-view", "audit-view"],
            ),
        ])),
        ..Default::default()
    }
}

/// Whether text is drawn into the rightmost 12 columns; the full-width
/// separator lines are not text.
fn ink_in_padding(buf: &[PremultipliedRgbaColor]) -> bool {
    let bg = buf[(H - 4) * W + 4];
    let inked = |x: usize, y: usize| {
        let p = buf[y * W + x];
        (p.red, p.green, p.blue) != (bg.red, bg.green, bg.blue)
    };
    (0..H).any(|y| !(inked(2, y) && inked(W / 2, y)) && (W - 12..W).any(|x| inked(x, y)))
}

#[test]
fn search_texts_are_translated() {
    let texts = tr_texts(include_str!(
        "../../../archguard-desktop-ui/ui/services_search.slint"
    ));
    assert!(!texts.is_empty(), "the section has translatable texts");
    let po = crate::tab_support::texts_check::po_de();
    let missing: Vec<_> = texts
        .iter()
        .filter(|t| po.get(*t).is_none_or(|g| g.is_empty()))
        .collect();
    assert!(missing.is_empty(), "no German text for {missing:?}");
}

/// The section appears with Elasticsearch data, not without, and 400 px
/// cut nothing off.
#[test]
fn the_section_appears_only_with_artifacts_and_fits_the_width() {
    let window = window();
    let page = Page::new().expect("component");
    page.show().expect("shown");
    let data = page.global::<ServicesData>();
    data.set_model(model(SvcSearch::default()));
    let without = frame(&window);
    data.set_search_open(true);
    assert!(
        same(&frame(&window), &without),
        "an open section without data draws nothing"
    );
    data.set_model(model(two_kinds()));
    let with = frame(&window);
    assert!(!same(&with, &without), "the section appears with two kinds");
    assert!(
        !ink_in_padding(&with),
        "something is drawn into the right padding"
    );
}
