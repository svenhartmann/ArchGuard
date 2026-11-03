use archguard_engine::core::{WorkspaceDependency, WorkspaceReport};
use archguard_engine::{analyze_services, analyze_workspace_sources, workspace_test_impact};
use std::{fs, path::Path};

fn put(root: &Path, path: &str, value: &str) {
    fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
    fs::write(root.join(path), value).unwrap();
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in ["app", "lib", "unrelated"] {
        put(
            dir.path(),
            &format!("{name}/pom.xml"),
            &format!(
                "<project><modelVersion>4.0.0</modelVersion><groupId>x</groupId><artifactId>{name}</artifactId><version>1</version><properties><maven.compiler.release>17</maven.compiler.release></properties></project>"
            ),
        );
        put(
            dir.path(),
            &format!("{name}/archguard.toml"),
            "[project]\nsource_roots=['src/main/java','src/test/java']\nexclude=[]\nlanguages=['java']\n",
        );
    }
    put(
        dir.path(),
        "app/src/main/java/app/Main.java",
        "package app; public class Main { public int run() { return lib.Shared.value(); } }\n",
    );
    put(
        dir.path(),
        "lib/src/main/java/lib/Shared.java",
        "package lib; public class Shared { public static int value() { return 1; } }\n",
    );
    put(
        dir.path(),
        "unrelated/src/main/java/lib/Shared.java",
        "package lib; public class Shared { public static int value() { return 2; } }\n",
    );
    dir
}
fn workspace(root: &Path, edges: &[(&str, &str, &str)]) -> WorkspaceReport {
    WorkspaceReport {
        schema_version: 1,
        root: root.display().to_string(),
        status: "ready".into(),
        projects: analyze_services(root).unwrap().projects,
        dependencies: edges
            .iter()
            .map(|(from, to, scope)| WorkspaceDependency {
                from: format!("project:{from}/pom.xml"),
                to: format!("project:{to}/pom.xml"),
                scope: (*scope).into(),
                kind: "maven_dependency".into(),
                evidence: vec![],
            })
            .collect(),
        cycles: vec![],
        unresolved: vec![],
        diagnostics: vec![],
        limitations: vec![],
    }
}
#[test]
fn qualified_files_and_calls_only_link_declared_java_projects() {
    let dir = fixture();
    let root = dir.path();
    let report =
        analyze_workspace_sources(root, &workspace(root, &[("app", "lib", "compile")])).unwrap();
    assert_eq!(
        report.calls.java_release, 0,
        "independent projects have no shared Java release"
    );
    assert_eq!(
        report.files.len(),
        3,
        "each physical source belongs to one project"
    );
    assert_eq!(
        report
            .files
            .iter()
            .filter(|f| f.name == "lib/Shared.java")
            .count(),
        2
    );
    let main = report
        .files
        .iter()
        .find(|f| f.project == "project:app/pom.xml")
        .unwrap();
    let shared = report
        .files
        .iter()
        .find(|f| f.project == "project:lib/pom.xml")
        .unwrap();
    let main_node = report
        .graph
        .nodes
        .iter()
        .find(|n| n.name == main.id)
        .unwrap()
        .id;
    let shared_node = report
        .graph
        .nodes
        .iter()
        .find(|n| n.name == shared.id)
        .unwrap()
        .id;
    assert!(
        report
            .graph
            .edges
            .iter()
            .any(|e| e.from == main_node && e.to == shared_node),
        "compiler-resolved cross-project dependency"
    );
    assert!(
        report
            .calls
            .calls
            .iter()
            .any(|c| c.from_class.starts_with("project:app/pom.xml::")
                && c.to_class.as_deref() == Some("project:lib/pom.xml::lib.Shared")),
        "calls preserve exact project identities: {:?}",
        report.calls
    );
    assert!(!report.calls.calls.iter().any(|c| {
        c.from_class.starts_with("project:app/pom.xml::")
            && c.to_class
                .as_deref()
                .is_some_and(|id| id.starts_with("project:unrelated/"))
    }));
    assert!(
        !root.join("app/.archguard").exists(),
        "source analysis is read-only"
    );
    let impact = workspace_test_impact(&report, std::slice::from_ref(&shared.path));
    assert_eq!(
        impact.affected_projects,
        vec!["project:app/pom.xml", "project:lib/pom.xml"]
    );
    assert!(!impact.safe_to_skip_other_tests);
}

#[test]
fn missing_declaration_and_ambiguous_closure_never_create_cross_project_calls() {
    let dir = fixture();
    let root = dir.path();
    for edges in [
        vec![],
        vec![("app", "lib", "compile"), ("app", "unrelated", "compile")],
    ] {
        let report = analyze_workspace_sources(root, &workspace(root, &edges)).unwrap();
        assert_eq!(report.files.len(), 3);
        assert!(
            !report
                .calls
                .calls
                .iter()
                .any(|c| c.from_class.starts_with("project:app/")
                    && c.to_class
                        .as_deref()
                        .is_some_and(|id| id.starts_with("project:lib/")
                            || id.starts_with("project:unrelated/")))
        );
        assert_eq!(report.status, "partial");
    }
}

#[test]
fn test_only_dependency_does_not_grant_production_visibility() {
    let dir = fixture();
    let root = dir.path();
    put(
        root,
        "app/src/test/java/app/MainTest.java",
        "package app; public class MainTest { public int test() { return lib.Shared.value(); } }\n",
    );
    let report =
        analyze_workspace_sources(root, &workspace(root, &[("app", "lib", "test")])).unwrap();
    assert!(
        report
            .calls
            .calls
            .iter()
            .any(|c| c.from_class == "project:app/pom.xml::app.MainTest"
                && c.to_class.as_deref() == Some("project:lib/pom.xml::lib.Shared"))
    );
    assert!(
        !report
            .calls
            .calls
            .iter()
            .any(|c| c.from_class == "project:app/pom.xml::app.Main"
                && c.to_class.as_deref() == Some("project:lib/pom.xml::lib.Shared"))
    );
}

#[test]
fn nested_projects_and_unmapped_test_ids_do_not_merge() {
    let dir = fixture();
    let root = dir.path();
    put(
        root,
        "app/nested/pom.xml",
        "<project><modelVersion>4.0.0</modelVersion><groupId>x</groupId><artifactId>nested</artifactId><version>1</version></project>",
    );
    put(root, "app/nested/Other.java", "class Other {}\n");
    let report = analyze_workspace_sources(root, &workspace(root, &[])).unwrap();
    assert_eq!(
        report
            .files
            .iter()
            .filter(|f| f.path.ends_with("Other.java"))
            .count(),
        1
    );
    assert!(
        report
            .files
            .iter()
            .any(|f| f.path.ends_with("Other.java") && f.project == "project:app/nested/pom.xml")
    );
    assert_eq!(
        workspace_test_impact(&report, &["lib/Shared.java".into()]).unresolved_targets,
        vec!["lib/Shared.java"]
    );
}

#[test]
fn cross_project_cycles_and_imported_tests_keep_exact_identities() {
    let dir = fixture();
    let root = dir.path();
    put(
        root,
        "lib/src/main/java/lib/Shared.java",
        "package lib; public class Shared { public static int value() { return new app.Main().run(); } }\n",
    );
    for project in ["app", "lib"] {
        let path = root.join(project);
        put(
            &path,
            "junit.xml",
            "<testsuite name='unit'><testcase name='same test name' time='0.4'/></testsuite>",
        );
        let source = if project == "app" {
            "src/main/java/app/Main.java"
        } else {
            "src/main/java/lib/Shared.java"
        };
        put(&path,"map.json",&serde_json::json!({"schema_version":1,"tests":[{"name":"same test name","sources":[{"path":source}]}]}).to_string());
        let loaded = archguard_engine::load_config(None, &path).unwrap();
        let (graph, _) =
            archguard_engine::analyze_effective(&path, &loaded, archguard_engine::Level::File)
                .unwrap();
        archguard_engine::import_test_evidence(
            &path,
            &loaded.config,
            &graph,
            Path::new("junit.xml"),
            Some(Path::new("map.json")),
            None,
        )
        .unwrap();
    }
    let report = analyze_workspace_sources(
        root,
        &workspace(
            root,
            &[("app", "lib", "compile"), ("lib", "app", "compile")],
        ),
    )
    .unwrap();
    assert_eq!(report.cycles.len(), 1);
    assert_eq!(report.cycles[0].len(), 2);
    assert_eq!(report.tests.len(), 2);
    assert_ne!(report.tests[0].id, report.tests[1].id);
    let impact = workspace_test_impact(&report, &["lib/src/main/java/lib/Shared.java".into()]);
    assert_eq!(impact.selected.len(), 2);
    assert!(
        impact
            .selected
            .iter()
            .any(|t| t.project == "project:app/pom.xml" && t.reason == "mapped_static_dependent")
    );
    assert!(!impact.safe_to_skip_other_tests);
    put(
        root,
        "lib/src/main/java/lib/Shared.java",
        "package lib; public class Shared { public static int value() { return 3; } }\n",
    );
    let changed =
        analyze_workspace_sources(root, &workspace(root, &[("app", "lib", "compile")])).unwrap();
    let impact = workspace_test_impact(&changed, &["lib/src/main/java/lib/Shared.java".into()]);
    assert_eq!(impact.unknown_tests, 1);
    assert_eq!(impact.status, "partial");
}

#[test]
fn source_graph_keeps_semantic_coverage_and_fingerprint_tracks_inputs() {
    let dir = fixture();
    let root = dir.path();
    let ws = workspace(root, &[("app", "lib", "compile")]);
    let report = analyze_workspace_sources(root, &ws).unwrap();
    let semantic = report
        .graph
        .semantic
        .as_ref()
        .expect("source graph must retain its compiler coverage");
    assert!(semantic.projected_calls > 0);
    assert!(
        report
            .calls
            .classes
            .iter()
            .all(|class| class.package.starts_with("project:"))
    );
    let first = archguard_engine::workspace_sources_fingerprint(root, &ws).unwrap();
    assert_eq!(
        first,
        archguard_engine::workspace_sources_fingerprint(root, &ws).unwrap()
    );
    put(
        root,
        "lib/src/main/java/lib/Shared.java",
        "package lib; public class Shared { public static int value() { return 8; } }\n",
    );
    let second = archguard_engine::workspace_sources_fingerprint(root, &ws).unwrap();
    assert_ne!(first, second);
    put(root, "app/.archguard/tests.json", "{}");
    assert_ne!(
        second,
        archguard_engine::workspace_sources_fingerprint(root, &ws).unwrap()
    );
    let mut ws2 = ws.clone();
    ws2.dependencies.clear();
    assert_ne!(
        archguard_engine::workspace_sources_fingerprint(root, &ws).unwrap(),
        archguard_engine::workspace_sources_fingerprint(root, &ws2).unwrap()
    );
}

#[test]
fn oversized_sources_are_partial_without_aborting_other_projects() {
    let dir = fixture();
    let root = dir.path();
    put(root, "app/TooLarge.java", &" ".repeat(8 * 1024 * 1024 + 1));
    let report = analyze_workspace_sources(root, &workspace(root, &[])).unwrap();
    assert_eq!(report.status, "partial");
    assert_eq!(report.files.len(), 3);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|message| message.contains("TooLarge.java"))
    );
}

#[test]
fn project_budget_and_outside_paths_are_visible_coverage_gaps() {
    let dir = tempfile::tempdir().unwrap();
    for index in 0..65 {
        put(
            dir.path(),
            &format!("p{index}/pom.xml"),
            &format!(
                "<project><groupId>x</groupId><artifactId>p{index}</artifactId><version>1</version></project>"
            ),
        );
    }
    let mut ws = workspace(dir.path(), &[]);
    let report = analyze_workspace_sources(dir.path(), &ws).unwrap();
    assert_eq!(report.projects.len(), 64);
    assert_eq!(report.status, "partial");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|message| message.contains("Project limit"))
    );
    ws.projects.truncate(1);
    ws.projects[0].path = "..".into();
    let report = analyze_workspace_sources(dir.path(), &ws).unwrap();
    assert!(report.files.is_empty());
    assert_eq!(report.status, "partial");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|message| message.contains("escapes"))
    );
}

#[test]
fn ambiguous_graph_files_mark_the_project_partial_even_with_ready_compiler() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "app/pom.xml",
        "<project><groupId>x</groupId><artifactId>app</artifactId><version>1</version><properties><maven.compiler.release>17</maven.compiler.release></properties></project>",
    );
    put(root, "app/one/Same.java", "package p; class First {}\n");
    put(root, "app/two/Same.java", "package p; class Second {}\n");
    let report = analyze_workspace_sources(root, &workspace(root, &[])).unwrap();
    assert_eq!(report.projects[0].status, "partial");
    assert!(
        report.projects[0]
            .diagnostics
            .iter()
            .any(|message| message.contains("Ambiguous source identity"))
    );
    assert!(report.files.is_empty());
}

#[test]
fn affected_projects_without_test_imports_prevent_ready_impact() {
    use archguard_engine::core::{CallAnalysis, GraphBuilder, Ref};
    use archguard_engine::{
        WorkspaceSourceFile, WorkspaceSourceReport, WorkspaceTestEvidence, WorkspaceTestMapping,
    };
    let mut graph = GraphBuilder::new();
    let a = graph.ensure_path("app::A.java", Some("java"));
    let b = graph.ensure_path("lib::B.java", Some("java"));
    graph
        .edge(
            a,
            b,
            Ref {
                file: "app/A.java".into(),
                line: 1,
                raw: "B.run()".into(),
            },
        )
        .unwrap();
    let report = WorkspaceSourceReport {
        schema_version: 1,
        status: "ready".into(),
        graph: graph.build("fixture".into()),
        calls: CallAnalysis::default(),
        projects: vec![],
        cycles: vec![],
        diagnostics: vec![],
        limitations: vec![],
        files: vec![
            WorkspaceSourceFile {
                id: "app::A.java".into(),
                project: "app".into(),
                name: "A.java".into(),
                path: "app/A.java".into(),
            },
            WorkspaceSourceFile {
                id: "lib::B.java".into(),
                project: "lib".into(),
                name: "B.java".into(),
                path: "lib/B.java".into(),
            },
        ],
        tests: vec![WorkspaceTestEvidence {
            id: "lib::test".into(),
            project: "lib".into(),
            suite: "unit".into(),
            name: "test".into(),
            outcome: "passed".into(),
            duration_seconds: None,
            mappings: vec![WorkspaceTestMapping {
                node: Some("lib::B.java".into()),
                path: "lib/B.java".into(),
                status: "unchanged_since_import".into(),
            }],
        }],
    };
    let impact = workspace_test_impact(&report, &["lib/B.java".into()]);
    assert_eq!(impact.selected.len(), 1);
    assert_eq!(
        impact.unknown_tests, 0,
        "no fabricated test count for unimported projects"
    );
    assert_eq!(impact.status, "partial");
    assert!(
        impact
            .diagnostics
            .iter()
            .any(|message| message.contains("app") && message.contains("test"))
    );
}

#[test]
fn project_qualifiers_cannot_collide_with_colons_in_real_paths() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for path in ["a", "a/pom.xml::nested"] {
        put(
            root,
            &format!("{path}/pom.xml"),
            "<project><groupId>x</groupId><artifactId>p</artifactId><version>1</version></project>",
        );
        put(
            root,
            &format!("{path}/archguard.toml"),
            "[project]\nsource_roots=[]\nexclude=[]\nlanguages=['py']\n",
        );
    }
    put(root, "a/nested/pom.xml::Foo.py", "value = 1\n");
    put(root, "a/pom.xml::nested/Foo.py", "value = 2\n");
    let report = analyze_workspace_sources(root, &workspace(root, &[])).unwrap();
    assert_eq!(report.files.len(), 2);
    assert_ne!(
        report.files[0].id, report.files[1].id,
        "project+source identities must be injective"
    );
    assert_eq!(
        report
            .graph
            .nodes
            .iter()
            .filter(|node| node.kind == archguard_engine::core::NodeKind::File)
            .count(),
        2
    );
}

#[test]
fn semantic_counts_do_not_repeat_dependency_types_across_compiler_scopes() {
    let dir = fixture();
    let root = dir.path();
    put(
        root,
        "unrelated/src/main/java/base/Base.java",
        "package base; public class Base {}\n",
    );
    fs::remove_file(root.join("unrelated/src/main/java/lib/Shared.java")).unwrap();
    put(
        root,
        "lib/src/main/java/lib/Shared.java",
        "package lib; public class Shared extends base.Base {}\n",
    );
    put(
        root,
        "app/src/main/java/app/Main.java",
        "package app; public class Main extends lib.Shared {}\n",
    );
    let report = analyze_workspace_sources(
        root,
        &workspace(
            root,
            &[("app", "lib", "compile"), ("lib", "unrelated", "compile")],
        ),
    )
    .unwrap();
    assert_eq!(report.calls.classes.len(), 3);
    assert_eq!(
        report.graph.semantic.as_ref().unwrap().projected_supertypes,
        2,
        "each actual source base clause counted once"
    );
}

#[test]
fn existing_java_analyzer_keeps_relative_project_roots_working() {
    let root = tempfile::Builder::new()
        .prefix(".archguard-relative-java-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    put(
        root.path(),
        "A.java",
        "class A { int value() { return 1; } }\n",
    );
    let relative = Path::new(root.path().file_name().unwrap());
    let mut config = archguard_engine::core::Config::default();
    config.project.source_roots.clear();
    config.project.exclude.clear();
    let report = archguard_engine::analyze_java_calls(relative, &config);
    assert_eq!(
        report.status, "ready",
        "relative source paths must resolve against the supplied root: {report:?}"
    );
    assert_eq!(report.classes[0].id, "A");
}

#[test]
fn workspace_qualifies_type_uses_and_class_state_without_cross_project_collisions() {
    let dir = fixture();
    let root = dir.path();
    put(
        root,
        "app/src/main/java/app/Main.java",
        "package app; public class Main { lib.Shared field; public lib.Shared get(){ return field; } }\n",
    );
    let report =
        analyze_workspace_sources(root, &workspace(root, &[("app", "lib", "compile")])).unwrap();
    let details = report
        .calls
        .details
        .as_ref()
        .expect("workspace retains semantic details");
    assert!(
        !details.method_metrics.is_empty(),
        "workspace must preserve method measurement evidence"
    );
    for metric in &details.method_metrics {
        assert!(
            report
                .calls
                .methods
                .iter()
                .any(|m| m.id == metric.method_id),
            "method metrics must use the qualified workspace identity"
        );
    }
    let usage = details
        .type_uses
        .iter()
        .find(|u| u.role == "field" && u.file == "app/src/main/java/app/Main.java")
        .unwrap();
    assert!(usage.from_class.starts_with("project:app/pom.xml"));
    assert!(
        usage
            .to_class
            .as_ref()
            .unwrap()
            .starts_with("project:lib/pom.xml")
    );
    assert_eq!(usage.resolution, "project");
    let state = details
        .class_states
        .iter()
        .find(|s| s.class_id == usage.from_class)
        .unwrap();
    assert!(
        state
            .accesses
            .iter()
            .all(|a| a.method.starts_with("project:app/pom.xml")
                && a.field.starts_with("project:app/pom.xml"))
    );
    assert!(report.graph.edges.iter().any(|e| {
        e.refs
            .iter()
            .any(|r| r.raw.starts_with("[semantic type:") && r.file == usage.file)
    }));
    assert!(
        details
            .cohesion
            .iter()
            .any(|m| m.class_id == usage.from_class && m.lcom4 == Some(1))
    );
}
