//! Tests for the Slint canvas, ticket 09.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { EsClassCanvas, EsClassBox, EsClassEdge, EsClassCompartment, EsClassLine, EsClassChip } from "../archguard-desktop-ui/ui/es_class_canvas.slint";
    export { EsClassBox, EsClassEdge, EsClassCompartment, EsClassLine, EsClassChip }
    export component Page inherits Window {
        in property <[EsClassBox]> boxes;
        in property <[EsClassEdge]> edges;
        in property <float> zoom: 1.0;
        canvas := EsClassCanvas {
            boxes: root.boxes;
            edges: root.edges;
            canvas-w: 400;
            canvas-h: 300;
            zoom: root.zoom;
        }
    }
}

const W: usize = 400;
const H: usize = 300;
/// Rows above the zoom bar, which the empty canvas still draws.
const BODY: usize = 200;

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

fn key(p: &PremultipliedRgbaColor) -> (u8, u8, u8, u8) {
    (p.red, p.green, p.blue, p.alpha)
}

fn same(a: &[PremultipliedRgbaColor], b: &[PremultipliedRgbaColor]) -> bool {
    a.iter().map(key).eq(b.iter().map(key))
}

/// Pixels above the zoom bar that differ from the corner pixel (the background).
fn ink(buf: &[PremultipliedRgbaColor]) -> usize {
    let bg = key(&buf[BODY * W - 1]);
    buf[..BODY * W].iter().filter(|p| key(p) != bg).count()
}

fn line(text: &str) -> EsClassLine {
    EsClassLine {
        depth: 0,
        prefix: 1,
        text: text.into(),
        kind: "keyword".into(),
        note: Default::default(),
        summary: 0,
    }
}

fn index_box() -> EsClassBox {
    EsClassBox {
        x: 20.0,
        y: 20.0,
        w: 150.0,
        h: 90.0,
        kind: 0,
        stereotype: "index".into(),
        name: "products".into(),
        root: "infra".into(),
        compartments: ModelRc::new(VecModel::from(vec![EsClassCompartment {
            title: 0,
            y: 48.0,
            lines: ModelRc::new(VecModel::from(vec![line("id")])),
            chips: ModelRc::default(),
        }])),
        ..Default::default()
    }
}

fn edge() -> EsClassEdge {
    EsClassEdge {
        line: "M 170 60 L 300 60".into(),
        head: "M 292 56 L 300 60 L 292 64".into(),
        style: 1,
        ..Default::default()
    }
}

/// The page's frame for `boxes` and `edges`.
fn shot(
    window: &MinimalSoftwareWindow,
    page: &Page,
    boxes: Vec<EsClassBox>,
    edges: Vec<EsClassEdge>,
) -> Vec<PremultipliedRgbaColor> {
    page.set_boxes(ModelRc::new(VecModel::from(boxes)));
    page.set_edges(ModelRc::new(VecModel::from(edges)));
    frame(window)
}

#[test]
fn boxes_edges_selection_and_ghosts_are_drawn() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(W as u32, H as u32));
    slint::platform::set_platform(Box::new(Pixels(window.clone()))).expect("platform");
    let page = Page::new().expect("component");
    page.show().expect("shown");
    let empty = shot(&window, &page, vec![], vec![]);
    assert_eq!(ink(&empty), 0, "an empty model draws nothing");
    let plain = shot(&window, &page, vec![index_box()], vec![]);
    assert!(ink(&plain) > 500, "the box appears");
    let with_edge = shot(&window, &page, vec![index_box()], vec![edge()]);
    assert!(ink(&with_edge) > ink(&plain), "the edge appears");
    let selected = EsClassBox {
        selected: true,
        ..index_box()
    };
    let sel = shot(&window, &page, vec![selected], vec![]);
    assert!(!same(&plain, &sel), "the selected box has another border");
    let ghost = EsClassBox {
        ghost: true,
        border: "M 20 20 L 40 20 M 60 20 L 80 20 M 20 20 L 20 40".into(),
        ..index_box()
    };
    let g = shot(&window, &page, vec![ghost], vec![]);
    assert!(!same(&plain, &g), "the ghost differs");
    assert!(
        ink(&g) < ink(&plain),
        "the ghost has no solid border or fill"
    );
}

fn tr_texts(slint: &str) -> Vec<String> {
    slint
        .split("@tr(\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn canvas_texts_are_translated() {
    let src = include_str!("../../../../archguard-desktop-ui/ui/es_class_canvas.slint");
    let mut texts = tr_texts(src);
    // "Settings" is the Settings tab's own, in its context.
    assert!(src.contains("@tr(\"Settings\" => \"Settings\")"));
    texts.retain(|t| t != "Settings");
    for t in [
        "Chain",
        "Parameters",
        "Points to",
        "Access",
        "Patterns",
        "{0} fields",
        "{0} more",
        "not declared",
    ] {
        assert!(texts.iter().any(|x| x == t), "the canvas uses {t:?}");
    }
    let po = crate::tab_support::texts_check::po_de();
    let missing: Vec<_> = texts
        .iter()
        .filter(|t| po.get(*t).is_none_or(|g| g.is_empty()))
        .collect();
    assert!(missing.is_empty(), "no German text for {missing:?}");
}
