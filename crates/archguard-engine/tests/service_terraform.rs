//! Terraform HCL as static deployment evidence (ADR 0015).
use archguard_core::{ServiceDependencyKind as Kind, ServiceTopology};
use archguard_engine::analyze_services;
use std::path::Path;

fn fixture() -> ServiceTopology {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terraform-services");
    analyze_services(&root).unwrap()
}

fn usage<'a>(
    r: &'a ServiceTopology,
    service: &str,
    datastore: &str,
) -> &'a [archguard_core::ServiceEvidence] {
    &r.datastores
        .usages
        .iter()
        .find(|u| u.service == service && u.datastore == datastore)
        .unwrap_or_else(|| {
            panic!(
                "{service} -> {datastore} missing: {:#?}",
                r.datastores.usages
            )
        })
        .evidence
}

fn engine(r: &ServiceTopology, id: &str) -> Option<String> {
    r.datastores
        .nodes
        .iter()
        .find(|n| n.id == id)
        .unwrap_or_else(|| panic!("datastore {id} missing: {:#?}", r.datastores.nodes))
        .engine
        .clone()
}

fn gap(r: &ServiceTopology, file: &str, text: &str) -> bool {
    r.diagnostics
        .iter()
        .any(|d| d.file == file && d.message.contains(text))
}

#[test]
fn terraform_workloads_and_managed_datastores_become_topology_nodes() {
    let r = fixture();
    let ids: Vec<_> = r.services.iter().map(|s| s.id.as_str()).collect();
    for id in [
        "terraform:.:aws_ecs_service.orders",
        "terraform:.:aws_lambda_function.billing",
        "terraform:.:module.reports.google_cloud_run_v2_service.reports",
        "terraform:k8s:kubernetes_deployment.web",
        "terraform:k8s:kubernetes_deployment.backend",
        "terraform:azure:azurerm_linux_web_app.shop",
    ] {
        assert!(ids.contains(&id), "{id} missing: {ids:?}");
    }
    assert!(
        !ids.iter().any(|id| id.contains("aws_ecs_task_definition")
            || id.starts_with("terraform:modules/reports:")),
        "task definitions used by a service and called modules are no own workloads: {ids:?}"
    );
    assert_eq!(
        engine(&r, "terraform:.:aws_db_instance.orders").as_deref(),
        Some("postgresql")
    );
    assert_eq!(
        engine(&r, "terraform:.:aws_elasticache_replication_group.cache").as_deref(),
        Some("redis")
    );
    assert_eq!(
        engine(
            &r,
            "terraform:.:module.reports.google_sql_database_instance.reports"
        )
        .as_deref(),
        Some("postgresql")
    );
    assert_eq!(
        engine(
            &r,
            "terraform:azure:azurerm_postgresql_flexible_server.main"
        )
        .as_deref(),
        Some("postgresql")
    );
}

#[test]
fn terraform_references_link_workloads_to_datastores_and_services() {
    let r = fixture();
    let orders = "terraform:.:aws_ecs_service.orders";
    let db = usage(&r, orders, "terraform:.:aws_db_instance.orders");
    assert!(
        db.iter().any(|e| e.file == "main.tf"
            && e.line == Some(34)
            && e.description.starts_with(
                "Declared in Terraform resource aws_ecs_task_definition.orders (main.tf:34)"
            )),
        "{db:#?}"
    );
    usage(
        &r,
        orders,
        "terraform:.:aws_elasticache_replication_group.cache",
    );
    usage(
        &r,
        "terraform:.:module.reports.google_cloud_run_v2_service.reports",
        "terraform:.:module.reports.google_sql_database_instance.reports",
    );
    usage(
        &r,
        "terraform:azure:azurerm_linux_web_app.shop",
        "terraform:azure:azurerm_postgresql_flexible_server.main",
    );
    let edge = |from: &str, to: &str| {
        r.dependencies
            .iter()
            .find(|e| e.from == from && e.to == to)
            .unwrap_or_else(|| panic!("{from} -> {to} missing: {:#?}", r.dependencies))
    };
    assert_eq!(
        edge("terraform:.:aws_lambda_function.billing", orders).kind,
        Kind::ConfiguredReference
    );
    assert_eq!(
        edge(
            "terraform:k8s:kubernetes_deployment.web",
            "terraform:k8s:kubernetes_deployment.backend"
        )
        .kind,
        Kind::ConfiguredHttp
    );
}

#[test]
fn terraform_values_that_are_not_literal_stay_unknown_and_secrets_are_never_read() {
    let r = fixture();
    assert!(!r.complete);
    assert!(
        gap(&r, "main.tf", "var.payment_url"),
        "{:#?}",
        r.diagnostics
    );
    assert!(
        gap(&r, "main.tf", "aws_secretsmanager_secret_version.api"),
        "{:#?}",
        r.diagnostics
    );
    assert!(
        gap(&r, "main.tf", "terraform-aws-modules/vpc/aws"),
        "remote modules are gaps: {:#?}",
        r.diagnostics
    );
    assert!(
        !serde_json::to_string(&r)
            .unwrap()
            .contains("do-not-display")
    );
}

#[test]
fn heredoc_container_definitions_are_read_as_literal_json() {
    let r = fixture();
    let shipping = "terraform:.:aws_ecs_service.shipping";
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.message.contains("aws_ecs_task_definition.shipping")),
        "{:#?}",
        r.diagnostics
    );
    let usage = r
        .datastores
        .usages
        .iter()
        .find(|u| u.service == shipping)
        .unwrap_or_else(|| panic!("{:#?}", r.datastores.usages));
    assert_eq!(usage.database.as_deref(), Some("shipping"), "{usage:#?}");
}

fn put(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn module_nesting_beyond_the_limit_is_a_gap() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(root, "main.tf", "module \"m\" {\n  source = \"./m0\"\n}\n");
    for i in 0..12 {
        put(
            root,
            &format!("m{i}/main.tf"),
            &format!("module \"m\" {{\n  source = \"../m{}\"\n}}\n", i + 1),
        );
    }
    let r = analyze_services(root).unwrap();
    assert!(!r.complete);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.message.contains("nested deeper than")),
        "{:#?}",
        r.diagnostics
    );
}

#[test]
fn files_beyond_the_file_limit_are_a_gap() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for i in 0..2001 {
        put(root, &format!("f{i}.tf"), "");
    }
    let r = analyze_services(root).unwrap();
    assert!(!r.complete);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.message.contains("later .tf files are not read")),
        "{:#?}",
        r.diagnostics
    );
}
