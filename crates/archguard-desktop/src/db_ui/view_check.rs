//! The Database tab in the software renderer, and its German texts.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { DbView, DatabaseData, DbModel, DbEngine, DbRow, DbRel } from "../archguard-desktop-ui/ui/db_view.slint";
    import { EsClassData, EsClassModel } from "../archguard-desktop-ui/ui/es_class.slint";
    export { DatabaseData, DbModel, DbEngine, DbRow, DbRel, EsClassData, EsClassModel }
    export component Page inherits Window {
        out property <bool> listed: view.listed;
        view := DbView {}
    }
}

const W: usize = 600;
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

fn frame(window: &MinimalSoftwareWindow) -> Vec<(u8, u8, u8, u8)> {
    let mut buf = vec![PremultipliedRgbaColor::default(); W * H];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, W);
    });
    buf.iter()
        .map(|p| (p.red, p.green, p.blue, p.alpha))
        .collect()
}

fn engine(name: &str, count: i32) -> DbEngine {
    DbEngine {
        name: name.into(),
        count,
        orphan: false,
    }
}

fn model(active: i32, es: bool, rows: Vec<DbRow>) -> DbModel {
    DbModel {
        state: 1,
        engines: ModelRc::new(VecModel::from(vec![
            engine("elasticsearch", 3),
            engine("postgresql", 1),
            engine("redis", 1),
        ])),
        active,
        es,
        relational: false,
        rel: ModelRc::default(),
        problem: "".into(),
        rows: ModelRc::new(VecModel::from(rows)),
    }
}

fn redis() -> Vec<DbRow> {
    vec![DbRow {
        id: "cache".into(),
        endpoint: "cache:6379".into(),
        services: "orders, billing".into(),
        database: "0".into(),
    }]
}

#[test]
fn db_texts_are_translated() {
    let slint = include_str!("../../../archguard-desktop-ui/ui/db_view.slint");
    let texts: Vec<&str> = slint
        .split("@tr(\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    assert!(
        texts.len() >= 9 && texts.contains(&"Database name"),
        "the view has translatable texts"
    );
    let po = crate::tab_support::texts_check::po_de();
    let missing: Vec<_> = texts
        .iter()
        .filter(|t| po.get(**t).is_none_or(|g| g.is_empty()))
        .collect();
    assert!(missing.is_empty(), "no German text for {missing:?}");
}

#[test]
fn sub_items_and_the_list_appear_and_elasticsearch_shows_its_graph() {
    let window = window();
    let page = Page::new().expect("component");
    page.show().expect("shown");
    let data = page.global::<DatabaseData>();
    data.set_model(DbModel {
        state: 1,
        ..Default::default()
    });
    let empty = frame(&window);
    assert!(!page.get_listed(), "no sub-item, no list");
    data.set_model(model(2, false, Vec::new()));
    let strip = frame(&window);
    assert!(page.get_listed());
    assert_ne!(strip, empty, "the sub-items appear");
    data.set_model(model(2, false, redis()));
    assert_ne!(frame(&window), strip, "the list rows appear");
    let es = page.global::<EsClassData>();
    es.set_model(EsClassModel {
        state: 1,
        ..Default::default()
    });
    data.set_model(model(0, true, Vec::new()));
    let graph = frame(&window);
    assert!(!page.get_listed(), "Elasticsearch shows no list");
    assert_ne!(graph, strip, "the Elasticsearch view takes the body");
}

#[test]
fn a_relational_sub_item_shows_its_lines_and_a_problem() {
    let window = window();
    let page = Page::new().expect("component");
    page.show().expect("shown");
    let data = page.global::<DatabaseData>();
    let line = |kind, text: &str, detail: &str| DbRel {
        kind,
        text: text.into(),
        detail: detail.into(),
        file: "orders/Order.java".into(),
        line: 7,
        clickable: kind == 2,
    };
    let mut m = model(1, false, Vec::new());
    m.relational = true;
    data.set_model(m.clone());
    let strip = frame(&window);
    assert!(!page.get_listed(), "a relational sub-item shows no list");
    m.rel = ModelRc::new(VecModel::from(vec![
        line(0, "pg", ""),
        line(1, "orders", "shop"),
        line(2, "orders", ""),
        line(3, "orders", "billing, orders"),
    ]));
    data.set_model(m.clone());
    let lines = frame(&window);
    assert_ne!(lines, strip, "the lines appear");
    m.problem = "editor: not found".into();
    data.set_model(m);
    assert_ne!(frame(&window), lines, "the problem appears");
}
