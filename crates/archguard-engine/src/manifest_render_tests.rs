use crate::services::manifest_render::*;
use archguard_core::{ServiceDependencyKind as Kind, ServiceTopology};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn put(root: &Path, path: &str, body: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}
fn root() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap();
    (dir, path)
}
fn no_tools(enabled: bool) -> Tools {
    Tools {
        enabled,
        helm: None,
        kustomize: None,
    }
}
fn analyze(root: &Path, tools: &Tools) -> ServiceTopology {
    crate::services::analyze_services_with(root, tools).unwrap()
}

#[test]
fn remote_missing_and_out_of_root_references_are_not_local() {
    let (_dir, root) = root();
    put(&root, "base/kustomization.yaml", "resources: [app.yaml]\n");
    put(&root, "base/app.yaml", "kind: ConfigMap\n");
    for entry in [
        "github.com/org/repo//deploy/base?ref=v1.0.0",
        "https://github.com/org/repo/deploy/base",
        "git@github.com:org/repo.git",
        "ssh://git@example.org/repo",
        "../outside",
        "missing-dir",
    ] {
        assert_eq!(local(&root, "base", entry), None, "{entry}");
    }
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("app.yaml"), "kind: ConfigMap\n").unwrap();
    let existing_outside = outside.path().join("app.yaml");
    assert_eq!(
        local(&root, "base", &existing_outside.to_string_lossy()),
        None
    );
    assert_eq!(
        local(&root, "base", "app.yaml").as_deref(),
        Some("base/app.yaml")
    );
    assert_eq!(
        local(&root, "overlay", "../base").as_deref(),
        None,
        "overlay/ does not exist"
    );
}

#[test]
fn a_remote_reference_in_a_base_blocks_the_overlay_and_names_the_field() {
    let (_dir, root) = root();
    put(
        &root,
        "overlay/kustomization.yaml",
        "resources: [../base]\npatches:\n- path: patch.yaml\n",
    );
    put(&root, "overlay/patch.yaml", "kind: Deployment\n");
    put(
        &root,
        "base/kustomization.yaml",
        "resources:\n- app.yaml\n- https://raw.githubusercontent.com/org/repo/main/x.yaml\n",
    );
    put(&root, "base/app.yaml", "kind: ConfigMap\n");
    let mut regions = BTreeSet::new();
    let error = inspect(
        &root,
        "overlay/kustomization.yaml",
        &mut BTreeSet::new(),
        &mut regions,
    )
    .unwrap_err();
    assert!(
        error.contains("base/kustomization.yaml") && error.contains("resources"),
        "{error}"
    );
    assert!(
        !error.contains("raw.githubusercontent"),
        "references are not echoed: {error}"
    );
}

#[test]
fn a_remote_reference_behind_a_generator_directory_blocks_the_overlay() {
    for field in ["generators", "transformers", "validators"] {
        let (_dir, root) = root();
        put(
            &root,
            "overlay/kustomization.yaml",
            &format!("resources: [app.yaml]\n{field}: [gen]\n"),
        );
        put(&root, "overlay/app.yaml", "kind: ConfigMap\n");
        put(
            &root,
            "overlay/gen/kustomization.yaml",
            "resources:\n- https://evil.example/x.yaml\n",
        );
        let error = inspect(
            &root,
            "overlay/kustomization.yaml",
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
        )
        .unwrap_err();
        assert!(
            error.contains("overlay/gen/kustomization.yaml") && error.contains("resources"),
            "{field}: {error}"
        );
    }
}

#[test]
fn local_overlays_consume_their_files_and_bases_but_not_unrelated_subdirectories() {
    let (_dir, root) = root();
    put(
        &root,
        "overlay/kustomization.yaml",
        "resources: [../base]\npatches:\n- path: patches/p.yaml\nconfigMapGenerator:\n- name: c\n  envs: [app.env]\n",
    );
    put(&root, "overlay/patches/p.yaml", "kind: Deployment\n");
    put(&root, "overlay/app.env", "A_URL=http://a\n");
    put(&root, "overlay/other/x.yaml", "kind: ConfigMap\n");
    put(&root, "base/kustomization.yaml", "resources: [app.yaml]\n");
    put(&root, "base/app.yaml", "kind: ConfigMap\n");
    let mut regions = BTreeSet::new();
    inspect(
        &root,
        "overlay/kustomization.yaml",
        &mut BTreeSet::new(),
        &mut regions,
    )
    .unwrap();
    let render = Render {
        bundle: "overlay".into(),
        documents: vec![],
        regions,
        note: String::new(),
    };
    for consumed in [
        "overlay/kustomization.yaml",
        "overlay/patches/p.yaml",
        "base/app.yaml",
        "base/kustomization.yaml",
    ] {
        assert!(render.covers(consumed), "{consumed}");
    }
    assert!(!render.covers("overlay/other/x.yaml"));
    assert_eq!(
        top_level_kustomizations(
            &root,
            &[
                "base/kustomization.yaml".into(),
                "overlay/kustomization.yaml".into()
            ]
        ),
        vec!["overlay/kustomization.yaml".to_owned()]
    );
}

#[test]
fn helm_charts_in_kustomizations_need_enable_helm_and_are_not_run() {
    let (_dir, root) = root();
    put(
        &root,
        "k/kustomization.yaml",
        "helmCharts:\n- name: redis\n  repo: oci://registry-1.docker.io/bitnamicharts\n",
    );
    let error = inspect(
        &root,
        "k/kustomization.yaml",
        &mut BTreeSet::new(),
        &mut BTreeSet::new(),
    )
    .unwrap_err();
    assert!(error.contains("--enable-helm"), "{error}");
}

#[test]
fn helm_source_comments_point_to_template_files() {
    let text = "---\n# Source: shop/templates/web.yaml\nkind: Deployment\nmetadata: {name: web}\n---\nkind: ConfigMap\n";
    let rendered = documents(text, Some("charts/shop"), "charts/shop/Chart.yaml").unwrap();
    let files: Vec<_> = rendered.iter().map(|(file, _)| file.as_str()).collect();
    assert_eq!(
        files,
        ["charts/shop/templates/web.yaml", "charts/shop/Chart.yaml"]
    );
}

#[test]
fn source_comments_cannot_point_outside_the_chart() {
    let text = "# Source: shop/../../etc/passwd\nkind: ConfigMap\n";
    let rendered = documents(text, Some("charts/shop"), "charts/shop/Chart.yaml").unwrap();
    assert_eq!(rendered[0].0, "charts/shop/Chart.yaml");
}

#[test]
fn opted_in_without_tools_explains_which_tool_is_missing() {
    let (_dir, root) = root();
    put(
        &root,
        "chart/Chart.yaml",
        "apiVersion: v2\nname: shop\nversion: 0.1.0\n",
    );
    put(&root, "k/kustomization.yaml", "resources: []\n");
    let report = analyze(&root, &no_tools(true));
    let message = |file: &str| {
        report
            .diagnostics
            .iter()
            .find(|d| d.file == file)
            .map(|d| d.message.clone())
            .unwrap_or_default()
    };
    assert!(
        message("chart/Chart.yaml").contains("ARCHGUARD_HELM"),
        "{:?}",
        report.diagnostics
    );
    assert!(
        message("k/kustomization.yaml").contains("ARCHGUARD_KUSTOMIZE"),
        "{:?}",
        report.diagnostics
    );
}

/// A stand-in tool that records that it ran.
#[cfg(unix)]
fn recording_tool(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let tool = dir.join("tool");
    fs::write(
        &tool,
        format!(
            "#!/bin/sh\n/usr/bin/touch '{}'\n",
            dir.join("RAN").display()
        ),
    )
    .unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    tool
}

#[cfg(unix)]
#[test]
fn the_recording_tool_records_a_run_of_a_local_overlay() {
    let (_dir, root) = root();
    let tools_dir = tempfile::tempdir().unwrap();
    let tool = recording_tool(tools_dir.path());
    put(&root, "k/kustomization.yaml", "resources: []\n");
    let tools = Tools {
        enabled: true,
        helm: None,
        kustomize: Some((tool, "build")),
    };
    analyze(&root, &tools);
    assert!(
        tools_dir.path().join("RAN").exists(),
        "the negative tests below rely on this"
    );
}

#[cfg(unix)]
#[test]
fn rendered_secrets_are_dropped_unread() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, root) = root();
    let tools_dir = tempfile::tempdir().unwrap();
    let output = tools_dir.path().join("rendered.yaml");
    fs::write(&output, "apiVersion: v1\nkind: Secret\nmetadata: {name: creds}\nstringData: {API_URL: 'http://secret-api:1', DB_URL: 'postgres://u:hunter2@db/x'}\n---\nkind: List\nitems:\n- {kind: Secret, metadata: {name: more}, stringData: {API_URL: 'http://secret-api:2'}}\n- {kind: ConfigMap, metadata: {name: endpoints}, data: {API_URL: 'http://api:8080'}}\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata: {name: web}\nspec:\n  template:\n    metadata: {labels: {app: web}}\n    spec:\n      containers:\n      - name: web\n        image: web\n        envFrom: [{secretRef: {name: creds}}, {configMapRef: {name: endpoints}}]\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata: {name: api}\nspec:\n  template:\n    metadata: {labels: {app: api}}\n    spec:\n      containers: [{name: api, image: api}]\n---\napiVersion: v1\nkind: Service\nmetadata: {name: api}\nspec: {selector: {app: api}}\n").unwrap();
    let tool = tools_dir.path().join("tool");
    fs::write(
        &tool,
        format!("#!/bin/sh\n/bin/cat '{}'\n", output.display()),
    )
    .unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    put(&root, "k/kustomization.yaml", "resources: []\n");
    let tools = Tools {
        enabled: true,
        helm: None,
        kustomize: Some((tool, "build")),
    };
    let report = analyze(&root, &tools);
    assert!(
        report
            .dependencies
            .iter()
            .any(|e| e.from == "kubernetes:k:default:Deployment:web"
                && e.to == "kubernetes:k:default:Deployment:api"),
        "the ConfigMap is read: {report:#?}"
    );
    let json = serde_json::to_string(&report).unwrap();
    assert!(
        !json.contains("secret-api") && !json.contains("hunter2"),
        "{json}"
    );
}

#[cfg(unix)]
#[test]
fn remote_kustomizations_are_never_handed_to_the_tool() {
    let (_dir, root) = root();
    let tools_dir = tempfile::tempdir().unwrap();
    let tool = recording_tool(tools_dir.path());
    put(
        &root,
        "k/kustomization.yaml",
        "resources:\n- github.com/org/repo//base?ref=main\n",
    );
    let tools = Tools {
        enabled: true,
        helm: None,
        kustomize: Some((tool, "build")),
    };
    let report = analyze(&root, &tools);
    assert!(
        !tools_dir.path().join("RAN").exists(),
        "kustomize must not run"
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.file == "k/kustomization.yaml" && d.message.contains("remote")),
        "{:?}",
        report.diagnostics
    );
}

#[cfg(unix)]
#[test]
fn charts_with_symlinked_directories_leading_out_of_the_root_are_not_rendered() {
    let (_dir, root) = root();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.txt"), "x").unwrap();
    let tools_dir = tempfile::tempdir().unwrap();
    let tool = recording_tool(tools_dir.path());
    put(
        &root,
        "chart/Chart.yaml",
        "apiVersion: v2\nname: shop\nversion: 0.1.0\n",
    );
    fs::create_dir_all(root.join("chart/templates")).unwrap();
    fs::create_dir_all(root.join("shared")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.txt"),
        root.join("shared/x.yaml"),
    )
    .unwrap();
    std::os::unix::fs::symlink("../../shared", root.join("chart/templates/l")).unwrap();
    // A loop back into the chart must not hang the walk.
    std::os::unix::fs::symlink("..", root.join("shared/up")).unwrap();
    let tools = Tools {
        enabled: true,
        helm: Some(tool),
        kustomize: None,
    };
    let report = analyze(&root, &tools);
    assert!(!tools_dir.path().join("RAN").exists(), "helm must not run");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.file == "chart/Chart.yaml" && d.message.contains("x.yaml")),
        "{:?}",
        report.diagnostics
    );
}

#[cfg(unix)]
#[test]
fn charts_with_symlinks_out_of_the_root_are_not_rendered() {
    let (_dir, root) = root();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.txt"), "x").unwrap();
    let tools_dir = tempfile::tempdir().unwrap();
    let tool = recording_tool(tools_dir.path());
    put(
        &root,
        "chart/Chart.yaml",
        "apiVersion: v2\nname: shop\nversion: 0.1.0\n",
    );
    std::os::unix::fs::symlink(
        outside.path().join("secret.txt"),
        root.join("chart/link.txt"),
    )
    .unwrap();
    let tools = Tools {
        enabled: true,
        helm: Some(tool),
        kustomize: None,
    };
    let report = analyze(&root, &tools);
    assert!(!tools_dir.path().join("RAN").exists(), "helm must not run");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.file == "chart/Chart.yaml" && d.message.contains("chart/link.txt")),
        "{:?}",
        report.diagnostics
    );
}

#[cfg(unix)]
#[test]
fn output_over_the_budget_is_discarded_as_partial() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, root) = root();
    let tools_dir = tempfile::tempdir().unwrap();
    let tool = tools_dir.path().join("tool");
    fs::write(&tool, "#!/bin/sh\nexec /usr/bin/yes 'kind: ConfigMap'\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    put(&root, "k/kustomization.yaml", "resources: []\n");
    let tools = Tools {
        enabled: true,
        helm: None,
        kustomize: Some((tool, "build")),
    };
    let report = analyze(&root, &tools);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.file == "k/kustomization.yaml"
                && d.message.contains("budget exceeded")
                && d.message.contains("partial")),
        "{:?}",
        report.diagnostics
    );
    assert!(!report.complete);
}

// Integration tests with the real tools. Without the tool they print why they skip.

fn real_tools() -> Tools {
    let kustomize = executable("kustomize".as_ref())
        .map(|p| (p, "build"))
        .or_else(|| executable("kubectl".as_ref()).map(|p| (p, "kustomize")));
    Tools {
        enabled: true,
        helm: executable("helm".as_ref()),
        kustomize,
    }
}
const WEB: &str = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: {{ .Release.Name }}-web\nspec:\n  template:\n    metadata: {labels: {app: web}}\n    spec:\n      containers:\n      - name: web\n        image: web\n        env:\n        - {name: API_URL, value: \"http://{{ .Release.Name }}-api:{{ .Values.port }}\"}\n        - {name: DATABASE_URL, value: \"postgres://{{ .Release.Name }}-db:5432/shop\"}\n---\napiVersion: v1\nkind: Secret\nmetadata: {name: creds}\nstringData: {PAYMENT_URL: \"http://payments-secret-host:1\"}\n";
const API: &str = "apiVersion: apps/v1\nkind: Deployment\nmetadata: {name: {{ .Release.Name }}-api}\nspec:\n  template:\n    metadata: {labels: {app: api}}\n    spec:\n      containers: [{name: api, image: api}]\n---\napiVersion: v1\nkind: Service\nmetadata: {name: {{ .Release.Name }}-api}\nspec: {selector: {app: api}}\n";
const DB: &str = "apiVersion: apps/v1\nkind: StatefulSet\nmetadata: {name: {{ .Release.Name }}-db}\nspec:\n  template:\n    metadata: {labels: {app: db}}\n    spec:\n      containers: [{name: db, image: postgres:16}]\n---\napiVersion: v1\nkind: Service\nmetadata: {name: {{ .Release.Name }}-db}\nspec: {selector: {app: db}}\n";

#[test]
fn helm_template_renders_a_chart_with_a_vendored_subchart() {
    let tools = real_tools();
    if tools.helm.is_none() {
        eprintln!("Helm integration test skipped: helm not found on PATH");
        return;
    }
    let (_dir, root) = root();
    put(
        &root,
        "charts/shop/Chart.yaml",
        "apiVersion: v2\nname: shop\nversion: 0.1.0\ndependencies:\n- name: db\n  version: 0.1.0\n",
    );
    put(&root, "charts/shop/values.yaml", "port: 8080\n");
    put(&root, "charts/shop/templates/web.yaml", WEB);
    put(&root, "charts/shop/templates/api.yaml", API);
    put(
        &root,
        "charts/shop/charts/db/Chart.yaml",
        "apiVersion: v2\nname: db\nversion: 0.1.0\n",
    );
    put(&root, "charts/shop/charts/db/templates/db.yaml", DB);
    let report = analyze(&root, &tools);
    let web = "kubernetes:charts/shop:default:Deployment:shop-web";
    let api = "kubernetes:charts/shop:default:Deployment:shop-api";
    let link = report
        .dependencies
        .iter()
        .find(|e| e.from == web && e.to == api && e.kind == Kind::ConfiguredHttp)
        .unwrap_or_else(|| panic!("{:#?}", report));
    let evidence = &link.evidence[0];
    assert_eq!(evidence.file, "charts/shop/templates/web.yaml");
    assert!(
        evidence
            .description
            .contains("Rendered by helm template from charts/shop"),
        "{}",
        evidence.description
    );
    let db = "kubernetes:charts/shop:default:StatefulSet:shop-db";
    assert!(
        report
            .datastores
            .usages
            .iter()
            .any(|u| u.service == web && u.datastore == db),
        "subchart datastore in the chart scope: {:#?}",
        report.datastores
    );
    assert!(
        report
            .datastores
            .usages
            .iter()
            .flat_map(|u| &u.evidence)
            .all(|e| e.description.contains("Rendered by helm"))
    );
    let json = serde_json::to_string(&report).unwrap();
    assert!(
        !json.contains("payments-secret-host"),
        "Secrets are never read"
    );
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("unrendered template")),
        "templates are not read as plain YAML once rendered: {:?}",
        report.diagnostics
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.file == "charts/shop/Chart.yaml" && d.message.contains("lookup"))
    );
}

#[test]
fn helm_reports_missing_dependencies_without_fetching_them() {
    let tools = real_tools();
    if tools.helm.is_none() {
        eprintln!("Helm integration test skipped: helm not found on PATH");
        return;
    }
    let (_dir, root) = root();
    put(
        &root,
        "chart/Chart.yaml",
        "apiVersion: v2\nname: shop\nversion: 0.1.0\ndependencies:\n- name: redis\n  version: 1.0.0\n  repository: https://charts.example.invalid\n",
    );
    put(&root, "chart/templates/api.yaml", API);
    let report = analyze(&root, &tools);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.file == "chart/Chart.yaml" && d.message.contains("helm dependency build")),
        "{:?}",
        report.diagnostics
    );
    assert!(
        !root.join("chart/charts").exists(),
        "nothing was downloaded"
    );
    assert!(!report.services.iter().any(|s| s.name == "shop-api"));
}

#[test]
fn kustomize_renders_an_overlay_and_prefers_it_over_the_plain_base() {
    let tools = real_tools();
    if tools.kustomize.is_none() {
        eprintln!(
            "Kustomize integration test skipped: neither kustomize nor kubectl found on PATH"
        );
        return;
    }
    let (_dir, root) = root();
    let base_web = "apiVersion: apps/v1\nkind: Deployment\nmetadata: {name: web}\nspec:\n  template:\n    metadata: {labels: {app: web}}\n    spec:\n      containers:\n      - name: web\n        image: web\n        envFrom: [{configMapRef: {name: endpoints}}]\n";
    let base_api = "apiVersion: apps/v1\nkind: Deployment\nmetadata: {name: api}\nspec:\n  template:\n    metadata: {labels: {app: api}}\n    spec:\n      containers: [{name: api, image: api}]\n---\napiVersion: v1\nkind: Service\nmetadata: {name: api}\nspec: {selector: {app: api}}\n";
    put(
        &root,
        "deploy/base/kustomization.yaml",
        "resources: [web.yaml, api.yaml]\n",
    );
    put(&root, "deploy/base/web.yaml", base_web);
    put(&root, "deploy/base/api.yaml", base_api);
    put(
        &root,
        "deploy/overlays/prod/kustomization.yaml",
        "resources: [../../base]\nnamespace: shop\nnamePrefix: prod-\nconfigMapGenerator:\n- name: endpoints\n  literals: [API_URL=http://prod-api:8080]\nsecretGenerator:\n- name: creds\n  literals: [PAYMENT_URL=http://payments-secret-host:1]\n",
    );
    let report = analyze(&root, &tools);
    let scope = "kubernetes:deploy/overlays/prod:shop";
    let link = report
        .dependencies
        .iter()
        .find(|e| {
            e.from == format!("{scope}:Deployment:prod-web")
                && e.to == format!("{scope}:Deployment:prod-api")
        })
        .unwrap_or_else(|| panic!("{:#?}", report));
    assert!(
        link.evidence
            .iter()
            .all(|e| e.file == "deploy/overlays/prod/kustomization.yaml"
                && e.description.contains("from deploy/overlays/prod")),
        "{:?}",
        link.evidence
    );
    assert!(
        !report
            .services
            .iter()
            .any(|s| s.id.starts_with("kubernetes:deploy/base:")),
        "the base is read only through the overlay: {:?}",
        report.services
    );
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("payments-secret-host")
    );
}
