//! "Follow left" for the Services tab ([`crate::follow`]): a package or
//! file selects the service built from the directory holding it, else
//! the project there; the service or project picked here is reported as
//! that directory.
//!
//! Build directories are paths from the repository root; tree node names
//! may start below it (a common source root such as `services/` is not
//! part of them). A directory therefore matches by its trailing path
//! segments: `services/orders` holds the node `orders/src/server.js`.

use std::rc::Rc;

use super::state::ServicesState;
use crate::follow::{self, Nav, Place};
use crate::tab_support;
use crate::tree::Tree;
use crate::ui::AppWindow;

/// The trailing parts of directory `dir`, longest first: `a/b/c`, `b/c`,
/// `c`. The repository root (`.`, empty) has none.
fn tails(dir: &str) -> impl Iterator<Item = &str> {
    let root = matches!(dir, "" | ".");
    let starts = std::iter::once(0).chain(dir.match_indices('/').map(|(i, _)| i + 1));
    starts.filter(move |_| !root).map(move |i| &dir[i..])
}

/// The one of `dirs` (index, directory) that holds `name` with the
/// longest trailing part.
fn deepest<'a>(name: &str, dirs: impl Iterator<Item = (usize, &'a str)>) -> Option<usize> {
    dirs.filter_map(|(i, d)| {
        let tail = tails(d).find(|t| follow::within(name, t))?;
        Some((i, tail.len()))
    })
    .max_by_key(|&(_, n)| n)
    .map(|(i, _)| i)
}

impl ServicesState {
    /// Do what the left pane did: leaving a package clears the selection,
    /// a package entered or a node selected selects its service or
    /// project.
    pub fn follow(&mut self, nav: &Nav) {
        match nav {
            Nav::Scope(None) => {
                self.selected = None;
                self.project = None;
                self.clear_focus();
            }
            Nav::Scope(Some(name)) | Nav::Select(name) => self.select_node(name),
            Nav::Expand { .. } => {}
        }
    }

    fn select_node(&mut self, name: &str) {
        let Some(t) = self.topology() else { return };
        let contexts = t.services.iter().enumerate();
        let contexts = contexts.filter_map(|(i, s)| Some((i, s.build_context.as_deref()?)));
        if let Some(i) = deepest(name, contexts) {
            return self.select_service(i);
        }
        let paths = self.project_order.iter().enumerate();
        let paths = paths.map(|(row, &p)| (row, t.projects[p].path.as_str()));
        if let Some(row) = deepest(name, paths) {
            self.select_project(row);
        }
    }
}

/// The selected service's build directory, else the selected project's,
/// as the document's tree node (without a tree yet: the whole directory).
fn place(s: &ServicesState) -> Place {
    let known = |name: &str| {
        crate::documents::with_active_state(|t: &mut Rc<Tree>| {
            t.nodes.iter().any(|n| n.name == name)
        })
        .unwrap_or(true)
    };
    place_in(s, known)
}

/// [`place`] with the tree node names `known`: the longest trailing part
/// of the directory that is one.
fn place_in(s: &ServicesState, known: impl Fn(&str) -> bool) -> Place {
    let Some(t) = s.topology() else {
        return Place::default();
    };
    let service = s
        .selected
        .and_then(|i| t.services.get(i)?.build_context.as_deref());
    let project = || {
        s.project
            .and_then(|p| Some(t.projects.get(p)?.path.as_str()))
    };
    let dir = service.or_else(project).unwrap_or_default();
    Place {
        scope: tails(dir).find(|d| known(d)).map(str::to_string),
        selected: None,
    }
}

/// A Services callback that reports the selected service or project.
pub(super) fn navigation<A: 'static>(
    ui: &AppWindow,
    f: impl Fn(A) + 'static,
) -> impl Fn(A) + 'static {
    crate::follow_ui::reporting(ui, "services", place, f)
}

/// The tab beside a leading view follows it (at start-up).
pub(super) fn wire() {
    crate::follow_ui::on_follow("services", |ui, _, nav| {
        tab_support::update(ui, |s: &mut ServicesState, _| s.follow(nav));
    });
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::state::build_from;
    use super::*;

    fn shop() -> ServicesState {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/service-shop");
        build_from(std::sync::Arc::new(
            archguard_engine::analyze_services(&root).map_err(|e| e.to_string()),
        ))
    }

    fn context(s: &ServicesState) -> Option<String> {
        let t = s.topology()?;
        t.services.get(s.selected?)?.build_context.clone()
    }

    /// A file of a service's build directory selects that service, also
    /// when the tree starts below the repository root; the tab reports the
    /// directory as its tree node; leaving the package clears it.
    #[test]
    fn a_node_selects_the_service_built_from_it() {
        let mut s = shop();
        s.follow(&Nav::Select("orders/src/server.js".into()));
        assert_eq!(context(&s).as_deref(), Some("services/orders"));
        let tree = |n: &str| n == "orders";
        assert_eq!(place_in(&s, tree).scope.as_deref(), Some("orders"));
        s.follow(&Nav::Scope(Some("services/risk".into())));
        assert_eq!(context(&s).as_deref(), Some("services/risk"));
        s.follow(&Nav::Select("runtime".into()));
        assert_eq!(
            context(&s).as_deref(),
            Some("services/risk"),
            "no service: kept"
        );
        s.follow(&Nav::Scope(None));
        assert_eq!((s.selected, place_in(&s, tree)), (None, Place::default()));
    }

    /// Directories match by trailing path segments, the longest wins; the
    /// root holds everything and matches nothing.
    #[test]
    fn directories_match_by_their_trailing_segments() {
        assert_eq!(tails("a/b/c").collect::<Vec<_>>(), ["a/b/c", "b/c", "c"]);
        assert_eq!(tails(".").count(), 0);
        let dirs = [(0, "."), (1, "x/a"), (2, "x/a/b"), (3, "")];
        assert_eq!(deepest("a/b/c.rs", dirs.into_iter()), Some(2));
        assert_eq!(deepest("x/a/y.rs", dirs.into_iter()), Some(1));
        assert_eq!(deepest("ab/y.rs", dirs.into_iter()), None);
        assert_eq!(deepest("z.rs", dirs.into_iter()), None);
    }
}
