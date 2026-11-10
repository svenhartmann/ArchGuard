//! `ARCHGUARD_DESKTOP_BENCH=<samples>` (snapshot builds, with
//! `ARCHGUARD_DESKTOP_SNAPSHOT` for the off-screen window): time the
//! interactions that felt sluggish, then quit without a PNG.
//!
//! Each sample runs a step's action as the UI would (`callback`), waits
//! for deferred work (debounce, frame timers), sums the measured pushes
//! (`timing`, with their count) and renders the window once (`render`,
//! which also instantiates changed repeaters). Medians go to stderr.

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::ComponentHandle;

use crate::panes_ui;
use crate::timing;
use crate::ui::{
    AppWindow, ArchitectureData, CallsData, QualityData, SecurityData, ServicesData, StartData,
};

type Act = Rc<dyn Fn(&AppWindow, usize)>;

struct Step {
    name: &'static str,
    /// Runs once before the samples (unmeasured).
    setup: fn(&AppWindow),
    act: Act,
}

/// Prefixes of a typed search, one more key per sample.
fn typed(i: usize) -> slint::SharedString {
    let word = "state_view";
    word[..1 + i % word.len()].into()
}

fn graph_id() -> i32 {
    panes_ui::shown_id("graph").map_or(-1, |id| id as i32)
}

/// A package row halfway down the shown Graph, after "Expand all".
fn middle_package() -> i32 {
    let row = crate::instances::with(graph_id() as u32, |g: &mut crate::graph_ui::Graph| {
        let rows = &g.g.view.rows;
        eprintln!("bench graph rows: {}", rows.len());
        (rows.len() / 2..rows.len()).find(|&r| rows[r].expandable)
    });
    row.flatten().map_or(0, |r| r as i32)
}

fn steps() -> Vec<Step> {
    let mut all = graph_steps();
    all.extend(view_steps());
    all.extend(service_steps());
    all.extend(document_steps());
    all
}

/// Architecture and Calls: the actions of the arc42 and call views.
fn document_steps() -> Vec<Step> {
    vec![
        Step {
            name: "architecture chapter",
            setup: |ui| ui.invoke_navigate("architecture".into()),
            act: Rc::new(|ui, i| {
                ui.global::<ArchitectureData>()
                    .invoke_select_chapter(1 + (i % 12) as i32)
            }),
        },
        Step {
            name: "calls tree select",
            setup: |ui| ui.invoke_navigate("calls".into()),
            act: Rc::new(|ui, i| ui.global::<CallsData>().invoke_tree_select((i % 8) as i32)),
        },
    ]
}

fn graph_steps() -> Vec<Step> {
    let none: fn(&AppWindow) = |_| {};
    let row = Rc::new(std::cell::Cell::new(-1));
    let toggled = row.clone();
    vec![
        Step {
            name: "idle render",
            setup: |ui| {
                ui.invoke_navigate("graph".into());
                ui.invoke_expand_all(graph_id());
            },
            act: Rc::new(|_, _| {}),
        },
        Step {
            name: "graph expand/collapse",
            setup: none,
            act: Rc::new(move |ui, _| {
                if toggled.get() < 0 {
                    toggled.set(middle_package());
                }
                ui.invoke_toggle_row(graph_id(), toggled.get());
            }),
        },
        Step {
            name: "graph select",
            setup: none,
            act: Rc::new(|ui, i| ui.invoke_select_row(graph_id(), 1 + (i % 8) as i32)),
        },
        Step {
            name: "tree hover",
            setup: |ui| ui.invoke_select_row(graph_id(), -1),
            act: Rc::new(|ui, i| ui.set_tree_hovered(if i % 2 == 0 { 2 } else { -1 })),
        },
        Step {
            name: "header search key",
            setup: none,
            act: Rc::new(|ui, i| ui.invoke_search(typed(i))),
        },
    ]
}

/// Matrix, Quality and World, each after the one before.
fn view_steps() -> Vec<Step> {
    vec![
        Step {
            name: "matrix search key",
            setup: |ui| {
                ui.invoke_search("".into());
                ui.invoke_navigate("dsm".into());
                let id = panes_ui::shown_id("dsm").map_or(-1, |id| id as i32);
                ui.invoke_m_expand_all(id);
            },
            act: Rc::new(|ui, i| {
                let id = panes_ui::shown_id("dsm").map_or(-1, |id| id as i32);
                ui.invoke_m_search(id, typed(i));
            }),
        },
        Step {
            name: "quality search key",
            setup: |ui| ui.invoke_navigate("quality".into()),
            act: Rc::new(|ui, i| ui.global::<QualityData>().invoke_query(typed(i))),
        },
        Step {
            name: "world rotate (4 moves)",
            setup: |ui| ui.invoke_navigate("start".into()),
            act: Rc::new(|ui, _| {
                for _ in 0..4 {
                    ui.global::<StartData>().invoke_rotate_by(0.02);
                }
            }),
        },
    ]
}

/// Services and Security, after World.
fn service_steps() -> Vec<Step> {
    vec![
        Step {
            name: "services select",
            setup: |ui| ui.invoke_navigate("services".into()),
            act: Rc::new(|ui, i| {
                ui.global::<ServicesData>()
                    .invoke_select_service((i % 4) as i32)
            }),
        },
        Step {
            name: "services relation kind",
            setup: |ui| ui.global::<ServicesData>().invoke_show_all(),
            act: Rc::new(|ui, i| {
                ui.global::<ServicesData>()
                    .invoke_select_kind((i % 2) as i32)
            }),
        },
        Step {
            name: "security severity",
            setup: |ui| ui.invoke_navigate("security".into()),
            act: Rc::new(|ui, i| {
                ui.global::<SecurityData>()
                    .invoke_set_severity((i % 3) as i32)
            }),
        },
        Step {
            name: "security search key",
            setup: |ui| ui.global::<SecurityData>().invoke_set_severity(0),
            act: Rc::new(|ui, i| ui.global::<SecurityData>().invoke_set_query(typed(i))),
        },
    ]
}

#[derive(Default)]
struct Samples {
    callback: Vec<f64>,
    push: Vec<f64>,
    pushes: Vec<usize>,
    render: Vec<f64>,
}

fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

fn report(name: &str, s: &Samples) {
    let pushes = s.pushes.iter().sum::<usize>() as f64 / s.pushes.len().max(1) as f64;
    eprintln!(
        "bench {name:<24} callback {:>8.2} ms  push {:>8.2} ms ({pushes:.1} pushes)  render {:>8.2} ms",
        median(&s.callback),
        median(&s.push),
        median(&s.render),
    );
}

fn render(ui: &AppWindow) -> f64 {
    let start = Instant::now();
    let _ = ui.window().take_snapshot();
    start.elapsed().as_secs_f64() * 1000.0
}

/// Run every step `n` times, one sample per timer tick, then quit.
pub fn run(ui: &AppWindow, n: usize) {
    let steps = Rc::new(steps());
    (steps[0].setup)(ui);
    render(ui);
    next(ui.as_weak(), steps, 0, 0, Samples::default(), n);
}

fn next(
    weak: slint::Weak<AppWindow>,
    steps: Rc<Vec<Step>>,
    step: usize,
    sample: usize,
    mut acc: Samples,
    n: usize,
) {
    let Some(ui) = weak.upgrade() else { return };
    let Some(s) = steps.get(step) else {
        let _ = slint::quit_event_loop();
        return;
    };
    timing::take();
    let start = Instant::now();
    (s.act)(&ui, sample);
    acc.callback.push(start.elapsed().as_secs_f64() * 1000.0);
    // Long enough for the search debounce and the frame timer.
    slint::Timer::single_shot(Duration::from_millis(250), move || {
        let Some(ui) = weak.upgrade() else { return };
        let (ms, count) = timing::take();
        acc.push.push(ms);
        acc.pushes.push(count);
        acc.render.push(render(&ui));
        if sample + 1 < n {
            return next(weak, steps, step, sample + 1, acc, n);
        }
        report(steps[step].name, &acc);
        if let Some(t) = steps.get(step + 1) {
            (t.setup)(&ui);
            render(&ui);
        }
        next(weak, steps, step + 1, 0, Samples::default(), n);
    });
}
