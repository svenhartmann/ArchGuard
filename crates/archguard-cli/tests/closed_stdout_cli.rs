//! `archguard check --format json | head -1`: a reader that closes the pipe
//! early must not crash the binary (panic, exit 101); the command keeps the
//! exit code it decided on.

use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};

use tempfile::tempdir;

#[test]
fn closed_stdout_pipe_keeps_the_commands_exit_code() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("pyproject.toml"),
        "[project]\nname = \"mypkg\"\n",
    )
    .expect("toml");
    fs::create_dir(dir.path().join("mypkg")).expect("mypkg");
    fs::write(dir.path().join("mypkg/__init__.py"), "").expect("init");
    fs::write(dir.path().join("mypkg/a.py"), "from mypkg import b\n").expect("a");
    fs::write(dir.path().join("mypkg/b.py"), "from mypkg import a\n").expect("b");

    let mut child = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["check", ".", "--format", "json"])
        .current_dir(dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    // Close the read end before the analysis prints anything.
    drop(child.stdout.take());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut stderr)
        .expect("stderr");
    let status = child.wait().expect("wait");
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert_eq!(status.code(), Some(1), "cycle found, stderr: {stderr}");
}
