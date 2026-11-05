//! `archguard-desktop`: native viewer for the module dependency graph.
//!
//! Usage: `archguard-desktop [PROJECT_DIR]` (default: the current
//! directory). The analysis runs on a worker thread with the same
//! cached pipeline as the CLI; the window fills once it is done.

mod layout;
mod model;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use archguard_engine::{Level, analyze_run_quiet, load_config};
use slint::{Color, Model, ModelRc, SharedString, VecModel};

use model::{Module, Overview};

slint::include_modules!();

thread_local! {
    static STATE: RefCell<Option<Overview>> = const { RefCell::new(None) };
}

fn main() -> Result<(), slint::PlatformError> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current directory is readable"));
    let root = root.canonicalize().unwrap_or(root);
    let project = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());

    let ui = AppWindow::new()?;
    ui.set_project(project.clone().into());
    ui.set_root_path(root.display().to_string().into());
    ui.set_status(format!("Scanne {}", root.display()).into());

    let modules_model: Rc<VecModel<ModuleItem>> = Rc::new(VecModel::default());
    ui.set_modules(ModelRc::from(modules_model.clone()));

    {
        let weak = ui.as_weak();
        let modules_model = modules_model.clone();
        ui.on_select(move |i| {
            let Some(ui) = weak.upgrade() else { return };
            select(&ui, &modules_model, (i >= 0).then_some(i as usize));
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_search(move |q| {
            let Some(ui) = weak.upgrade() else { return };
            STATE.with_borrow(|s| {
                if let Some(o) = s {
                    ui.set_list(list_model(o, &o.search(&q)));
                }
            });
        });
    }

    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = load_config(None, &root)
            .and_then(|loaded| analyze_run_quiet(&root, &loaded, Level::Module))
            .map(|run| Overview::from_run(project, &run))
            .map_err(|e| e.to_string());
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            match result {
                Ok(o) => show(&ui, o),
                Err(e) => {
                    ui.set_loading(false);
                    ui.set_error(e.into());
                }
            }
        });
    });

    ui.run()
}

fn show(ui: &AppWindow, o: Overview) {
    let modules = ui.get_modules();
    if let Some(model) = modules.as_any().downcast_ref::<VecModel<ModuleItem>>() {
        model.set_vec(o.modules.iter().map(module_item).collect::<Vec<_>>());
    }
    ui.set_edges(ModelRc::new(VecModel::from(
        o.deps
            .iter()
            .map(|d| EdgeItem {
                from: d.from as i32,
                to: d.to as i32,
                weight: d.weight.min(i32::MAX as u64) as i32,
                path: d.path.clone().into(),
                cyclic: d.cyclic,
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_list(list_model(&o, &o.search("")));
    ui.set_canvas_w(o.width);
    ui.set_canvas_h(o.height);
    ui.set_dep_count(o.deps.len() as i32);
    ui.set_cycles(o.cycles as i32);
    ui.set_passed(o.passed);
    ui.set_acd(format!("{:.2}", o.acd).into());
    ui.set_loading(false);
    STATE.set(Some(o));
    ui.invoke_fit();
    #[cfg(feature = "snapshot")]
    snapshot::schedule(ui);
}

fn select(ui: &AppWindow, modules_model: &VecModel<ModuleItem>, selected: Option<usize>) {
    STATE.with_borrow(|s| {
        let Some(o) = s else { return };
        let bright = o.bright(selected);
        for (i, b) in bright.into_iter().enumerate() {
            if let Some(mut row) = modules_model.row_data(i)
                && row.bright != b
            {
                row.bright = b;
                modules_model.set_row_data(i, row);
            }
        }
        let neighbors = |list: Vec<model::Neighbor>| {
            ModelRc::new(VecModel::from(
                list.into_iter()
                    .map(|n| {
                        let m = &o.modules[n.index];
                        NeighborItem {
                            index: n.index as i32,
                            label: m.label.clone().into(),
                            scope: m.scope.clone().into(),
                            weight: n.weight.min(i32::MAX as u64) as i32,
                            accent: accent(m),
                        }
                    })
                    .collect::<Vec<_>>(),
            ))
        };
        match selected {
            Some(i) => {
                ui.set_outgoing(neighbors(o.outgoing(i)));
                ui.set_incoming(neighbors(o.incoming(i)));
            }
            None => {
                ui.set_outgoing(ModelRc::default());
                ui.set_incoming(ModelRc::default());
            }
        }
        ui.set_selected(selected.map_or(-1, |i| i as i32));
    });
}

fn list_model(o: &Overview, indices: &[usize]) -> ModelRc<ListItem> {
    ModelRc::new(VecModel::from(
        indices
            .iter()
            .map(|&i| {
                let m = &o.modules[i];
                ListItem {
                    index: i as i32,
                    label: m.label.clone().into(),
                    scope: m.scope.clone().into(),
                    accent: accent(m),
                    in_cycle: m.in_cycle,
                }
            })
            .collect::<Vec<_>>(),
    ))
}

fn module_item(m: &Module) -> ModuleItem {
    ModuleItem {
        name: SharedString::from(m.name.as_str()),
        label: m.label.clone().into(),
        scope: m.scope.clone().into(),
        fan_in: m.fan_in as i32,
        fan_out: m.fan_out as i32,
        instability: m.instability,
        in_cycle: m.in_cycle,
        x: m.x,
        y: m.y,
        bright: true,
        accent: accent(m),
    }
}

/// Teal for stable, amber for unstable modules.
fn accent(m: &Module) -> Color {
    let t = m.instability.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color::from_rgb_u8(mix(0x2d, 0xf5), mix(0xd4, 0xa5), mix(0xbf, 0x24))
}

#[cfg(feature = "snapshot")]
mod snapshot {
    use super::AppWindow;
    use slint::ComponentHandle;

    /// After the layout settled: optionally select a module
    /// (`ARCHGUARD_DESKTOP_SELECT=<index>`), render, write the PNG, quit.
    pub fn schedule(ui: &AppWindow) {
        let Some(path) = std::env::var_os("ARCHGUARD_DESKTOP_SNAPSHOT") else {
            return;
        };
        if let Some(i) = std::env::var("ARCHGUARD_DESKTOP_SELECT")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
        {
            ui.invoke_select(i);
        }
        let weak = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(900), move || {
            let Some(ui) = weak.upgrade() else { return };
            let shot = ui
                .window()
                .take_snapshot()
                .expect("renderer supports snapshots");
            let file = std::fs::File::create(&path).expect("snapshot file is writable");
            let mut enc = png::Encoder::new(file, shot.width(), shot.height());
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().expect("png header");
            writer.write_image_data(shot.as_bytes()).expect("png data");
            drop(writer);
            let _ = slint::quit_event_loop();
        });
    }
}
