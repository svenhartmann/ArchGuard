//! Pixel checks of the app's own single-line text fields (`SearchField`,
//! `LineEdit`): a text longer than the field stays inside it, and the end
//! of the text scrolls with the caret while typing, as in a native field.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize};

slint::slint! {
    import { SearchField } from "ui/widgets.slint";
    import { LineEdit, TextEdit } from "ui/fields.slint";
    export component Fields inherits Window {
        width: 200px;
        height: 240px;
        background: black;
        in property <string> text;
        SearchField { x: 0; y: 0; width: 100px; text: root.text; }
        LineEdit { x: 0; y: 40px; width: 100px; text: root.text; }
        TextEdit { x: 0; y: 80px; width: 100px; height: 40px; text: root.text; }
    }
}

const W: usize = 200;
const H: usize = 240;
/// Top and text start of each single-line field (SearchField, LineEdit),
/// and the rows checked.
const FIELDS: [(usize, usize); 2] = [(0, 33), (40, 10)];
const ROWS: std::ops::Range<usize> = 6..22;

struct Pixels(Rc<MinimalSoftwareWindow>);

impl slint::platform::Platform for Pixels {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn show(text: &str) -> (Rc<MinimalSoftwareWindow>, Fields) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(W as u32, H as u32));
    slint::platform::set_platform(Box::new(Pixels(window.clone()))).expect("platform");
    let fields = Fields::new().expect("component");
    fields.set_text(text.into());
    fields.show().expect("shown");
    window.dispatch_event(WindowEvent::WindowActiveChanged(true));
    (window, fields)
}

fn frame(window: &MinimalSoftwareWindow) -> Vec<PremultipliedRgbaColor> {
    let mut buf = vec![PremultipliedRgbaColor::default(); W * H];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, W);
    });
    buf
}

/// Whether the text (light on the dark field) is drawn in `xs` of a row band.
fn ink(buf: &[PremultipliedRgbaColor], top: usize, xs: std::ops::Range<usize>) -> bool {
    ROWS.flat_map(|y| xs.clone().map(move |x| (top + y) * W + x))
        .any(|i| buf[i].red > 150 && buf[i].green > 150)
}

fn click(window: &MinimalSoftwareWindow, top: usize, button: PointerEventButton) {
    let position = LogicalPosition::new(60.0, top as f32 + 14.0);
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

/// Click into a field and type `text`.
fn type_into(window: &MinimalSoftwareWindow, top: usize, text: &str) {
    click(window, top, PointerEventButton::Left);
    for c in text.chars() {
        let key: slint::SharedString = c.to_string().into();
        window.dispatch_event(WindowEvent::KeyPressed { text: key.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text: key });
    }
}

#[test]
fn a_long_text_is_not_drawn_past_the_field() {
    let (window, _fields) = show("python:domain/money.py::Money_with_a_long_tail");
    let buf = frame(&window);
    for (top, _) in FIELDS {
        assert!(ink(&buf, top, 40..90), "text drawn in the field at {top}");
        assert!(!ink(&buf, top, 106..W), "text drawn past the field at {top}");
    }
}

/// Typing past the right edge scrolls the text: the caret and the last
/// letter stay in view, the first scrolls out on the left.
#[test]
fn typing_past_the_edge_keeps_the_caret_in_view() {
    let (window, _fields) = show("");
    let text = format!("M{}M", " ".repeat(60));
    for (top, start) in FIELDS {
        type_into(&window, top, "M");
        assert!(ink(&frame(&window), top, start..start + 12), "the first letter at {top}");
        type_into(&window, top, &text[1..]);
        let buf = frame(&window);
        assert!(!ink(&buf, top, start..start + 12), "the first letter scrolled away at {top}");
        assert!(ink(&buf, top, 70..92), "the last letter is shown at {top}");
    }
}

