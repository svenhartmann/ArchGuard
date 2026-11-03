use archguard_engine::analyze_workspace;
use std::{fs, path::Path};

fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn pom(name: &str, body: &str) -> String {
    format!(
        "<project><modelVersion>4.0.0</modelVersion><groupId>demo</groupId><artifactId>{name}</artifactId><version>1.0</version>{body}</project>"
    )
}

#[test]
fn maven_dependencies_need_full_unique_matching_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "app/pom.xml",
        &pom(
            "app",
            "<dependencies><dependency><groupId>demo</groupId><artifactId>lib</artifactId><version>1.0</version></dependency></dependencies>",
        ),
    );
    write(root, "lib/pom.xml", &pom("lib", ""));
    write(
        root,
        "unrelated/pom.xml",
        "<project><groupId>other</groupId><artifactId>lib</artifactId><version>1.0</version></project>",
    );
    let report = analyze_workspace(root).unwrap();
    assert_eq!(
        report.dependencies.len(),
        1,
        "literal local Maven dependency must resolve"
    );
    assert_eq!(report.dependencies[0].from, "project:app/pom.xml");
    assert_eq!(report.dependencies[0].to, "project:lib/pom.xml");
    write(root, "duplicate/pom.xml", &pom("lib", ""));
    let ambiguous = analyze_workspace(root).unwrap();
    assert!(ambiguous.dependencies.is_empty());
    assert_eq!(ambiguous.status, "partial");
    assert!(
        ambiguous
            .unresolved
            .iter()
            .any(|u| u.reason.contains("ambiguous"))
    );
}

#[test]
fn maven_inheritance_properties_and_management_resolve_without_aggregator_edges() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "pom.xml",
        "<project><groupId>demo</groupId><artifactId>parent</artifactId><version>1.0</version><packaging>pom</packaging><properties><lib.version>1.0</lib.version></properties><modules><module>app</module><module>lib</module></modules><dependencyManagement><dependencies><dependency><groupId>demo</groupId><artifactId>lib</artifactId><version>${lib.version}</version></dependency></dependencies></dependencyManagement></project>",
    );
    write(
        root,
        "app/pom.xml",
        "<project><parent><groupId>demo</groupId><artifactId>parent</artifactId><version>1.0</version></parent><artifactId>app</artifactId><dependencies><dependency><groupId>${project.groupId}</groupId><artifactId>lib</artifactId><scope>test</scope></dependency></dependencies></project>",
    );
    write(
        root,
        "lib/pom.xml",
        "<project><parent><groupId>demo</groupId><artifactId>parent</artifactId><version>1.0</version></parent><artifactId>lib</artifactId></project>",
    );
    let report = analyze_workspace(root).unwrap();
    assert_eq!(report.dependencies.len(), 1);
    assert_eq!(report.dependencies[0].scope, "test");
    assert_eq!(report.dependencies[0].to, "project:lib/pom.xml");
    assert!(
        !report
            .dependencies
            .iter()
            .any(|d| d.from == "project:pom.xml")
    );
}

#[test]
fn maven_mismatches_and_conditional_profiles_never_create_local_edges() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "app/pom.xml",
        &pom(
            "app",
            "<dependencies><dependency><groupId>demo</groupId><artifactId>lib</artifactId><version>2.0</version></dependency><dependency><groupId>external</groupId><artifactId>ordinary</artifactId><version>9</version></dependency></dependencies><profiles><profile><id>optional</id><dependencies><dependency><groupId>demo</groupId><artifactId>lib</artifactId><version>1.0</version></dependency></dependencies></profile></profiles>",
        ),
    );
    write(root, "lib/pom.xml", &pom("lib", ""));
    let report = analyze_workspace(root).unwrap();
    assert!(report.dependencies.is_empty());
    assert_eq!(report.status, "partial");
    assert!(
        report
            .unresolved
            .iter()
            .any(|u| u.reason.contains("version"))
    );
    assert!(
        report
            .unresolved
            .iter()
            .any(|u| u.reason.contains("profile"))
    );
    assert!(
        !report
            .unresolved
            .iter()
            .any(|u| u.target.contains("ordinary"))
    );
}

#[test]
fn npm_explicit_workspace_and_file_links_preserve_scopes_and_cycles() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "package.json",
        r#"{"private":true,"workspaces":["packages/*"]}"#,
    );
    write(
        root,
        "packages/app/package.json",
        r#"{"name":"app","version":"1.0.0","dependencies":{"lib":"workspace:*"},"devDependencies":{"test":"file:../test"}}"#,
    );
    write(
        root,
        "packages/lib/package.json",
        r#"{"name":"lib","version":"1.0.0","dependencies":{"app":"workspace:*"}}"#,
    );
    write(
        root,
        "packages/test/package.json",
        r#"{"name":"test","version":"1.0.0","dependencies":{"lib":"^1.0.0"}}"#,
    );
    let report = analyze_workspace(root).unwrap();
    assert_eq!(report.dependencies.len(), 3);
    assert_eq!(
        report.cycles,
        vec![vec![
            "project:packages/app/package.json".to_string(),
            "project:packages/lib/package.json".to_string()
        ]]
    );
    assert!(report.dependencies.iter().any(|d| d.scope == "test"));
}

#[test]
fn npm_workspace_membership_version_and_name_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "package.json",
        r#"{"private":true,"workspaces":["packages/*"]}"#,
    );
    write(
        root,
        "packages/app/package.json",
        r#"{"name":"app","version":"1.0.0","dependencies":{"lib":"workspace:2.0.0","outside":"workspace:*","wrong":"file:../lib"}}"#,
    );
    write(
        root,
        "packages/lib/package.json",
        r#"{"name":"lib","version":"1.0.0"}"#,
    );
    write(
        root,
        "outside/package.json",
        r#"{"name":"outside","version":"1.0.0"}"#,
    );
    let report = analyze_workspace(root).unwrap();
    assert!(report.dependencies.is_empty());
    assert_eq!(report.unresolved.len(), 3);
    assert_eq!(report.status, "partial");
}

#[test]
fn dotnet_project_references_keep_conditions_and_build_only_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "app/App.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><ProjectReference Include="..\lib\Lib.csproj"/><ProjectReference Include="../tool/Tool.csproj"><ReferenceOutputAssembly>false</ReferenceOutputAssembly></ProjectReference></ItemGroup><ItemGroup Condition="'$(Configuration)' == 'Debug'"><ProjectReference Include="../other/Other.csproj"/></ItemGroup></Project>"#,
    );
    for p in ["lib/Lib.csproj", "tool/Tool.csproj", "other/Other.csproj"] {
        write(root, p, "<Project Sdk=\"Microsoft.NET.Sdk\"/>");
    }
    let report = analyze_workspace(root).unwrap();
    assert_eq!(report.dependencies.len(), 2);
    assert!(report.dependencies.iter().any(|d| d.scope == "build"));
    assert_eq!(report.unresolved.len(), 1);
    assert_eq!(report.status, "partial");
}

#[test]
fn cargo_workspace_inherited_paths_and_renamed_optional_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers=['app','lib','tool']\n[workspace.dependencies]\nshared={package='lib',path='lib'}\n",
    );
    write(
        root,
        "app/Cargo.toml",
        "[package]\nname='app'\nversion='1.0.0'\n[dependencies]\nshared={workspace=true}\n[build-dependencies]\ntool={path='../tool'}\n",
    );
    write(
        root,
        "lib/Cargo.toml",
        "[package]\nname='lib'\nversion='1.0.0'\n[dependencies]\napp={path='../app',optional=true}\n",
    );
    write(
        root,
        "tool/Cargo.toml",
        "[package]\nname='tool'\nversion='1.0.0'\n",
    );
    let report = analyze_workspace(root).unwrap();
    assert_eq!(report.dependencies.len(), 3);
    assert!(report.dependencies.iter().any(|d| d.scope == "optional"));
    assert!(report.dependencies.iter().any(|d| d.scope == "build"));
    assert_eq!(report.cycles.len(), 1);
}

#[cfg(unix)]
#[test]
fn path_references_cannot_escape_or_follow_internal_symlinks() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let outside = tempfile::tempdir().unwrap();
    write(root, "lib/Lib.csproj", "<Project/>");
    write(outside.path(), "Outside.csproj", "<Project/>");
    symlink(root.join("lib"), root.join("linked")).unwrap();
    let xml = format!(
        "<Project><ItemGroup><ProjectReference Include='../linked/Lib.csproj'/><ProjectReference Include='{}'/></ItemGroup></Project>",
        outside.path().join("Outside.csproj").display()
    );
    write(root, "app/App.csproj", &xml);
    let report = analyze_workspace(root).unwrap();
    assert!(report.dependencies.is_empty());
    assert_eq!(report.unresolved.len(), 2);
    assert!(!root.join(".archguard").exists());
}

#[test]
fn npm_workspace_globs_keep_directory_boundaries_and_include_root_package() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "package.json",
        r#"{"name":"root","version":"1.0.0","workspaces":["packages/*"],"dependencies":{"lib":"workspace:*"}}"#,
    );
    write(
        root,
        "packages/lib/package.json",
        r#"{"name":"lib","version":"1.0.0","dependencies":{"nested":"workspace:*"}}"#,
    );
    write(
        root,
        "packages/container/deep/package.json",
        r#"{"name":"nested","version":"1.0.0"}"#,
    );
    let report = analyze_workspace(root).unwrap();
    assert_eq!(report.dependencies.len(), 1);
    assert_eq!(report.dependencies[0].from, "project:package.json");
    assert_eq!(
        report.dependencies[0].to,
        "project:packages/lib/package.json"
    );
    assert_eq!(report.unresolved.len(), 1);
}

#[test]
fn dependency_management_scope_and_interpolated_optional_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "app/pom.xml",
        &pom(
            "app",
            "<properties><is.optional>true</is.optional></properties><dependencyManagement><dependencies><dependency><groupId>demo</groupId><artifactId>lib</artifactId><version>1.0</version><scope>test</scope></dependency></dependencies></dependencyManagement><dependencies><dependency><groupId>demo</groupId><artifactId>lib</artifactId></dependency><dependency><groupId>demo</groupId><artifactId>other</artifactId><version>1.0</version><optional>${is.optional}</optional></dependency></dependencies>",
        ),
    );
    write(root, "lib/pom.xml", &pom("lib", ""));
    write(root, "other/pom.xml", &pom("other", ""));
    let report = analyze_workspace(root).unwrap();
    assert_eq!(report.dependencies.len(), 2);
    assert_eq!(
        report
            .dependencies
            .iter()
            .find(|d| d.to == "project:lib/pom.xml")
            .unwrap()
            .scope,
        "test"
    );
    assert_eq!(
        report
            .dependencies
            .iter()
            .find(|d| d.to == "project:other/pom.xml")
            .unwrap()
            .scope,
        "optional"
    );
}

#[test]
fn msbuild_otherwise_and_targets_are_not_unconditional_compile_references() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "app/App.csproj",
        r#"<Project><Choose><When Condition="'$(Mode)' == 'A'"/><Otherwise><ItemGroup><ProjectReference Include="../lib/Lib.csproj"/></ItemGroup></Otherwise></Choose><Target Name="Generated"><ItemGroup><ProjectReference Include="../lib/Lib.csproj"/></ItemGroup></Target></Project>"#,
    );
    write(root, "lib/Lib.csproj", "<Project/>");
    let report = analyze_workspace(root).unwrap();
    assert!(report.dependencies.is_empty());
    assert_eq!(report.status, "partial");
    assert!(!report.unresolved.is_empty());
}

#[test]
fn excluded_cargo_package_does_not_inherit_outer_workspace_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers=['crates/*']\nexclude=['crates/excluded']\n[workspace.dependencies]\nlib={path='crates/lib'}\n",
    );
    write(
        root,
        "crates/excluded/Cargo.toml",
        "[package]\nname='excluded'\nversion='1.0.0'\n[dependencies]\nlib={workspace=true}\n",
    );
    write(
        root,
        "crates/lib/Cargo.toml",
        "[package]\nname='lib'\nversion='1.0.0'\n",
    );
    let report = analyze_workspace(root).unwrap();
    assert!(report.dependencies.is_empty());
    assert_eq!(report.status, "partial");
    assert!(
        report
            .unresolved
            .iter()
            .any(|u| u.reason.contains("workspace"))
    );
}

#[test]
fn external_parent_and_managed_nondefault_artifact_do_not_claim_complete_jar_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "app/pom.xml",
        "<project><parent><groupId>external</groupId><artifactId>parent</artifactId><version>1</version><relativePath/></parent><groupId>demo</groupId><artifactId>app</artifactId><version>1.0</version><dependencyManagement><dependencies><dependency><groupId>demo</groupId><artifactId>lib</artifactId><version>1.0</version><type>test-jar</type><classifier>tests</classifier></dependency></dependencies></dependencyManagement><dependencies><dependency><groupId>demo</groupId><artifactId>lib</artifactId></dependency></dependencies></project>",
    );
    write(root, "lib/pom.xml", &pom("lib", ""));
    let report = analyze_workspace(root).unwrap();
    assert!(
        report.dependencies.is_empty(),
        "managed test artifact cannot supply a normal jar version"
    );
    assert_eq!(report.status, "partial");
    assert!(
        report
            .unresolved
            .iter()
            .any(|u| u.reason.contains("parent")),
        "external inherited dependencies remain unknown"
    );
}

#[test]
fn npm_optional_dependency_overrides_the_regular_declaration() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "app/package.json",
        r#"{"name":"app","dependencies":{"lib":"file:../lib"},"optionalDependencies":{"lib":"file:../lib"}}"#,
    );
    write(root, "lib/package.json", r#"{"name":"lib"}"#);
    let report = analyze_workspace(root).unwrap();
    assert_eq!(report.dependencies.len(), 1);
    assert_eq!(report.dependencies[0].scope, "optional");
}

#[test]
fn conditional_reference_metadata_does_not_force_an_unconditional_build_scope() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "app/App.csproj",
        r#"<Project><ItemGroup><ProjectReference Include="../lib/Lib.csproj"><ReferenceOutputAssembly Condition="'$(Mode)' == 'Tools'">false</ReferenceOutputAssembly></ProjectReference></ItemGroup></Project>"#,
    );
    write(root, "lib/Lib.csproj", "<Project/>");
    let report = analyze_workspace(root).unwrap();
    assert!(report.dependencies.is_empty());
    assert_eq!(report.status, "partial");
}

#[test]
fn msbuild_reference_output_boolean_is_case_insensitive_and_dynamic_is_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "app/App.csproj",
        r#"<Project><ItemGroup><ProjectReference Include="../lib/Lib.csproj" ReferenceOutputAssembly="False"/><ProjectReference Include="../other/Other.csproj"><ReferenceOutputAssembly>$(UseOutput)</ReferenceOutputAssembly></ProjectReference></ItemGroup></Project>"#,
    );
    write(root, "lib/Lib.csproj", "<Project/>");
    write(root, "other/Other.csproj", "<Project/>");
    let report = analyze_workspace(root).unwrap();
    assert_eq!(report.dependencies.len(), 1);
    assert_eq!(report.dependencies[0].scope, "build");
    assert_eq!(report.unresolved.len(), 1);
}
