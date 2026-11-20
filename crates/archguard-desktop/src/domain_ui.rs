//! Domain tab (ADR 0022), laid out like the Screens tab: top bar (service,
//! simulated user shared with the Screens tab, summary), the tree by
//! service and aggregate with the events, the focused entity, event or use
//! case in the centre, the note form on the right and "Show here" over the
//! view. The model comes from `archguard_engine::domain_model::analyze`.

mod chain;
#[cfg(test)]
mod chain_tests;
mod entities;
mod flow;
mod focus;
mod ids;
mod services;
mod state;
mod states;
mod tree;
mod view;

use slint::ComponentHandle;

use crate::context::Loaded;
use crate::screens_ui::{fill_form, form_of, notes};
use crate::tab_support::{self, Actions, TabState, Texts, index};
use crate::ui::{AppWindow, DomainData, DomainViewModel};
pub use state::DomainState;
use state::{Focus, build_from};

pub fn wire(ui: &AppWindow) {
    let d = ui.global::<DomainData>();
    let a = Actions::<DomainState>::new(ui);
    d.on_select(a.on2(|s, _, kind: i32, i: i32| {
        if let Some(f) = index(i).and_then(|i| Focus::of(kind, i)) {
            s.select(f)
        }
    }));
    d.on_select_state(a.on(|s, _, name: slint::SharedString| s.select_state(&name)));
    d.on_select_role(a.on(|s, _, i: i32| {
        let i = index(i).unwrap_or(0);
        s.select_role(i);
        crate::screens_ui::share_role(i);
    }));
    d.on_select_service(a.on(|s, _, i: i32| s.select_service(index(i).unwrap_or(0))));
    d.on_toggle_group(a.on(|s, _, k: slint::SharedString| s.toggle_group(&k)));
    d.on_set_whole(a.on(|s, _, w: bool| s.whole = w));
    d.on_set_details_open(a.on(|s, _, open: bool| s.details_open = open));
    d.on_close_details(a.on0(|s, _| {
        s.note_for = None;
        s.problem.clear();
    }));
    d.on_open_editor(a.on2(|s, _, file: slint::SharedString, line: i32| {
        s.problem = crate::editor::open(&s.root, &file, line.max(0) as u32)
            .err()
            .unwrap_or_default();
    }));
    d.on_show_line(a.on2(|s, _, file: slint::SharedString, line: i32| {
        s.code.show(&s.root, &file, line.max(0) as u32)
    }));
    d.on_close_code(a.on0(|s, _| s.code.close()));
    d.on_open_note(a.on2(
        |s, ui, id: slint::SharedString, what: slint::SharedString| {
            s.open_note(&id, &what);
            fill_form(ui, s.notes.get(id.as_str()));
        },
    ));
    d.on_save_note(a.on0(|s, ui| {
        s.problem = save_note(s, &form_of(ui))
            .err()
            .map(|e| Texts::of(ui).t(&e).to_string())
            .unwrap_or_default();
    }));
}

fn save_note(s: &mut DomainState, form: &notes::Form) -> Result<(), String> {
    let Some((id, _)) = s.note_for.clone() else {
        return Ok(());
    };
    s.notes = notes::save_here(&s.root, s.config.as_ref(), &id, form)?;
    s.note_for = None;
    Ok(())
}

impl TabState for DomainState {
    fn push(&mut self, ui: &AppWindow) {
        // The Screens tab may have changed the simulated user.
        if let Some(r) = crate::screens_ui::shared_role() {
            self.select_role(r);
        }
        let m = view::build(self, &Texts::of(ui));
        ui.global::<DomainData>().set_model(m);
    }

    fn pending(ui: &AppWindow, error: Option<&str>) {
        let model = match error {
            None => DomainViewModel::default(),
            Some(e) => DomainViewModel {
                state: 2,
                error: e.into(),
                ..Default::default()
            },
        };
        ui.global::<DomainData>().set_model(model);
    }
}

/// Show the active document's tab; the model, the UI flow (for the
/// chains), the services and the notes are read once per document, on a
/// worker thread.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let root = data.root.to_path_buf();
        move || {
            let model = archguard_engine::domain_model::analyze(&root).map_err(|e| e.to_string());
            let flow = archguard_engine::ui_flow::analyze(&root).map_err(|e| e.to_string());
            let mut s = build_from(model, flow);
            if let Some(d) = s.model() {
                s.services = services::Services::find(&root, d, (*s.flow).as_ref().ok());
            }
            s.fold_other_services();
            s.config = archguard_engine::load_config(None, &root)
                .ok()
                .map(|l| l.config);
            s.notes = notes::load(&root);
            s.root = root;
            snapshot_overrides(&mut s);
            s
        }
    });
}

/// `ARCHGUARD_DESKTOP_DOMAIN_ENTITY=<name>` focuses an entity,
/// `…_DOMAIN_EVENT=<type>` an integration or domain event,
/// `…_DOMAIN_STATE=<state>` a state of the entity, `…_DOMAIN_SERVICE=<name>`
/// the service, `…_DOMAIN_WHOLE=1` the whole model,
/// `…_DOMAIN_NOTE=<rule index of the entity>` the note form of that rule,
/// `…_DOMAIN_CODE=<file>:<line>` the code overlay, and
/// `ARCHGUARD_DESKTOP_FLOW_ROLE=<index>` the simulated user for snapshot runs.
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut DomainState) {
    let var = |k: &str| std::env::var(format!("ARCHGUARD_DESKTOP_DOMAIN_{k}")).ok();
    if let Some(r) = std::env::var("ARCHGUARD_DESKTOP_FLOW_ROLE")
        .ok()
        .and_then(|r| r.parse().ok())
    {
        s.select_role(r);
    }
    if let Some(name) = var("SERVICE")
        && let Some(i) = s.services.names.iter().position(|n| *n == name)
    {
        s.select_service(i + 1);
    }
    let Some(d) = s.model().cloned() else { return };
    if let Some(name) = var("ENTITY")
        && let Some(i) = d.entities.iter().position(|e| e.name == name)
    {
        s.select(Focus::Entity(i));
    }
    if let Some(name) = var("EVENT") {
        let ie = d
            .integration_events
            .iter()
            .position(|e| e.type_name == name);
        let de = d.events.iter().position(|e| e.type_name == name);
        if let Some(f) = ie.map(Focus::Integration).or(de.map(Focus::Event)) {
            s.select(f);
        }
    }
    if let Some(state) = var("STATE") {
        s.select_state(&state);
    }
    s.whole = var("WHOLE").is_some();
    if let (Some(k), Some(e)) = (
        var("NOTE").and_then(|k| k.parse::<usize>().ok()),
        s.selected(),
    ) {
        let ids = ids::rules(&d);
        let rule = (d.rules.iter().zip(&ids))
            .filter(|(r, _)| r.entity == d.entities[e].id)
            .nth(k);
        if let Some((r, id)) = rule {
            s.open_note(
                id,
                &format!("{} · {}", r.method, r.condition.clone().unwrap_or_default()),
            );
        }
    }
    if let Some((file, line)) = var("CODE").as_deref().and_then(|c| c.rsplit_once(':')) {
        s.code.show(&s.root, file, line.parse().unwrap_or(1));
    }
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut DomainState) {}
