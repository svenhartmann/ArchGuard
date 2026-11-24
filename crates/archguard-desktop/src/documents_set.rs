//! The project set as a document source: member list, what `open` accepts
//! (ADR 0024). Tests only for now.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{is_set_file, members, openable};
    use crate::documents::{Store, project_root};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/es-set")
            .join(name)
            .canonicalize()
            .expect("fixture")
    }

    fn set_file() -> PathBuf {
        fixture("search.archguard-set.toml")
    }

    #[test]
    fn members_become_projects_and_open_as_single_documents() {
        let (set, infra, adapter) = (set_file(), fixture("infra"), fixture("adapter"));
        let list = archguard_engine::read_project_set(&set).expect("set");
        let projects = members(&list.projects);
        let rows: Vec<(&str, &str)> = projects
            .iter()
            .map(|p| (p.name.as_str(), p.kind.as_str()))
            .collect();
        assert_eq!(rows, [("infra", "project"), ("adapter", "project")]);
        assert_eq!(project_root(&infra, &projects[1]), adapter);
        let mut s = Store::default();
        s.open_set(&set, &infra, "search");
        let (id, new) = s.open(&project_root(&infra, &projects[1]));
        assert!(new);
        assert_eq!(s.get(id).expect("member").set, None);
    }

    #[test]
    fn only_folders_and_set_files_can_be_opened() {
        assert!(is_set_file(&set_file()));
        assert!(!is_set_file(&fixture("infra")));
        assert!(openable(&set_file()) && openable(&fixture("infra")));
        assert!(!openable(&fixture("infra").join("nothing.archguard-set.toml")));
        assert!(!openable(&fixture("infra").join("terraform/main.tf")));
    }
}
