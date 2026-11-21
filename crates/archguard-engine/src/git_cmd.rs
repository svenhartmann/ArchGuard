//! The one `git` launcher for queries on an analysed repository. Its local
//! config is untrusted: no pager, filesystem monitor, signature program,
//! implicit bare repository or inherited repository selection (an
//! operator's index file and GIT_CONFIG_* stay). Diff and log
//! callers still pass `--no-ext-diff --no-textconv`; callers that read
//! working-tree content add `quality_gate::filter_driver_overrides`.
use std::path::Path;
use std::process::{Command, Stdio};

pub(crate) fn git(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(["--no-pager", "--no-optional-locks"])
        .args(
            [
                "core.fsmonitor=false",
                "log.showSignature=false",
                "gpg.program=false",
                "gpg.openpgp.program=false",
                "gpg.x509.program=false",
                "gpg.ssh.program=false",
                // A committed directory holding HEAD/objects/config must not be discovered as a repository.
                "safe.bareRepository=explicit",
            ]
            .iter()
            .flat_map(|value| ["-c", value]),
        )
        .stdin(Stdio::null());
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_EXTERNAL_DIFF",
    ] {
        command.env_remove(key);
    }
    command
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    #[test]
    fn operator_index_and_config_variables_reach_git_but_repository_selection_does_not() {
        let command = super::git(std::path::Path::new("."));
        let removed: Vec<&OsStr> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key)
            .collect();
        // A pre-commit hook's temporary index and CI's GIT_CONFIG_* safe.directory must stay.
        for kept in ["GIT_INDEX_FILE", "GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS"] {
            assert!(!removed.contains(&OsStr::new(kept)), "{kept} removed");
        }
        assert!(removed.contains(&OsStr::new("GIT_DIR")));
    }
}
