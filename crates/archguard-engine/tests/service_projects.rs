use archguard_engine::analyze_services;
use serde_json::{Value, json};
use std::{fs, path::Path};
use tempfile::tempdir;

fn put(root: &Path, path: &str, body: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, body).unwrap();
}
fn pom(name: &str) -> String {
    format!(
        r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><parent><artifactId>parent-wrong-name</artifactId></parent><artifactId>{name}</artifactId><build><plugins><plugin><groupId>com.microsoft.azure</groupId><artifactId>azure-functions-maven-plugin</artifactId></plugin></plugins></build></project>"#
    )
}
fn data(root: &Path) -> Value {
    serde_json::to_value(analyze_services(root).unwrap()).unwrap()
}

#[test]
fn four_manifest_apps_and_two_test_tools_are_distinct_from_local_infrastructure() {
    let dir = tempdir().unwrap();
    for name in ["search", "cms-import", "reporting-import", "events"] {
        put(dir.path(), &format!("functions/{name}/pom.xml"), &pom(name));
        put(
            dir.path(),
            &format!("functions/{name}/src/main/java/same/Client.java"),
            "package same; class Client {} ",
        );
    }
    put(
        dir.path(),
        "integration-tests/package.json",
        r#"{"name":"smoke","scripts":{"dev":"playwright test --ui","test":"playwright test"},"devDependencies":{"@playwright/test":"1"}}"#,
    );
    put(
        dir.path(),
        "local-test-cluster/package.json",
        r#"{"name":"cluster-tests","scripts":{"test":"bats"},"devDependencies":{"bats":"1"}}"#,
    );
    put(
        dir.path(),
        "local-test-cluster/compose.yaml",
        "services:\n  elasticsearch:\n    image: elasticsearch\n  kibana:\n    image: kibana\n    depends_on: [elasticsearch]\n    environment:\n      ELASTICSEARCH_HOSTS: http://elasticsearch:9200\n",
    );
    let report = data(dir.path());
    assert_eq!(report["monorepo"], true);
    let projects = report["projects"].as_array().expect("project inventory");
    assert_eq!(projects.len(), 6);
    assert_eq!(
        projects
            .iter()
            .filter(|p| p["kind"] == "application")
            .count(),
        4
    );
    assert_eq!(
        projects.iter().filter(|p| p["kind"] == "tooling").count(),
        2
    );
    assert_eq!(report["services"].as_array().unwrap().len(), 6);
    assert_eq!(
        report["dependencies"].as_array().unwrap().len(),
        2,
        "do not invent app-to-app or cloud-to-local API edges"
    );
    for name in ["search", "cms-import", "reporting-import", "events"] {
        let id = format!("project:functions/{name}/pom.xml");
        let service = report["services"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .unwrap();
        assert_eq!(service["project"], id);
        let project = projects.iter().find(|p| p["id"] == id).unwrap();
        assert_eq!(project["name"], name);
        assert_eq!(project["runtime"], "azure-functions");
        assert!(!project["evidence"].as_array().unwrap().is_empty());
    }
}

#[test]
fn compose_build_reuses_project_identity_without_duplicate_application() {
    let dir = tempdir().unwrap();
    put(dir.path(), "api/pom.xml", &pom("api"));
    put(
        dir.path(),
        "compose.yaml",
        "services:\n  api-dev:\n    build: ./api\n",
    );
    let report = data(dir.path());
    assert_eq!(report["services"].as_array().unwrap().len(), 1);
    assert_eq!(report["services"][0]["id"], "compose.yaml::api-dev");
    assert_eq!(report["services"][0]["project"], "project:api/pom.xml");
    assert_eq!(report["monorepo"], false);
}

#[test]
fn project_metadata_is_not_confused_with_dependencies_profiles_or_managed_plugins() {
    let dir = tempdir().unwrap();
    put(
        dir.path(),
        "lib/pom.xml",
        r#"<project><parent><artifactId>parent</artifactId></parent><artifactId>shared-library</artifactId><dependencies><dependency><artifactId>azure-functions-maven-plugin</artifactId></dependency></dependencies><build><pluginManagement><plugins><plugin><artifactId>spring-boot-maven-plugin</artifactId></plugin></plugins></pluginManagement></build><profiles><profile><build><plugins><plugin><artifactId>azure-functions-maven-plugin</artifactId></plugin></plugins></build></profile></profiles></project>"#,
    );
    put(
        dir.path(),
        "app/package.json",
        r#"{"name":"api","scripts":{"start":"node server.js"}}"#,
    );
    let report = data(dir.path());
    assert_eq!(report["projects"][1]["name"], "shared-library");
    assert_eq!(report["projects"][1]["kind"], "library");
    assert_eq!(report["services"].as_array().unwrap().len(), 1);
    assert_eq!(report["services"][0]["id"], "project:app/package.json");
}

#[test]
fn dotnet_test_projects_are_not_services_and_two_web_apps_keep_identical_names_separate() {
    let dir = tempdir().unwrap();
    // Include test projects here to exercise classification independently of default scope.
    put(dir.path(), "archguard.toml", "[project]\nexclude = []\n");
    for name in ["a", "b"] {
        put(
            dir.path(),
            &format!("{name}/App.csproj"),
            r#"<Project Sdk="Microsoft.NET.Sdk.Web"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>"#,
        );
    }
    put(
        dir.path(),
        "tests/Tests.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><IsTestProject>true</IsTestProject><OutputType>Exe</OutputType></PropertyGroup></Project>"#,
    );
    let report = data(dir.path());
    assert_eq!(report["projects"].as_array().unwrap().len(), 3);
    assert_eq!(report["services"].as_array().unwrap().len(), 2);
    assert_ne!(report["services"][0]["id"], report["services"][1]["id"]);
    assert_eq!(report["projects"][2]["kind"], "tooling");
}

#[test]
fn malformed_xml_dtd_and_gitignored_manifests_are_not_executed_or_silently_classified() {
    let dir = tempdir().unwrap();
    put(dir.path(), ".gitignore", "ignored/\n");
    put(dir.path(), "ignored/pom.xml", &pom("hidden"));
    put(
        dir.path(),
        "node_modules/pkg/package.json",
        r#"{"name":"hidden","scripts":{"start":"node server.js"}}"#,
    );
    put(dir.path(), "bad/pom.xml", "<project><artifactId>broken");
    put(
        dir.path(),
        "entity/pom.xml",
        r#"<!DOCTYPE project [<!ENTITY secret SYSTEM "file:///etc/passwd">]><project><artifactId>&secret;</artifactId></project>"#,
    );
    let report = data(dir.path());
    assert_eq!(report["projects"], json!([]));
    assert_eq!(report["complete"], false);
    assert_eq!(report["diagnostics"].as_array().unwrap().len(), 2);
    assert!(!report.to_string().contains("root:"));
}

#[test]
fn repository_root_orchestrator_maps_compose_children_without_an_extra_application() {
    let dir = tempdir().unwrap();
    put(
        dir.path(),
        "package.json",
        r#"{"name":"orchestrator","scripts":{"start":"node dev.js"}}"#,
    );
    fs::create_dir_all(dir.path().join("services/a")).unwrap();
    fs::create_dir_all(dir.path().join("services/b")).unwrap();
    put(
        dir.path(),
        "compose.yml",
        "services:\n  a:\n    build: ./services/a\n  b:\n    build: ./services/b\n",
    );
    let report = data(dir.path());
    assert_eq!(report["projects"].as_array().unwrap().len(), 1);
    assert_eq!(report["services"].as_array().unwrap().len(), 2);
    assert!(
        report["services"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["project"] == "project:package.json")
    );
    assert_eq!(
        report["monorepo"], true,
        "distinct verified build contexts remain monorepo evidence"
    );
}

#[test]
fn workspace_root_is_not_counted_as_a_second_project_and_cargo_binary_metadata_is_explicit() {
    let dir = tempdir().unwrap();
    put(dir.path(), "Cargo.toml", "[workspace]\nmembers = ['app']\n");
    put(
        dir.path(),
        "app/Cargo.toml",
        "[package]\nname = 'app'\nversion = '0.1.0'\n",
    );
    put(dir.path(), "app/src/main.rs", "fn main() {}\n");
    let report = data(dir.path());
    assert_eq!(report["projects"][0]["kind"], "workspace");
    assert_eq!(report["projects"][1]["kind"], "application");
    assert_eq!(
        report["services"],
        json!([]),
        "an executable is not deployment evidence"
    );
    assert_eq!(
        report["monorepo"], false,
        "one app plus a virtual workspace is not two independent projects"
    );
    put(
        dir.path(),
        "app/Cargo.toml",
        "[package]\nname = 'app'\nversion = '0.1.0'\nautobins = false\n",
    );
    assert_eq!(data(dir.path())["services"], json!([]));
}

#[cfg(unix)]
#[test]
fn manifest_and_parent_symlinks_never_follow_outside_the_repository() {
    use std::os::unix::fs::symlink;
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    put(outside.path(), "pom.xml", &pom("OUTSIDE_SHOULD_NOT_APPEAR"));
    symlink(outside.path(), dir.path().join("linked-directory")).unwrap();
    symlink(outside.path().join("pom.xml"), dir.path().join("pom.xml")).unwrap();
    let report = data(dir.path());
    assert_eq!(report["projects"], json!([]));
    assert_eq!(report["complete"], false);
    assert!(!report.to_string().contains("OUTSIDE_SHOULD_NOT_APPEAR"));
}

#[test]
fn conditional_application_or_test_flags_and_script_strings_are_not_executed() {
    let dir = tempdir().unwrap();
    put(
        dir.path(),
        "api/App.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup Condition="'$(Mode)' == 'app'"><OutputType>Exe</OutputType></PropertyGroup></Project>"#,
    );
    put(
        dir.path(),
        "tool/package.json",
        r#"{"name":"unknown","scripts":{"start":"echo node server.js; touch SHOULD_NOT_APPEAR"}}"#,
    );
    let report = data(dir.path());
    assert_eq!(report["services"], json!([]));
    assert!(!dir.path().join("SHOULD_NOT_APPEAR").exists());
    assert!(!report.to_string().contains("touch SHOULD_NOT_APPEAR"));
}

#[test]
fn aggregator_plugins_and_build_or_test_script_names_do_not_create_deployable_apps() {
    let dir = tempdir().unwrap();
    put(
        dir.path(),
        "aggregator/pom.xml",
        r#"<project><artifactId>parent</artifactId><packaging>pom</packaging><build><plugins><plugin><groupId>org.springframework.boot</groupId><artifactId>spring-boot-maven-plugin</artifactId></plugin></plugins></build></project>"#,
    );
    put(
        dir.path(),
        "test/package.json",
        r#"{"name":"tests","scripts":{"start":"bun run test","test":"bun test"}}"#,
    );
    put(
        dir.path(),
        "build/package.json",
        r#"{"name":"assets","scripts":{"start":"vite build"}}"#,
    );
    put(
        dir.path(),
        "version/package.json",
        r#"{"name":"version-check","scripts":{"start":"node --version"}}"#,
    );
    let report = data(dir.path());
    assert_eq!(report["projects"][0]["kind"], "workspace");
    assert_eq!(report["services"], json!([]));
}

#[test]
fn later_unconditional_dotnet_properties_determine_test_and_executable_classification() {
    let dir = tempdir().unwrap();
    put(
        dir.path(),
        "Tests.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><IsTestProject>false</IsTestProject><OutputType>Exe</OutputType></PropertyGroup><PropertyGroup><IsTestProject>true</IsTestProject></PropertyGroup></Project>"#,
    );
    put(
        dir.path(),
        "Library.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType></PropertyGroup><PropertyGroup><OutputType>Library</OutputType></PropertyGroup></Project>"#,
    );
    let report = data(dir.path());
    assert_eq!(report["projects"][1]["kind"], "tooling");
    assert_eq!(report["projects"][0]["kind"], "library");
    assert_eq!(report["services"], json!([]));
}

#[test]
fn npm_build_test_and_version_commands_are_not_application_entrypoints() {
    let roots: Vec<_> = ["bun run test", "vite build", "node --version"]
        .iter()
        .map(|command| {
            let dir = tempdir().unwrap();
            put(
                dir.path(),
                "package.json",
                &json!({"name":"tools","scripts":{"start":command}}).to_string(),
            );
            let services = data(dir.path())["services"].as_array().unwrap().len();
            ((*command).to_owned(), services)
        })
        .collect();
    assert_eq!(
        roots,
        vec![
            ("bun run test".into(), 0),
            ("vite build".into(), 0),
            ("node --version".into(), 0)
        ]
    );
}

#[test]
fn explicit_sharpfuzz_reference_identifies_tooling_even_with_web_sdk_and_executable_output() {
    let dir = tempdir().unwrap();
    for (name, sdk) in [
        ("Harness", "Microsoft.NET.Sdk"),
        ("ApiHarness", "Microsoft.NET.Sdk.Web"),
    ] {
        put(
            dir.path(),
            &format!("components/{name}/{name}.csproj"),
            &format!(
                r#"<Project Sdk="{sdk}"><PropertyGroup><OutputType>Exe</OutputType></PropertyGroup><ItemGroup><PackageReference Include="SharpFuzz" /></ItemGroup></Project>"#
            ),
        );
    }
    let report = data(dir.path());
    let projects = report["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 2);
    assert!(
        projects.iter().all(|project| project["kind"] == "tooling"),
        "{projects:?}"
    );
    assert_eq!(
        report["services"],
        json!([]),
        "fuzz harnesses are project tooling, not independently deployed services"
    );
}

#[test]
fn shared_repository_context_does_not_bind_services_to_root_tooling() {
    let dir = tempdir().unwrap();
    put(
        dir.path(),
        "package.json",
        r#"{"name":"lint","devDependencies":{"markdownlint":"1"}}"#,
    );
    put(
        dir.path(),
        "compose.yaml",
        "services:\n  alpha:\n    build:\n      context: .\n      dockerfile: src/alpha/Dockerfile\n  beta:\n    build:\n      context: .\n      dockerfile: src/beta/Dockerfile\n",
    );
    let report = data(dir.path());
    assert!(
        report["services"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["project"].is_null())
    );
}

#[test]
fn additional_language_manifests_create_source_boundaries_without_execution() {
    let dir = tempdir().unwrap();
    put(
        dir.path(),
        "checkout/go.mod",
        "module example.org/checkout\n\ngo 1.23\n",
    );
    put(
        dir.path(),
        "email/pyproject.toml",
        "[project]\nname = \"email\"\nversion = \"1\"\n",
    );
    put(
        dir.path(),
        "ad/build.gradle",
        "plugins { id 'application' }\n",
    );
    put(dir.path(), "legacy/requirements.txt", "flask==3\n");
    let report = data(dir.path());
    let projects = report["projects"].as_array().unwrap();
    for (path, eco) in [
        ("checkout", "go"),
        ("email", "python"),
        ("ad", "gradle"),
        ("legacy", "python"),
    ] {
        assert!(
            projects
                .iter()
                .any(|p| p["path"] == path && p["ecosystem"] == eco),
            "missing {path}"
        );
    }
}

#[test]
fn npm_placeholder_test_script_does_not_turn_runtime_dependencies_into_tooling() {
    let d = tempdir().unwrap();
    put(
        d.path(),
        "worker/package.json",
        r#"{"name":"worker","scripts":{"test":"echo no-test && exit 1"},"dependencies":{"@grpc/grpc-js":"1"}}"#,
    );
    let r = data(d.path());
    assert_ne!(r["projects"][0]["kind"], "tooling");
}

#[test]
fn service_discovery_excludes_samples_worktrees_and_configured_paths_consistently() {
    let dir = tempdir().unwrap();
    put(
        dir.path(),
        "archguard.toml",
        "[project]\nexclude = ['scratch/**']\n",
    );
    put(dir.path(), ".gitignore", "/ignored/\n");
    for name in [
        "real",
        "examples/shop",
        "fixtures/app",
        ".claude/worktrees/copy",
        ".codex/copy",
        "scratch/app",
        "ignored/app",
    ] {
        put(
            dir.path(),
            &format!("{name}/Cargo.toml"),
            "[package]\nname='app'\nversion='0.1.0'\n",
        );
        put(dir.path(), &format!("{name}/src/main.rs"), "fn main() {}\n");
        put(
            dir.path(),
            &format!("{name}/compose.yaml"),
            "services:\n  app:\n    build: .\n",
        );
        put(
            dir.path(),
            &format!("{name}/deploy.yaml"),
            "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: worker\nspec:\n  template:\n    spec:\n      containers:\n        - name: worker\n          image: worker:1\n",
        );
    }
    let report = data(dir.path());
    assert_eq!(report["projects"].as_array().unwrap().len(), 1);
    assert_eq!(report["projects"][0]["path"], "real");
    assert_eq!(report["compose_files"], json!(["real/compose.yaml"]));
    assert!(
        report["services"]
            .as_array()
            .unwrap()
            .iter()
            .all(|service| {
                service["compose_file"]
                    .as_str()
                    .unwrap()
                    .starts_with("real/")
            }),
        "{report}"
    );
    let sample = data(&dir.path().join("examples/shop"));
    assert_eq!(sample["projects"].as_array().unwrap().len(), 1);
    assert_eq!(sample["compose_files"], json!(["compose.yaml"]));
}

#[test]
fn a_deployed_rust_binary_is_still_a_service() {
    let dir = tempdir().unwrap();
    put(
        dir.path(),
        "Cargo.toml",
        "[package]\nname='api'\nversion='0.1.0'\n",
    );
    put(dir.path(), "src/main.rs", "fn main() {}\n");
    assert_eq!(data(dir.path())["services"], json!([]));
    put(
        dir.path(),
        "compose.yaml",
        "services:\n  api:\n    build: .\n",
    );
    let report = data(dir.path());
    assert_eq!(report["services"].as_array().unwrap().len(), 1);
    assert_eq!(report["services"][0]["project"], "project:Cargo.toml");
}
