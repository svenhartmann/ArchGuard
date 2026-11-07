use archguard_core::ServiceDependencyKind as Kind;
use archguard_engine::analyze_services;
use std::{fs, path::Path};
use tempfile::tempdir;
fn put(root: &Path, path: &str, body: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}
#[test]
fn compose_dockerfile_selects_source_and_public_endpoint_defaults_resolve() {
    let d = tempdir().unwrap();
    put(d.path(), "src/client/go.mod", "module example.org/client\n");
    put(d.path(), "src/server/requirements.txt", "flask\n");
    put(d.path(), "src/client/Dockerfile", "FROM golang\n");
    put(d.path(), "src/server/Dockerfile", "FROM python\n");
    put(
        d.path(),
        ".env",
        "CLIENT_DOCKERFILE=./src/client/Dockerfile\nSERVER_DOCKERFILE=./src/server/Dockerfile\nSERVER_ADDR=server:${SERVER_PORT}\nSERVER_PORT=8080\nTOKEN=do-not-display\n",
    );
    put(
        d.path(),
        "compose.yaml",
        "services:\n  client:\n    build:\n      context: .\n      dockerfile: ${CLIENT_DOCKERFILE}\n    environment: [SERVER_ADDR, TOKEN]\n  server:\n    build:\n      context: .\n      dockerfile: ${SERVER_DOCKERFILE}\n",
    );
    let r = analyze_services(d.path()).unwrap();
    assert_eq!(
        r.services
            .iter()
            .find(|s| s.name == "client")
            .unwrap()
            .project
            .as_deref(),
        Some("project:src/client/go.mod")
    );
    assert!(
        r.dependencies
            .iter()
            .any(|e| e.from == "compose.yaml::client"
                && e.to == "compose.yaml::server"
                && e.kind == Kind::ConfiguredReference)
    );
    assert!(
        !serde_json::to_string(&r)
            .unwrap()
            .contains("do-not-display")
    );
}
#[test]
fn kubernetes_selectors_and_namespaces_resolve_declared_endpoints_only() {
    let d = tempdir().unwrap();
    put(d.path(), "client/go.mod", "module test/client\n");
    put(
        d.path(),
        "server/pyproject.toml",
        "[project]\nname='server'\n",
    );
    put(
        d.path(),
        "deploy.yaml",
        "kind: Deployment\nmetadata: {name: client, namespace: shop}\nspec:\n  template:\n    metadata:\n      labels: {app: client}\n    spec:\n      containers:\n      - name: client\n        image: client\n        env:\n        - {name: RPC_ADDR, value: 'public-api:5000'}\n        - {name: SECRET, valueFrom: {secretKeyRef: {name: credentials, key: url}}}\n---\nkind: Deployment\nmetadata: {name: backend, namespace: shop}\nspec:\n  template:\n    metadata:\n      labels: {app: backend}\n    spec:\n      containers:\n      - {name: api, image: server}\n---\nkind: Service\nmetadata: {name: public-api, namespace: shop}\nspec:\n  selector: {app: backend}\n",
    );
    let r = analyze_services(d.path()).unwrap();
    let client = r.services.iter().find(|s| s.name == "client").unwrap();
    let server = r.services.iter().find(|s| s.name == "backend").unwrap();
    assert_eq!(
        server.project.as_deref(),
        Some("project:server/pyproject.toml")
    );
    assert!(
        r.dependencies.iter().any(|e| e.from == client.id
            && e.to == server.id
            && e.kind == Kind::ConfiguredReference)
    );
    assert!(!r.complete);
}

#[test]
fn kubernetes_namespaces_do_not_merge_short_hostnames() {
    let d = tempdir().unwrap();
    put(d.path(), "client/go.mod", "module client\n");
    put(
        d.path(),
        "deploy.yaml",
        "kind: Deployment\nmetadata: {name: client, namespace: one}\nspec:\n  template:\n    spec:\n      containers:\n      - name: client\n        image: client\n        env: [{name: API_URL, value: 'http://api:8080'}]\n---\nkind: Deployment\nmetadata: {name: backend, namespace: two}\nspec:\n  template:\n    metadata: {labels: {app: backend}}\n    spec: {containers: [{name: api, image: backend}]}\n---\nkind: Service\nmetadata: {name: api, namespace: two}\nspec: {selector: {app: backend}}\n",
    );
    assert!(analyze_services(d.path()).unwrap().dependencies.is_empty());
}

const CONFIGMAP_BACKENDS: &str = "kind: Deployment\nmetadata: {name: orders, namespace: shop}\nspec:\n  template:\n    metadata: {labels: {app: orders}}\n    spec: {containers: [{name: api, image: orders}]}\n---\nkind: Service\nmetadata: {name: orders-svc, namespace: shop}\nspec: {selector: {app: orders}}\n---\nkind: Deployment\nmetadata: {name: stock, namespace: shop}\nspec:\n  template:\n    metadata: {labels: {app: stock}}\n    spec: {containers: [{name: api, image: stock}]}\n---\nkind: Service\nmetadata: {name: stock-svc, namespace: shop}\nspec: {selector: {app: stock}}\n";

#[test]
fn kubernetes_configmaps_in_the_same_bundle_resolve_endpoints() {
    let d = tempdir().unwrap();
    put(
        d.path(),
        "k8s/config.yaml",
        "kind: ConfigMap\nmetadata: {name: web-config, namespace: shop}\ndata:\n  ORDERS_URL: 'http://orders-svc:8080'\n  API_TOKEN: do-not-display\n---\napiVersion: v1\nkind: ConfigMap\nmetadata: {name: web-hosts, namespace: shop}\ndata: {stock: 'stock-svc.shop.svc.cluster.local:9000'}\n",
    );
    put(d.path(), "k8s/backends.yaml", CONFIGMAP_BACKENDS);
    put(
        d.path(),
        "k8s/web.yaml",
        "kind: Deployment\nmetadata: {name: web, namespace: shop}\nspec:\n  template:\n    spec:\n      containers:\n      - name: web\n        image: web\n        envFrom: [{configMapRef: {name: web-config}}]\n        env:\n        - name: STOCK_ADDR\n          valueFrom: {configMapKeyRef: {name: web-hosts, key: stock}}\n",
    );
    let r = analyze_services(d.path()).unwrap();
    let id = |name: &str| {
        r.services
            .iter()
            .find(|s| s.name == name)
            .unwrap()
            .id
            .clone()
    };
    let edge = |to: &str| {
        r.dependencies
            .iter()
            .find(|e| e.from == id("web") && e.to == id(to))
            .unwrap_or_else(|| panic!("web -> {to} missing: {:?}", r.dependencies))
    };
    let orders = edge("orders");
    assert_eq!(orders.kind, Kind::ConfiguredHttp);
    assert!(
        orders
            .evidence
            .iter()
            .any(|p| p.file == "k8s/config.yaml" && p.pointer == "/data/ORDERS_URL")
    );
    assert!(orders.evidence.iter().any(|p| p.file == "k8s/web.yaml"));
    let stock = edge("stock");
    assert_eq!(stock.kind, Kind::ConfiguredReference);
    assert!(
        stock
            .evidence
            .iter()
            .any(|p| p.file == "k8s/config.yaml" && p.pointer == "/data/stock")
    );
    assert!(
        !serde_json::to_string(&r)
            .unwrap()
            .contains("do-not-display")
    );
}

#[test]
fn kubernetes_configmaps_from_other_namespaces_or_bundles_stay_unresolved() {
    let d = tempdir().unwrap();
    put(
        d.path(),
        "k8s/config.yaml",
        "kind: ConfigMap\nmetadata: {name: web-config, namespace: other}\ndata: {ORDERS_URL: 'http://orders-svc:8080'}\n",
    );
    put(
        d.path(),
        "elsewhere/config.yaml",
        "kind: ConfigMap\nmetadata: {name: web-config, namespace: shop}\ndata: {ORDERS_URL: 'http://orders-svc:8080'}\n",
    );
    put(d.path(), "k8s/backends.yaml", CONFIGMAP_BACKENDS);
    put(
        d.path(),
        "k8s/web.yaml",
        "kind: Deployment\nmetadata: {name: web, namespace: shop}\nspec:\n  template:\n    spec: {containers: [{name: web, image: web, envFrom: [{configMapRef: {name: web-config}}]}]}\n",
    );
    let r = analyze_services(d.path()).unwrap();
    assert!(r.dependencies.is_empty(), "{:?}", r.dependencies);
    assert!(!r.complete);
}

#[test]
fn malformed_yaml_templates_terminate_without_inventing_services() {
    let d = tempdir().unwrap();
    put(
        d.path(),
        "templates/deploy.yaml",
        "{{ if .Values.enabled }}\nkind: Deployment\nmetadata:\n name: {{ .Release.Name }}\n{{ end }}",
    );
    let report = analyze_services(d.path()).unwrap();
    assert!(report.services.is_empty());
    assert!(!report.complete, "unrendered templates must be diagnosed");
}

#[test]
fn compose_unique_application_name_can_offer_inferred_source_navigation() {
    let d = tempdir().unwrap();
    put(
        d.path(),
        "config/pom.xml",
        r#"<project><artifactId>config</artifactId><build><plugins><plugin><groupId>org.springframework.boot</groupId><artifactId>spring-boot-maven-plugin</artifactId></plugin></plugins></build></project>"#,
    );
    put(
        d.path(),
        "compose.yaml",
        "services:\n  config:\n    image: company/system-config\n",
    );
    let r = analyze_services(d.path()).unwrap();
    assert_eq!(r.services.iter().filter(|s| s.name == "config").count(), 1);
    assert_eq!(
        r.services[0].project.as_deref(),
        Some("project:config/pom.xml")
    );
    assert!(r.diagnostics.iter().any(|d| d.message.contains("inferred")));
}

#[test]
fn unrendered_helm_charts_and_kustomize_overlays_explain_how_to_render_them() {
    if std::env::var_os("ARCHGUARD_RENDER_MANIFESTS").is_some() {
        eprintln!("Default-off rendering test skipped: ARCHGUARD_RENDER_MANIFESTS is set");
        return;
    }
    let d = tempdir().unwrap();
    put(
        d.path(),
        "charts/shop/Chart.yaml",
        "apiVersion: v2\nname: shop\nversion: 0.1.0\n",
    );
    put(
        d.path(),
        "deploy/overlays/prod/kustomization.yaml",
        "resources: [../../base]\n",
    );
    put(
        d.path(),
        "deploy/base/kustomization.yaml",
        "resources: [app.yaml]\n",
    );
    let r = analyze_services(d.path()).unwrap();
    for file in [
        "charts/shop/Chart.yaml",
        "deploy/overlays/prod/kustomization.yaml",
    ] {
        assert!(
            r.diagnostics
                .iter()
                .any(|g| g.file == file && g.message.contains("ARCHGUARD_RENDER_MANIFESTS=1")),
            "{file}: {:?}",
            r.diagnostics
        );
    }
    assert!(
        !r.diagnostics
            .iter()
            .any(|g| g.file == "deploy/base/kustomization.yaml"),
        "a base consumed by an overlay is reported with the overlay: {:?}",
        r.diagnostics
    );
    assert!(!r.complete);
}
