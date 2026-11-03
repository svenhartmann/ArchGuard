use archguard_engine::core::Status;
use archguard_engine::{Level, analyze_run, load_config};
use std::fs;

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src/ui")).unwrap();
    fs::create_dir_all(dir.path().join("src/persistence")).unwrap();
    fs::write(
        dir.path().join("src/ui/view.ts"),
        "import {value} from '../persistence/db';\nexport const view=value;\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/persistence/db.ts"),
        "export const value=1;\n",
    )
    .unwrap();
    dir
}

#[test]
fn declared_dependency_rule_gates_original_file_imports_at_every_zoom() {
    let dir = project();
    for depth in [0, 1, 8] {
        fs::write(
            dir.path().join("archguard.toml"),
            format!(
                r#"
derive_layers = false
[project]
source_roots = ["src"]
languages = ["ts"]
exclude = []
[modules]
depth = {depth}
[rules]
deny_cycles = "off"
max_scc_size = 100
[[dependency_rules]]
name = "ui-no-db"
from = ["ui/**"]
to = ["persistence/**"]
severity = "error"
message = "UI must call the application API."
"#
            ),
        )
        .unwrap();
        let loaded =
            load_config(None, dir.path()).expect("valid standalone dependency rules must load");
        let report = analyze_run(dir.path(), &loaded, Level::Module)
            .unwrap()
            .report;
        assert_eq!(
            report.summary.status,
            Status::Fail,
            "zoom={depth} must not hide the declared file-edge rule"
        );
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.id.starts_with("dependency-rule:"))
            .expect("the rule violation has a stable distinct identity");
        let value = serde_json::to_value(finding).unwrap();
        assert_eq!(value["kind"], "dependency_rule");
        assert!(
            value["message"]
                .as_str()
                .unwrap()
                .contains("UI must call the application API.")
        );
        assert_eq!(value["evidence"][0]["file"], "ui/view.ts");
        assert_eq!(value["evidence"][0]["line"], 1);
    }
}

#[test]
fn rule_severity_and_direction_are_explicit_and_invalid_rules_fail_early() {
    let dir = project();
    for (severity, status) in [
        ("warning", Status::Pass),
        ("info", Status::Pass),
        ("error", Status::Fail),
    ] {
        fs::write(dir.path().join("archguard.toml"),format!("derive_layers=false\n[rules]\ndeny_cycles='off'\n[[dependency_rules]]\nname='ui-no-db'\nfrom=['ui/**']\nto=['persistence/**']\nseverity='{severity}'")).unwrap();
        let report = analyze_run(
            dir.path(),
            &load_config(None, dir.path()).unwrap(),
            Level::Module,
        )
        .unwrap()
        .report;
        assert_eq!(report.summary.status, status);
        assert_eq!(report.findings.len(), 1);
        assert!(
            report.findings[0]
                .message
                .as_ref()
                .unwrap()
                .contains("ui/view.ts → persistence/db.ts")
        );
    }
    fs::write(dir.path().join("archguard.toml"),"derive_layers=false\n[[dependency_rules]]\nname='reverse'\nfrom=['persistence/**']\nto=['ui/**']").unwrap();
    let report = analyze_run(
        dir.path(),
        &load_config(None, dir.path()).unwrap(),
        Level::File,
    )
    .unwrap()
    .report;
    assert!(report.findings.is_empty());
    let valid = "[[dependency_rules]]\nname='rule'\nfrom=['ui/**']\nto=['persistence/**']";
    for invalid in [
        valid.replace("ui/**", "["),
        valid.replace("['ui/**']", "[]"),
        valid.replace("name='rule'", "name=''"),
        format!("{valid}\n{valid}"),
        format!("{valid}\nseverity='fatal'"),
        format!("{valid}\nsevertiy='warning'"),
        "[review]\norphans='dead'".into(),
        "[[ownership_rules]]\nfrom=''\nto='@org/team'".into(),
        "[performance]\nmin_barrel_exports=0".into(),
        "[performance]\nmin_barrel_ratio=1.1".into(),
        "[performance]\nheavy_imports=['']".into(),
    ] {
        fs::write(dir.path().join("archguard.toml"), invalid).unwrap();
        assert!(
            load_config(None, dir.path()).is_err(),
            "invalid configuration must not be silently ignored"
        );
    }
}

#[test]
fn orphan_gate_is_opt_in_honest_and_independent_of_display_depth() {
    let dir = project();
    fs::write(dir.path().join("src/loose.ts"), "export const loose=1;").unwrap();
    fs::write(dir.path().join("src/main.ts"), "console.log('entry');").unwrap();
    for (policy, status, count) in [
        ("off", Status::Pass, 0),
        ("info", Status::Pass, 1),
        ("warning", Status::Pass, 1),
        ("error", Status::Fail, 1),
    ] {
        for depth in [0, 8] {
            fs::write(dir.path().join("archguard.toml"),format!("derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['ts']\nexclude=[]\n[modules]\ndepth={depth}\n[rules]\ndeny_cycles='off'\n[review]\norphans='{policy}'\nentry_points=['main.ts','ui/**']")).unwrap();
            let report = analyze_run(
                dir.path(),
                &load_config(None, dir.path()).unwrap(),
                Level::Module,
            )
            .unwrap()
            .report;
            let candidates: Vec<_> = report
                .findings
                .iter()
                .filter(|finding| finding.kind == archguard_engine::core::FindingKind::Orphan)
                .collect();
            assert_eq!(report.summary.status, status);
            assert_eq!(candidates.len(), count);
            if count > 0 {
                assert_eq!(candidates[0].nodes, vec!["loose.ts"]);
                assert!(
                    candidates[0]
                        .message
                        .as_ref()
                        .unwrap()
                        .contains("dynamic consumers before removing code")
                );
                assert_eq!(candidates[0].id, "orphan:loose.ts");
            }
        }
    }
    fs::write(dir.path().join("archguard.toml"),"derive_layers=false\n[rules]\ndeny_cycles='off'\n[review]\norphans='warning'\nentry_points=[]").unwrap();
    let report = analyze_run(
        dir.path(),
        &load_config(None, dir.path()).unwrap(),
        Level::File,
    )
    .unwrap()
    .report;
    assert_eq!(report.findings.len(), 3);
    assert!(report.findings.iter().any(|finding| {
        finding
            .message
            .as_ref()
            .is_some_and(|message| message.starts_with("Unreferenced source file"))
    }));
}
