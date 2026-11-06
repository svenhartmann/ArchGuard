//! What the World reports to "Follow left": entering, leaving and
//! selecting packages.

use super::state::StartState;
use crate::follow::Place;
use crate::ui::AppWindow;

/// A World callback that reports entering, leaving and selecting
/// packages for "Follow left".
pub(super) fn navigation<A: 'static>(
    ui: &AppWindow,
    f: impl Fn(A) + 'static,
) -> impl Fn(A) + 'static {
    crate::follow_ui::reporting(ui, "start", place, f)
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
