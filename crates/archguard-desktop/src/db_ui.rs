//! Database tab: one sub-item per datastore engine of the topology (R11). Elasticsearch embeds
//! the Elasticsearch graph (`es_ui`), the other engines list their datastores. The topology is
//! the Services tab's, read once per document. The chosen sub-item is not stored.

mod model;
mod relational;
#[cfg(test)]
mod view_check;

use slint::ComponentHandle;

use crate::context::Loaded;
use crate::tab_support::{self, Actions, Shared, TabState, Topology, index, model as list};
use crate::ui::{AppWindow, DatabaseData, DbEngine, DbModel, DbRow};

pub struct DbState {
    topology: Topology,
    engines: Vec<model::Engine>,
    active: usize,
}

fn build_from(topology: Topology) -> DbState {
    let engines = (*topology).as_ref().map(model::engines).unwrap_or_default();
    DbState {
        topology,
        engines,
        active: 0,
    }
}

pub fn wire(ui: &AppWindow) {
    let a = Actions::<DbState>::new(ui);
    ui.global::<DatabaseData>().on_select(a.on(|s, _, i: i32| {
        s.active = index(i).filter(|i| *i < s.engines.len()).unwrap_or(0);
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
        let engine = self.engines.get(self.active).map(|e| e.name.as_str());
        let es = engine == Some("elasticsearch");
        let rows = match engine {
            Some(e) if !es => model::rows(t, e),
            _ => Vec::new(),
        };
        DbModel {
            state: 1,
            engines: list(self.engines.iter().map(|e| DbEngine {
                name: e.name.as_str().into(),
                count: e.count as i32,
            })),
            active: self.active as i32,
            es,
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
            let mut s = build_from(tab_support::topology(&slot, &root, set.as_deref()));
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
