//! End-to-end check of the real window on Slint's testing backend: open a
//! copy of `examples/service-shop`, wait for its analysis on the worker
//! thread, then click every view in the view bar by its label (as a user
//! would, found through the accessibility tree) and check the view opens.
//! Then edit the Architecture chapter as formatted text: a click into the
//! editor, typed keys, Enter and the bold shortcut.
//!
//! Slint keeps one event-loop proxy per process, and the analysis reaches
//! the window through `invoke_from_event_loop`; so this is one test that
//! walks the whole journey, not one per view. Settings go to a temporary
//! file, never to the user's.

use std::cell::RefCell;
use std::path::Path;
use std::time::{Duration, Instant};

use i_slint_backend_testing::{ElementHandle, TestingBackend, TestingBackendOptions};
use slint::ComponentHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};

use crate::documents::{Status, with_store};
use crate::ui::{AppWindow, ArchitectureData};
use crate::{documents_ui, format, settings_file, shell_ui};

/// The analysis of the small example takes a few seconds; a debug build
/// on a busy machine more.
const TIMEOUT: Duration = Duration::from_secs(120);

/// Each view of the view bar: its id (`AppWindow::tab`) and English label.
const VIEWS: [(&str, &str); 8] = [
    ("start", "Start"),
    ("architecture", "Architecture"),
    ("graph", "Graph"),
    ("dsm", "Matrix"),
    ("calls", "Calls"),
    ("services", "Services"),
    ("quality", "Quality"),
    ("security", "Security"),
];

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("target dir");
    for entry in std::fs::read_dir(from)
        .expect("example is readable")
        .flatten()
    {
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            // An analysis cache left by a manual run is not part of the example.
            if entry.file_name() != ".archguard" {
                copy_dir(&path, &target);
            }
        } else {
            std::fs::copy(&path, &target).expect("file copied");
        }
    }
}

/// The view bar's tab with `label`: the visible text of the tab (each tab
/// also holds a hidden bold copy for sizing, at the same place).
fn view_tab(ui: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(ui, label)
        .find(|e| e.size().width > 0.0)
        .unwrap_or_else(|| panic!("no element labelled {label:?}"))
}

/// Status of the only open document.
fn status() -> Option<Status> {
    with_store(|s| s.docs().first().map(|d| d.status.clone()))
}

/// The views shown after each click, and the chapter draft after editing.
type Walk = (Vec<String>, String);

thread_local! {
    /// What the walk through the views found, set on the event loop.
    static OUTCOME: RefCell<Option<Result<Walk, String>>> = const { RefCell::new(None) };
    /// The views shown after each click, once clicked.
    static VIEWS_SHOWN: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// A key press and release, then a moment as between a person's keys
/// (pending pointer handling and timers run in it).
fn key(ui: &AppWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(80));
}

fn type_text(ui: &AppWindow, text: &str) {
    for c in text.chars() {
        key(ui, c.to_string());
    }
}

/// Open the chapter editor, click into its last block (the empty one to
/// type in after the generated text) and type there; the draft. `None`
/// while the chapter is still being written (a worker writes it).
fn edit_chapter(ui: &AppWindow) -> Option<Result<String, String>> {
    view_tab(ui, "Architecture").mock_single_click(PointerEventButton::Left);
    let edit = ElementHandle::find_by_accessible_label(ui, "Edit chapter")
        .find(|e| e.size().width > 0.0)?;
    edit.mock_single_click(PointerEventButton::Left);
    Some(type_into_editor(ui))
}

fn type_into_editor(ui: &AppWindow) -> Result<String, String> {
    let block = ElementHandle::find_by_element_type_name(ui, "RichBlock")
        .filter(|e| e.size().width > 0.0)
        .last()
        .ok_or("no editable block in the chapter editor")?;
    let (at, size) = (block.absolute_position(), block.size());
    let point = slint::LogicalPosition::new(at.x + 20.0, at.y + size.height - 6.0);
    for event in [
        WindowEvent::PointerMoved { position: point },
        WindowEvent::PointerPressed {
            position: point,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position: point,
            button: PointerEventButton::Left,
        },
    ] {
        ui.window().dispatch_event(event);
    }
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(300));
    type_text(ui, "Neu");
    key(ui, Key::Return);
    // Bold: Control, which Slint reads as Command on macOS.
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    key(ui, "b");
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    type_text(ui, "x");
    Ok(ui.global::<ArchitectureData>().get_draft().to_string())
}

/// One step on the event loop: advance the mock clock (so the app's own
/// timers fire), then, once the analysis has landed, click through the
/// views. `None` while the analysis is still running.
fn step(ui: &AppWindow, started: Instant) -> Option<Result<Walk, String>> {
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(100));
    match status() {
        Some(Status::Loading) if started.elapsed() < TIMEOUT => None,
        Some(Status::Loading) => Some(Err(format!("analysis still loading after {TIMEOUT:?}"))),
        Some(Status::Failed(e)) => Some(Err(format!("analysis failed: {e}"))),
        None => Some(Err("no document open".into())),
        Some(Status::Ready) => {
            if VIEWS_SHOWN.with_borrow(Option::is_none) {
                let views = VIEWS
                    .iter()
                    .map(|(_, label)| {
                        view_tab(ui, label).mock_single_click(PointerEventButton::Left);
                        ui.get_tab().to_string()
                    })
                    .collect();
                VIEWS_SHOWN.set(Some(views));
            }
            let draft = match edit_chapter(ui) {
                None if started.elapsed() < TIMEOUT => return None,
                None => Err(format!("no chapter to edit after {TIMEOUT:?}")),
                Some(draft) => draft,
            };
            let views = VIEWS_SHOWN.take().unwrap_or_default();
            Some(draft.map(|draft| (views, draft)))
        }
    }
}

#[test]
fn every_view_opens_from_the_view_bar_after_a_project_loads() {
    // Mock time: the simulated clicks advance Slint's clock themselves.
    slint::platform::set_platform(Box::new(TestingBackend::new(TestingBackendOptions {
        mock_time: true,
        threading: true,
    })))
    .expect("platform");

    let dir = tempfile::tempdir().expect("temp dir");
    let project = dir.path().join("service-shop");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/service-shop"),
        &project,
    );
    settings_file::init(Some(dir.path().join("desktop.toml")));
    let _ = slint::select_bundled_translation("en");

    let ui = AppWindow::new().expect("window");
    ui.window().set_size(slint::LogicalSize::new(1480.0, 920.0));
    format::wire(&ui);
    documents_ui::wire(&ui);
    shell_ui::wire(&ui);
    documents_ui::open(&ui, project);
    ui.show().expect("shown");
    // A shown window has the keyboard, as on screen: focused inputs keep it.
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));

    // With mock time the event loop runs no timers: a helper thread posts
    // a step every 100 ms until one reports an outcome and quits the loop.
    let started = Instant::now();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let weak = ui.as_weak();
    let ticks = {
        let done = done.clone();
        std::thread::spawn(move || {
            while !done.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(100));
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak.upgrade() {
                        OUTCOME.with(|o| {
                            if o.borrow().is_none() {
                                *o.borrow_mut() = step(&ui, started);
                            }
                            if o.borrow().is_some() {
                                let _ = slint::quit_event_loop();
                            }
                        });
                    }
                });
            }
        })
    };
    slint::run_event_loop().expect("event loop");
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    ticks.join().expect("tick thread");

    let (shown, draft) = OUTCOME
        .take()
        .expect("the check ran")
        .unwrap_or_else(|e| panic!("{e}"));
    let expected: Vec<String> = VIEWS.iter().map(|(v, _)| v.to_string()).collect();
    assert_eq!(shown, expected, "the view shown after each click");
    // Typed after the generated text: a paragraph, then a bold one.
    let end = &draft[draft.len().saturating_sub(80)..];
    assert!(draft.ends_with("\n\nNeu\n\n**x**\n"), "{end:?}");
}
