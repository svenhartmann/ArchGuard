//! Bounded static deployment declarations. Never contacts a cluster or executes interpolation.
use super::support::{diagnostic, relative};
use archguard_core::{
    Service, ServiceDependency, ServiceDependencyKind as Kind, ServiceEvidence, ServiceTopology,
};
use serde::Deserialize;
use serde_yaml_ng::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path},
};

#[derive(Clone)]
pub(super) struct Deployment {
    pub id: String,
    pub scope: String,
    pub file: String,
    pub env: BTreeMap<String, String>,
    pub aliases: BTreeSet<String>,
}
pub(super) fn read(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    if relative
        .components()
        .any(|p| !matches!(p, Component::Normal(_)))
    {
        return None;
    }
    crate::service_projects::read_manifest(root, relative).ok()
}
fn endpoint_key(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    ![
        "SECRET",
        "PASSWORD",
        "TOKEN",
        "CREDENTIAL",
        "PRIVATE",
        "KEY",
    ]
    .iter()
    .any(|x| k.contains(x))
        && ["ADDR", "HOST", "URL", "URI", "ENDPOINT", "BROKER", "PORT"]
            .iter()
            .any(|x| k.contains(x))
}
fn public_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() < 512
        && !value.contains('@')
        && !value.contains("$(")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/+${}?=".contains(&b))
}
fn defaults(root: &Path, base: &Path) -> BTreeMap<String, String> {
    read(root, &base.join(".env"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (k, v) = line.trim().split_once('=')?;
            let v = v.trim().trim_matches(['\'', '"']);
            ((endpoint_key(k) || k.ends_with("_DOCKERFILE")) && public_value(v))
                .then(|| (k.to_owned(), v.to_owned()))
        })
        .collect()
}
fn expand(value: &str, vars: &BTreeMap<String, String>) -> Option<String> {
    let mut result = value.to_owned();
    for _ in 0..8 {
        let Some(at) = result.find("${") else {
            return public_value(&result).then_some(result);
        };
        let end = at + result[at..].find('}')?;
        let key = &result[at + 2..end];
        let (key, fallback) = key
            .split_once(":-")
            .map_or((key, None), |(k, v)| (k, Some(v)));
        let replacement = vars.get(key).map(String::as_str).or(fallback)?.to_owned();
        result.replace_range(at..=end, &replacement);
        if result.len() > 1024 {
            return None;
        }
    }
    None
}
pub(super) fn host(value: &str) -> Option<String> {
    if !public_value(value) || value.contains('$') {
        return None;
    }
    let value = value.split_once("://").map_or(value, |(_, v)| v);
    let host = value.split([':', '/', '?', '#']).next()?;
    (!host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)))
    .then(|| host.to_ascii_lowercase())
}
pub(super) fn proof(file: &str, pointer: &str, description: &str) -> ServiceEvidence {
    ServiceEvidence {
        file: file.into(),
        pointer: pointer.into(),
        line: None,
        description: description.into(),
    }
}
pub(super) fn add(
    report: &mut ServiceTopology,
    from: &str,
    to: &str,
    kind: Kind,
    evidence: Vec<ServiceEvidence>,
) {
    if from == to {
        return;
    }
    if let Some(e) = report
        .dependencies
        .iter_mut()
        .find(|e| e.from == from && e.to == to && e.kind == kind)
    {
        e.evidence.extend(evidence);
        e.evidence.sort();
        e.evidence.dedup();
    } else {
        report.dependencies.push(ServiceDependency {
            from: from.into(),
            to: to.into(),
            kind,
            evidence,
        });
    }
}
fn project_for(report: &ServiceTopology, image: &str, name: &str) -> Option<String> {
    let image = image
        .rsplit('/')
        .next()
        .unwrap_or(image)
        .split([':', '@'])
        .next()
        .unwrap_or("");
    let candidates: Vec<_> = report
        .projects
        .iter()
        .filter(|p| {
            !matches!(p.kind.as_str(), "workspace" | "tooling")
                && (p.name == image
                    || Path::new(&p.path).file_name().and_then(|p| p.to_str()) == Some(image)
                    || p.name == name)
        })
        .collect();
    (candidates.len() == 1).then(|| candidates[0].id.clone())
}
fn yaml(root: &Path, file: &str, report: &mut ServiceTopology) -> Vec<Value> {
    let Some(text) = read(root, &root.join(file)) else {
        diagnostic(
            report,
            file,
            None,
            "YAML deployment input could not be read within the safe file limits.",
        );
        return vec![];
    };
    let mut result = Vec::new();
    for doc in serde_yaml_ng::Deserializer::from_str(&text).take(2000) {
        match Value::deserialize(doc) {
            Ok(value) => result.push(value),
            Err(_) => {
                diagnostic(
                    report,
                    file,
                    None,
                    "YAML is malformed or is an unrendered template; deployment coverage is incomplete. Templates are not executed.",
                );
                break;
            }
        }
    }
    result
}
fn string(v: &Value) -> Option<&str> {
    v.as_str()
}
fn labels(v: &Value) -> BTreeMap<String, String> {
    v.as_mapping()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| Some((k.as_str()?.into(), v.as_str()?.into())))
        .collect()
}
pub(super) fn augment(
    root: &Path,
    discovery: &crate::services::Discovery,
    report: &mut ServiceTopology,
) -> Vec<Deployment> {
    let mut out = Vec::new();
    for file in report.compose_files.clone() {
        let base = root.join(&file).parent().unwrap().to_path_buf();
        let vars = defaults(root, &base);
        for mut doc in yaml(root, &file, report) {
            let _ = doc.apply_merge();
            for (key, value) in doc["services"].as_mapping().into_iter().flatten() {
                let Some(name) = key.as_str() else { continue };
                let id = format!("{file}::{name}");
                let Some(index) = report.services.iter().position(|s| s.id == id) else {
                    continue;
                };
                if let Some(dockerfile) = value["build"]["dockerfile"]
                    .as_str()
                    .and_then(|s| expand(s, &vars))
                    && let Some(context) = report.services[index].build_context.as_ref()
                {
                    let path = root.join(context).join(&dockerfile);
                    if let Ok(path) = fs::canonicalize(&path)
                        && path.starts_with(root)
                        && read(root, &path).is_some()
                    {
                        let mut candidates: Vec<_> = report
                            .projects
                            .iter()
                            .filter(|p| {
                                !matches!(p.kind.as_str(), "workspace" | "tooling")
                                    && p.path != "."
                                    && path.starts_with(root.join(&p.path))
                            })
                            .collect();
                        candidates.sort_by_key(|p| std::cmp::Reverse(p.path.len()));
                        if let Some(p) = candidates.first().filter(|p| {
                            candidates
                                .get(1)
                                .is_none_or(|q| q.path.len() < p.path.len())
                        }) {
                            report.services[index].project = Some(p.id.clone());
                        }
                    }
                }
                if report.services[index].project.is_none()
                    && report.services[index].build_context.is_none()
                {
                    let candidates: Vec<_> = report
                        .projects
                        .iter()
                        .filter(|p| p.kind == "application" && p.name == name)
                        .map(|p| p.id.clone())
                        .collect();
                    if candidates.len() == 1 {
                        report.services[index].project = Some(candidates[0].clone());
                        diagnostic(
                            report,
                            &file,
                            Some(name),
                            "Source navigation is inferred from a unique application/service name match; image contents are not verified.",
                        );
                    }
                }
                let mut env = BTreeMap::new();
                let raw = &value["environment"];
                if let Some(mapping) = raw.as_mapping() {
                    for (k, v) in mapping {
                        if let Some(k) = k.as_str().filter(|k| endpoint_key(k))
                            && let Some(v) = v
                                .as_str()
                                .or_else(|| vars.get(k).map(String::as_str))
                                .and_then(|v| expand(v, &vars))
                        {
                            env.insert(k.into(), v);
                        }
                    }
                }
                if let Some(list) = raw.as_sequence() {
                    for entry in list.iter().filter_map(string) {
                        let (k, v) = entry
                            .split_once('=')
                            .map_or((entry, None), |(k, v)| (k, Some(v)));
                        if endpoint_key(k)
                            && let Some(v) = v
                                .or_else(|| vars.get(k).map(String::as_str))
                                .and_then(|v| expand(v, &vars))
                        {
                            env.insert(k.into(), v);
                        }
                    }
                }
                out.push(Deployment {
                    id,
                    scope: file.clone(),
                    file: file.clone(),
                    env,
                    aliases: BTreeSet::from([name.to_ascii_lowercase()]),
                });
            }
        }
    }
    // Workloads and selectors, never Helm evaluation, Secret reads or cluster queries.
    let mut documents = Vec::new();
    let mut budget = 0usize;
    let walker = discovery.walk(root);
    for entry in walker.take(50_000).flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file())
            || !matches!(
                entry.path().extension().and_then(|e| e.to_str()),
                Some("yaml" | "yml")
            )
        {
            continue;
        }
        let file = relative(root, entry.path());
        if report.compose_files.contains(&file) {
            continue;
        }
        budget += entry.metadata().map(|m| m.len() as usize).unwrap_or(0);
        if budget > 32_000_000 || documents.len() >= 2000 {
            diagnostic(
                report,
                &file,
                None,
                "Deployment discovery input limit reached.",
            );
            break;
        }
        for doc in yaml(root, &file, report) {
            if doc["kind"].as_str() == Some("List") {
                if let Some(items) = doc["items"].as_sequence() {
                    documents.extend(items.iter().cloned().map(|d| (file.clone(), d)));
                }
            } else {
                documents.push((file.clone(), doc));
            }
        }
    }
    let mut workloads = BTreeMap::new();
    for (file, doc) in &documents {
        if !matches!(
            doc["kind"].as_str(),
            Some("Deployment" | "StatefulSet" | "DaemonSet" | "Job" | "CronJob" | "Pod")
        ) {
            continue;
        }
        let Some(name) = doc["metadata"]["name"].as_str() else {
            continue;
        };
        let namespace = doc["metadata"]["namespace"].as_str().unwrap_or("default");
        if name.contains(['$', '{']) || namespace.contains(['$', '{']) {
            continue;
        }
        let pod = match doc["kind"].as_str() {
            Some("Pod") => doc,
            Some("CronJob") => &doc["spec"]["jobTemplate"]["spec"]["template"],
            _ => &doc["spec"]["template"],
        };
        let bundle = Path::new(file)
            .parent()
            .and_then(Path::to_str)
            .unwrap_or("");
        let scope = format!("kubernetes:{bundle}:{namespace}");
        let id = format!("{scope}:{}:{name}", doc["kind"].as_str().unwrap());
        // Repeated manifests may be variants: only identical declarations are merged.
        if let Some(previous) = workloads.get(&id) {
            if previous != doc {
                diagnostic(
                    report,
                    file,
                    None,
                    "Conflicting Kubernetes workload variants; environment from these variants is not merged.",
                );
                out.retain(|d| d.id != id);
            }
            continue;
        }
        workloads.insert(id.clone(), doc.clone());
        let containers = pod["spec"]["containers"]
            .as_sequence()
            .cloned()
            .unwrap_or_default();
        let project = if containers.len() == 1 {
            project_for(report, containers[0]["image"].as_str().unwrap_or(""), name)
        } else {
            None
        };
        if !report.services.iter().any(|s| s.id == id) {
            report.services.push(Service {
                id: id.clone(),
                name: name.into(),
                compose_file: file.clone(),
                build_context: None,
                networks: vec![scope.clone()],
                profiles: vec![],
                project,
            });
        }
        let mut env = BTreeMap::new();
        for container in &containers {
            if container["envFrom"].is_sequence() {
                diagnostic(
                    report,
                    file,
                    Some(name),
                    "Kubernetes envFrom is unresolved; ConfigMap/Secret content is not loaded.",
                );
            }
            for e in container["env"].as_sequence().into_iter().flatten() {
                if !e["valueFrom"].is_null() {
                    diagnostic(
                        report,
                        file,
                        Some(name),
                        "Kubernetes valueFrom is unresolved; Secret/ConfigMap contents are not read.",
                    );
                }
                if let (Some(k), Some(v)) = (e["name"].as_str(), e["value"].as_str())
                    && endpoint_key(k)
                    && public_value(v)
                    && !v.contains('$')
                {
                    env.insert(k.into(), v.into());
                }
            }
        }
        let workload_labels = labels(&pod["metadata"]["labels"]);
        let mut aliases = BTreeSet::new();
        for (service_file, service) in &documents {
            if Path::new(service_file).parent() != Path::new(file).parent()
                || service["kind"].as_str() != Some("Service")
                || service["metadata"]["namespace"]
                    .as_str()
                    .unwrap_or("default")
                    != namespace
            {
                continue;
            }
            let selector = labels(&service["spec"]["selector"]);
            if !selector.is_empty()
                && selector
                    .iter()
                    .all(|(k, v)| workload_labels.get(k) == Some(v))
                && let Some(host) = service["metadata"]["name"].as_str()
            {
                aliases.extend([
                    host.to_lowercase(),
                    format!("{host}.{namespace}"),
                    format!("{host}.{namespace}.svc"),
                    format!("{host}.{namespace}.svc.cluster.local"),
                ]);
            }
        }
        out.push(Deployment {
            id,
            scope,
            file: file.clone(),
            env,
            aliases,
        });
    }
    let mapped: BTreeSet<_> = report
        .services
        .iter()
        .filter(|s| s.id != s.project.as_deref().unwrap_or(""))
        .filter_map(|s| s.project.clone())
        .collect();
    report
        .services
        .retain(|s| !(s.id == s.project.as_deref().unwrap_or("") && mapped.contains(&s.id)));
    for d in &out {
        for (key, value) in &d.env {
            let Some(host) = host(value) else { continue };
            let targets: Vec<_> = out
                .iter()
                .filter(|t| t.scope == d.scope && t.aliases.contains(&host))
                .collect();
            if targets.len() == 1 {
                add(
                    report,
                    &d.id,
                    &targets[0].id,
                    if value.starts_with("http://") || value.starts_with("https://") {
                        Kind::ConfiguredHttp
                    } else {
                        Kind::ConfiguredReference
                    },
                    vec![proof(
                        &d.file,
                        &if d.scope.starts_with("kubernetes:") {
                            format!("/spec/template/spec/containers/env/{key}")
                        } else {
                            format!(
                                "/services/{}/environment/{key}",
                                d.id.rsplit("::")
                                    .next()
                                    .unwrap_or("")
                                    .replace('~', "~0")
                                    .replace('/', "~1")
                            )
                        },
                        "Declared endpoint maps to a service in the same deployment scope. Local public defaults only; no process environment, credentials or runtime traffic read.",
                    )],
                );
            }
        }
    }
    report.services.sort_by(|a, b| a.id.cmp(&b.id));
    out
}
