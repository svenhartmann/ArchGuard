//! "Follow left" for the Quality tab ([`crate::follow`]): a package
//! entered becomes the scope, a file selected becomes the scope with the
//! card of its first class open; what the user picks here is reported
//! as the same.

use super::state::{PAGE, QualityState};
use crate::follow::{Nav, Place};
use crate::tab_support;
use crate::ui::AppWindow;

impl QualityState {
    /// Do what the left pane did: a package entered or a node selected.
    pub fn follow(&mut self, nav: &Nav) {
        match nav {
            Nav::Scope(within) => self.narrow(within.as_deref(), None),
            Nav::Select(name) => self.select_node(name),
            Nav::Expand { .. } => {}
        }
    }

    /// Scope the tab to `target` (a source node; `None` or unknown: the
    /// entire source scope) with class `class` open.
    fn narrow(&mut self, target: Option<&str>, class: Option<String>) {
        let v = &mut self.view;
        v.target = target
            .filter(|t| self.names.contains(*t))
            .unwrap_or_default()
            .to_string();
        v.query.clear();
        v.limit = PAGE;
        v.selected = class;
    }

    /// A file with classes: its scope and its first class, by id; a
    /// package: its scope.
    fn select_node(&mut self, name: &str) {
        let class = self.inputs.as_ref().and_then(|i| {
            i.code
                .classes
                .iter()
                .filter(|c| c.node == name || c.file == name)
                .map(|c| c.class_id.clone())
                .min()
        });
        if class.is_some() || self.names.contains(name) {
            self.narrow(Some(name), class);
        }
    }
}

/// The scope by node name (a file target counts as its package) and the
/// open class's node.
fn place(s: &QualityState) -> Place {
    let Some(inputs) = &s.inputs else {
        return Place::default();
    };
    let nodes = &inputs.source.nodes;
    let target = nodes.iter().find(|n| n.name == s.view.target);
    let scope = target.and_then(|n| match n.module {
        true => Some(n.name.clone()),
        false => n.parent.and_then(|p| nodes.get(p)).map(|p| p.name.clone()),
    });
    let selected = s.view.selected.as_deref().and_then(|id| {
        let c = inputs.code.classes.iter().find(|c| c.class_id == id)?;
        Some(c.node.clone())
    });
    Place { scope, selected }
}

/// A Quality callback that reports its scope and open class.
pub(super) fn navigation<A: 'static>(
    ui: &AppWindow,
    f: impl Fn(A) + 'static,
) -> impl Fn(A) + 'static {
    crate::follow_ui::reporting(ui, "quality", place, f)
}

/// The tab beside a leading view follows it, also while it still builds;
/// a pane beside it starts at its place (at start-up).
pub(super) fn wire() {
    crate::follow_ui::on_follow("quality", |ui, _, nav| {
        let nav = nav.clone();
        tab_support::update_or_queue(ui, move |s: &mut QualityState| s.follow(&nav));
    });
    crate::follow_ui::on_state_place("quality", place);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality_ui::inputs::Inputs;
    use crate::quality_ui::rows::NodeInfo;
    use crate::rating::tests::class;
    use crate::tab_support::Texts;

    /// Packages `a` and `b`, files `a/x.rs` (classes `X2`, `X1`) and
    /// `b/y.rs` (none).
    fn state() -> QualityState {
        let root = std::path::Path::new("/nonexistent-archguard-root");
        let graph = archguard_engine::core::GraphBuilder::new().build(root.display().to_string());
        let config = archguard_engine::core::Config::default();
        let sources = archguard_engine::core::ReviewSources::default();
        let calls = archguard_engine::analyze_calls(root, &config);
        let mut i = Inputs::compute(root, &graph, &config, &sources, &calls);
        let node = |name: &str, parent: Option<usize>, module| NodeInfo {
            name: name.into(),
            parent,
            module,
        };
        i.source.nodes = vec![
            node("a", None, true),
            node("a/x.rs", Some(0), false),
            node("b", None, true),
            node("b/y.rs", Some(2), false),
        ];
        i.code.classes = vec![
            class("rust:a/x.rs::X2", "a/x.rs", Some(1), None),
            class("rust:a/x.rs::X1", "a/x.rs", Some(2), None),
        ];
        QualityState::new(Texts::default(), i)
    }

    /// A package entered becomes the scope; leaving it, or an unknown
    /// node, the entire source scope.
    #[test]
    fn a_scope_narrows_the_tab() {
        let mut s = state();
        s.view.query = "old".into();
        s.view.limit = 3 * PAGE;
        s.follow(&Nav::Scope(Some("b".into())));
        assert_eq!(s.view.target, "b");
        assert_eq!((s.view.query.as_str(), s.view.limit), ("", PAGE));
        assert_eq!(place(&s).scope.as_deref(), Some("b"));
        s.follow(&Nav::Scope(None));
        assert_eq!(s.view.target, "");
        s.follow(&Nav::Scope(Some("no/such".into())));
        assert_eq!(s.view.target, "");
    }

    /// A file selected scopes to it and opens its first class, which the
    /// tab reports as its file in its package.
    #[test]
    fn a_selected_file_opens_its_class() {
        let mut s = state();
        s.follow(&Nav::Select("a/x.rs".into()));
        assert_eq!(s.view.target, "a/x.rs");
        assert_eq!(s.view.selected.as_deref(), Some("rust:a/x.rs::X1"));
        let page = s.page();
        assert_eq!(
            page.detail.as_ref().map(|c| c.title.as_str()),
            Some("rust:a/x.rs::X1")
        );
        assert_eq!(
            place(&s),
            Place {
                scope: Some("a".into()),
                selected: Some("a/x.rs".into()),
            }
        );
        s.follow(&Nav::Select("b/y.rs".into()));
        assert_eq!(
            (s.view.target.as_str(), s.view.selected.as_deref()),
            ("b/y.rs", None)
        );
        s.follow(&Nav::Select("no/such.rs".into()));
        assert_eq!(s.view.target, "b/y.rs", "unknown nodes are ignored");
    }
}
