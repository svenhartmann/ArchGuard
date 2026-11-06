//! Static service topology; no Docker/build execution, process environment or secret resolution.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use support::{Discovery, diagnostic, relative};

use archguard_core::{
    ArchguardError, Edge, Graph, Node, NodeId, NodeKind, Service, ServiceCycle, ServiceDependency,
    ServiceDependencyKind, ServiceEvidence, ServiceTopology,
};
use serde_yaml_ng::Value;

#[path = "service_clients.rs"]
mod service_clients;
#[path = "service_communications.rs"]
mod service_communications;
#[path = "service_datastore_settings.rs"]
mod service_datastore_settings;
#[path = "service_datastores.rs"]
mod service_datastores;
#[path = "service_deployments.rs"]
mod service_deployments;
#[path = "service_spring.rs"]
mod service_spring;
#[path = "service_traces.rs"]
mod service_traces;
#[path = "service_support.rs"]
pub(crate) mod support;

const COMPOSE_NAMES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];
const MAX_FILES: usize = 200;
const MAX_ENTRIES: usize = 50_000;
const MAX_SERVICES: usize = 2_000;
const MAX_BYTES: u64 = 1_048_576;

struct ParsedService {
    service: Service,
    value: Value,
    // Actual network identity -> hostnames on that network. Networks with an
    // explicit name may be shared across Compose projects; defaults never are.
    hosts: BTreeMap<String, BTreeSet<String>>,
}

/// Discover Compose projects and their explicit startup/configured HTTP edges.
/// Configuration is read as YAML, with anchor/merge support. Includes, overlays,
/// env files and interpolation are reported as gaps rather than executed.
pub fn analyze_services(root: &Path) -> Result<ServiceTopology, ArchguardError> {
    let root = fs::canonicalize(root)?;
    if !root.is_dir() {
        return Err(ArchguardError::Config(
            "service analysis root must be a directory".into(),
        ));
    }
    let mut report = ServiceTopology {
        schema_version: 1,
        root: root.display().to_string(),
        monorepo: false,
        monorepo_reason: "No Compose files discovered; no Compose-backed monorepo evidence.".into(),
        complete: true,
        ..Default::default()
    };
    let discovery = Discovery::new(&root, &mut report)?;
    let mut files = Vec::new();
    discover(&root, &discovery, &mut files, &mut report)?;
    files.sort();
    let mut parsed = Vec::new();
    for path in files {
        let file = relative(&root, &path);
        report.compose_files.push(file.clone());
        parse_compose(&root, &path, &file, &mut parsed, &mut report)?;
    }
    parsed.sort_by(|a, b| a.service.id.cmp(&b.service.id));
    let mut dependencies: BTreeMap<
        (String, String, ServiceDependencyKind),
        BTreeSet<ServiceEvidence>,
    > = BTreeMap::new();
    let local_names: BTreeMap<(String, String), String> = parsed
        .iter()
        .map(|s| {
            (
                (s.service.compose_file.clone(), s.service.name.clone()),
                s.service.id.clone(),
            )
        })
        .collect();
    let mut network_hosts: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for source in &parsed {
        for (network, hosts) in &source.hosts {
            for host in hosts {
                network_hosts
                    .entry((network.clone(), host.clone()))
                    .or_default()
                    .insert(source.service.id.clone());
            }
        }
    }
    for source in &parsed {
        collect_startup(source, &local_names, &mut dependencies, &mut report);
        collect_http(source, &network_hosts, &mut dependencies, &mut report);
    }
    report.services = parsed.into_iter().map(|p| p.service).collect();
    report.dependencies = dependencies
        .into_iter()
        .map(|((from, to, kind), evidence)| ServiceDependency {
            from,
            to,
            kind,
            evidence: evidence.into_iter().collect(),
        })
        .collect();
    crate::service_projects::augment(&root, &discovery, &mut report)?;
    let deployments = service_deployments::augment(&root, &discovery, &mut report);
    service_communications::augment(&root, &mut report);
    let spring_datasources = service_spring::augment(&root, &mut report);
    service_clients::augment(&root, &mut report, &deployments);
    service_datastores::augment(&mut report, &deployments, &spring_datasources);
    service_traces::augment(&root, &mut report);
    if report
        .projects
        .iter()
        .any(|p| matches!(p.ecosystem.as_str(), "go" | "python" | "gradle"))
    {
        diagnostic(
            &mut report,
            ".",
            None,
            "Communication coverage is partial: Go calls use bounded local value flow; Python/Gradle source clients, dynamic endpoints, generated deployment templates and runtime overrides are not fully resolved. Missing edges do not establish isolation.",
        );
    }
    report.cycles = service_cycles(&report.services, &report.dependencies);
    let contexts: BTreeSet<&str> = report
        .services
        .iter()
        .filter(|s| !s.compose_file.is_empty())
        .filter_map(|s| s.build_context.as_deref())
        .collect();
    let project_paths: BTreeSet<_> = report
        .projects
        .iter()
        .filter(|project| project.kind != "workspace")
        .map(|project| project.path.as_str())
        .collect();
    report.monorepo = project_paths.len() >= 2 || contexts.len() >= 2;
    report.monorepo_reason = format!(
        "{} distinct manifest project roots (excluding workspace aggregators), {} verified local Compose build contexts; at least two project roots or build contexts establish monorepo evidence.",
        project_paths.len(),
        contexts.len()
    );
    report.diagnostics.sort();
    report.diagnostics.dedup();
    Ok(report)
}
fn discover(
    root: &Path,
    discovery: &Discovery,
    files: &mut Vec<PathBuf>,
    report: &mut ServiceTopology,
) -> Result<(), ArchguardError> {
    let mut candidates: BTreeMap<PathBuf, Vec<(usize, PathBuf)>> = BTreeMap::new();
    for (index, entry) in discovery.walk(root).enumerate() {
        if index >= MAX_ENTRIES {
            diagnostic(report, ".", None, "Compose discovery entry limit reached.");
            break;
        }
        let entry = entry.map_err(|error| std::io::Error::other(error.to_string()))?;
        let name = entry.file_name().to_string_lossy();
        let file = relative(root, entry.path());
        if entry.depth() == 64 && entry.file_type().is_some_and(|kind| kind.is_dir()) {
            diagnostic(
                report,
                &file,
                None,
                "Compose discovery depth limit reached.",
            );
        }
        if entry.file_type().is_some_and(|kind| kind.is_symlink()) {
            if COMPOSE_NAMES.contains(&name.as_ref()) || compose_variant(&name) {
                diagnostic(report, &file, None, "Symlinked Compose file was not read.");
            }
            continue;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if let Some(priority) = COMPOSE_NAMES.iter().position(|n| *n == name) {
            candidates
                .entry(entry.path().parent().unwrap_or(root).to_path_buf())
                .or_default()
                .push((priority, entry.path().to_path_buf()));
        } else if compose_variant(&name) {
            diagnostic(
                report,
                &file,
                None,
                "Compose override/variant file was not merged; only the preferred base file per directory is analyzed.",
            );
        }
    }
    for mut paths in candidates.into_values() {
        if files.len() >= MAX_FILES {
            diagnostic(
                report,
                ".",
                None,
                "Compose discovery limit reached; remaining paths were not analyzed.",
            );
            break;
        }
        paths.sort();
        files.push(paths[0].1.clone());
        for (_, path) in paths.iter().skip(1) {
            diagnostic(
                report,
                &relative(root, path),
                None,
                "Alternative Compose base file was not merged; the preferred base file in this directory is analyzed.",
            );
        }
    }
    Ok(())
}
fn compose_variant(name: &str) -> bool {
    (name.starts_with("compose.") || name.starts_with("docker-compose."))
        && (name.ends_with(".yaml") || name.ends_with(".yml"))
}
fn value<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or(&Value::Null)
}
fn text(v: &Value) -> Option<&str> {
    v.as_str()
}
fn pointer(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}
fn literal_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}
fn strings(v: &Value) -> Option<Vec<String>> {
    v.as_sequence()?
        .iter()
        .map(|item| text(item).map(str::to_owned))
        .collect()
}
fn safe_context(root: &Path, base: &Path, context: &str) -> Option<String> {
    if context.contains('$') || context.contains("://") || context.starts_with("git@") {
        return None;
    }
    let raw = base.join(context);
    let mut clean = PathBuf::new();
    for c in raw.components() {
        match c {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            other => clean.push(other.as_os_str()),
        }
    }
    let rel = clean.strip_prefix(root).ok()?;
    let mut current = root.to_path_buf();
    for c in rel.components() {
        current.push(c);
        if fs::symlink_metadata(&current)
            .ok()?
            .file_type()
            .is_symlink()
        {
            return None;
        }
    }
    if !current.is_dir() {
        return None;
    }
    Some(if rel.as_os_str().is_empty() {
        ".".into()
    } else {
        relative(root, &current)
    })
}

fn parse_compose(
    root: &Path,
    path: &Path,
    file: &str,
    parsed: &mut Vec<ParsedService>,
    report: &mut ServiceTopology,
) -> Result<(), ArchguardError> {
    if fs::metadata(path)?.len() > MAX_BYTES {
        diagnostic(
            report,
            file,
            None,
            "Compose file exceeds the 1 MiB analysis limit.",
        );
        return Ok(());
    }
    let mut source = String::new();
    if fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_string(&mut source)
        .is_err()
    {
        diagnostic(
            report,
            file,
            None,
            "Compose file could not be read as UTF-8.",
        );
        return Ok(());
    }
    if source.len() as u64 > MAX_BYTES {
        diagnostic(
            report,
            file,
            None,
            "Compose file exceeds the 1 MiB analysis limit.",
        );
        return Ok(());
    }
    let mut doc: Value = match serde_yaml_ng::from_str(&source) {
        Ok(v) => v,
        Err(_) => {
            diagnostic(
                report,
                file,
                None,
                "Invalid or unsupported YAML; this Compose file was not analyzed.",
            );
            return Ok(());
        }
    };
    if doc.apply_merge().is_err() {
        diagnostic(
            report,
            file,
            None,
            "Unsupported YAML merge; this Compose file was not analyzed.",
        );
        return Ok(());
    }
    if !value(&doc, "name").is_null() {
        diagnostic(
            report,
            file,
            None,
            "Declared Compose project name is not used to merge separate files; shared implicit networks across those files are not resolved.",
        );
    }
    if !value(&doc, "include").is_null() {
        diagnostic(report, file, None, "Compose include is not expanded.");
    }
    let Some(services) = value(&doc, "services").as_mapping() else {
        diagnostic(report, file, None, "Compose services must be a mapping.");
        return Ok(());
    };
    let mut sorted: Vec<_> = services.iter().collect();
    sorted.sort_by_key(|(k, _)| text(k).unwrap_or_default());
    for (key, definition) in sorted {
        if parsed.len() >= MAX_SERVICES {
            diagnostic(
                report,
                file,
                None,
                "Service count limit reached; remaining services were not analyzed.",
            );
            break;
        }
        let Some(name) = text(key).filter(|name| literal_name(name)) else {
            diagnostic(
                report,
                file,
                None,
                "Invalid or interpolated service name was not analyzed.",
            );
            continue;
        };
        if !definition.is_mapping() {
            diagnostic(
                report,
                file,
                Some(name),
                "Service definition must be a mapping.",
            );
            continue;
        }
        for key in ["extends", "env_file", "configs", "links", "external_links"] {
            if !value(definition, key).is_null() {
                diagnostic(
                    report,
                    file,
                    Some(name),
                    format!("Service {key} is not expanded; dependencies may be missing."),
                );
            }
        }
        let build = value(definition, "build");
        let context = if build.is_mapping() {
            text(value(build, "context"))
        } else {
            text(build)
        };
        let build_context =
            context.and_then(|c| safe_context(root, path.parent().unwrap_or(root), c));
        if !build.is_null() && build_context.is_none() {
            diagnostic(
                report,
                file,
                Some(name),
                "Build context is unresolved, missing, remote, symlinked or outside the analysis root.",
            );
        }
        let profiles = strings(value(definition, "profiles")).unwrap_or_default();
        if !value(definition, "profiles").is_null() {
            diagnostic(
                report,
                file,
                Some(name),
                "All declared profiles are considered together; runtime profile activation is not evaluated.",
            );
        }
        let (networks, hosts) =
            service_networks(file, name, definition, value(&doc, "networks"), report);
        parsed.push(ParsedService {
            service: Service {
                id: format!("{file}::{name}"),
                name: name.into(),
                compose_file: file.into(),
                build_context,
                networks,
                profiles,
                project: None,
            },
            value: definition.clone(),
            hosts,
        });
    }
    Ok(())
}

fn service_networks(
    file: &str,
    name: &str,
    service: &Value,
    top: &Value,
    report: &mut ServiceTopology,
) -> (Vec<String>, BTreeMap<String, BTreeSet<String>>) {
    if !value(service, "network_mode").is_null() {
        diagnostic(
            report,
            file,
            Some(name),
            "network_mode is not resolved; HTTP hostname dependencies for this service are omitted.",
        );
        return (Vec::new(), BTreeMap::new());
    }
    let configured = value(service, "networks");
    let entries: Vec<(String, &Value)> = if configured.is_null()
        || configured.as_sequence().is_some_and(Vec::is_empty)
        || configured.as_mapping().is_some_and(|m| m.is_empty())
    {
        vec![("default".into(), &Value::Null)]
    } else if let Some(mapping) = configured.as_mapping() {
        mapping
            .iter()
            .filter_map(|(key, v)| text(key).map(|k| (k.to_owned(), v)))
            .collect()
    } else if let Some(names) = strings(configured) {
        names.into_iter().map(|n| (n, &Value::Null)).collect()
    } else {
        diagnostic(
            report,
            file,
            Some(name),
            "Unsupported service networks definition.",
        );
        return (Vec::new(), BTreeMap::new());
    };
    let mut networks = Vec::new();
    let mut hosts = BTreeMap::new();
    for (network, options) in entries {
        if !literal_name(&network) {
            diagnostic(
                report,
                file,
                Some(name),
                "Invalid or interpolated network name was omitted.",
            );
            continue;
        }
        let definition = value(top, &network);
        if network != "default"
            && definition.is_null()
            && !top
                .as_mapping()
                .is_some_and(|m| m.contains_key(Value::String(network.clone())))
        {
            diagnostic(
                report,
                file,
                Some(name),
                "Service refers to an undeclared network.",
            );
            continue;
        }
        let explicit_name = value(definition, "name");
        let external = value(definition, "external").as_bool() == Some(true);
        let identity = if let Some(explicit) = text(explicit_name) {
            if !literal_name(explicit) {
                diagnostic(
                    report,
                    file,
                    Some(name),
                    "Interpolated or unsupported network identity was omitted.",
                );
                continue;
            }
            format!("named:{explicit}")
        } else if external {
            format!("named:{network}")
        } else {
            format!("{file}::{network}")
        };
        let mut aliases = BTreeSet::from([name.to_ascii_lowercase()]);
        if let Some(container) = text(value(service, "container_name")) {
            if literal_name(container) {
                aliases.insert(container.to_ascii_lowercase());
            } else {
                diagnostic(
                    report,
                    file,
                    Some(name),
                    "Interpolated container name is not resolved.",
                );
            }
        }
        let raw_aliases = value(options, "aliases");
        if !raw_aliases.is_null() {
            match strings(raw_aliases) {
                Some(items) => {
                    for alias in items {
                        if literal_name(&alias) {
                            aliases.insert(alias.to_ascii_lowercase());
                        } else {
                            diagnostic(
                                report,
                                file,
                                Some(name),
                                "Interpolated or invalid network alias was omitted.",
                            );
                        }
                    }
                }
                None => diagnostic(
                    report,
                    file,
                    Some(name),
                    "Unsupported network aliases definition.",
                ),
            }
        }
        networks.push(network);
        hosts
            .entry(identity)
            .or_insert_with(BTreeSet::new)
            .extend(aliases);
    }
    networks.sort();
    networks.dedup();
    (networks, hosts)
}

type Dependencies = BTreeMap<(String, String, ServiceDependencyKind), BTreeSet<ServiceEvidence>>;
fn add_edge(
    edges: &mut Dependencies,
    source: &Service,
    to: &str,
    kind: ServiceDependencyKind,
    pointer: String,
    description: String,
) {
    if source.id == to && kind == ServiceDependencyKind::ConfiguredHttp {
        return;
    }
    edges
        .entry((source.id.clone(), to.into(), kind))
        .or_default()
        .insert(ServiceEvidence {
            file: source.compose_file.clone(),
            pointer,
            line: None,
            description,
        });
}
fn collect_startup(
    source: &ParsedService,
    names: &BTreeMap<(String, String), String>,
    edges: &mut Dependencies,
    report: &mut ServiceTopology,
) {
    let deps = value(&source.value, "depends_on");
    if deps.is_null() {
        return;
    }
    let names_list = if let Some(map) = deps.as_mapping() {
        map.keys()
            .map(|k| text(k).map(str::to_owned))
            .collect::<Option<Vec<_>>>()
    } else {
        strings(deps)
    };
    let Some(names_list) = names_list else {
        diagnostic(
            report,
            &source.service.compose_file,
            Some(&source.service.name),
            "Unsupported depends_on definition.",
        );
        return;
    };
    for name in names_list {
        if let Some(to) = names.get(&(source.service.compose_file.clone(), name.clone())) {
            add_edge(
                edges,
                &source.service,
                to,
                ServiceDependencyKind::Startup,
                format!("/services/{}/depends_on", pointer(&source.service.name)),
                "Declared startup dependency (not evidence of an API request).".into(),
            );
        } else {
            diagnostic(
                report,
                &source.service.compose_file,
                Some(&source.service.name),
                "depends_on target is unknown or unresolved in this Compose project.",
            );
        }
    }
}
fn collect_http(
    source: &ParsedService,
    hosts: &BTreeMap<(String, String), BTreeSet<String>>,
    edges: &mut Dependencies,
    report: &mut ServiceTopology,
) {
    if !value(&source.value, "extra_hosts").is_null() {
        diagnostic(
            report,
            &source.service.compose_file,
            Some(&source.service.name),
            "extra_hosts can override service DNS; outgoing HTTP hostname dependencies for this service are omitted.",
        );
        return;
    }
    let env = value(&source.value, "environment");
    if env.is_null() {
        return;
    }
    let prefix = format!("/services/{}/environment", pointer(&source.service.name));
    let entries: Vec<(String, Option<&str>)> = if let Some(map) = env.as_mapping() {
        map.iter()
            .filter_map(|(key, value)| {
                text(key).map(|key| (format!("{prefix}/{}", pointer(key)), text(value)))
            })
            .collect()
    } else if let Some(seq) = env.as_sequence() {
        seq.iter()
            .enumerate()
            .map(|(i, v)| {
                (
                    format!("{prefix}/{i}"),
                    text(v).and_then(|v| v.split_once('=').map(|(_, v)| v)),
                )
            })
            .collect()
    } else {
        diagnostic(
            report,
            &source.service.compose_file,
            Some(&source.service.name),
            "Unsupported environment definition.",
        );
        return;
    };
    for (pointer, raw) in entries {
        let Some(raw) = raw else {
            diagnostic(
                report,
                &source.service.compose_file,
                Some(&source.service.name),
                "Environment value is inherited or non-string; it is not resolved.",
            );
            continue;
        };
        if raw.contains('$') {
            diagnostic(
                report,
                &source.service.compose_file,
                Some(&source.service.name),
                "Environment interpolation is not expanded; configured endpoints may be missing.",
            );
            continue;
        }
        for host in http_hosts(raw) {
            let targets: BTreeSet<String> = source
                .hosts
                .keys()
                .flat_map(|network| {
                    hosts
                        .get(&(network.clone(), host.clone()))
                        .into_iter()
                        .flatten()
                        .cloned()
                })
                .collect();
            if targets.len() == 1 {
                add_edge(edges,&source.service,targets.first().unwrap(),ServiceDependencyKind::ConfiguredHttp,pointer.clone(),"Literal HTTP(S) endpoint configured for a service on a shared Compose network; no runtime call is proven. URL values are omitted.".into());
            } else if targets.len() > 1 {
                diagnostic(
                    report,
                    &source.service.compose_file,
                    Some(&source.service.name),
                    "Configured HTTP hostname is ambiguous on shared networks; no target was guessed.",
                );
            } else {
                diagnostic(
                    report,
                    &source.service.compose_file,
                    Some(&source.service.name),
                    "Configured HTTP hostname has no uniquely reachable service in the discovered Compose networks (external, isolated, or unresolved).",
                );
            }
        }
    }
}
fn http_hosts(raw: &str) -> BTreeSet<String> {
    let mut hosts = BTreeSet::new();
    for scheme in ["http://", "https://"] {
        for (_, tail) in raw
            .match_indices(scheme)
            .map(|(i, s)| (s, &raw[i + s.len()..]))
        {
            let authority = tail
                .split(|c: char| {
                    matches!(
                        c,
                        '/' | '?' | '#' | '\"' | '\'' | '<' | '>' | '{' | '}' | '[' | ']'
                    ) || c.is_whitespace()
                })
                .next()
                .unwrap_or_default();
            let authority = authority.rsplit('@').next().unwrap_or_default();
            let host = authority
                .split(':')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if literal_name(&host) {
                hosts.insert(host);
            }
        }
    }
    hosts
}
fn service_cycles(services: &[Service], dependencies: &[ServiceDependency]) -> Vec<ServiceCycle> {
    let indices: BTreeMap<&str, NodeId> = services
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), NodeId(i as u32)))
        .collect();
    let mut graph = Graph {
        semantic: None,
        review_kpis: None,
        schema_version: 1,
        root: String::new(),
        nodes: services
            .iter()
            .enumerate()
            .map(|(i, s)| Node {
                id: NodeId(i as u32),
                name: s.id.clone(),
                kind: NodeKind::File,
                parent: None,
                lang: None,
            })
            .collect(),
        edges: Vec::new(),
    };
    let mut cycles = Vec::new();
    for kind in [
        ServiceDependencyKind::Startup,
        ServiceDependencyKind::ConfiguredHttp,
        ServiceDependencyKind::HttpCall,
        ServiceDependencyKind::GrpcCall,
        ServiceDependencyKind::Event,
        ServiceDependencyKind::ConfiguredReference,
    ] {
        graph.edges = dependencies
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| Edge {
                from: indices[e.from.as_str()],
                to: indices[e.to.as_str()],
                refs: Vec::new(),
                count: Some(1),
            })
            .collect();
        for scc in archguard_analysis::find_sccs(&graph) {
            cycles.push(ServiceCycle {
                kind,
                nodes: scc
                    .node_ids
                    .into_iter()
                    .map(|id| services[id.0 as usize].id.clone())
                    .collect(),
            });
        }
    }
    cycles
}
