//! What the World reports to "Follow left": entering, leaving and
//! selecting packages; and how it follows a Graph on the left.

use super::state::{Mode, Selection, StartState};
use crate::follow::{Nav, Place};
use crate::ui::AppWindow;

/// A World callback that reports entering, leaving and selecting
/// packages for "Follow left".
pub(super) fn navigation<A: 'static>(
    ui: &AppWindow,
    f: impl Fn(A) + 'static,
) -> impl Fn(A) + 'static {
    crate::follow_ui::reporting(ui, "start", place, f)
}

impl StartState {
    /// "Follow left" with the Graph on the left: its package becomes the
    /// architecture World's scope, its selection the area or entity of
    /// that node.
    pub(super) fn follow(&mut self, nav: &Nav) {
        match nav {
            Nav::Scope(within) => {
                if self.services_mode() {
                    self.set_mode(Mode::Architecture);
                }
                self.navigate(within.clone());
            }
            Nav::Select(name) => self.select_node(name),
            Nav::Expand { .. } => {}
        }
    }

    /// Select the area named `name`, else the entity of that node.
    fn select_node(&mut self, name: &str) {
        let areas = &self.scope.areas;
        let sel = match areas.iter().position(|a| a.name == name) {
            Some(area) => Selection {
                area,
                entity: None,
                members: None,
            },
            None => {
                let Some((area, e)) = areas.iter().enumerate().find_map(|(i, a)| {
                    let e = a
                        .entities
                        .iter()
                        .find(|&&e| self.ev.entity(e).node == name)?;
                    Some((i, *e))
                }) else {
                    return;
                };
                Selection {
                    area,
                    entity: Some(e),
                    members: None,
                }
            }
        };
        self.select(sel);
    }
}

/// The World's package and selection by node name: the selected entity,
/// else the selected area.
fn place(s: &StartState) -> Place {
    let selected = s.selection.as_ref().and_then(|sel| match sel.entity {
        Some(e) => Some(s.ev.entity(e).node.clone()),
        None => Some(s.scope.areas.get(sel.area)?.name.clone()),
    });
    Place {
        scope: s.within.clone(),
        selected,
    }
}
