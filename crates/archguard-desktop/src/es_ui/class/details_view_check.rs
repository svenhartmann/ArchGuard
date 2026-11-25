//! Tests for the Slint details, ticket 11.
//! View wiring to Slint, ticket 12.

use std::cell::RefCell;
use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { EsClassDetails, EsDetails, EsDetailHead, EsDetailField, EsDetailRow, EsDetailHint, EsDetailPair, EsDetailUse } from "../archguard-desktop-ui/ui/es_class_details.slint";
    import { SvcSearchDetail } from "../archguard-desktop-ui/ui/services_search.slint";
    export { EsDetails, EsDetailHead, EsDetailField, EsDetailRow, EsDetailHint, EsDetailPair, EsDetailUse, SvcSearchDetail }
    export component Page inherits Window {
        in property <EsDetails> details;
        callback open(string, int);
        EsClassDetails {
            details: root.details;
            open(f, l) => { root.open(f, l); }
        }
    }
}

const W: usize = 360;
const H: usize = 600;

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

/// Pixels that differ from the corner pixel (the background).
fn ink(buf: &[PremultipliedRgbaColor]) -> usize {
    let bg = key(&buf[W - 1]);
    buf.iter().filter(|p| key(p) != bg).count()
}

fn s<T: From<&'static str>>(t: &'static str) -> T {
    t.into()
}

fn row(stage: i32, n: i32, chip: &'static str, class: i32) -> EsDetailRow {
    EsDetailRow {
        stage,
        n,
        chip: s(chip),
        class,
        params: s(""),
    }
}

fn index_details() -> EsDetails {
    EsDetails {
        mode: 0,
        head: EsDetailHead {
            title: s("products"),
            subtitle: s("index · infra"),
            location: s("infra/indices.tf:42"),
        },
        fields: model(vec![EsDetailField {
            name: s("title"),
            kind: s("text"),
            analysis: s("index: german_custom"),
        }]),
        chain_name: s("german_custom"),
        chain: model(vec![row(0, 0, "html_strip", 0), row(2, 1, "lowercase", 1)]),
        hint: EsDetailHint {
            field: s("title"),
            search_only: s("synonyme"),
            index_only: s("-"),
        },
        settings: model(vec![EsDetailPair {
            key: s("shards"),
            value: s("3"),
        }]),
        ..Default::default()
    }
}

fn block_details() -> EsDetails {
    EsDetails {
        mode: 1,
        head: EsDetailHead {
            title: s("german_custom"),
            ..Default::default()
        },
        chain: model(vec![row(1, 0, "standard", 0), row(2, 1, "lowercase", 2)]),
        params: model(vec![EsDetailPair {
            key: s("synonyms_path"),
            value: s("analysis/synonyme.txt"),
        }]),
        used_by: model(vec![s("title")]),
        definition: s("infra/indices.tf:60"),
        ..Default::default()
    }
}

fn other_details() -> EsDetails {
    EsDetails {
        mode: 2,
        other: SvcSearchDetail {
            scope: s("infra"),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

fn usage(clickable: bool) -> EsDetails {
    EsDetails {
        mode: 0,
        usages: model(vec![EsDetailUse {
            text: s("search-api read src/Repo.cs:7"),
            file: s("src/Repo.cs"),
            line: 7,
            clickable,
        }]),
        ..Default::default()
    }
}

/// Clicks over a grid and returns what `open` reported.
fn clicks(window: &MinimalSoftwareWindow, page: &Page) -> Vec<(String, i32)> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    page.on_open(move |f, l| sink.borrow_mut().push((f.to_string(), l)));
    for y in (0..H as i32).step_by(4) {
        for x in [10, 60, 120] {
            let position = LogicalPosition::new(x as f32, y as f32);
            window.dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
            window.dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        }
    }
    let out = seen.borrow().clone();
    out
}

#[test]
fn the_three_kinds_and_a_clickable_usage_render() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(W as u32, H as u32));
    slint::platform::set_platform(Box::new(Pixels(window.clone()))).ok();
    let page = Page::new().expect("component");
    page.show().expect("show");
    let mut drawn = Vec::new();
    for d in [index_details(), block_details(), other_details()] {
        page.set_details(d);
        drawn.push(ink(&frame(&window)));
    }
    assert!(
        drawn.iter().all(|n| *n > 0),
        "every kind draws something: {drawn:?}"
    );
    assert_ne!(drawn[0], drawn[1], "index and block differ: {drawn:?}");

    page.set_details(usage(false));
    frame(&window);
    assert!(clicks(&window, &page).is_empty(), "foreign usage is inert");
    page.set_details(usage(true));
    frame(&window);
    let got = clicks(&window, &page);
    assert!(
        got.contains(&("src/Repo.cs".to_string(), 7)),
        "own usage opens the editor: {got:?}"
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
fn details_texts_are_translated() {
    let src = include_str!("../../../../archguard-desktop-ui/ui/es_class_details.slint");
    let mut texts = tr_texts(src);
    // "Settings" is the Settings tab's own, in its context.
    texts.retain(|t| t != "Settings");
    for t in [
        "Fields with analysis",
        "Analysis chain {0}",
        "Char filter",
        "Tokenizer",
        "Filter {0}",
        "built in",
        "not declared (plugin or Elasticsearch default)",
        "Definition",
        "Index and search analyzer differ at {0}: search only {1}; index only {2}.",
        "Static hint, no rating.",
        "The file lies on the Elasticsearch nodes; its content cannot be read statically.",
        "Usage in code",
        "Declaration",
        "Used by",
        "Chain",
        "Parameters",
    ] {
        assert!(texts.iter().any(|x| x == t), "the details use {t:?}");
    }
    let po = crate::tab_support::texts_check::po_de();
    let missing: Vec<_> = texts
        .iter()
        .filter(|t| po.get(*t).is_none_or(|g| g.is_empty()))
        .collect();
    assert!(missing.is_empty(), "no German text for {missing:?}");
}
