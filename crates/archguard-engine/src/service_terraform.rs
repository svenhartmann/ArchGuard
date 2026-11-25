//! Terraform HCL as static deployment evidence (ADR 0015). `*.tf` files are parsed only:
//! no `terraform` binary, providers, state or cloud access. Variables, locals, module
//! outputs and data sources are never evaluated, and secret resources are never read.
use super::service_datastore_settings as datastores;
use super::service_deployments::{Deployment, Env, add, endpoint_key, project_for, read};
use archguard_core::{Service, ServiceDependencyKind as Kind, ServiceEvidence, ServiceTopology};
use archguard_flow_common::support::{Discovery, diagnostic};
use archguard_terraform as terraform;
use archguard_terraform::hcl::{References, attr, block, get, http, labels, literal, object};
use archguard_terraform::module::{Resource, TfFile};
use hcl_edit::expr::Expression;
use hcl_edit::structure::Body;
use hcl_edit::visit::Visit;
use hcl_edit::{Span, parser};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const WORKLOADS: [&str; 12] = [
    "kubernetes_deployment",
    "kubernetes_deployment_v1",
    "kubernetes_stateful_set",
    "kubernetes_stateful_set_v1",
    "aws_ecs_service",
    "aws_ecs_task_definition",
    "aws_lambda_function",
    "google_cloud_run_v2_service",
    "google_cloud_run_service",
    "azurerm_container_app",
    "azurerm_linux_web_app",
    "azurerm_windows_web_app",
];
const K8S_SERVICES: [&str; 2] = ["kubernetes_service", "kubernetes_service_v1"];

/// One environment entry; `value` is `None` when it comes from a secret or `value_from`.
struct Entry {
    key: String,
    value: Option<Expression>,
    line: Option<u32>,
}

struct Workload<'r, 'a> {
    resource: &'r Resource<'a>,
    /// The resource declaring the environment: the task definition of an ECS service.
    declared_in: &'r Resource<'a>,
    id: String,
    name: String,
    entries: Vec<Entry>,
    images: Vec<String>,
    aliases: BTreeSet<String>,
}

const LITERAL: &str =
    "Literal value read statically; no terraform run, state, variables or cloud access.";
const REFERENCE: &str = "Resource reference resolved statically within the same Terraform module; no state or cloud access.";
const STORE: &str = "Managed datastore resource; only its engine setting is read.";

/// Workloads and managed datastores declared in Terraform, as deployments.
pub(super) fn deployments(
    root: &Path,
    discovery: &Discovery,
    report: &mut ServiceTopology,
    // Called once per module instance; keeps readers of other resource families out of this file.
    each_instance: &mut dyn FnMut(&mut ServiceTopology, &str, &[Resource]),
) -> Vec<Deployment> {
    let mut out = Vec::new();
    terraform::module::walk(
        root,
        discovery,
        report,
        &read,
        &mut |report, scope, resources| {
            each_instance(report, scope, resources);
            module_instance(report, scope, resources, &mut out);
        },
    );
    out
}

/// Environment entries and images anywhere in a workload body: `env` blocks
/// (Kubernetes, Cloud Run, Container Apps), Lambda `variables` and `app_settings`.
fn collect(file: &TfFile, body: &Body, entries: &mut Vec<Entry>, images: &mut Vec<String>) {
    for structure in body.iter() {
        if let Some(a) = structure.as_attribute() {
            match a.key.as_str() {
                "image" | "image_uri" => images.extend(literal(&a.value)),
                "variables" | "app_settings" if matches!(a.value, Expression::Object(_)) => {
                    for (key, value) in object(&a.value) {
                        entries.push(Entry {
                            key,
                            value: Some(value.clone()),
                            line: file.line(value.span()),
                        });
                    }
                }
                key @ ("variables" | "app_settings") => entries.push(Entry {
                    key: key.into(),
                    value: Some(a.value.clone()),
                    line: file.line(a.span()),
                }),
                _ => {}
            }
        } else if let Some(b) = structure.as_block() {
            if b.ident.as_str() != "env" {
                collect(file, &b.body, entries, images);
            } else if let Some(key) = attr(&b.body, "name").and_then(literal) {
                entries.push(Entry {
                    key,
                    value: attr(&b.body, "value").cloned(),
                    line: file.line(b.span()),
                });
            }
        }
    }
}

/// Images and `environment` of ECS `container_definitions`, given as `jsonencode(...)`
/// or literal JSON. `None` when the definitions are computed.
fn ecs_containers(resource: &Resource) -> Option<(Vec<Entry>, Vec<String>)> {
    let expr = attr(&resource.block.body, "container_definitions")?;
    let parsed;
    let (definitions, spans) = match expr {
        Expression::FuncCall(call) if call.name.name.as_str() == "jsonencode" => {
            (call.args.iter().next()?, true)
        }
        other => {
            // JSON is valid HCL expression syntax; its spans are relative to the string.
            // A heredoc ends with a newline, which `parse_expr` rejects.
            parsed = parser::parse_expr(literal(other)?.trim()).ok()?;
            (&parsed, false)
        }
    };
    let Expression::Array(items) = definitions else {
        return None;
    };
    let line = |e: &Expression| {
        if spans {
            resource.file.line(e.span())
        } else {
            resource.line()
        }
    };
    let (mut entries, mut images) = (Vec::new(), Vec::new());
    for item in items.iter() {
        images.extend(get(item, "image").and_then(literal));
        let environment = get(item, "environment");
        let list = match environment {
            Some(Expression::Array(list)) => list.iter().collect(),
            _ => vec![],
        };
        for entry in list {
            if let Some(key) = get(entry, "name").and_then(literal) {
                entries.push(Entry {
                    key,
                    value: get(entry, "value").cloned(),
                    line: line(entry),
                });
            }
        }
    }
    Some((entries, images))
}

/// Engines of an `engine` or `database_version` setting, matched by prefix in order.
const ENGINE_PREFIXES: [(&str, &str); 10] = [
    ("aurora-postgresql", "postgresql"),
    ("postgres", "postgresql"),
    ("aurora", "mysql"),
    ("mysql", "mysql"),
    ("mariadb", "mariadb"),
    ("sqlserver", "sqlserver"),
    ("oracle", "oracle"),
    ("redis", "redis"),
    ("valkey", "redis"),
    ("memcached", "memcached"),
];
/// Datastore resources whose engine comes from their type.
const FIXED_ENGINES: [(&str, &str); 8] = [
    ("aws_dynamodb_table", "dynamodb"),
    ("google_redis_instance", "redis"),
    ("azurerm_redis_cache", "redis"),
    ("azurerm_postgresql_flexible_server", "postgresql"),
    ("azurerm_postgresql_server", "postgresql"),
    ("azurerm_mysql_flexible_server", "mysql"),
    ("azurerm_mssql_server", "sqlserver"),
    // `engine` is optional here and defaults to Redis.
    ("aws_elasticache_replication_group", "redis"),
];
/// Datastore resources and the setting naming their engine.
const ENGINE_SETTINGS: [(&str, &str); 6] = [
    ("aws_db_instance", "engine"),
    ("aws_rds_cluster", "engine"),
    ("aws_elasticache_cluster", "engine"),
    ("aws_elasticache_replication_group", "engine"),
    ("aws_elasticache_serverless_cache", "engine"),
    ("google_sql_database_instance", "database_version"),
];

/// The managed datastore engine of a resource: `Some(None)` for a datastore whose
/// engine is not literal or not known, `None` for any other resource.
fn store_engine(resource: &Resource) -> Option<Option<&'static str>> {
    let fixed = FIXED_ENGINES.iter().find(|(k, _)| *k == resource.kind);
    let setting = ENGINE_SETTINGS.iter().find(|(k, _)| *k == resource.kind);
    if fixed.is_none() && setting.is_none() {
        return None;
    }
    let Some(declared) = setting.and_then(|(_, key)| attr(&resource.block.body, key)) else {
        return Some(fixed.map(|(_, engine)| *engine));
    };
    Some(literal(declared).and_then(|value| {
        let value = value.to_ascii_lowercase();
        ENGINE_PREFIXES
            .iter()
            .find_map(|(prefix, engine)| value.starts_with(prefix).then_some(*engine))
    }))
}

/// Resource address -> (deployment id, is a datastore).
type Targets = BTreeMap<String, (String, bool)>;

/// The module prefix of a resource address, e.g. `module.db.`.
fn module_prefix<'r>(r: &'r Resource) -> &'r str {
    &r.address[..r.address.len() - r.kind.len() - r.name.len() - 1]
}

fn module_instance(
    report: &mut ServiceTopology,
    scope: &str,
    resources: &[Resource],
    out: &mut Vec<Deployment>,
) {
    let mut targets = Targets::new();
    for (r, engine) in resources.iter().filter_map(|r| Some((r, store_engine(r)?))) {
        let id = format!("{scope}:{}", r.address);
        targets.insert(r.address.clone(), (id.clone(), true));
        out.push(Deployment {
            id,
            scope: scope.into(),
            file: r.file.path.clone(),
            store: Some((engine, r.evidence(r.line(), "", STORE))),
            ..Default::default()
        });
    }
    let task_of = ecs_tasks(resources);
    let run_by_service: BTreeSet<&String> = task_of.values().collect();
    let mut workloads = Vec::new();
    let candidates = resources
        .iter()
        .filter(|r| WORKLOADS.contains(&r.kind) && !run_by_service.contains(&r.address));
    for r in candidates {
        let task = match r.kind {
            "aws_ecs_service" => task_of
                .get(&r.address)
                .and_then(|a| resources.iter().find(|t| &t.address == a)),
            "aws_ecs_task_definition" => Some(r),
            _ => None,
        };
        let w = new_workload(report, scope, r, task);
        targets.insert(r.address.clone(), (w.id.clone(), false));
        workloads.push(w);
    }
    kubernetes_services(resources, &mut workloads, &mut targets);
    for w in workloads {
        workload(report, scope, w, &targets, out);
    }
}

/// ECS service address -> address of the task definition of this module it runs.
fn ecs_tasks(resources: &[Resource]) -> BTreeMap<String, String> {
    let mut task_of = BTreeMap::new();
    for r in resources.iter().filter(|r| r.kind == "aws_ecs_service") {
        let mut refs = References::default();
        if let Some(e) = attr(&r.block.body, "task_definition") {
            refs.visit_expr(e);
        }
        let task = refs
            .0
            .into_iter()
            .find(|p| p.len() >= 2 && p[0] == "aws_ecs_task_definition");
        if let Some(task) = task {
            let address = format!("{}{}.{}", module_prefix(r), task[0], task[1]);
            task_of.insert(r.address.clone(), address);
        }
    }
    task_of
}

/// A workload with its environment entries and images; `task` is the ECS task definition.
fn new_workload<'r, 'a>(
    report: &mut ServiceTopology,
    scope: &str,
    r: &'r Resource<'a>,
    task: Option<&'r Resource<'a>>,
) -> Workload<'r, 'a> {
    let body = &r.block.body;
    let (mut entries, mut images) = (Vec::new(), Vec::new());
    let problem = match task {
        _ if !r.kind.starts_with("aws_ecs_") => {
            collect(r.file, body, &mut entries, &mut images);
            None
        }
        Some(task) => match ecs_containers(task) {
            Some(found) => {
                (entries, images) = found;
                None
            }
            None => Some(format!(
                "{}: container_definitions is neither literal JSON nor jsonencode(...); its containers are unknown.",
                task.address
            )),
        },
        None => Some(format!(
            "{}: task_definition does not reference a task definition of this module; its containers are unknown.",
            r.address
        )),
    };
    if let Some(message) = problem {
        diagnostic(report, &r.file.path, Some(r.name), message);
    }
    let name = block(body, "metadata")
        .and_then(|m| attr(m, "name"))
        .or_else(|| attr(body, "name"))
        .or_else(|| attr(body, "function_name"))
        .and_then(literal)
        .unwrap_or_else(|| r.name.to_owned());
    Workload {
        resource: r,
        declared_in: task.unwrap_or(r),
        id: format!("{scope}:{}", r.address),
        name,
        entries,
        images,
        aliases: BTreeSet::new(),
    }
}

fn namespace(body: &Body) -> String {
    block(body, "metadata")
        .and_then(|m| attr(m, "namespace"))
        .and_then(literal)
        .unwrap_or_else(|| "default".into())
}

/// Kubernetes Services select workloads by labels: their names become cluster hostnames
/// of the one selected workload, and references to them resolve to it.
fn kubernetes_services(
    resources: &[Resource],
    workloads: &mut [Workload],
    targets: &mut BTreeMap<String, (String, bool)>,
) {
    for service in resources.iter().filter(|r| K8S_SERVICES.contains(&r.kind)) {
        let body = &service.block.body;
        let selector = labels(block(body, "spec").and_then(|s| attr(s, "selector")));
        let ns = namespace(body);
        let name = block(body, "metadata")
            .and_then(|m| attr(m, "name"))
            .and_then(literal);
        let mut selected: Vec<_> = workloads
            .iter_mut()
            .filter(|w| {
                let body = &w.resource.block.body;
                let pod = block(body, "spec")
                    .and_then(|s| block(s, "template"))
                    .and_then(|t| block(t, "metadata"))
                    .and_then(|m| attr(m, "labels"));
                let pod = labels(pod);
                w.resource.kind.starts_with("kubernetes_")
                    && namespace(body) == ns
                    && !selector.is_empty()
                    && selector.iter().all(|(k, v)| pod.get(k) == Some(v))
            })
            .collect();
        let [workload] = selected.as_mut_slice() else {
            continue;
        };
        targets.insert(service.address.clone(), (workload.id.clone(), false));
        if let Some(host) = name {
            let host = host.to_ascii_lowercase();
            workload.aliases.extend([
                format!("{host}.{ns}"),
                format!("{host}.{ns}.svc"),
                format!("{host}.{ns}.svc.cluster.local"),
                host,
            ]);
        }
    }
}

/// Registers the workload as a service unless it is already known.
fn register(report: &mut ServiceTopology, scope: &str, w: &Workload) {
    let project = match w.images.as_slice() {
        [image] => project_for(report, image, &w.name),
        _ => None,
    };
    if !report.services.iter().any(|s| s.id == w.id) {
        report.services.push(Service {
            id: w.id.clone(),
            name: w.name.clone(),
            compose_file: w.resource.file.path.clone(),
            build_context: None,
            networks: vec![scope.into()],
            profiles: vec![],
            project,
        });
    }
}

/// Registers the workload as a service and resolves its environment: literal values as
/// endpoints, resource references as links, everything else as a gap.
fn workload(
    report: &mut ServiceTopology,
    scope: &str,
    w: Workload,
    targets: &Targets,
    out: &mut Vec<Deployment>,
) {
    register(report, scope, &w);
    let mut resolver = Resolver {
        report,
        w: &w,
        targets,
        env: Env::default(),
        store_links: Vec::new(),
    };
    for e in &w.entries {
        resolver.entry(e);
    }
    let Resolver {
        env, store_links, ..
    } = resolver;
    let r = w.resource;
    let images = w
        .images
        .iter()
        .map(|image| {
            (
                format!("/{}/image", r.address.replace('.', "/")),
                image.clone(),
            )
        })
        .collect();
    out.push(Deployment {
        id: w.id.clone(),
        scope: scope.into(),
        file: r.file.path.clone(),
        env: env.values,
        origins: env.origins,
        aliases: w.aliases.clone(),
        images,
        datastore_env: env.datastore,
        store_links,
        ..Default::default()
    });
}

/// Roots of references that Terraform evaluates and ArchGuard does not.
const EVALUATED: [&str; 9] = [
    "var",
    "local",
    "module",
    "data",
    "each",
    "count",
    "self",
    "path",
    "terraform",
];

/// Resolves the environment entries of one workload.
struct Resolver<'x, 'r, 'a> {
    report: &'x mut ServiceTopology,
    w: &'x Workload<'r, 'a>,
    targets: &'x Targets,
    env: Env,
    store_links: Vec<(String, ServiceEvidence)>,
}

impl Resolver<'_, '_, '_> {
    fn gap(&mut self, key: &str, reason: String) {
        let r = self.w.resource;
        let message = format!("{}: {key} {reason}", r.address);
        diagnostic(self.report, &r.file.path, Some(&self.w.name), message);
    }

    fn entry(&mut self, e: &Entry) {
        let key = &e.key;
        let relevant = endpoint_key(key)
            || datastores::setting_name(key)
            || matches!(key.as_str(), "variables" | "app_settings");
        let pointer = format!("/env/{}", key.replace('~', "~0").replace('/', "~1"));
        let Some(value) = &e.value else {
            if relevant {
                self.gap(
                    key,
                    "comes from a secret or value_from, which is never read; its value is unknown."
                        .into(),
                );
            }
            return;
        };
        if let Some(text) = literal(value) {
            let evidence = self.w.declared_in.evidence(e.line, &pointer, LITERAL);
            self.env.insert(key.clone(), &text, vec![evidence]);
            return;
        }
        let mut refs = References::default();
        refs.visit_expr(value);
        if refs.0.is_empty() && relevant {
            self.gap(key, "is computed by a Terraform expression, which is not evaluated; its value is unknown.".into());
        }
        for path in refs.0 {
            let evidence = self.w.declared_in.evidence(e.line, &pointer, REFERENCE);
            self.reference(key, value, &path, relevant, evidence);
        }
    }

    /// One reference `path` in the value of `key`.
    fn reference(
        &mut self,
        key: &str,
        value: &Expression,
        path: &[String],
        relevant: bool,
        evidence: ServiceEvidence,
    ) {
        let n = if path[0] == "data" { 3 } else { 2 };
        let shown = path[..path.len().min(n)].join(".");
        if EVALUATED.contains(&path[0].as_str()) || path.len() < 2 {
            if relevant {
                self.gap(key, format!("uses {shown}; variables, locals, module outputs and data sources are not evaluated, so its value is unknown."));
            }
            return;
        }
        let address = format!("{}{shown}", module_prefix(self.w.resource));
        match self.targets.get(&address) {
            Some((id, true)) => self.store_links.push((id.clone(), evidence)),
            Some((id, false)) => {
                let kind = if http(value) {
                    Kind::ConfiguredHttp
                } else {
                    Kind::ConfiguredReference
                };
                add(self.report, &self.w.id, id, kind, vec![evidence]);
            }
            None if path[0].contains("secret") => self.gap(
                key,
                format!("references {shown}, a secret that is never read; its value is unknown."),
            ),
            None if relevant => self.gap(
                key,
                format!("references {shown}, which is no workload or datastore of this module; the target is unknown."),
            ),
            None => {}
        }
    }
}
