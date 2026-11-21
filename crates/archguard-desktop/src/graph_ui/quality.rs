//! Imported quality findings on the Graph: the tree rows' "Q" count badges
//! and the inspector section of the selected node (the web viewer's
//! `qualityNodeCount` and `renderQualityInspector`).

use std::collections::{BTreeMap, BTreeSet};

use archguard_engine::{QualityFinding, QualityReport};
use slint::ModelRc;

use crate::model_sync::sync_rows_with;
use crate::ui::QualityCard;

use crate::quality_ui::findings;
use crate::tab_support::Texts;
use crate::tree::Tree;

/// Findings the inspector shows as cards; the rest are counted.
const SHOWN: usize = 10;

#[derive(Default)]
pub struct Findings {
    report: Option<QualityReport>,
    /// Per tree node: active findings in its file or subtree.
    counts: Vec<i32>,
    /// Per tree node: indices into the report's sorted findings mapped to it.
    own: BTreeMap<usize, Vec<usize>>,
}

impl Findings {
    /// `report` placed on `tree` by node name.
    pub fn new(report: QualityReport, tree: &Tree) -> Self {
        let mut s = Findings {
            report: Some(report),
            ..Findings::default()
        };
        s.retree(tree);
        s
    }

    /// Whether the report has arrived (it is read on a worker).
    pub fn loaded(&self) -> bool {
        self.report.is_some()
    }

    /// Place the findings on `tree` again (a simulation rebuilt it).
    pub fn retree(&mut self, tree: &Tree) {
        self.counts = vec![0; tree.nodes.len()];
        self.own.clear();
        let Some(report) = &self.report else { return };
        let by_name: BTreeMap<&str, usize> = tree
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.name.as_str(), i))
            .collect();
        for (i, f) in findings::sorted(report).into_iter().enumerate() {
            let Some(&node) = f.node.as_deref().and_then(|n| by_name.get(n)) else {
                continue;
            };
            self.own.entry(node).or_default().push(i);
            if findings::active(report, f) {
                let mut at = Some(node);
                while let Some(n) = at {
                    self.counts[n] += 1;
                    at = tree.nodes[n].parent;
                }
            }
        }
    }

    /// The badge count of tree node `node`.
    pub fn count(&self, node: usize) -> i32 {
        self.counts.get(node).copied().unwrap_or(0)
    }

    /// All findings mapped to `node` or below it, in the report's order.
    fn below(&self, tree: &Tree, node: usize) -> Vec<&QualityFinding> {
        let Some(report) = &self.report else {
            return Vec::new();
        };
        let mut at = vec![node];
        let mut picked = BTreeSet::new();
        while let Some(n) = at.pop() {
            picked.extend(self.own.get(&n).into_iter().flatten().copied());
            at.extend(&tree.nodes[n].children);
        }
        let sorted = findings::sorted(report);
        picked.into_iter().map(|i| sorted[i]).collect()
    }

    /// The inspector's cards for `node` (the first [`SHOWN`]) and how many
    /// findings there are.
    pub fn inspector(
        &self,
        lang: &Texts,
        tree: &Tree,
        node: Option<usize>,
    ) -> (Vec<crate::quality_ui::card::Card>, usize) {
        let rows = node.map(|n| self.below(tree, n)).unwrap_or_default();
        let none = BTreeSet::new();
        let cards = rows
            .iter()
            .take(SHOWN)
            .map(|f| findings::card(lang, &none, f, false))
            .collect();
        (cards, rows.len())
    }

    /// [`Self::inspector`] in the models of `old`, changed in place.
    pub fn data(
        &self,
        lang: &Texts,
        tree: &Tree,
        node: Option<usize>,
        old: &ModelRc<QualityCard>,
    ) -> (ModelRc<QualityCard>, i32) {
        let (cards, total) = self.inspector(lang, tree, node);
        let cards = cards.into_iter().map(crate::quality_ui::card_model);
        let models = sync_rows_with(old, cards, crate::quality_ui::adopt_card);
        (models, total as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality_ui::findings::tests::imported_fixture;

    #[test]
    fn findings_are_loaded_once_the_report_is_placed() {
        assert!(!Findings::default().loaded());
        let (root, run) = imported_fixture("graph-loaded");
        let report = archguard_engine::read_quality_findings(
            &root,
            &run.config,
            (&run.graph, &run.source_names),
        );
        let tree = Tree::from_graph(&run.graph);
        assert!(Findings::new(report, &tree).loaded());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The file and its packages count the two active findings; the
    /// unmapped one counts nowhere; the inspector lists the file's.
    #[test]
    fn rows_count_active_findings_of_their_subtree() {
        let (root, run) = imported_fixture("graph");
        let report = archguard_engine::read_quality_findings(
            &root,
            &run.config,
            (&run.graph, &run.source_names),
        );
        let tree = Tree::from_graph(&run.graph);
        let f = Findings::new(report, &tree);
        let file = tree
            .nodes
            .iter()
            .position(|n| n.name.ends_with("service.py"))
            .expect("service.py");
        assert_eq!(f.count(file), 2);
        let parent = tree.nodes[file].parent.expect("package");
        assert_eq!(f.count(parent), 2);
        let other = tree
            .nodes
            .iter()
            .position(|n| n.name.ends_with("order.py"))
            .expect("order.py");
        assert_eq!(f.count(other), 0);
        let (cards, total) = f.inspector(&Texts::default(), &tree, Some(parent));
        assert_eq!(total, 2);
        assert_eq!(cards[0].title, "E1 · Lint · error");
        let (cards, total) = f.inspector(&Texts::default(), &tree, Some(other));
        assert_eq!((cards.len(), total), (0, 0));
        let _ = std::fs::remove_dir_all(&root);
    }
}
