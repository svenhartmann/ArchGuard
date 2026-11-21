//! The detail panel's actions besides Explore and Source graph
//! (`worldDetails` in `viewer_world.js`): Cycles, Matrix, Calls, Class
//! metrics, Quality scope, and the texts of Copy path and AI context.
//! They open the other view in the focused pane and, once the view's
//! state exists, reveal the selection there: Calls focuses the class,
//! Class metrics opens its card in the heatmap.

use std::time::Duration;

use slint::{ComponentHandle, Timer};

use super::state::StartState;
use crate::calls_ui::{self, CallsState};
use crate::documents::{self, DocId};
use crate::panes_ui;
use crate::quality_ui::{self, QualityState};
use crate::ui::{AppWindow, CallsData, QualityData};

/// `StartData.detail-action` values.
pub mod action {
    pub const CYCLES: i32 = 0;
    pub const MATRIX: i32 = 1;
    pub const CALLS: i32 = 2;
    pub const CLASS_METRICS: i32 = 3;
    pub const QUALITY_SCOPE: i32 = 4;
}

/// What the actions are about: the selection's graph node, its class and
/// whether it is a package area.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub node: String,
    pub class_id: Option<String>,
    pub module: bool,
}

impl StartState {
    /// The selection the actions apply to (architecture World only).
    pub(super) fn action_target(&self) -> Option<Target> {
        let sel = self.selection.as_ref()?;
        let area = self.scope.areas.get(sel.area)?;
        let entity = sel.entity.map(|e| self.ev.entity(e));
        Some(Target {
            node: entity.map_or_else(|| area.name.clone(), |e| e.node.clone()),
            class_id: entity.and_then(|e| e.class_id.clone()),
            module: entity.is_none() && area.module,
        })
    }

    /// `worldCopyValue`: `file:line`, the file, the node or the area.
    pub(super) fn copy_value(&self) -> String {
        let Some(sel) = &self.selection else {
            return String::new();
        };
        let area = || self.scope.areas.get(sel.area).map(|a| a.name.clone());
        match sel.entity.map(|e| self.ev.entity(e)) {
            Some(e) => match (&e.file, e.line) {
                (Some(f), Some(l)) => format!("{f}:{l}"),
                (Some(f), None) => f.clone(),
                _ => e.node.clone(),
            },
            None => area().unwrap_or_default(),
        }
    }

    /// The web's "AI context": read-only MCP requests for the selection.
    pub(super) fn ai_context(&self) -> String {
        let Some(t) = self.action_target() else {
            return String::new();
        };
        let area = self.selection.as_ref().map(|s| &self.scope.areas[s.area]);
        let root = self.root.as_str();
        let mut world = serde_json::json!({
            "path": root,
            "within": t.node,
            "expected_revision": self.ev.world.revision,
        });
        if let Some(c) = &t.class_id {
            world["query"] = c.clone().into();
        }
        let within = match area {
            Some(a) if a.module => Some(a.name.clone()),
            _ => self.scope.name.clone(),
        };
        let selection = t.class_id.clone().or_else(|| area.map(|a| a.name.clone()));
        let v = serde_json::json!({
            "project": root,
            "selection": selection,
            "tools": [
                {"name": "architecture_world", "arguments": world},
                {"name": "architecture_atlas", "arguments": {"path": root, "within": within}},
                {"name": "architecture_findings", "arguments": {"path": root, "query": t.node}},
            ],
        });
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }
}

/// Whether an action for document `doc` waiting for its `T` runs now
/// (`Some(true)`), waits (`Some(false)`) or is dropped (`None`).
fn ready<T: 'static>(doc: DocId) -> Option<bool> {
    if documents::active_id() != Some(doc) {
        return None;
    }
    Some(documents::with_state(doc, |_: &mut T| ()).is_some())
}

/// Run `f` once the active document has a `T` (a view's state is built
/// when the view is first shown); dropped when the user switches to
/// another document or closes it, and after a minute.
fn when_ready<T: 'static>(ui: &AppWindow, f: impl FnOnce(&AppWindow) + 'static) {
    if let Some(doc) = documents::active_id() {
        poll::<T>(ui.as_weak(), doc, 0, f);
    }
}

/// One check of [`when_ready`]; a single-shot timer per retry, so no
/// timer outlives its wait.
fn poll<T: 'static>(
    w: slint::Weak<AppWindow>,
    doc: DocId,
    tries: u32,
    f: impl FnOnce(&AppWindow) + 'static,
) {
    let Some(ui) = w.upgrade() else { return };
    match ready::<T>(doc) {
        Some(true) => f(&ui),
        Some(false) if tries < 600 => {
            Timer::single_shot(Duration::from_millis(100), move || {
                poll::<T>(w, doc, tries + 1, f)
            });
        }
        _ => {}
    }
}

/// Run one of the [`action`]s for `t`; `node` is its graph node index.
pub fn run(ui: &AppWindow, t: Target, node: Option<usize>, which: i32) {
    match which {
        action::CYCLES => {
            if let Some(n) = node {
                panes_ui::reveal(ui, n);
            }
            // The Graph's explorer on its cycles list.
            if let Some(id) = panes_ui::shown_id("graph") {
                ui.invoke_set_explorer_tab(id as i32, 1);
            }
        }
        action::MATRIX => match node {
            Some(n) => panes_ui::reveal_in_view(ui, "dsm", n),
            None => panes_ui::change(ui, |l| l.navigate("dsm")),
        },
        action::CALLS => {
            panes_ui::change(ui, |l| l.navigate("calls"));
            when_ready::<CallsState>(ui, move |ui| match &t.class_id {
                Some(c) => calls_ui::focus_class(ui, c),
                None => ui
                    .global::<CallsData>()
                    .invoke_set_query(t.node.as_str().into()),
            });
        }
        action::CLASS_METRICS | action::QUALITY_SCOPE => {
            panes_ui::change(ui, |l| l.navigate("quality"));
            when_ready::<QualityState>(ui, move |ui| match &t.class_id {
                Some(c) if which == action::CLASS_METRICS => quality_ui::focus_class(ui, c),
                _ => {
                    let q = ui.global::<QualityData>();
                    q.invoke_section(5);
                    q.invoke_package(t.node.as_str().into());
                }
            });
        }
        _ => {}
    }
}

/// `ARCHGUARD_DESKTOP_WORLD_ACTION=<action>:<class id>` (snapshot runs):
/// run [`action`] `CALLS` (2) or `CLASS_METRICS` (3) for the class as its
/// World detail button does.
#[cfg(feature = "snapshot")]
pub fn snapshot_hook(ui: &AppWindow) {
    let Ok(spec) = std::env::var("ARCHGUARD_DESKTOP_WORLD_ACTION") else {
        return;
    };
    let Some((which, id)) = spec.split_once(':') else {
        return;
    };
    let t = Target {
        node: String::new(),
        class_id: Some(id.to_string()),
        module: false,
    };
    run(ui, t, None, which.parse().unwrap_or(-1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::{set_state, with_store};
    use std::path::Path;

    /// An action waits only for the document it was started in: switching
    /// to or closing that document drops it.
    #[test]
    fn a_waiting_action_belongs_to_its_document() {
        with_store(|s| *s = Default::default());
        let a = with_store(|s| s.open(Path::new("/a")).0);
        let b = with_store(|s| s.open(Path::new("/b")).0);
        assert_eq!(ready::<u16>(b), Some(false), "b is active, no state yet");
        set_state(a, 1_u16);
        assert_eq!(ready::<u16>(a), None, "a is not active");
        set_state(b, 2_u16);
        assert_eq!(ready::<u16>(b), Some(true));
        with_store(|s| s.activate(0));
        assert_eq!(ready::<u16>(b), None, "switched to a");
        assert_eq!(ready::<u16>(a), Some(true));
        with_store(|s| s.close(0));
        assert_eq!(ready::<u16>(a), None, "a is closed");
    }
}
