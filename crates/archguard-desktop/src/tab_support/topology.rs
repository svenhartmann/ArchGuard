//! The topology of a document: the one switch between a single project and a
//! project set (ADR 0024); `tab_support::topology` caches it per document.

use std::path::{Path, PathBuf};

use archguard_engine::core::ServiceTopology;
use archguard_engine::{analyze_service_set, analyze_services, read_project_set};

/// The topology of `root`; with a set file the combined picture of its
/// members (ES model only, the rest is the primary project's).
pub(super) fn read(root: &Path, set: Option<&Path>) -> Result<ServiceTopology, String> {
    let Some(set) = set else {
        return analyze_services(root).map_err(|e| e.to_string());
    };
    let members: Vec<PathBuf> = read_project_set(set).map_err(|e| e.to_string())?.projects;
    analyze_service_set(&members).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::read;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/es-set")
            .join(name)
    }

    fn set_file() -> PathBuf {
        fixture("search.archguard-set.toml")
    }

    #[test]
    fn a_set_shows_the_combined_elasticsearch_picture() {
        let t = read(&fixture("infra"), Some(&set_file()));
        let t = t.as_ref().expect("topology");
        assert_eq!(t.elasticsearch.projects, ["infra", "adapter"]);
        assert!(
            t.elasticsearch
                .findings
                .iter()
                .all(|f| f.kind != "undeclared_index")
        );
        let audit: Vec<_> = t
            .elasticsearch
            .artifacts
            .iter()
            .filter(|a| a.name.as_deref() == Some("audit"))
            .collect();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].origins, ["infra", "adapter"]);
    }

    #[test]
    fn the_elasticsearch_graph_is_built_from_the_set() {
        let t = read(&fixture("infra"), Some(&set_file()));
        let graph = archguard_engine::search_graph(t.as_ref().expect("topology"));
        assert!(graph.nodes.iter().any(|n| n.label == "audit"));
    }

    #[test]
    fn without_a_set_the_project_alone_is_read() {
        let t = read(&fixture("adapter"), None);
        let t = t.as_ref().expect("topology");
        assert!(t.elasticsearch.projects.is_empty());
        assert!(
            t.elasticsearch
                .findings
                .iter()
                .any(|f| f.kind == "undeclared_index")
        );
    }

    #[test]
    fn a_broken_set_file_is_the_slots_text() {
        let dir = tempfile::tempdir().expect("temp dir");
        let bad = dir.path().join("bad.archguard-set.toml");
        std::fs::write(&bad, "name = \"x\"\nprojects = [\"only-one\"]\n").expect("write");
        let t = read(dir.path(), Some(&bad));
        assert!(t.is_err(), "{t:?}");
    }
}
