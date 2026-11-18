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
