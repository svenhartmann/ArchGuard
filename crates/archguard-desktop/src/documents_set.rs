//! The project set as a document source: member list, what `open` accepts
//! (ADR 0024).

use std::path::{Path, PathBuf};

use crate::documents::{DocId, SubProject, with_store};

const SET_SUFFIX: &str = ".archguard-set.toml";

/// Whether `path` is named like a project set file.
pub fn is_set_file(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(SET_SUFFIX))
}

/// What "Open" accepts: a folder, or an existing project set file.
pub fn openable(path: &Path) -> bool {
    path.is_dir() || (path.is_file() && is_set_file(path))
}

/// The members of a set as sub-projects (absolute paths), for the menu "Projects".
pub fn members(projects: &[PathBuf]) -> Vec<SubProject> {
    projects
        .iter()
        .map(|p| SubProject {
            name: p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            kind: "project".into(),
            path: p.display().to_string(),
        })
        .collect()
}

/// What `open` was given: a project folder, or a project set file with its
/// primary project.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    Folder(PathBuf),
    Set {
        file: PathBuf,
        primary: PathBuf,
        name: String,
    },
}

/// Tell a folder from a set file; a set file that cannot be read is the
/// error text shown to the user.
pub fn target(path: &Path) -> Result<Target, String> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !is_set_file(&path) {
        return Ok(Target::Folder(path));
    }
    let set = archguard_engine::read_project_set(&path).map_err(|e| e.to_string())?;
    let primary = set
        .projects
        .first()
        .cloned()
        .ok_or("the set has no project")?;
    Ok(Target::Set {
        file: path,
        primary,
        name: set.name,
    })
}

/// The active document's set file, if it opens a set.
pub fn active_set() -> Option<PathBuf> {
    with_store(|s| s.active()?.set.clone())
}

/// The set file of document `id`, if it opens a set.
pub fn active_set_of(id: DocId) -> Option<PathBuf> {
    with_store(|s| s.get(id)?.set.clone())
}

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
        assert!(!openable(
            &fixture("infra").join("nothing.archguard-set.toml")
        ));
        assert!(!openable(&fixture("infra").join("terraform/main.tf")));
    }
}
