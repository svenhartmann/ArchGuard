//! Database tab: one sub-item per datastore engine of the topology (R11). Elasticsearch embeds
//! the Elasticsearch graph (`es_ui`), the other engines list their datastores. The topology is
//! the Services tab's, read once per document. The chosen sub-item is not stored.

mod model;
mod relational;
#[cfg(test)]
mod view_check;

use std::path::PathBuf;

use slint::ComponentHandle;

use crate::context::Loaded;
use crate::tab_support::{self, Actions, Shared, TabState, Topology, index, model as list};
use crate::ui::{AppWindow, DatabaseData, DbEngine, DbModel, DbRel, DbRow};

pub struct DbState {
    root: PathBuf,
    topology: Topology,
    engines: Vec<model::Engine>,
    active: usize,
    /// Why the editor did not open, shown in the tab.
    problem: String,
}

fn build_from(root: PathBuf, topology: Topology) -> DbState {
    let mut engines = (*topology).as_ref().map(model::engines).unwrap_or_default();
    engines.extend((*topology).as_ref().ok().and_then(relational::sub_item));
    DbState {
        root,
        problem: String::new(),
        topology,
        engines,
        active: 0,
    }
}

pub fn wire(ui: &AppWindow) {
    let a = Actions::<DbState>::new(ui);
    ui.global::<DatabaseData>().on_select(a.on(|s, _, i: i32| {
        s.active = index(i).filter(|i| *i < s.engines.len()).unwrap_or(0);
        s.problem.clear();
    }));
    ui.global::<DatabaseData>()
        .on_open(a.on2(|s, _, file: slint::SharedString, line: i32| {
            s.problem = crate::editor::open(&s.root, &file, line.max(0) as u32)
                .err()
                .unwrap_or_default();
        }));
}

impl DbState {
    fn view(&self) -> DbModel {
        let Ok(t) = &*self.topology else {
            return DbModel {
                state: 2,
                ..Default::default()
            };
        };
        let engine = self.engines.get(self.active);
        let name = engine.map(|e| e.name.as_str());
        let es = name == Some("elasticsearch");
        let relational = engine.is_some_and(|e| e.orphan || model::is_relational(&e.name));
        let rows = match name {
            Some(e) if !es && !relational => model::rows(t, e),
            _ => Vec::new(),
        };
        let rel = engine
            .filter(|_| relational)
            .map(|e| relational::lines(t, e))
            .unwrap_or_default();
        DbModel {
            state: 1,
            engines: list(self.engines.iter().map(|e| DbEngine {
                name: e.name.as_str().into(),
                count: e.count as i32,
                orphan: e.orphan,
            })),
            active: self.active as i32,
            es,
            relational,
            problem: self.problem.as_str().into(),
            rel: list(rel.iter().map(|l| DbRel {
                kind: l.kind,
                text: l.text.as_str().into(),
                detail: l.detail.as_str().into(),
                file: l.file.as_str().into(),
                line: l.line,
                clickable: l.clickable,
            })),
            rows: list(rows.iter().map(|r| DbRow {
                id: r.id.as_str().into(),
                endpoint: r.endpoint.as_str().into(),
                services: r.services.as_str().into(),
                database: r.database.as_str().into(),
            })),
        }
    }
}

impl TabState for DbState {
    fn push(&mut self, ui: &AppWindow) {
        ui.global::<DatabaseData>().set_model(self.view());
    }

    fn pending(ui: &AppWindow, error: Option<&str>) {
        ui.global::<DatabaseData>().set_model(DbModel {
            state: if error.is_some() { 2 } else { 0 },
            ..Default::default()
        });
    }
}

/// Show the active document's tab; the topology is the Services tab's.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let (root, slot) = (data.root.to_path_buf(), Shared::<Topology>::active());
        let set = crate::documents_set::active_set();
        move || {
            let topology = tab_support::topology(&slot, &root, set.as_deref());
            let mut s = build_from(root, topology);
            snapshot_overrides(&mut s);
            s
        }
    });
}

/// `ARCHGUARD_DESKTOP_DB_ENGINE=redis` opens that sub-item (snapshot runs).
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut DbState) {
    if let Ok(name) = std::env::var("ARCHGUARD_DESKTOP_DB_ENGINE")
        && let Some(i) = s.engines.iter().position(|e| e.name == name)
    {
        s.active = i;
    }
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut DbState) {}
