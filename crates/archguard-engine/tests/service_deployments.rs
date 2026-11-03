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
