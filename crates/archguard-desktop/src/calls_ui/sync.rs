//! The Calls window model across pushes: a new [`CallsModel`] keeps the
//! list models the view already shows, with only the changed rows set
//! (`model_sync`), so a click no longer rebuilds every tree row and card.

use slint::Model;

use crate::model_sync::{sync_rows, sync_rows_with};
use crate::ui::CallsModel;

/// `new` takes over the list models of `old`, synced to its contents.
pub fn keep(old: &CallsModel, new: &mut CallsModel) {
    new.languages = sync_rows(&old.languages, new.languages.iter());
    new.packages = sync_rows(&old.packages, new.packages.iter());
    new.diagnostics = sync_rows(&old.diagnostics, new.diagnostics.iter());
    new.analyzers = sync_rows_with(&old.analyzers, new.analyzers.iter(), |o, n| {
        n.diagnostics = sync_rows(&o.diagnostics, n.diagnostics.iter());
    });
    new.tree = sync_rows(&old.tree, new.tree.iter());
    new.relations = sync_rows(&old.relations, new.relations.iter());
    new.ranks = sync_rows(&old.ranks, new.ranks.iter());
    new.cycle_rows = sync_rows(&old.cycle_rows, new.cycle_rows.iter());
    new.path = sync_rows(&old.path, new.path.iter());
    new.members = sync_rows(&old.members, new.members.iter());
    new.edges = sync_rows(&old.edges, new.edges.iter());
    new.groups = sync_rows_with(&old.groups, new.groups.iter(), |o, n| {
        n.metrics = sync_rows(&o.metrics, n.metrics.iter());
        n.sites = sync_rows(&o.sites, n.sites.iter());
    });
    new.suggestions = sync_rows(&old.suggestions, new.suggestions.iter());
    let (o, n) = (&old.declaration, &mut new.declaration);
    n.groups = sync_rows_with(&o.groups, n.groups.iter(), |o, n| {
        n.methods = sync_rows(&o.methods, n.methods.iter());
        n.ids = sync_rows(&o.ids, n.ids.iter());
    });
}

#[cfg(test)]
mod tests {
    use slint::ModelRc;

    use super::super::actions;
    use super::super::model::fixture;
    use super::super::state::State;
    use super::super::view::{UiLabels, fill};
    use super::*;
    use crate::tab_support::Texts;

    fn labels() -> UiLabels {
        UiLabels {
            all_languages: "All".into(),
            all_packages: "All".into(),
            default_package: "".into(),
            external_group: "".into(),
            unresolved_group: "".into(),
            texts: Texts::default(),
        }
    }

    fn rows<T: Clone + 'static>(m: &ModelRc<T>) -> Vec<T> {
        m.iter().collect()
    }

    /// Focusing a class keeps the shown models and leaves them equal to
    /// a fresh fill of the new state.
    #[test]
    fn a_push_keeps_the_shown_models() {
        let mut s = State::new(fixture::analysis());
        s.refresh();
        let mut old = fill(&s, &labels());
        keep(&CallsModel::default(), &mut old);
        actions::focus_class(&mut s, "red.Red");
        s.refresh();
        let mut new = fill(&s, &labels());
        let fresh = fill(&s, &labels());
        keep(&old, &mut new);
        assert_eq!(new.tree, old.tree, "the same tree rows model");
        assert_eq!(new.relations, old.relations);
        assert_eq!(new.languages, old.languages);
        assert_eq!(new.packages, old.packages);
        assert_eq!(new.analyzers, old.analyzers);
        assert_eq!(new.diagnostics, old.diagnostics);
        assert_eq!(rows(&new.tree), rows(&fresh.tree));
        assert_ne!(rows(&old.tree).len(), 0, "the fixture has a tree");
        assert!(rows(&new.tree).iter().any(|r| r.pressed), "the focus shows");
        assert_eq!(rows(&new.relations), rows(&fresh.relations));
        assert_eq!(rows(&new.packages), rows(&fresh.packages));
    }
}
