use archguard_engine::{Level, analyze_run_quiet, compare_to_ref, load_config};
use std::fs;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init(root: &Path) {
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Comparison Test"]);
    git(
        root,
        &["config", "user.email", "comparison@example.invalid"],
    );
    git(root, &["config", "commit.gpgsign", "false"]);
}

fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn project(root: &Path, ownership: bool) {
    write(root, "src/ui/View.ts", "import '../data/Db';\n");
    write(root, "src/data/Db.ts", "export const value=1;\n");
    let config = "derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['ts']\nexclude=[]\n[modules]\ndepth=0\n[rules]\ndeny_cycles='off'\nmax_scc_size=100\n";
    let ownership = if ownership {
        "\n[[ownership_rules]]\nfrom='@org/ui'\nto='@org/data'\nseverity='error'\n"
    } else {
        ""
    };
    write(root, "archguard.toml", &format!("{config}{ownership}"));
}

fn commit(root: &Path) {
    git(root, &["add", "--force", "."]);
    git(root, &["commit", "--quiet", "-m", "base"]);
}

#[test]
fn nested_project_keeps_repository_codeowners_and_path_prefix_in_base() {
    let repo = tempfile::tempdir().unwrap();
    init(repo.path());
    let root = repo.path().join("project");
    project(&root, true);
    write(
        repo.path(),
        ".github/CODEOWNERS",
        "/project/src/ui/ @org/ui\n/project/src/data/ @org/data\n",
    );
    commit(repo.path());
    let run = analyze_run_quiet(&root, &load_config(None, &root).unwrap(), Level::Module).unwrap();
    assert!(
        run.report
            .findings
            .iter()
            .any(|finding| finding.id.starts_with("ownership-"))
    );
    let comparison = compare_to_ref(
        &root,
        &run.effective_config(),
        &run.graph,
        &run.report,
        "HEAD",
    )
    .expect("a nested project's base must retain the same repository CODEOWNERS context");
    assert!(comparison.new_findings.is_empty());
    assert!(comparison.resolved_findings.is_empty());
    assert_eq!(comparison.unchanged_findings, run.report.findings.len());
    assert_eq!(comparison.acd_delta, 0.0);
}

#[test]
fn nested_project_keeps_ancestor_gitignore_scope_in_base() {
    let repo = tempfile::tempdir().unwrap();
    init(repo.path());
    let root = repo.path().join("project");
    project(&root, false);
    write(&root, "src/generated/a.ts", "import './b';\n");
    write(&root, "src/generated/b.ts", "import './a';\n");
    write(repo.path(), ".gitignore", "/project/src/generated/\n");
    commit(repo.path());
    let run = analyze_run_quiet(&root, &load_config(None, &root).unwrap(), Level::Module).unwrap();
    assert!(
        !run.graph
            .nodes
            .iter()
            .any(|node| node.name.contains("generated"))
    );
    let comparison = compare_to_ref(
        &root,
        &run.effective_config(),
        &run.graph,
        &run.report,
        "HEAD",
    )
    .unwrap();
    assert_eq!(
        comparison.base_summary.nodes, comparison.current_summary.nodes,
        "the same revision must not gain nodes because the temporary base lost ancestor ignore rules"
    );
    assert_eq!(comparison.acd_delta, 0.0);
    assert!(comparison.resolved_findings.is_empty());
}

#[test]
fn repository_directory_whitespace_is_not_trimmed_from_git_output() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join(" repository ");
    fs::create_dir(&root).unwrap();
    init(&root);
    project(&root, false);
    commit(&root);
    let run = analyze_run_quiet(&root, &load_config(None, &root).unwrap(), Level::Module).unwrap();
    let comparison = compare_to_ref(
        &root,
        &run.effective_config(),
        &run.graph,
        &run.report,
        "HEAD",
    )
    .expect("only Git's output line ending may be removed, not path whitespace");
    assert_eq!(comparison.acd_delta, 0.0);
}
