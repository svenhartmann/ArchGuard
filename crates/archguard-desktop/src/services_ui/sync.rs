//! The Services window model across pushes: a new [`ServicesModel`] keeps
//! the list models the view already shows, with only the changed rows set
//! (`model_sync`), so a click no longer rebuilds every tree row and arc.

use slint::Model;

use crate::model_sync::sync_rows;
use crate::ui::{ServicesModel, SvcInspector};

/// `new` takes over the list models of `old`, synced to its contents.
pub fn keep(old: &ServicesModel, new: &mut ServicesModel) {
    new.project_list = sync_rows(&old.project_list, new.project_list.iter());
    new.diagnostics = sync_rows(&old.diagnostics, new.diagnostics.iter());
    new.cycles = sync_rows(&old.cycles, new.cycles.iter());
    new.rows = sync_rows(&old.rows, new.rows.iter());
    new.edges = sync_rows(&old.edges, new.edges.iter());
    keep_inspector(&old.inspector, &mut new.inspector);
}

fn keep_inspector(old: &SvcInspector, new: &mut SvcInspector) {
    new.project_evidence = sync_rows(&old.project_evidence, new.project_evidence.iter());
    new.evidence = sync_rows(&old.evidence, new.evidence.iter());
    new.links = sync_rows(&old.links, new.links.iter());
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use slint::ModelRc;

    use super::super::state::{ServicesState, build_from};
    use super::super::view::build;
    use super::*;

    fn shop() -> ServicesState {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/service-shop");
        let mut s = build_from(std::sync::Arc::new(
            archguard_engine::analyze_services(&root).map_err(|e| e.to_string()),
        ));
        s.viewport = 1480.0;
        s
    }

    fn rows<T: Clone + 'static>(m: &ModelRc<T>) -> Vec<T> {
        m.iter().collect()
    }

    /// Selecting a service keeps the shown models and leaves them equal
    /// to a fresh build of the new state.
    #[test]
    fn a_push_keeps_the_shown_models() {
        let mut s = shop();
        let old = build(&s, "");
        s.select_service(1);
        let mut new = build(&s, "");
        let fresh = build(&s, "");
        keep(&old, &mut new);
        assert_eq!(new.rows, old.rows, "the same tree rows model");
        assert_eq!(new.edges, old.edges, "the same arcs model");
        assert_eq!(new.project_list, old.project_list);
        assert_eq!(new.cycles, old.cycles);
        assert_eq!(new.diagnostics, old.diagnostics);
        assert_eq!(rows(&new.rows), rows(&fresh.rows));
        assert_eq!(rows(&new.edges), rows(&fresh.edges));
        assert_eq!(rows(&new.project_list), rows(&fresh.project_list));
        assert!(new.edges.row_count() < 15, "focused on the selection");
    }

    /// The details panel keeps its lists too, also those it left empty
    /// (after the first push they are list models as well).
    #[test]
    fn the_inspector_keeps_its_lists() {
        let mut s = shop();
        s.select_service(1);
        let mut old = build(&s, "").inspector;
        keep_inspector(&SvcInspector::default(), &mut old);
        s.select_service(2);
        let mut new = build(&s, "").inspector;
        let fresh = build(&s, "").inspector;
        keep_inspector(&old, &mut new);
        assert_eq!(new.links, old.links);
        assert_eq!(new.project_evidence, old.project_evidence);
        assert_eq!(new.evidence, old.evidence);
        assert_eq!(rows(&new.links), rows(&fresh.links));
    }
}
