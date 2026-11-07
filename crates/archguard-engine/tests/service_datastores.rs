use archguard_engine::{
    analyze_services,
    core::{ServiceTopology, SharedDatastoreScope},
};
use std::{fs, path::Path};

fn put(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn compose(text: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "compose.yaml", text);
    dir
}

fn shared(
    report: &ServiceTopology,
    scope: SharedDatastoreScope,
) -> Vec<(String, Option<String>, Vec<String>)> {
    report
        .datastores
        .shared
        .iter()
        .filter(|s| s.scope == scope)
        .map(|s| (s.datastore.clone(), s.database.clone(), s.services.clone()))
        .collect()
}

fn ids(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn compose_services_with_the_same_database_share_it_and_credentials_stay_out() {
    let dir = compose(
        r#"
services:
  orders:
    image: demo/orders
    environment:
      DATABASE_URL: postgres://orders_user:hunter2@db:5432/shop?sslmode=disable
  billing:
    image: demo/billing
    environment: [DB_HOST=db, DB_PORT=5432, DB_NAME=shop, DB_USER=billing_user, DB_PASSWORD=hunter3]
  reporting:
    image: demo/reporting
    environment:
      SPRING_DATASOURCE_URL: jdbc:postgresql://db:5432/analytics
  db:
    image: postgres:16-alpine
    environment: {POSTGRES_DB: shop, POSTGRES_PASSWORD: hunter4}
"#,
    );
    let report = analyze_services(dir.path()).unwrap();
    let node = report
        .datastores
        .nodes
        .iter()
        .find(|n| n.id == "compose.yaml::db")
        .expect("postgres image is a datastore node");
    assert_eq!(node.engine.as_deref(), Some("postgresql"));
    assert_eq!(node.service.as_deref(), Some("compose.yaml::db"));
    assert_eq!(
        shared(&report, SharedDatastoreScope::Database),
        vec![(
            "compose.yaml::db".to_owned(),
            Some("shop".to_owned()),
            ids(&["compose.yaml::billing", "compose.yaml::orders"])
        )]
    );
    assert_eq!(
        shared(&report, SharedDatastoreScope::Server),
        vec![(
            "compose.yaml::db".to_owned(),
            None,
            ids(&[
                "compose.yaml::billing",
                "compose.yaml::orders",
                "compose.yaml::reporting"
            ])
        )]
    );
    assert!(
        report
            .datastores
            .usages
            .iter()
            .all(|u| !u.evidence.is_empty())
    );
    assert!(!report.datastores.note.is_empty());
    let json = serde_json::to_string(&report).unwrap();
    for leaked in [
        "hunter2",
        "hunter3",
        "hunter4",
        "orders_user",
        "billing_user",
        "sslmode",
    ] {
        assert!(!json.contains(leaked), "{leaked} leaked into the topology");
    }
    assert_eq!(report, analyze_services(dir.path()).unwrap());
}

#[test]
fn same_server_with_different_databases_is_only_a_shared_server() {
    let dir = compose(
        r#"
services:
  orders:
    environment: {MYSQL_HOST: mysql, MYSQL_DATABASE: orders}
  billing:
    environment: {MYSQL_HOST: mysql, MYSQL_DATABASE: billing}
  mysql:
    image: docker.io/library/mysql:8
  audit:
    environment: {AUDIT_DB_URL: 'mysql://audit:pw@db.example.com/audit'}
  ledger:
    environment: {LEDGER_DB_URL: 'jdbc:mysql://db.example.com:3306/audit'}
  local:
    environment: {DATABASE_URL: 'postgres://localhost:5432/scratch'}
  other:
    environment: {DATABASE_URL: 'postgres://127.0.0.1:5432/scratch'}
"#,
    );
    let report = analyze_services(dir.path()).unwrap();
    assert_eq!(
        shared(&report, SharedDatastoreScope::Server),
        vec![(
            "compose.yaml::mysql".to_owned(),
            None,
            ids(&["compose.yaml::billing", "compose.yaml::orders"])
        )]
    );
    // An external host with the same database name and default port is one database.
    assert_eq!(
        shared(&report, SharedDatastoreScope::Database),
        vec![(
            "external:db.example.com:3306".to_owned(),
            Some("audit".to_owned()),
            ids(&["compose.yaml::audit", "compose.yaml::ledger"])
        )]
    );
    let external = report
        .datastores
        .nodes
        .iter()
        .find(|n| n.id == "external:db.example.com:3306")
        .unwrap();
    assert_eq!(external.service, None);
    assert_eq!(external.host.as_deref(), Some("db.example.com"));
    assert_eq!(external.port, Some(3306));
    assert_eq!(external.engine.as_deref(), Some("mysql"));
    // Loopback hosts name each container itself, never a shared server.
    assert!(
        report
            .datastores
            .usages
            .iter()
            .all(|u| !u.datastore.contains("localhost") && !u.datastore.contains("127.0.0.1"))
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("loopback"))
    );
    assert!(!serde_json::to_string(&report).unwrap().contains("audit:pw"));
}

#[test]
fn kubernetes_datastore_references_resolve_and_secrets_stay_unknown() {
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        "deploy/shop.yaml",
        r#"apiVersion: apps/v1
kind: StatefulSet
metadata: {name: postgres, namespace: shop}
spec:
  template:
    metadata: {labels: {app: postgres}}
    spec:
      containers: [{name: db, image: "bitnami/postgresql:16"}]
---
apiVersion: v1
kind: Service
metadata: {name: postgres, namespace: shop}
spec: {selector: {app: postgres}}
---
apiVersion: v1
kind: ConfigMap
metadata: {name: orders-db, namespace: shop}
data: {DB_HOST: postgres, DB_NAME: shop}
---
apiVersion: apps/v1
kind: Deployment
metadata: {name: orders, namespace: shop}
spec:
  template:
    spec:
      containers:
        - name: orders
          image: demo/orders
          envFrom: [{configMapRef: {name: orders-db}}]
---
apiVersion: apps/v1
kind: Deployment
metadata: {name: catalog, namespace: shop}
spec:
  template:
    spec:
      containers:
        - name: catalog
          image: demo/catalog
          env:
            - {name: DATABASE_URL, value: "postgresql://cat:s3cr3t@postgres.shop.svc.cluster.local:5432/shop"}
---
apiVersion: apps/v1
kind: Deployment
metadata: {name: billing, namespace: shop}
spec:
  template:
    spec:
      containers:
        - name: billing
          image: demo/billing
          env:
            - name: DATABASE_URL
              valueFrom: {secretKeyRef: {name: billing-db, key: url}}
"#,
    );
    let report = analyze_services(dir.path()).unwrap();
    let postgres = "kubernetes:deploy:shop:StatefulSet:postgres";
    assert!(
        report
            .datastores
            .nodes
            .iter()
            .any(|n| n.id == postgres && n.engine.as_deref() == Some("postgresql"))
    );
    assert_eq!(
        shared(&report, SharedDatastoreScope::Database),
        vec![(
            postgres.to_owned(),
            Some("shop".to_owned()),
            ids(&[
                "kubernetes:deploy:shop:Deployment:catalog",
                "kubernetes:deploy:shop:Deployment:orders"
            ])
        )]
    );
    assert!(
        report
            .datastores
            .usages
            .iter()
            .all(|u| !u.service.ends_with(":billing"))
    );
    assert!(report.diagnostics.iter().any(|d| {
        d.service.as_deref() == Some("billing")
            && d.message.contains("DATABASE_URL")
            && d.message.contains("Secret")
    }));
    let orders = report
        .datastores
        .usages
        .iter()
        .find(|u| u.service.ends_with(":orders"))
        .unwrap();
    assert!(orders.evidence.iter().any(|e| e.pointer == "/data/DB_HOST"));
    assert!(!serde_json::to_string(&report).unwrap().contains("s3cr3t"));
}

#[test]
fn spring_datasource_urls_count_as_datastore_references() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["orders", "billing"] {
        put(
            dir.path(),
            &format!("{name}/pom.xml"),
            &format!("<project><artifactId>{name}</artifactId></project>"),
        );
        put(
            dir.path(),
            &format!("{name}/src/main/resources/application.yml"),
            &format!(
                "spring:\n  application:\n    name: {name}\n  datasource:\n    url: jdbc:postgresql://db:5432/shop\n    password: hunter5\n"
            ),
        );
    }
    put(
        dir.path(),
        "compose.yml",
        "services:\n  orders:\n    image: demo/orders\n  billing:\n    image: demo/billing\n  db:\n    image: postgres\n",
    );
    let report = analyze_services(dir.path()).unwrap();
    assert_eq!(
        shared(&report, SharedDatastoreScope::Database),
        vec![(
            "compose.yml::db".to_owned(),
            Some("shop".to_owned()),
            ids(&["compose.yml::billing", "compose.yml::orders"])
        )]
    );
    assert!(!serde_json::to_string(&report).unwrap().contains("hunter5"));
}
