//! "Follow left" for the Architecture tab ([`crate::follow`]): a package
//! entered becomes the building-block map's scope, a node selected
//! selects the building block holding it. A chapter without a map turns
//! to the building blocks (chapter 5) first.

use super::projection::{Group, Labels};
use super::state::{ArchitectureState, current, derived, labels, open_scope};
use super::visual::{Kind, RELATIONS_PAGE};
use crate::follow::{self, Nav, Place};
use crate::tab_support;
use crate::ui::AppWindow;

/// The chapter of the building blocks.
const BLOCKS: u8 = 5;

/// Do what the left pane did: a package entered or a node selected.
pub(super) fn follow(s: &mut ArchitectureState, labels: &Labels, nav: &Nav) {
    if matches!(nav, Nav::Expand { .. }) {
        return;
    }
    if derived(s, s.chapter).is_none() && derived(s, BLOCKS).is_some() && !s.editor.is_open() {
        s.chapter = BLOCKS;
    }
    let Some(kind) = derived(s, s.chapter) else {
        return;
    };
    match nav {
        Nav::Scope(within) => {
            let node = within
                .as_deref()
                .and_then(|w| s.graph.nodes.iter().position(|n| n.name == w));
            open_scope(s, kind, node);
        }
        Nav::Select(name) => {
            if !reveal(s, labels, name) && !in_scope(s, kind, name) {
                open_scope(s, kind, None);
                reveal(s, labels, name);
            }
        }
        Nav::Expand { .. } => {}
    }
}

/// Whether `name` lies in the scope of map `kind` (the scope package
/// itself or a node below it); `false` on the overview.
fn in_scope(s: &ArchitectureState, kind: Kind, name: &str) -> bool {
    let scope = s.maps[kind as usize].scope.and_then(|n| s.graph.nodes.get(n));
    scope.is_some_and(|dir| follow::within(name, &dir.name))
}

/// Select the deepest building block of the open map that holds `name`.
fn reveal(s: &mut ArchitectureState, labels: &Labels, name: &str) -> bool {
    let Some((kind, map)) = current(s, labels) else {
        return false;
    };
    let packages = map.shown.nodes.iter().filter_map(|n| match n.group {
        Group::Package(p) => Some((p, s.graph.nodes.get(p)?.name.as_str())),
        _ => None,
    });
    let block = packages
        .filter(|(_, dir)| follow::within(name, dir))
        .max_by_key(|(_, dir)| dir.len())
        .map(|(p, _)| p);
    let Some(p) = block else {
        return false;
    };
    let state = &mut s.maps[kind as usize];
    state.scope = map.scope;
    state.selected = Some(Group::Package(p));
    state.relations = RELATIONS_PAGE;
    true
}

/// A building-block callback that reports the map's scope and selected
/// package for "Follow left".
pub(super) fn navigation<A: 'static>(
    ui: &AppWindow,
    f: impl Fn(A) + 'static,
) -> impl Fn(A) + 'static {
    crate::follow_ui::reporting(ui, "architecture", place, f)
}

/// The open chapter's map: its package scope and selected package, by
/// node name.
fn place(s: &ArchitectureState) -> Place {
    let Some(kind) = derived(s, s.chapter) else {
        return Place::default();
    };
    let map = &s.maps[kind as usize];
    let name = |n: usize| s.graph.nodes.get(n).map(|n| n.name.clone());
    let selected = match map.selected {
        Some(Group::Package(n)) => name(n),
        _ => None,
    };
    Place {
        scope: map.scope.and_then(name),
        selected,
    }
}

/// The tab beside a leading view follows it, also while it still builds;
/// a pane beside it starts at its place (at start-up).
pub(super) fn wire() {
    crate::follow_ui::on_follow("architecture", |ui, _, nav| {
        let (labels, nav) = (labels(ui), nav.clone());
        tab_support::update_or_queue(ui, move |s: &mut ArchitectureState| {
            follow(s, &labels, &nav);
        });
    });
    crate::follow_ui::on_state_place("architecture", place);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::state::build_from;
    use super::*;

    fn state() -> ArchitectureState {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/py-layers")
            .canonicalize()
            .expect("fixture");
        let loaded = archguard_engine::load_config(None, &root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        let calls = archguard_engine::analyze_calls(&root, &run.config);
        build_from(&root, &run.config, run.graph.clone(), &run.report, &calls)
    }

    fn labels() -> Labels {
        Labels {
            direct_root: "root".into(),
            direct: "direct".into(),
            outside: "outside".into(),
        }
    }

    /// A file selected selects the building block holding it, also from a
    /// chapter without a map; a package entered scopes the map.
    #[test]
    fn architecture_reveals_the_building_block() {
        let mut s = state();
        s.chapter = 1;
        follow(&mut s, &labels(), &Nav::Select("domain/ledger.py".into()));
        assert_eq!(s.chapter, BLOCKS);
        assert_eq!(
            place(&s),
            Place {
                scope: None,
                selected: Some("domain".into()),
            }
        );
        follow(&mut s, &labels(), &Nav::Scope(Some("app".into())));
        assert_eq!(place(&s).scope.as_deref(), Some("app"));
        // Outside the scope: back to the overview, the block selected.
        follow(&mut s, &labels(), &Nav::Select("api/rest.py".into()));
        assert_eq!(place(&s).selected.as_deref(), Some("api"));
        follow(&mut s, &labels(), &Nav::Scope(None));
        assert_eq!(place(&s), Place::default());
    }

    /// A package entered in a Graph is also its selection: the scope
    /// stays, as do files inside it; only a node outside leaves it.
    #[test]
    fn a_selection_inside_the_scope_keeps_it() {
        let mut s = state();
        follow(&mut s, &labels(), &Nav::Scope(Some("domain".into())));
        follow(&mut s, &labels(), &Nav::Select("domain".into()));
        assert_eq!(place(&s).scope.as_deref(), Some("domain"));
        follow(&mut s, &labels(), &Nav::Select("domain/ledger.py".into()));
        assert_eq!(place(&s).scope.as_deref(), Some("domain"));
        follow(&mut s, &labels(), &Nav::Select("api/rest.py".into()));
        assert_eq!(place(&s).scope, None);
    }

    /// A new pair with Architecture on the left starts the right pane at
    /// its place.
    #[test]
    fn architecture_leads_a_new_pair() {
        wire();
        assert!(crate::follow_ui::leads("architecture"));
    }
}
