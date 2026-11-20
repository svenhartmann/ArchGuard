//! The one `git` launcher for queries on an analysed repository. Its local
//! config is untrusted: no pager, filesystem monitor, signature program,
//! implicit bare repository or inherited repository selection. Diff and log
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
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_EXTERNAL_DIFF",
    ] {
        command.env_remove(key);
    }
    command
}
