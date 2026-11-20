//! The comparison on a Git repository made in the test: `main` with three
//! packages, `feature` with a new package and an import that closes a
//! cycle between `core` and `ui`.

use std::fs;
use std::path::Path;
use std::process::Command;

use super::diff::{Comparison, Cycle, Status};
use super::git::{Target, preselect, refs, run};

pub(crate) fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(root: &Path, file: &str, text: &str) {
    let path = root.join(file);
    fs::create_dir_all(path.parent().expect("a parent")).expect("dir");
    fs::write(path, text).expect("written");
}

/// `main`: ui → core, util alone. `feature`: core → ui (a cycle) and a
/// new package extra → core. Checked out: `main`.
pub(crate) fn repo(root: &Path) {
    git(root, &["init", "-q", "--template=", "-b", "main"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    write(
        root,
        ".archguard/archguard.toml",
        "derive_layers=false\n[project]\nlanguages=['ts']\n",
    );
    write(root, "core/a.ts", "export const a = 1;\n");
    write(root, "ui/v.ts", "import '../core/a';\n");
    write(root, "util/u.ts", "export const u = 1;\n");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    git(root, &["switch", "-qc", "feature"]);
    write(
        root,
        "core/a.ts",
        "import '../ui/v';\nexport const a = 1;\n",
    );
    write(root, "extra/e.ts", "import '../core/a';\n");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "feature"]);
    git(root, &["switch", "-q", "main"]);
}

fn compared(from: &str, to: Target) -> (tempfile::TempDir, Comparison) {
    let dir = tempfile::tempdir().expect("temp dir");
    repo(dir.path());
    let c = run(dir.path(), from, &to).expect("compared");
    (dir, c)
}

fn row<'a>(c: &'a Comparison, name: &str) -> &'a super::diff::Row {
    c.rows
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("row {name} in {:?}", c.rows))
}

fn relation(c: &Comparison, from: &str, to: &str) -> (Status, Cycle) {
    let r = c
        .relations
        .iter()
        .find(|r| r.from == from && r.to == to)
        .expect("relation");
    (r.status, r.cycle)
}

#[test]
fn branches_mark_packages_files_relations_and_new_cycles() {
    let (_dir, c) = compared("main", Target::Ref("feature".into()));
    assert!(!c.empty);
    assert_eq!((c.from.as_str(), c.to.as_str()), ("main", "feature"));
    assert_eq!(row(&c, "extra").status, Status::Added);
    assert_eq!(
        (row(&c, "core").status, row(&c, "core").cycle),
        (Status::Changed, Cycle::New)
    );
    assert_eq!(row(&c, "ui").cycle, Cycle::New);
    assert_eq!(
        (row(&c, "util").status, row(&c, "util").cycle),
        (Status::Unchanged, Cycle::None)
    );
    let file = c
        .rows
        .iter()
        .position(|r| r.name == "core/a.ts")
        .expect("changed file row");
    assert!(c.rows[file].file);
    assert_eq!(c.rows[file - 1].name, "core", "a file follows its package");
    assert_eq!(relation(&c, "core", "ui"), (Status::Added, Cycle::New));
    assert_eq!(relation(&c, "ui", "core"), (Status::Unchanged, Cycle::New));
    assert_eq!(relation(&c, "extra", "core").0, Status::Added);
    let counts = c.counts();
    assert_eq!(
        (counts.packages_added, counts.added, counts.removed),
        (1, 2, 0)
    );
    assert_eq!((counts.cycles_new, counts.cycles_removed), (2, 0));
}

#[test]
fn reversed_branches_remove_the_cycle() {
    let (_dir, c) = compared("feature", Target::Ref("main".into()));
    assert_eq!(row(&c, "extra").status, Status::Removed);
    assert_eq!(
        relation(&c, "core", "ui"),
        (Status::Removed, Cycle::Removed)
    );
    assert_eq!(row(&c, "core").cycle, Cycle::Removed);
    let counts = c.counts();
    assert_eq!(
        (
            counts.packages_removed,
            counts.removed,
            counts.cycles_removed
        ),
        (1, 2, 2)
    );
}

#[test]
fn a_selection_lists_its_relations_changes_first_with_evidence() {
    let (_dir, c) = compared("main", Target::Ref("feature".into()));
    let core = c.rows.iter().position(|r| r.name == "core").expect("core");
    let listed = c.relations_of(core);
    assert_eq!(listed.len(), 3, "core → ui, extra → core, ui → core");
    assert_eq!(listed[0].status, Status::Added);
    assert_eq!(listed.last().map(|r| r.status), Some(Status::Unchanged));
    let new = listed.iter().find(|r| r.to == "ui").expect("core → ui");
    assert_eq!(
        (new.evidence[0].file.as_str(), new.evidence[0].line),
        ("core/a.ts", 1)
    );
    let file = c
        .rows
        .iter()
        .position(|r| r.name == "core/a.ts")
        .expect("file");
    let of_file = c.relations_of(file);
    assert!(
        of_file.iter().all(|r| r.status != Status::Unchanged),
        "{of_file:?}"
    );
    assert!(
        of_file
            .iter()
            .any(|r| r.from == "core/a.ts" && r.to == "ui/v.ts")
    );
}

#[test]
fn the_index_is_compared_with_a_branch() {
    let dir = tempfile::tempdir().expect("temp dir");
    repo(dir.path());
    write(dir.path(), "util/u.ts", "import '../core/a';\n");
    git(dir.path(), &["add", "util/u.ts"]);
    let c = run(dir.path(), "main", &Target::Staged).expect("compared");
    assert_eq!(c.to, "", "the view names the index");
    assert_eq!(relation(&c, "util", "core"), (Status::Added, Cycle::None));
    assert_eq!(row(&c, "util").status, Status::Changed);
}

#[test]
fn the_same_ref_on_both_sides_is_empty() {
    let (_dir, c) = compared("main", Target::Ref("main".into()));
    assert!(c.empty);
    assert!(
        c.rows
            .iter()
            .all(|r| r.status == Status::Unchanged && !r.file)
    );
    assert_eq!(c.counts(), Default::default());
}

#[test]
fn refs_list_local_branches_and_preselect_main_and_the_current_branch() {
    let dir = tempfile::tempdir().expect("temp dir");
    repo(dir.path());
    git(dir.path(), &["switch", "-q", "feature"]);
    let r = refs(dir.path()).expect("refs");
    assert_eq!(r.branches, ["feature", "main"]);
    assert_eq!((r.from, r.to), (1, Some(0)));
    assert!(refs(&dir.path().join("missing")).is_err());
}

#[test]
fn preselection_falls_back_to_master_and_the_index() {
    let names = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let on_main = preselect(names(&["dev", "main"]), Some("main"));
    assert_eq!(
        (on_main.from, on_main.to),
        (1, None),
        "main against its index"
    );
    let master = preselect(names(&["dev", "master"]), Some("dev"));
    assert_eq!((master.from, master.to), (1, Some(0)));
    let detached = preselect(names(&["dev"]), None);
    assert_eq!((detached.from, detached.to), (0, None));
}
