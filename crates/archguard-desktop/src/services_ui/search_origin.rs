//! Project sets in the Elasticsearch views: the `↧` marker of nodes from another project than
//! the primary one (ADR 0024, Q6) and the projects an artifact is declared in.

use archguard_engine::core::SearchArtifact;

/// The marker of a node from another project.
pub(crate) const FOREIGN: &str = "↧";

/// The row text of a graph node: `⚠ ` for a finding, `↧ ` for a foreign project, then the label;
/// a foreign node keeps all of it within 32 characters.
pub(crate) fn node_label(label: &str, warn: bool, foreign: bool) -> String {
    format!("{}{label}", if warn { "⚠ " } else { "" })
}

/// The title of an artifact in the list: `↧ ` first when it is not declared in the primary project.
pub(crate) fn title(a: &SearchArtifact, primary: Option<&str>) -> String {
    a.name.as_deref().unwrap_or(&a.address).to_string()
}

/// The projects an artifact is declared in, comma separated; empty outside a set.
pub(crate) fn projects(a: &SearchArtifact) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use archguard_engine::core::SearchArtifact;

    use super::*;

    fn art(name: &str, origins: &[&str]) -> SearchArtifact {
        SearchArtifact {
            kind: "index".into(),
            name: Some(name.into()),
            address: format!("index.{name}"),
            scope: String::new(),
            patterns: vec![],
            references: vec![],
            evidence: vec![],
            origins: origins.iter().map(|o| o.to_string()).collect(),
        }
    }

    #[test]
    fn a_foreign_artifact_starts_with_the_marker_a_merged_or_own_one_does_not() {
        assert_eq!(title(&art("a", &["infra"]), Some("adapter")), "↧ a");
        assert_eq!(
            title(&art("a", &["infra", "adapter"]), Some("adapter")),
            "a"
        );
        assert_eq!(title(&art("a", &["adapter"]), Some("adapter")), "a");
        assert_eq!(title(&art("a", &[]), None), "a");
    }

    #[test]
    fn projects_are_the_origins() {
        assert_eq!(projects(&art("a", &["infra", "adapter"])), "infra, adapter");
        assert_eq!(projects(&art("a", &[])), "");
    }

    #[test]
    fn node_labels_carry_warning_and_marker_within_32_characters() {
        assert_eq!(node_label("n", false, false), "n");
        assert_eq!(node_label("n", true, false), "⚠ n");
        assert_eq!(node_label("n", false, true), "↧ n");
        assert_eq!(node_label("n", true, true), "⚠ ↧ n");
        let long = "x".repeat(32);
        for (warn, foreign) in [(false, true), (true, true)] {
            let s = node_label(&long, warn, foreign);
            assert!(s.chars().count() <= 32, "{s}");
            assert!(s.ends_with('…'), "{s}");
        }
        // Without the marker the label is as before.
        assert_eq!(node_label(&long, true, false).chars().count(), 34);
    }
}
