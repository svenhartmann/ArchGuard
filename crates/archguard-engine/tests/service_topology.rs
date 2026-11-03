use archguard_engine::{analyze_services, core::ServiceDependencyKind};
use std::fs;

fn compose(text: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("compose.yaml"), text).unwrap();
    dir
}

#[test]
fn api_configuration_cycle_is_distinct_from_startup_dependencies() {
    let dir = compose(
        r#"
services:
  orders:
    build: ./orders
    environment:
      BILLING_URL: http://billing:8080
    depends_on: [db]
  billing:
    build: { context: ./billing }
    environment: ["ORDERS_URL=http://orders:8080/v1?token=secret"]
  db:
    image: postgres
"#,
    );
    fs::create_dir(dir.path().join("orders")).unwrap();
    fs::create_dir(dir.path().join("billing")).unwrap();
    let report = analyze_services(dir.path()).unwrap();
    assert!(report.monorepo);
    assert_eq!(report.services.len(), 3);
    assert_eq!(report.dependencies.len(), 3);
    assert_eq!(report.cycles.len(), 1);
    assert_eq!(report.cycles[0].kind, ServiceDependencyKind::ConfiguredHttp);
    assert_eq!(report.cycles[0].nodes.len(), 2);
    assert_eq!(
        report
            .dependencies
            .iter()
            .filter(|e| e.kind == ServiceDependencyKind::Startup)
            .count(),
        1
    );
    assert!(!serde_json::to_string(&report).unwrap().contains("secret"));
    assert_eq!(report, analyze_services(dir.path()).unwrap());
}

#[test]
fn missing_compose_is_an_empty_explicit_result() {
    let dir = tempfile::tempdir().unwrap();
    let report = analyze_services(dir.path()).unwrap();
    assert!(!report.monorepo);
    assert!(report.services.is_empty());
    assert!(report.compose_files.is_empty());
    assert!(!report.monorepo_reason.is_empty());
}

#[test]
fn aliases_are_network_scoped_and_ambiguous_hosts_are_not_edges() {
    let dir = compose(
        r#"
services:
  a:
    environment: {GOOD: 'http://api:80', WRONG_NETWORK: 'http://private-api', AMBIGUOUS: 'http://shared'}
    networks: [public]
  b:
    networks:
      public: {aliases: [api, shared]}
      private: {aliases: [private-api]}
  c:
    networks:
      public: {aliases: [shared]}
networks: {public: {}, private: {}}
"#,
    );
    let report = analyze_services(dir.path()).unwrap();
    assert_eq!(report.dependencies.len(), 1);
    assert_eq!(report.dependencies[0].to, "compose.yaml::b");
    assert!(!report.complete);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("ambiguous"))
    );
}

#[test]
fn interpolation_env_files_and_unsupported_models_are_visible_without_secret_expansion() {
    let dir = compose(
        r#"
include: [private.yaml]
services:
  a:
    env_file: .env
    environment: {API: '${API_URL:-http://b}', PASS_THROUGH: null}
    extends: {file: other.yaml, service: base}
  b:
    network_mode: host
"#,
    );
    fs::write(
        dir.path().join(".env"),
        "API_URL=http://b?SECRET=ENV_SECRET_DO_NOT_READ",
    )
    .unwrap();
    let report = analyze_services(dir.path()).unwrap();
    assert!(report.dependencies.is_empty());
    assert!(!report.complete);
    assert!(report.diagnostics.len() >= 4);
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("ENV_SECRET_DO_NOT_READ"));
    assert!(!json.contains("SECRET"));
}

#[test]
fn recursive_compose_projects_keep_distinct_service_identities() {
    let dir = tempfile::tempdir().unwrap();
    for stack in ["first", "second"] {
        fs::create_dir(dir.path().join(stack)).unwrap();
        fs::write(
            dir.path().join(stack).join("docker-compose.yml"),
            "services:\n  a:\n    environment: {API: 'http://b'}\n  b:\n    image: b\n",
        )
        .unwrap();
    }
    fs::create_dir(dir.path().join("node_modules")).unwrap();
    fs::write(dir.path().join("node_modules/compose.yaml"), "invalid: [").unwrap();
    let report = analyze_services(dir.path()).unwrap();
    assert_eq!(report.compose_files.len(), 2);
    assert_eq!(report.services.len(), 4);
    assert_eq!(report.dependencies.len(), 2);
    assert!(
        report
            .dependencies
            .iter()
            .all(|e| e.from.split("::").next() == e.to.split("::").next())
    );
}

#[test]
fn startup_cycle_is_not_an_api_cycle_and_self_endpoints_are_not_inter_service_cycles() {
    let dir = compose(
        "services:\n  a:\n    depends_on: {b: {condition: service_started}}\n    environment: {HEALTH: 'http://a'}\n  b:\n    depends_on: [a]\n",
    );
    let report = analyze_services(dir.path()).unwrap();
    assert_eq!(report.cycles.len(), 1);
    assert_eq!(report.cycles[0].kind, ServiceDependencyKind::Startup);
    assert_eq!(report.dependencies.len(), 2);
}

#[test]
fn malformed_yaml_and_overrides_report_partial_analysis() {
    let dir = compose("services: [");
    fs::write(
        dir.path().join("compose.override.yaml"),
        "services: {hidden: {image: b}}",
    )
    .unwrap();
    let report = analyze_services(dir.path()).unwrap();
    assert!(!report.complete);
    assert!(report.services.is_empty());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("YAML"))
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("override"))
    );
}

#[test]
fn yaml_anchors_merge_maps_and_block_scalars_are_parsed_as_yaml() {
    let dir = compose(
        r#"
x-common: &common
  environment:
    API: >-
      http://b:8080
services:
  a:
    <<: *common
  b:
    image: b
"#,
    );
    let report = analyze_services(dir.path()).unwrap();
    assert_eq!(report.dependencies.len(), 1);
    assert_eq!(
        report.dependencies[0].evidence[0].pointer,
        "/services/a/environment/API"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_manifests_and_outside_build_contexts_are_not_followed() {
    let dir = compose("services:\n  a:\n    build: ./outside\n  b:\n    build: ./local\n");
    let outside = tempfile::tempdir().unwrap();
    fs::write(
        outside.path().join("compose.yaml"),
        "services: {secret: {image: OUTSIDE_SECRET_DO_NOT_READ}}",
    )
    .unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("outside")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("compose.yaml"),
        dir.path().join("docker-compose.yml"),
    )
    .unwrap();
    fs::create_dir(dir.path().join("local")).unwrap();
    let report = analyze_services(dir.path()).unwrap();
    assert_eq!(report.services.len(), 2);
    assert_eq!(report.services[0].build_context, None);
    assert!(!report.monorepo);
    assert!(!report.complete);
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("OUTSIDE_SECRET_DO_NOT_READ")
    );
}

#[test]
fn named_external_networks_connect_projects_without_merging_default_networks() {
    let dir = tempfile::tempdir().unwrap();
    for (stack, service, target) in [("one", "a", "b"), ("two", "b", "a")] {
        fs::create_dir(dir.path().join(stack)).unwrap();
        fs::write(dir.path().join(stack).join("compose.yaml"),format!("services:\n  {service}:\n    environment: {{API: 'http://{target}'}}\n    networks: [shared]\nnetworks:\n  shared: {{external: true, name: shared-app}}\n")).unwrap();
    }
    let report = analyze_services(dir.path()).unwrap();
    assert_eq!(report.dependencies.len(), 2);
    assert_eq!(report.cycles.len(), 1);
    assert_eq!(report.cycles[0].kind, ServiceDependencyKind::ConfiguredHttp);
}

#[test]
fn extra_hosts_cannot_create_an_invented_service_edge_and_dns_is_case_insensitive() {
    let dir = compose(
        "services:\n  a:\n    extra_hosts: {b: '127.0.0.1'}\n    environment: {API: 'http://b'}\n  b:\n    environment: {API: 'https://UPPER-ALIAS:8080'}\n  c:\n    networks:\n      default: {aliases: [UPPER-ALIAS]}\n",
    );
    let report = analyze_services(dir.path()).unwrap();
    assert_eq!(report.dependencies.len(), 1);
    assert_eq!(report.dependencies[0].from, "compose.yaml::b");
    assert_eq!(report.dependencies[0].to, "compose.yaml::c");
    assert!(!report.complete);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("extra_hosts"))
    );
}

#[test]
fn yaml_alias_expansion_is_bounded_even_in_unused_extension_fields() {
    let mut source = String::from("x-a0: &a0 [value, value]\n");
    for i in 1..18 {
        source.push_str(&format!("x-a{i}: &a{i} [*a{}, *a{}]\n", i - 1, i - 1));
    }
    source.push_str("services: {a: {image: a}}\n");
    let dir = compose(&source);
    let report = analyze_services(dir.path()).unwrap();
    assert!(!report.complete);
    assert!(report.services.is_empty());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("YAML"))
    );
}

#[test]
fn shared_contexts_are_not_monorepo_evidence_and_metadata_gaps_are_explicit() {
    let dir = compose(
        "name: same-project\nservices:\n  a:\n    build: .\n    profiles: [dev]\n  b:\n    build: .\n",
    );
    let report = analyze_services(dir.path()).unwrap();
    assert!(!report.monorepo);
    assert!(!report.complete);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("project name"))
    );
}

#[test]
fn extra_hosts_does_not_remove_a_service_as_an_incoming_target() {
    let dir = compose(
        "services:\n  a:\n    extra_hosts: {b: '127.0.0.1'}\n    environment: {API: 'http://b'}\n  b:\n    environment: {API: 'http://a'}\n",
    );
    let report = analyze_services(dir.path()).unwrap();
    assert_eq!(report.dependencies.len(), 1);
    assert_eq!(report.dependencies[0].from, "compose.yaml::b");
    assert_eq!(report.dependencies[0].to, "compose.yaml::a");
}
