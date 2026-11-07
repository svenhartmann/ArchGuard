//! Pixel checks of the app's own single-line text fields (`SearchField`,
//! `LineEdit`): a text longer than the field stays inside it, and the end
//! of the text scrolls with the caret while typing, as in a native field.
//! A right click on any of the fields (also `TextEdit`) opens the field
//! menu (Cut, Copy, Paste, Select all). A read-only field (`LineInput`
//! with `read-only`, `CommandField`) clips a long command, and its menu
//! copies and selects but neither cuts nor pastes.

use std::cell::RefCell;
use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize};

slint::slint! {
    import { SearchField } from "ui/widgets.slint";
    import { LineEdit, TextEdit, CommandField } from "ui/fields.slint";
    import { LineInput } from "ui/field_menu.slint";
    export component Fields inherits Window {
        width: 200px;
        height: 400px;
        background: black;
        in property <string> text;
        in-out property <string> read-only-text;
        SearchField { x: 0; y: 0; width: 100px; text: root.text; }
        LineEdit { x: 0; y: 40px; width: 100px; text: root.text; }
        TextEdit { x: 0; y: 80px; width: 100px; height: 40px; text: root.text; }
        CommandField { x: 0; y: 130px; width: 100px; command: root.text; }
        Rectangle {
            x: 0;
            y: 170px;
            width: 100px;
            height: 28px;
            LineInput { x: 10px; width: 80px; read-only: true; text <=> root.read-only-text; }
        }
        LineEdit { x: 0; y: 210px; width: 100px; placeholder-text: "Name"; }
    }
}

const W: usize = 200;
const H: usize = 400;
/// Top of the CommandField and of the read-only LineInput.
const COMMAND: usize = 130;
const READ_ONLY: usize = 170;
/// Top of the empty LineEdit with a placeholder.
const PLACEHOLDER: usize = 210;

thread_local! {
    /// The test platform's clipboard.
    static CLIPBOARD: RefCell<Option<String>> = const { RefCell::new(None) };
}
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

    fn set_clipboard_text(&self, text: &str, clipboard: slint::platform::Clipboard) {
        if clipboard == slint::platform::Clipboard::DefaultClipboard {
            CLIPBOARD.with(|c| *c.borrow_mut() = Some(text.to_owned()));
        }
    }

    fn clipboard_text(&self, clipboard: slint::platform::Clipboard) -> Option<String> {
        if clipboard != slint::platform::Clipboard::DefaultClipboard {
            return None;
        }
        CLIPBOARD.with(|c| c.borrow().clone())
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

/// Whether anything is drawn right of the fields, in the whole height.
fn beside(buf: &[PremultipliedRgbaColor]) -> bool {
    (0..H)
        .flat_map(|y| (110..W).map(move |x| y * W + x))
        .any(|i| buf[i].red > 40 || buf[i].green > 40 || buf[i].blue > 40)
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

/// A right click into the field at `top` draws the menu beside the fields.
fn right_click_opens_the_menu(top: usize) {
    let (window, _fields) = show("Money");
    assert!(!beside(&frame(&window)), "nothing beside the fields at first");
    click(&window, top, PointerEventButton::Right);
    assert!(beside(&frame(&window)), "a menu opened from the field at {top}");
}

#[test]
fn a_right_click_opens_the_search_field_menu() {
    right_click_opens_the_menu(FIELDS[0].0);
}

#[test]
fn a_right_click_opens_the_line_edit_menu() {
    right_click_opens_the_menu(FIELDS[1].0);
}

#[test]
fn a_right_click_opens_the_text_edit_menu() {
    right_click_opens_the_menu(80);
}


/// The placeholder starts at the text's start, as in a native field, not
/// centred in the field.
#[test]
fn the_placeholder_starts_where_the_text_does() {
    let (window, _fields) = show("");
    let buf = frame(&window);
    let muted = |xs: std::ops::Range<usize>| {
        ROWS.flat_map(|y| xs.clone().map(move |x| (PLACEHOLDER + y) * W + x))
            .any(|i| buf[i].red > 60 && buf[i].green > 60)
    };
    assert!(muted(10..16), "the placeholder's first letter at the text start");
    assert!(!muted(70..90), "nothing at the field's end");
}

/// Rows of the field menu below the right click that opened it.
#[derive(Clone, Copy)]
enum Item {
    Cut = 20,
    Copy = 52,
    Paste = 84,
    SelectAll = 125,
}

/// Right-click the field at `top` and choose `item` from its menu.
fn choose(window: &MinimalSoftwareWindow, top: usize, item: Item) {
    click(window, top, PointerEventButton::Right);
    frame(window);
    let position = LogicalPosition::new(60.0, top as f32 + 14.0 + item as i32 as f32);
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
    frame(window);
}

fn clipboard() -> Option<String> {
    CLIPBOARD.with(|c| c.borrow().clone())
}

#[test]
fn a_long_command_is_not_drawn_past_its_field() {
    let (window, _fields) = show("archguard security -- '/a/very/long/path/to/project'");
    let buf = frame(&window);
    assert!(ink(&buf, COMMAND, 40..90), "the command is drawn in its field");
    assert!(!ink(&buf, COMMAND, 106..W), "the command is drawn past its field");
}

/// The command stays selectable: Select all and Copy from its menu put
/// the whole command on the clipboard.
#[test]
fn the_command_menu_selects_and_copies_the_command() {
    let command = "archguard security -- '/a/very/long/path/to/project'";
    let (window, _fields) = show(command);
    choose(&window, COMMAND, Item::SelectAll);
    choose(&window, COMMAND, Item::Copy);
    assert_eq!(clipboard().as_deref(), Some(command));
}

/// Paste in a read-only field's menu is disabled: the text stays.
#[test]
fn a_read_only_field_does_not_paste() {
    let (window, fields) = show("");
    fields.set_read_only_text("Money".into());
    CLIPBOARD.with(|c| *c.borrow_mut() = Some("pasted".into()));
    choose(&window, READ_ONLY, Item::Paste);
    assert_eq!(fields.get_read_only_text(), "Money");
}

/// Cut in a read-only field's menu is disabled; Copy still copies.
#[test]
fn a_read_only_field_copies_but_does_not_cut() {
    let (window, fields) = show("");
    fields.set_read_only_text("Money".into());
    choose(&window, READ_ONLY, Item::SelectAll);
    choose(&window, READ_ONLY, Item::Cut);
    assert_eq!(fields.get_read_only_text(), "Money");
    assert_eq!(clipboard(), None, "a disabled Cut copies nothing");
    choose(&window, READ_ONLY, Item::Copy);
    assert_eq!(clipboard().as_deref(), Some("Money"));
}

