//! Headless check of the "Compose projects" dialog: it draws over the
//! window, Escape closes it, Enter in the name field saves only when
//! saving is not blocked, and every text has a German translation.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, PhysicalSize};

slint::slint! {
    import { ProjectSetDialog, ProjectSetData } from "../archguard-desktop-ui/ui/project_set_dialog.slint";
    export { ProjectSetData }
    export component Page inherits Window {
        width: 700px;
        height: 520px;
        ProjectSetDialog {}
    }
}

const W: usize = 700;
const H: usize = 520;

struct Pixels(Rc<MinimalSoftwareWindow>);

impl slint::platform::Platform for Pixels {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn key(window: &MinimalSoftwareWindow, k: Key) {
    let text: slint::SharedString = k.into();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

/// One test: the platform is set once per thread.
#[test]
fn the_dialog_draws_closes_on_escape_and_saves_only_when_allowed() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Pixels(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(W as u32, H as u32));
    let page = Page::new().expect("page");
    let data = page.global::<ProjectSetData>();
    data.set_name("search".into());
    data.set_folders(slint::ModelRc::new(slint::VecModel::from(vec![
        "/p/infra".into(),
        "/p/adapter".into(),
    ])));
    data.set_reason(0);
    let (saved, closed) = (
        Rc::new(std::cell::Cell::new(0)),
        Rc::new(std::cell::Cell::new(0)),
    );
    let s = saved.clone();
    data.on_save(move || s.set(s.get() + 1));
    let c = closed.clone();
    data.on_close(move || c.set(c.get() + 1));
    page.show().expect("shown");
    window.dispatch_event(WindowEvent::WindowActiveChanged(true));

    let mut buf = vec![PremultipliedRgbaColor::default(); W * H];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, W);
    });
    let (corner, middle) = (buf[0], buf[(H / 2) * W + W / 2]);
    assert_ne!(
        (corner.red, corner.green, corner.blue),
        (middle.red, middle.green, middle.blue),
        "the panel stands out from the shade"
    );

    key(&window, Key::Return);
    assert_eq!(saved.get(), 1, "Enter in the name field saves");
    data.set_reason(2);
    key(&window, Key::Return);
    assert_eq!(saved.get(), 1, "blocked: no save");
    key(&window, Key::Escape);
    assert_eq!(closed.get(), 1, "Escape closes");
}

#[test]
fn every_dialog_and_menu_text_has_german() {
    let po = crate::tab_support::texts_check::po_de();
    let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../archguard-desktop-ui/ui");
    let dialog = std::fs::read_to_string(ui.join("project_set_dialog.slint")).expect("dialog");
    let app = std::fs::read_to_string(ui.join("app.slint")).expect("app");
    let mut ids: Vec<&str> = dialog.split("@tr(\"").skip(1).collect();
    assert!(ids.len() > 10, "the dialog has texts");
    for menu in ["Compose projects…", "Open project set…"] {
        assert!(
            app.contains(&format!("@tr(\"{menu}\")")),
            "{menu} in the File menu"
        );
        ids.push(menu);
    }
    for id in ids {
        let id = id.split('"').next().unwrap_or_default();
        assert!(po.contains_key(id), "no German for {id}");
    }
}
