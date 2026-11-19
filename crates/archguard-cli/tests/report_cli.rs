//! `archguard report`: the shareable single-file report the desktop
//! writes with File → Export → Report….

use std::fs;
use std::path::Path;

use assert_cmd::Command;

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn fixture(dir: &Path) -> std::path::PathBuf {
    let root = dir.join("py-layers");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/py-layers"),
        &root,
    );
    root
}

fn archguard(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("archguard")
        .expect("binary")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("runs")
}

#[test]
fn report_prints_the_markdown_report() {
    let dir = tempfile::tempdir().unwrap();
    let root = fixture(dir.path());
    let output = archguard(&root, &["report", "."]);
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.starts_with("# ArchGuard report: py-layers\n"),
        "{text}"
    );
    for section in ["## Gate findings", "## Review priorities", "```mermaid"] {
        assert!(text.contains(section), "{section}: {text}");
    }
}

#[test]
fn report_out_writes_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = fixture(dir.path());
    let target = dir.path().join("share.md");
    let output = archguard(
        dir.path(),
        &[
            "report",
            root.to_str().unwrap(),
            "--out",
            target.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let text = fs::read_to_string(&target).expect("written");
    assert!(text.contains("## Diagrams"), "{text}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("share.md"), "{stdout}");
}
