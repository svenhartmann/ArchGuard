//! "Follow left" for the Calls tab ([`crate::follow`]): a package entered
//! picks the call package of the classes in it, a file selected focuses
//! its class; the package and the element picked here are reported as
//! the directory of the package's classes and the class's file.

use std::collections::BTreeMap;

use super::actions;
use super::model::ElementKind;
use super::state::State;
use crate::follow::{self, Nav, Place};
use crate::tab_support;
use crate::ui::AppWindow;

/// Do what the left pane did: a package entered or a node selected.
pub fn follow(s: &mut State, nav: &Nav) {
    match nav {
        Nav::Scope(None) => actions::set_package(s, None),
        Nav::Scope(Some(dir)) => scope_to(s, dir),
        Nav::Select(name) => match class_in(s, name) {
            Some(id) => {
                actions::focus_class(s, &id);
            }
            None => scope_to(s, name),
        },
        Nav::Expand { .. } => {}
    }
}

/// The first class declared in file `node`, by id.
fn class_in(s: &State, node: &str) -> Option<String> {
    let classes = s.idx.a.classes.iter();
    classes
        .filter(|c| c.graph_node == node)
        .map(|c| c.id.clone())
        .min()
}

/// The call package most classes below `dir` belong to, with its
/// language, as the package filter.
fn scope_to(s: &mut State, dir: &str) {
    let mut count: BTreeMap<(String, &str), usize> = BTreeMap::new();
    for c in s.idx.a.classes.iter() {
        if follow::within(&c.graph_node, dir) {
            *count.entry((s.idx.language(c), &c.package)).or_default() += 1;
        }
    }
    let Some(((language, package), _)) = count.into_iter().max_by_key(|&(_, n)| n) else {
        return;
    };
    let package = package.to_string();
    if s.filter.language.as_ref().is_some_and(|l| *l != language) {
        actions::set_language(s, Some(language).filter(|l| !l.is_empty()));
    }
    actions::set_package(s, Some(package));
}

/// The package filter as the directory its classes share, and the
/// element's class file.
fn place(s: &State) -> Place {
    let idx = &s.idx;
    let class = s.element.as_ref().and_then(|e| match e.kind {
        ElementKind::Class => idx.class(Some(&e.id)),
        ElementKind::Method => idx.class(idx.method(Some(&e.id)).map(|m| m.class_id.as_str())),
    });
    let scope = s.filter.package.as_deref().and_then(|p| {
        let files = idx.a.classes.iter().filter(|c| c.package == p);
        common_dir(files.map(|c| c.graph_node.as_str()))
    });
    Place {
        scope,
        selected: class.map(|c| c.graph_node.clone()),
    }
}

/// The longest directory all `files` lie in, if any.
fn common_dir<'a>(files: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut common: Option<Vec<&str>> = None;
    for f in files {
        let dir: Vec<&str> = f.split('/').collect();
        let dir = &dir[..dir.len().saturating_sub(1)];
        common = Some(match common {
            None => dir.to_vec(),
            Some(c) => c
                .iter()
                .zip(dir)
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| *a)
                .collect(),
        });
    }
    common.filter(|c| !c.is_empty()).map(|c| c.join("/"))
}

/// A Calls callback that reports its package and element.
pub(super) fn navigation<A: 'static>(
    ui: &AppWindow,
    f: impl Fn(A) + 'static,
) -> impl Fn(A) + 'static {
    crate::follow_ui::reporting(ui, "calls", place, f)
}

/// The tab beside a leading view follows it, also while it still builds;
/// a pane beside it starts at its place (at start-up).
pub(super) fn wire() {
    crate::follow_ui::on_follow("calls", |ui, _, nav| {
        let nav = nav.clone();
        tab_support::update_or_queue(ui, move |s: &mut State| follow(s, &nav));
    });
    crate::follow_ui::on_state_place("calls", place);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calls_ui::model::{Element, fixture};

    /// A file selected focuses its class; a directory entered picks the
    /// call package of its classes; leaving it, all packages.
    #[test]
    fn calls_follow_files_and_directories() {
        let mut s = State::new(fixture::analysis());
        follow(&mut s, &Nav::Select("red/Red.java".into()));
        s.refresh();
        assert_eq!(s.filter.package.as_deref(), Some("red"));
        assert_eq!(
            s.element,
            Some(Element {
                kind: ElementKind::Class,
                id: "red.Red".into()
            })
        );
        follow(&mut s, &Nav::Scope(Some("blue".into())));
        assert_eq!(
            (s.filter.package.as_deref(), &s.element),
            (Some("blue"), &None)
        );
        follow(&mut s, &Nav::Select("green".into()));
        assert_eq!(s.filter.package.as_deref(), Some("green"));
        follow(&mut s, &Nav::Scope(Some("no/such".into())));
        assert_eq!(s.filter.package.as_deref(), Some("green"), "unknown: kept");
        follow(&mut s, &Nav::Scope(None));
        assert_eq!(s.filter.package, None);
    }

    /// The tab reports its package as its classes' directory and its
    /// element (a class, or a method's class) as the class's file.
    #[test]
    fn calls_report_the_package_directory_and_the_class_file() {
        let mut s = State::new(fixture::analysis());
        actions::set_package(&mut s, Some("blue".into()));
        actions::select_element(&mut s, ElementKind::Method, "blue.Util#hint()");
        assert_eq!(
            place(&s),
            Place {
                scope: Some("blue".into()),
                selected: Some("blue/Util.java".into()),
            }
        );
        assert_eq!(
            common_dir(["a/b/c.rs", "a/b/d/e.rs"].into_iter()).as_deref(),
            Some("a/b")
        );
        assert_eq!(common_dir(["x.rs", "a/y.rs"].into_iter()), None);
        assert_eq!(common_dir(std::iter::empty()), None);
    }
}
