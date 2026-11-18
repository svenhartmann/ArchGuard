//! `graph --format structurizr` and `doc --arc42 --format asciidoc|docx` (ADR 0013).
use std::{fs, path::Path, process::Command};

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write(
        "compose.yaml",
        "services:\n  orders:\n    build: ./orders\n    environment:\n      BILLING_API: http://billing:8080/api\n  billing:\n    build: ./billing\n",
    );
    write(
        "orders/api/handler.ts",
        "import { total } from '../domain/order';\nexport const h = total;\n",
    );
    write("orders/domain/order.ts", "export const total = 1;\n");
    write("billing/app.ts", "export const b = 2;\n");
    dir
}

fn archguard(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(args)
        .arg(root)
        .output()
        .unwrap()
}

fn ok(output: &std::process::Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn graph_exports_the_service_topology_as_structurizr_dsl() {
    let dir = project();
    let dsl = ok(&archguard(
        dir.path(),
        &["graph", "--format", "structurizr", "--service", "orders"],
    ));
    assert!(dsl.starts_with("workspace "), "{dsl}");
    assert!(dsl.contains("= container \"orders\""), "{dsl}");
    assert!(dsl.contains("\"HTTP, configured\" \"HTTP\""), "{dsl}");
    assert!(dsl.contains("= component \"api\""), "{dsl}");
    assert_eq!(
        fs::read_to_string(dir.path().join(".archguard/reports/graph.dsl")).unwrap(),
        dsl.trim_end_matches('\n').to_owned() + "\n"
    );
}

#[test]
fn structurizr_rejects_unknown_services_and_foreign_flags() {
    let dir = project();
    let unknown = archguard(
        dir.path(),
        &["graph", "--format", "structurizr", "--service", "nope"],
    );
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown service 'nope'"));
    let focus = archguard(
        dir.path(),
        &["graph", "--format", "structurizr", "--focus", "api"],
    );
    assert_eq!(focus.status.code(), Some(2));
    let service = archguard(dir.path(), &["graph", "--service", "orders"]);
    assert_eq!(service.status.code(), Some(2));
}

#[test]
fn arc42_exports_asciidoc_with_own_text_from_arc42_md() {
    let dir = project();
    ok(&archguard(dir.path(), &["doc", "--arc42"]));
    let md = dir.path().join("arc42.md");
    let saved = fs::read_to_string(&md).unwrap();
    fs::write(
        &md,
        saved.replace(
            "<!-- Add your own explanations for chapter 1 here. -->",
            "Our own goal text.",
        ),
    )
    .unwrap();
    ok(&archguard(
        dir.path(),
        &["doc", "--arc42", "--format", "asciidoc"],
    ));
    let adoc = fs::read_to_string(dir.path().join("arc42.adoc")).unwrap();
    assert!(adoc.starts_with("= arc42"), "{adoc}");
    assert!(adoc.contains("\n== 1. Introduction and Goals\n"), "{adoc}");
    assert!(adoc.contains("Our own goal text."), "{adoc}");
    assert!(adoc.contains("[mermaid]\n....\n"), "{adoc}");
    assert!(adoc.contains("|===\n"), "{adoc}");
    let check = archguard(
        dir.path(),
        &["doc", "--arc42", "--format", "asciidoc", "--check"],
    );
    assert_eq!(check.status.code(), Some(0));
    let chapter = ok(&archguard(
        dir.path(),
        &[
            "doc",
            "--arc42",
            "--chapter",
            "5",
            "--format",
            "asciidoc",
            "--stdout",
        ],
    ));
    assert!(chapter.contains("== 5. Building Block View"), "{chapter}");
    assert!(!chapter.contains("== 1. "), "{chapter}");
}

#[test]
fn arc42_exports_a_word_document() {
    let dir = project();
    let out = dir.path().join("out/doc.docx");
    ok(&archguard(
        dir.path(),
        &[
            "doc",
            "--arc42",
            "--format",
            "docx",
            "--out",
            out.to_str().unwrap(),
        ],
    ));
    let bytes = fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"PK"), "a zip package");
    let names = String::from_utf8_lossy(&bytes);
    assert!(names.contains("word/document.xml") && names.contains("word/styles.xml"));
    let stdout = archguard(
        dir.path(),
        &["doc", "--arc42", "--format", "docx", "--stdout"],
    );
    assert_eq!(stdout.status.code(), Some(2));
    let plain = archguard(dir.path(), &["doc", "--format", "asciidoc"]);
    assert_eq!(plain.status.code(), Some(2));
}

#[test]
fn export_formats_are_refused_outside_their_commands() {
    let dir = project();
    for (command, format) in [
        ("metrics", "docx"),
        ("check", "asciidoc"),
        ("doc", "structurizr"),
    ] {
        let output = archguard(dir.path(), &[command, "--format", format]);
        assert_eq!(output.status.code(), Some(2), "{command} --format {format}");
    }
}

#[test]
fn arc42_export_never_overwrites_the_hand_written_arc42_md() {
    let dir = project();
    ok(&archguard(dir.path(), &["doc", "--arc42"]));
    let md = dir.path().join("arc42.md");
    let saved = fs::read(&md).unwrap();
    let absolute = md.to_str().unwrap().to_owned();
    let mut targets = vec!["arc42.md".to_owned(), "./arc42.md".to_owned(), absolute];
    // A case-insensitive file system (macOS, Windows) reaches it as ARC42.MD too.
    if dir.path().join("ARC42.MD").exists() {
        targets.push("ARC42.MD".into());
    }
    for format in ["asciidoc", "docx"] {
        for out in &targets {
            let output = archguard(
                dir.path(),
                &["doc", "--arc42", "--format", format, "--out", out],
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(
                output.status.code(),
                Some(2),
                "{format} --out {out}: {stderr}"
            );
            assert!(stderr.contains("arc42.md"), "{stderr}");
            assert_eq!(fs::read(&md).unwrap(), saved, "{format} --out {out}");
        }
    }
}

#[test]
fn one_chapter_is_exported_to_its_own_file() {
    let dir = project();
    ok(&archguard(
        dir.path(),
        &["doc", "--arc42", "--format", "asciidoc"],
    ));
    let full = fs::read(dir.path().join("arc42.adoc")).unwrap();
    for (format, name) in [("asciidoc", "arc42-05.adoc"), ("docx", "arc42-05.docx")] {
        let chapter = ["doc", "--arc42", "--chapter", "5", "--format", format];
        ok(&archguard(dir.path(), &chapter));
        assert!(dir.path().join(name).is_file(), "{name}");
        let check = archguard(dir.path(), &[&chapter[..], &["--check"]].concat());
        assert_eq!(check.status.code(), Some(0), "{format} --check");
    }
    assert_eq!(fs::read(dir.path().join("arc42.adoc")).unwrap(), full);
}
