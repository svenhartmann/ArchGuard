//! Project sets in the Elasticsearch views: the `[project]` prefix of nodes from another project than
//! the primary one (ADR 0024, Q6) and the projects an artifact is declared in.

use archguard_engine::core::SearchArtifact;

/// The most characters of a node's row text.
const MAX: usize = 32;

/// The row text of a graph node: `⚠ ` for a finding, `[project] ` for a project other than the
/// primary one, then the label; the label is shortened so all of it fits in 32 characters.
pub(crate) fn node_label(label: &str, warn: bool, project: Option<&str>) -> String {
    let prefix = format!(
        "{}{}",
        if warn { "⚠ " } else { "" },
        project.map_or(String::new(), |p| format!("[{p}] "))
    );
    let room = MAX.saturating_sub(prefix.chars().count());
    if project.is_none() || label.chars().count() <= room {
        return format!("{prefix}{label}");
    }
    let cut: String = label.chars().take(room.saturating_sub(1)).collect();
    format!("{prefix}{cut}…")
}

/// The title of an artifact in the list: `[project] ` first when it is not declared in the primary project.
pub(crate) fn title(a: &SearchArtifact, primary: Option<&str>) -> String {
    let name = a.name.as_deref().unwrap_or(&a.address);
    let foreign = primary.is_some_and(|p| a.foreign(p));
    node_label(
        name,
        false,
        foreign.then(|| a.origins.join(", ")).as_deref(),
    )
}

/// The projects an artifact is declared in, comma separated; empty outside a set.
pub(crate) fn projects(a: &SearchArtifact) -> String {
    a.origins.join(", ")
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
    fn a_foreign_artifact_starts_with_its_project_a_merged_or_own_one_does_not() {
        assert_eq!(title(&art("a", &["infra"]), Some("adapter")), "[infra] a");
        assert_eq!(title(&art("a", &["x", "y"]), Some("adapter")), "[x, y] a");
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
    fn node_labels_carry_warning_and_project_within_32_characters() {
        assert_eq!(node_label("n", false, None), "n");
        assert_eq!(node_label("n", true, None), "⚠ n");
        assert_eq!(node_label("n", false, Some("infra")), "[infra] n");
        assert_eq!(node_label("n", true, Some("infra")), "⚠ [infra] n");
        let long = "x".repeat(32);
        let s = node_label(&long, false, Some("infra"));
        assert_eq!(s, format!("[infra] {}…", "x".repeat(23)));
        assert_eq!(s.chars().count(), 32);
        let s = node_label(&long, true, Some("infra"));
        assert_eq!(s, format!("⚠ [infra] {}…", "x".repeat(21)));
        assert_eq!(s.chars().count(), 32);
        // Without a project the label is as before.
        assert_eq!(node_label(&long, true, None).chars().count(), 34);
    }
}
