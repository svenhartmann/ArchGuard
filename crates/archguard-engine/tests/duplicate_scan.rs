use archguard_engine::{DuplicateScanOptions, read_duplicates, scan_duplicates};
use std::fs;

fn function(name: &str) -> String {
    let body: String = (0..13)
        .map(|i| format!("  let x{i} = a + b * {i};\n"))
        .collect();
    format!("function {name}(a, b) {{\n{body}}}\n")
}

fn project(files: &[(&str, String)]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    for (name, text) in files {
        fs::write(root.path().join("src").join(name), text).unwrap();
    }
    fs::write(root.path().join("archguard.toml"),"derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['ts']\nexclude=[]\n[rules]\ndeny_cycles='off'\n").unwrap();
    root
}

#[test]
fn limits_are_partial_diagnostics_never_a_clean_empty_result() {
    let root = project(&[
        ("a.ts", function("f")),
        ("b.ts", function("f")),
        (
            "big.ts",
            format!("{}// {}\n", function("f"), "x".repeat(4096)),
        ),
    ]);
    let (_, run) = archguard_engine::generate_review(root.path(), None, None, 0).unwrap();
    let small = DuplicateScanOptions {
        max_file_bytes: 1024,
        ..DuplicateScanOptions::default()
    };
    let report = scan_duplicates(root.path(), &run.config, &run.graph, &small).unwrap();
    assert_eq!(report.status, "partial");
    assert_eq!(report.groups.len(), 1);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.contains("skipped src/big.ts")),
        "{:?}",
        report.diagnostics
    );
    let budget = DuplicateScanOptions {
        max_tokens: 200,
        replace: true,
        ..DuplicateScanOptions::default()
    };
    let report = scan_duplicates(root.path(), &run.config, &run.graph, &budget).unwrap();
    assert!(report.groups.is_empty());
    assert_eq!(report.status, "partial");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.contains("token budget"))
    );
    let read = read_duplicates(root.path(), &run.config, &run.graph);
    assert_eq!(read.status, "partial", "truncation survives the read path");
    assert_eq!(read.producer, "ArchGuard token scan");
}
