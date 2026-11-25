//! The Elasticsearch tab in the software renderer, and its German texts.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { EsView, ElasticsearchData, EsModel, EsKind } from "../archguard-desktop-ui/ui/es_view.slint";
    import { SvcRow, SvcEdge } from "../archguard-desktop-ui/ui/services_tree.slint";
    export { ElasticsearchData, EsModel, EsKind, SvcRow, SvcEdge }
    export component Page inherits Window {
        out property <bool> scrolls: view.header-scrolls;
        out property <bool> zoom-visible: view.zoom-visible;
        out property <bool> bar-packed: view.bar-packed;
        view := EsView {}
    }
}

const W: usize = 400;
const H: usize = 600;

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

/// Pixels in the band of rows `from..to` that differ from the background.
fn ink(buf: &[PremultipliedRgbaColor], from: usize, to: usize) -> usize {
    let bg = buf[(H - 4) * W + 4];
    buf[from * W..to * W]
        .iter()
        .filter(|p| (p.red, p.green, p.blue) != (bg.red, bg.green, bg.blue))
        .count()
}

/// The texts of `@tr("…")` in the view's file.
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

fn kind(kind: &str, count: i32, shown: i32) -> EsKind {
    EsKind {
        kind: kind.into(),
        count,
        shown,
        ghosts: 0,
    }
}

fn row(i: usize, name: &str) -> SvcRow {
    SvcRow {
        name: name.into(),
        service: i as i32,
        left: 24.0,
        y: 38.0 + i as f32 * 30.0,
        w: 200.0,
        h: 28.0,
        label_width: 100.0,
        relation_x: 300.0,
        compact: true,
        incoming: 1,
        outgoing: 1,
        ..Default::default()
    }
}

/// 2 services, 3 indices, 1 pipeline, and one arc per colour.
fn model(arcs: bool) -> EsModel {
    let names = ["svc-a", "svc-b", "idx-a", "idx-b", "idx-c", "pipe"];
    let edge = |from: usize, to: usize, color: i32| SvcEdge {
        line: format!(
            "M 300 {} C 340 {} 340 {} 300 {}",
            53 + from * 30,
            53 + from * 30,
            53 + to * 30,
            53 + to * 30
        )
        .into(),
        head: String::new().into(),
        color,
        edge: from as i32,
        top: 53.0 + from.min(to) as f32 * 30.0,
        bottom: 53.0 + from.max(to) as f32 * 30.0,
        ..Default::default()
    };
    let edges = if arcs {
        vec![edge(0, 2, 2), edge(1, 2, 1), edge(2, 5, 0)]
    } else {
        Vec::new()
    };
    EsModel {
        state: 1,
        rows: ModelRc::new(VecModel::from(
            names
                .iter()
                .enumerate()
                .map(|(i, n)| row(i, n))
                .collect::<Vec<_>>(),
        )),
        edges: ModelRc::new(VecModel::from(edges)),
        width: 400.0,
        height: 240.0,
        zoom: 1.0,
        kinds: ModelRc::new(VecModel::from(vec![
            kind("service", 2, 2),
            kind("index", 3, 3),
            kind("alias", 0, 0),
            kind("ingest_pipeline", 1, 0),
            kind("index_template", 4, 0),
        ])),
        total: 10,
        ..Default::default()
    }
}

#[test]
fn es_texts_are_translated() {
    let texts = tr_texts(include_str!(
        "../../../archguard-desktop-ui/ui/es_view.slint"
    ));
    assert!(!texts.is_empty(), "the view has translatable texts");
    let po = crate::tab_support::texts_check::po_de();
    let missing: Vec<_> = texts
        .iter()
        .filter(|t| po.get(*t).is_none_or(|g| g.is_empty()))
        .collect();
    assert!(missing.is_empty(), "no German text for {missing:?}");
}

#[test]
fn rows_and_arcs_appear_and_without_data_a_text_does() {
    let window = window();
    let page = Page::new().expect("component");
    page.show().expect("shown");
    let data = page.global::<ElasticsearchData>();
    data.set_model(EsModel {
        state: 1,
        ..Default::default()
    });
    let empty = frame(&window);
    data.set_model(model(false));
    let rows = frame(&window);
    data.set_model(model(true));
    let arcs = frame(&window);
    assert!(!same(&rows, &empty), "the rows appear");
    assert!(!same(&arcs, &rows), "the arcs appear");
    // The empty text sits mid-pane, where the tree has nothing without rows.
    let mut tree_only = model(false);
    tree_only.rows = ModelRc::default();
    data.set_model(tree_only);
    let no_rows = frame(&window);
    assert!(
        ink(&empty, H / 3, H / 2) > ink(&no_rows, H / 3, H / 2),
        "the empty state shows a text instead of the tree"
    );
}

/// At 400 px the header (filter, focus, legend) scrolls instead of being cut.
#[test]
fn the_header_scrolls_at_400_px() {
    let _window = window();
    let page = Page::new().expect("component");
    page.show().expect("shown");
    page.global::<ElasticsearchData>().set_model(model(true));
    frame(&_window);
    assert!(page.get_scrolls(), "the header is wider than 400 px");
}

/// Zoom and Fit stay reachable at 400 px: they do not scroll away with the kinds.
#[test]
fn zoom_and_fit_are_visible_at_400_px() {
    let window = window();
    let page = Page::new().expect("component");
    page.show().expect("shown");
    page.global::<ElasticsearchData>().set_model(model(true));
    frame(&window);
    assert!(page.get_zoom_visible(), "zoom and Fit lie beyond the pane");
}

fn with_detail(has: bool) -> EsModel {
    let mut m = model(true);
    m.has_detail = has;
    m.focus = if has { "idx-a".into() } else { "".into() };
    if has {
        m.detail.writers = "svc-a".into();
        m.detail.readers = "svc-b".into();
    }
    m
}

/// The bar appears with the focus (below the tree at 400 px) and goes with "Show all".
#[test]
fn the_detail_bar_appears_with_the_focus_and_goes_without() {
    let window = window();
    let page = Page::new().expect("component");
    page.show().expect("shown");
    let data = page.global::<ElasticsearchData>();
    data.set_model(with_detail(false));
    let closed = frame(&window);
    data.set_model(with_detail(true));
    let open = frame(&window);
    assert!(
        ink(&open, H * 3 / 4, H) > ink(&closed, H * 3 / 4, H),
        "the bar shows its texts below the tree"
    );
    data.set_model(with_detail(false));
    assert!(same(&frame(&window), &closed), "the bar is gone");
}

/// A short detail (an undeclared name) sits at the top of a bar taller than it; the entries are
/// not spread over the bar's height.
#[test]
fn the_detail_entries_are_packed_at_the_top_of_a_tall_bar() {
    let window = window();
    let page = Page::new().expect("component");
    page.show().expect("shown");
    page.global::<ElasticsearchData>()
        .set_model(with_detail(true));
    frame(&window);
    assert!(page.get_bar_packed(), "the entries are spread over the bar");
}

mod origin {
    use std::path::Path;
    use std::sync::Arc;

    use archguard_engine::analyze_service_set;
    use slint::Model;

    use super::super::state::{EsState, build_from};
    use super::super::view::build;

    fn state(first: &str, second: &str) -> EsState {
        let dir = |n: &str| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/es-set")
                .join(n)
        };
        let t = analyze_service_set(&[dir(first), dir(second)]).expect("set");
        build_from(Arc::new(Ok(t)))
    }

    fn names(s: &EsState) -> Vec<String> {
        build(s).rows.iter().map(|r| r.name.to_string()).collect()
    }

    #[test]
    fn a_foreign_service_carries_the_marker_the_primary_ones_do_not() {
        let n = names(&state("infra", "adapter"));
        assert!(
            n.iter()
                .any(|x| x.starts_with("↧ ") && x.ends_with("writer")),
            "{n:?}"
        );
        assert!(
            n.contains(&"orders".to_string()) && n.contains(&"audit".to_string()),
            "{n:?}"
        );
        assert!(
            !n.iter()
                .any(|x| x.contains("↧ orders") || x.contains("↧ audit")),
            "{n:?}"
        );
    }

    #[test]
    fn with_the_other_primary_project_the_declarations_carry_it() {
        let mut s = state("adapter", "infra");
        let n = names(&s);
        assert!(
            n.contains(&"↧ orders".to_string()) && n.contains(&"audit".to_string()),
            "{n:?}"
        );
        assert!(
            !n.iter().any(|x| x.contains("writer") && x.contains('↧')),
            "{n:?}"
        );
        let pipe = s
            .kinds()
            .iter()
            .position(|k| k == "ingest_pipeline")
            .unwrap();
        s.toggle_kind(pipe);
        assert!(names(&s).contains(&"↧ p1".to_string()));
    }

    #[test]
    fn a_warning_comes_before_the_marker_and_the_label_stays_within_32() {
        let mut s = state("infra", "adapter");
        let at = s.graph.nodes.iter().position(|n| n.foreign).unwrap();
        s.graph.nodes[at].warn = true;
        s.graph.nodes[at].label = "y".repeat(40);
        let n = names(&s);
        let row = n
            .iter()
            .find(|x| x.starts_with("⚠ ↧ "))
            .unwrap_or_else(|| panic!("{n:?}"));
        assert!(row.chars().count() <= 32, "{row}");
    }

    #[test]
    fn without_a_set_no_row_has_the_marker() {
        let t = archguard_engine::analyze_services(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-set/adapter"),
        )
        .expect("single");
        let s = build_from(Arc::new(Ok(t)));
        assert!(names(&s).iter().all(|x| !x.contains('↧')));
    }
}

/// A row of a foreign node renders (the marker adds ink next to the label).
#[test]
fn a_foreign_row_renders_with_the_marker() {
    let window = window();
    let page = Page::new().expect("component");
    page.show().expect("shown");
    let data = page.global::<ElasticsearchData>();
    let with = |name: &str| {
        let mut m = model(false);
        m.rows = ModelRc::new(VecModel::from(vec![row(0, name)]));
        m
    };
    data.set_model(with("idx"));
    let plain = frame(&window);
    data.set_model(with("↧ idx"));
    assert!(!same(&frame(&window), &plain), "the marker is drawn");
}
