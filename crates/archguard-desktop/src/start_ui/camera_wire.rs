//! The camera callbacks: drags and the wheel move at once (at most once
//! per frame), buttons and keys glide ([`motion`]).

use std::cell::RefCell;
use std::time::Instant;

use slint::{ComponentHandle, Timer, TimerMode};

use super::keys::{self, KeyAction};
use super::motion::{DURATION, Glide};
use super::scene::{self, Camera};
use super::state::StartState;
use crate::deferred::{Coalesce, FRAME};
use crate::documents;
use crate::tab_support::{self, Actions};
use crate::ui::{AppWindow, StartData};

struct Running {
    glide: Glide,
    start: Instant,
}

thread_local! {
    static RUNNING: RefCell<Option<Running>> = const { RefCell::new(None) };
    static TIMER: Timer = Timer::default();
}

/// Stop a glide where it is (a drag or the wheel takes over).
pub fn stop() {
    RUNNING.with_borrow_mut(|r| *r = None);
    TIMER.with(Timer::stop);
}

/// Glide to `target` of the pose a running glide is heading to (so two
/// quick clicks add up), else of the current one.
fn glide(ui: &AppWindow, target: impl FnOnce(Camera) -> Camera) {
    let Some(now) = documents::with_active_state(|s: &mut StartState| s.cam) else {
        return;
    };
    let base = RUNNING.with_borrow(|r| r.as_ref().map(|r| r.glide.to));
    let to = target(base.unwrap_or(now));
    RUNNING.with_borrow_mut(|r| {
        *r = Some(Running {
            glide: Glide { from: now, to },
            start: Instant::now(),
        })
    });
    let w = ui.as_weak();
    TIMER.with(|t| {
        t.start(TimerMode::Repeated, FRAME, move || {
            let Some(ui) = w.upgrade() else { return stop() };
            let Some((g, t)) = RUNNING.with_borrow(|r| {
                let r = r.as_ref()?;
                Some((
                    r.glide,
                    r.start.elapsed().as_secs_f64() / DURATION.as_secs_f64(),
                ))
            }) else {
                return stop();
            };
            tab_support::update(&ui, |s: &mut StartState, _| s.cam = g.at(t));
            if t >= 1.0 {
                stop();
            }
        });
    });
}

fn fit(cam: Camera) -> Camera {
    Camera {
        rotation: cam.rotation,
        ..Camera::default()
    }
}

/// Move a running glide by canvas pixels.
fn shift(dx: f64, dy: f64) {
    RUNNING.with_borrow_mut(|r| {
        let Some(r) = r else { return };
        for c in [&mut r.glide.from, &mut r.glide.to] {
            (c.x, c.y) = (c.x + dx, c.y + dy);
        }
    });
}

/// A key on the stage; whether it was used.
fn key(ui: &AppWindow, text: &str) -> bool {
    let Some(action) = keys::action(text) else {
        return false;
    };
    match action {
        KeyAction::Zoom(f) => glide(ui, |c| scene::zoom(c, f, (c.x, c.y))),
        KeyAction::Fit => glide(ui, fit),
        KeyAction::Turn(r) => glide(ui, |c| scene::rotate(c, r)),
        // At once, and a running glide moves along.
        KeyAction::Pan(dx, dy) => tab_support::update(ui, |s: &mut StartState, _| {
            shift(dx, dy);
            s.cam.x += dx;
            s.cam.y += dy;
        }),
        KeyAction::Details => tab_support::update(ui, |s: &mut StartState, _| {
            s.details_open = s.has_selection();
        }),
        KeyAction::Close => tab_support::update(ui, |s: &mut StartState, _| {
            s.details_open = false;
        }),
    }
    true
}

/// Register the buttons' and keys' camera moves.
pub fn wire(ui: &AppWindow, d: &StartData<'_>) {
    let on = |f: fn(&AppWindow, f32)| tab_support::with_ui(ui, move |ui, v: f32| f(ui, v));
    let zoom = on(|ui, f| glide(ui, |c| scene::zoom(c, f64::from(f), (c.x, c.y))));
    d.on_zoom_by(zoom);
    let turn = on(|ui, r| glide(ui, |c| scene::rotate(c, f64::from(r))));
    d.on_turn(turn);
    let orient = on(|ui, _| glide(ui, |c| Camera { rotation: 0.0, ..c }));
    d.on_orient(move || orient(0.0));
    let fit_now = on(|ui, _| glide(ui, fit));
    d.on_fit(move || fit_now(0.0));
    let w = ui.as_weak();
    d.on_key(move |text| w.upgrade().is_some_and(|ui| key(&ui, &text)));
    wire_gesture(ui, d);
}

/// Pinch zooms at the gesture's centre, twist turns; at most once per
/// frame, with the steps of the frame combined.
fn wire_gesture(ui: &AppWindow, d: &StartData<'_>) {
    type Step = (f64, f64, f64, f64);
    let a = Actions::<StartState>::new(ui);
    let apply = a.on(|s, _, (factor, turn, x, y): Step| {
        stop();
        let point = (x - s.canvas.0 / 2.0, y - s.canvas.1 / 2.0);
        s.cam = scene::rotate(scene::zoom(s.cam, factor, point), turn);
    });
    let merge = |a: Step, b: Step| (a.0 * b.0, a.1 + b.1, b.2, b.3);
    let step = Coalesce::throttle(FRAME, merge, apply);
    d.on_pinch(move |f, r, x, y| {
        step.call((f64::from(f), f64::from(r), f64::from(x), f64::from(y)));
    });
}
