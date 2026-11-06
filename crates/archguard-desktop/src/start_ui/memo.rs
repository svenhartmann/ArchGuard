//! What the Start tab keeps per project with the pane layout (the web's
//! deep link `?view=…&lens=…&kpi=…&node=…`): which World, the scope, lens,
//! KPI, the services filters and the selected area or island, as the
//! `world` table of the tab's entry in `desktop.toml`. The camera is not
//! kept: it changes on every drag.

use super::services::KINDS;
use super::services_view::{DIRECTIONS, SOURCES, position};
use super::state::{Lens, Mode, Selection, StartState};
use crate::tab_support::index;
use crate::ui::AppWindow;

pub type Memo = Vec<(String, String)>;

/// The kept view of the active document's Start tab.
pub fn kept() -> Memo {
    let l = crate::panes_ui::layout();
    l.find("start")
        .map(|(p, t)| l.panes[p].tabs[t].world.clone())
        .unwrap_or_default()
}

/// Keep `s`'s view with the layout when it changed (rarely: pans and
/// zooms are not kept), which stores the layout.
pub fn keep(ui: &AppWindow, s: &StartState) {
    let memo = s.memo();
    if memo == kept() {
        return;
    }
    crate::panes_ui::change(ui, |l| {
        if let Some((p, t)) = l.find("start") {
            l.panes[p].tabs[t].world = memo;
        }
    });
}

fn get<'a>(m: &'a Memo, key: &str) -> Option<&'a str> {
    m.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn num(m: &Memo, key: &str) -> Option<i32> {
    get(m, key)?.parse().ok()
}

impl StartState {
    /// The kept view, with defaults left out.
    pub(super) fn memo(&self) -> Memo {
        let mut m = Memo::new();
        let mut put = |k: &str, v: String| m.push((k.to_string(), v));
        if self.mode == Mode::Architecture && self.services.is_some() {
            put("mode", "architecture".into());
        }
        if let Some(w) = &self.within {
            put("within", w.clone());
        }
        let lens = match self.lens {
            Lens::Problems => 0,
            Lens::Explore => 1,
            Lens::Coverage => 2,
        };
        if lens != 0 {
            put("lens", lens.to_string());
        }
        if self.kpi != 0 {
            put("kpi", self.kpi.to_string());
        }
        if let Some(sel) = &self.selection {
            put("area", self.scope.areas[sel.area].name.clone());
        }
        if let Some(w) = &self.services {
            let f = &w.filter;
            if let Some(d) = &f.deployment {
                put("deployment", d.clone());
            }
            put("source", position(&SOURCES, &f.source).to_string());
            put(
                "kind",
                f.kind.map_or(0, |k| position(&KINDS, &k) + 1).to_string(),
            );
            put("direction", position(&DIRECTIONS, &f.direction).to_string());
            let flags = [
                (f.cycles_only, "cycles"),
                (f.flow, "flow"),
                (f.all_projects, "all"),
            ];
            let on: Vec<&str> = flags.iter().filter(|f| f.0).map(|f| f.1).collect();
            put("flags", on.join(","));
            if let Some(i) = w.selected {
                put("island", w.inventory[i].path.clone());
            }
        }
        m
    }

    /// Show a kept view; what no longer exists (a package, an area, an
    /// island) is left out.
    pub(super) fn restore(&mut self, m: &Memo) {
        self.restore_services(m);
        if get(m, "mode") == Some("architecture") {
            self.set_mode(Mode::Architecture);
        }
        self.lens = match num(m, "lens") {
            Some(1) => Lens::Explore,
            Some(2) => Lens::Coverage,
            _ => Lens::Problems,
        };
        self.kpi = num(m, "kpi").and_then(index).unwrap_or(0).min(6);
        if let Some(w) = get(m, "within") {
            self.navigate(Some(w.to_string()));
            self.history.clear();
        }
        self.refresh();
        let area = get(m, "area").and_then(|a| self.scope.areas.iter().position(|x| x.name == a));
        self.selection = area.map(|area| Selection {
            area,
            entity: None,
            members: None,
        });
    }

    /// The services filters and island; a memo without them (`flags` is
    /// always kept with a services World) keeps the defaults.
    fn restore_services(&mut self, m: &Memo) {
        let Some(w) = &mut self.services else { return };
        let Some(flags) = get(m, "flags") else { return };
        let f = &mut w.filter;
        f.source = num(m, "source")
            .and_then(index)
            .and_then(|i| SOURCES.get(i).copied())
            .unwrap_or(f.source);
        f.kind = num(m, "kind")
            .and_then(|k| index(k - 1))
            .and_then(|i| KINDS.get(i).copied());
        f.direction = num(m, "direction")
            .and_then(index)
            .and_then(|i| DIRECTIONS.get(i).copied())
            .unwrap_or(f.direction);
        let has = |flag: &str| flags.split(',').any(|f| f == flag);
        (f.cycles_only, f.flow) = (has("cycles"), has("flow"));
        let deployment = get(m, "deployment")
            .filter(|d| w.scopes.iter().any(|s| s == d))
            .map(str::to_string);
        w.set_islands(deployment, has("all"));
        let island = get(m, "island").and_then(|p| w.inventory.iter().position(|i| i.path == p));
        w.selected = island.filter(|i| w.shown.contains(i));
        self.refresh();
    }
}
