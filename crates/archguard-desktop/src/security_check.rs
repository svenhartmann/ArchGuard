//! Pixel check of the Security tab's scan panel in a narrow pane, where
//! its texts wrap and the page is taller than the pane: the open panel's
//! border encloses all its rows, the last one ("Scanner diagnostics") and
//! that list's entries included, so nothing spills into the gap below it.
//! And the page fits the pane: KPI cards, severity chips, the status line
//! and the scan command wrap or shrink instead of being drawn beyond the
//! right edge, where the pane would scroll sideways.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { SecurityView, SecurityData } from "ui/security_view.slint";
    export { SecurityData }
    export component Page inherits Window {
        width: 300px;
        height: 1000px;
        SecurityView {}
    }
}

const W: usize = 300;
const H: usize = 1000;
/// Inside the panel's left padding: left of its texts, right of its border.
const PANEL_X: usize = 20;
/// Where the panel's texts start (page and panel padding).
const TEXT_X: usize = 29;
const DIAGNOSTIC: &str = "crates/archguard-analysis/Cargo.toml: no package inventory reported by Trivy; dependency coverage is incomplete.";

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

fn line(text: &str) -> SecurityLine {
    SecurityLine {
        text: text.into(),
        packages: -1,
    }
}

/// A saved scan with wrapping facts and diagnostics, and enough findings
/// that the page is taller than the window.
fn model() -> SecurityModel {
    let finding = SecurityFindingItem {
        severity: "high".into(),
        package: "openssl".into(),
        id: "CVE-2026-0001".into(),
        title: "A finding that makes the page longer than the window.".into(),
        ..Default::default()
    };
    SecurityModel {
        loaded: true,
        metrics: rows(vec![KpiItem::default(); 4]),
        severities: rows(vec![slint::SharedString::from("0"); 5]),
        scanner: "trivy 0.76.0".into(),
        scanned_at: "2026-10-27T21:08:36.703483+01:00".into(),
        database_updated_at: "2026-10-27T13:12:49.474279883Z".into(),
        command: "archguard security -- '/a/very/long/path/to/project'".into(),
        targets: rows(vec![line("Cargo.lock")]),
        diagnostics: rows(vec![line(DIAGNOSTIC); 10]),
        findings: rows(vec![finding; 10]),
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

/// The runs of rows (first, last) in which column `x` is not the page
/// background, as cards, chips, the panel and fields cross it.
fn runs(buf: &[PremultipliedRgbaColor], x: usize) -> Vec<(usize, usize)> {
    let bg = buf[(H / 2) * W + 4];
    let mut out: Vec<(usize, usize)> = Vec::new();
    for y in 0..H {
        if same(&buf[y * W + x], &bg) {
            continue;
        }
        match out.last_mut() {
            Some(run) if run.1 + 1 == y => run.1 = y,
            _ => out.push((y, y)),
        }
    }
    out
}

/// The run of the closed scan panel: a header row with 10 px padding.
fn closed_panel(buf: &[PremultipliedRgbaColor]) -> (usize, usize) {
    *runs(buf, PANEL_X)
        .iter()
        .find(|(a, b)| (40..=56).contains(&(b - a + 1)))
        .expect("the closed scan panel")
}

fn click(window: &MinimalSoftwareWindow, x: usize, y: usize) {
    let position = LogicalPosition::new(x as f32, y as f32);
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

/// The panel's last row; asserts that no text is drawn in the 16 px gap
/// below it.
fn encloses_its_rows(buf: &[PremultipliedRgbaColor], top: usize) -> usize {
    let (_, bottom) = *runs(buf, PANEL_X)
        .iter()
        .find(|(a, _)| *a == top)
        .expect("the panel");
    let bg = buf[(H / 2) * W + 4];
    let ink =
        (bottom + 2..bottom + 14).find(|y| (TEXT_X..W - 20).any(|x| !same(&buf[y * W + x], &bg)));
    assert_eq!(ink, None, "text drawn below the panel's bottom at {bottom}");
    bottom
}

#[test]
fn the_open_scan_panel_encloses_its_last_row() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(W as u32, H as u32));
    slint::platform::set_platform(Box::new(Pixels(window.clone()))).expect("platform");
    let page = Page::new().expect("component");
    page.show().expect("shown");
    page.global::<SecurityData>().set_model(model());
    let (top, bottom) = closed_panel(&frame(&window));
    click(&window, 60, (top + bottom) / 2);
    let open = encloses_its_rows(&frame(&window), top);
    assert!(open > bottom + 150, "the panel opened (bottom at {open})");
    // The diagnostics list: the panel's last row, above its bottom padding.
    click(&window, 60, open - 22);
    let listed = encloses_its_rows(&frame(&window), top);
    assert!(listed > open + 100, "the list opened (bottom at {listed})");
}

fn kpi(value: &str, label: &str) -> KpiItem {
    KpiItem {
        value: value.into(),
        label: label.into(),
        ..Default::default()
    }
}

/// A stale scan with the real KPI labels and severity counts.
fn scan() -> SecurityModel {
    SecurityModel {
        status: "stale".into(),
        status_label: "Scan: stale".into(),
        metrics: rows(vec![
            kpi("165", "Packages scanned"),
            kpi("0", "Affected packages"),
            kpi("0", "Findings"),
            kpi("10", "Manifest / lockfile targets"),
        ]),
        severities: rows(vec![slint::SharedString::from("0"); 5]),
        ..model()
    }
}

/// Asserts that nothing is drawn in the page's right padding, between the
/// content's right edge and the vertical scroll bar.
fn fits(buf: &[PremultipliedRgbaColor], when: &str) {
    let bg = buf[(H / 2) * W + 4];
    let ink = (0..H - 20)
        .flat_map(|y| (W - 28..W - 17).map(move |x| (x, y)))
        .find(|&(x, y)| !same(&buf[y * W + x], &bg));
    assert_eq!(ink, None, "{when}: drawn in the page's right padding");
}

#[test]
fn the_page_fits_a_narrow_pane() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(W as u32, H as u32));
    slint::platform::set_platform(Box::new(Pixels(window.clone()))).expect("platform");
    let page = Page::new().expect("component");
    page.show().expect("shown");
    page.global::<SecurityData>().set_model(scan());
    let closed = frame(&window);
    fits(&closed, "closed");
    let (top, bottom) = closed_panel(&closed);
    click(&window, 60, (top + bottom) / 2);
    fits(&frame(&window), "scan panel open");
}
