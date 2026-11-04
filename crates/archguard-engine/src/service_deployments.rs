//! Bounded static deployment declarations. Never contacts a cluster or executes interpolation.
use super::support::{diagnostic, relative};
use archguard_core::{
    Service, ServiceDependency, ServiceDependencyKind as Kind, ServiceEvidence, ServiceProject,
    ServiceTopology,
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
    crate::project_files::read_manifest(root, relative).ok()
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
const KUBERNETES: &str = "kubernetes:";
const DEFAULT_NAMESPACE: &str = "default";
const WORKLOAD_KINDS: [&str; 6] = [
    "Deployment",
    "StatefulSet",
    "DaemonSet",
    "Job",
    "CronJob",
    "Pod",
];
const LINK_EVIDENCE: &str = "Declared endpoint maps to a service in the same deployment scope. Local public defaults only; no process environment, credentials or runtime traffic read.";
/// Resolves an environment value, falling back to the local `.env` default for its key.
fn resolve(key: &str, value: Option<&str>, vars: &BTreeMap<String, String>) -> Option<String> {
    value
        .or_else(|| vars.get(key).map(String::as_str))
        .and_then(|v| expand(v, vars))
}
/// Endpoint-like Compose `environment` entries, in mapping or `KEY=value` list form.
fn compose_env(raw: &Value, vars: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for (k, v) in raw.as_mapping().into_iter().flatten() {
        if let Some(k) = k.as_str().filter(|k| endpoint_key(k))
            && let Some(v) = resolve(k, v.as_str(), vars)
        {
            env.insert(k.into(), v);
        }
    }
    for entry in raw.as_sequence().into_iter().flatten().filter_map(string) {
        let (k, v) = entry
            .split_once('=')
            .map_or((entry, None), |(k, v)| (k, Some(v)));
        if endpoint_key(k)
            && let Some(v) = resolve(k, v, vars)
        {
            env.insert(k.into(), v);
        }
    }
    env
}
/// The deepest non-root source project containing `path`, unless another is equally deep.
fn enclosing_project(projects: &[ServiceProject], root: &Path, path: &Path) -> Option<String> {
    let mut candidates: Vec<_> = projects
        .iter()
        .filter(|p| {
            !matches!(p.kind.as_str(), "workspace" | "tooling")
                && p.path != "."
                && path.starts_with(root.join(&p.path))
        })
        .collect();
    candidates.sort_by_key(|p| std::cmp::Reverse(p.path.len()));
    candidates
        .first()
        .filter(|p| {
            candidates
                .get(1)
                .is_none_or(|q| q.path.len() < p.path.len())
        })
        .map(|p| p.id.clone())
}
fn is_yaml_file(entry: &ignore::DirEntry) -> bool {
    entry.file_type().is_some_and(|t| t.is_file())
        && matches!(
            entry.path().extension().and_then(|e| e.to_str()),
            Some("yaml" | "yml")
        )
}
/// Adds a manifest document, flattening the items of a Kubernetes `List`.
fn push_document(documents: &mut Vec<(String, Value)>, file: &str, doc: Value) {
    if doc["kind"].as_str() != Some("List") {
        documents.push((file.to_owned(), doc));
    } else if let Some(items) = doc["items"].as_sequence() {
        documents.extend(items.iter().cloned().map(|d| (file.to_owned(), d)));
    }
}
impl Deployment {
    /// Pointer to the environment entry that declares `key`.
    fn pointer(&self, key: &str) -> String {
        if self.scope.starts_with(KUBERNETES) {
            return format!("/spec/template/spec/containers/env/{key}");
        }
        let service = self
            .id
            .rsplit("::")
            .next()
            .unwrap_or("")
            .replace('~', "~0")
            .replace('/', "~1");
        format!("/services/{service}/environment/{key}")
    }
}
/// One statically named Kubernetes workload and the deployment scope it belongs to.
struct Workload<'d> {
    file: &'d str,
    name: &'d str,
    namespace: &'d str,
    scope: String,
    id: String,
    pod: &'d Value,
}
impl<'d> Workload<'d> {
    fn parse(file: &'d str, doc: &'d Value) -> Option<Self> {
        let kind = doc["kind"]
            .as_str()
            .filter(|kind| WORKLOAD_KINDS.contains(kind))?;
        let name = doc["metadata"]["name"].as_str()?;
        let namespace = doc["metadata"]["namespace"]
            .as_str()
            .unwrap_or(DEFAULT_NAMESPACE);
        if name.contains(['$', '{']) || namespace.contains(['$', '{']) {
            return None;
        }
        let pod = match kind {
            "Pod" => doc,
            "CronJob" => &doc["spec"]["jobTemplate"]["spec"]["template"],
            _ => &doc["spec"]["template"],
        };
        let bundle = Path::new(file)
            .parent()
            .and_then(Path::to_str)
            .unwrap_or("");
        let scope = format!("{KUBERNETES}{bundle}:{namespace}");
        let id = format!("{scope}:{kind}:{name}");
        Some(Self {
            file,
            name,
            namespace,
            scope,
            id,
            pod,
        })
    }
    /// Cluster hostnames of the Services that select this workload's pod labels.
    fn aliases(&self, documents: &[(String, Value)]) -> BTreeSet<String> {
        let pod_labels = labels(&self.pod["metadata"]["labels"]);
        let namespace = self.namespace;
        let mut aliases = BTreeSet::new();
        for (file, service) in documents {
            if let Some(host) = self.selecting_service(file, service, &pod_labels) {
                aliases.extend([
                    host.to_lowercase(),
                    format!("{host}.{namespace}"),
                    format!("{host}.{namespace}.svc"),
                    format!("{host}.{namespace}.svc.cluster.local"),
                ]);
            }
        }
        aliases
    }
    /// The name of `service` if it is a Service in the same bundle and namespace whose
    /// non-empty selector matches `pod_labels`.
    fn selecting_service<'v>(
        &self,
        file: &str,
        service: &'v Value,
        pod_labels: &BTreeMap<String, String>,
    ) -> Option<&'v str> {
        if Path::new(file).parent() != Path::new(self.file).parent()
            || service["kind"].as_str() != Some("Service")
            || service["metadata"]["namespace"]
                .as_str()
                .unwrap_or(DEFAULT_NAMESPACE)
                != self.namespace
        {
            return None;
        }
        let selector = labels(&service["spec"]["selector"]);
        if selector.is_empty() || !selector.iter().all(|(k, v)| pod_labels.get(k) == Some(v)) {
            return None;
        }
        service["metadata"]["name"].as_str()
    }
}
/// Shared state while deriving deployments: Compose services first, then Kubernetes workloads.
struct Deployments<'a> {
    root: &'a Path,
    report: &'a mut ServiceTopology,
    out: Vec<Deployment>,
}
impl Deployments<'_> {
    fn compose_file(&mut self, file: &str) {
        let base = self.root.join(file).parent().unwrap().to_path_buf();
        let vars = defaults(self.root, &base);
        for mut doc in yaml(self.root, file, self.report) {
            let _ = doc.apply_merge();
            for (key, value) in doc["services"].as_mapping().into_iter().flatten() {
                if let Some(name) = key.as_str() {
                    self.compose_service(file, name, value, &vars);
                }
            }
        }
    }
    fn compose_service(
        &mut self,
        file: &str,
        name: &str,
        value: &Value,
        vars: &BTreeMap<String, String>,
    ) {
        let id = format!("{file}::{name}");
        let Some(index) = self.report.services.iter().position(|s| s.id == id) else {
            return;
        };
        if let Some(project) = self.dockerfile_project(index, value, vars) {
            self.report.services[index].project = Some(project);
        }
        let service = &self.report.services[index];
        if service.project.is_none() && service.build_context.is_none() {
            self.infer_project_by_name(index, file, name);
        }
        self.out.push(Deployment {
            id,
            scope: file.to_owned(),
            file: file.to_owned(),
            env: compose_env(&value["environment"], vars),
            aliases: BTreeSet::from([name.to_ascii_lowercase()]),
        });
    }
    /// The source project holding the service's readable, in-root Dockerfile.
    fn dockerfile_project(
        &self,
        index: usize,
        value: &Value,
        vars: &BTreeMap<String, String>,
    ) -> Option<String> {
        let dockerfile = value["build"]["dockerfile"]
            .as_str()
            .and_then(|s| expand(s, vars))?;
        let context = self.report.services[index].build_context.as_ref()?;
        let path = fs::canonicalize(self.root.join(context).join(&dockerfile)).ok()?;
        if !path.starts_with(self.root) || read(self.root, &path).is_none() {
            return None;
        }
        enclosing_project(&self.report.projects, self.root, &path)
    }
    fn infer_project_by_name(&mut self, index: usize, file: &str, name: &str) {
        let candidates: Vec<_> = self
            .report
            .projects
            .iter()
            .filter(|p| p.kind == "application" && p.name == name)
            .map(|p| p.id.clone())
            .collect();
        if candidates.len() == 1 {
            self.report.services[index].project = Some(candidates[0].clone());
            diagnostic(
                self.report,
                file,
                Some(name),
                "Source navigation is inferred from a unique application/service name match; image contents are not verified.",
            );
        }
    }
    /// Non-Compose YAML documents within the discovery input limits.
    fn manifests(&mut self, discovery: &super::support::Discovery) -> Vec<(String, Value)> {
        let mut documents = Vec::new();
        let mut budget = 0usize;
        for entry in discovery.walk(self.root).take(50_000).flatten() {
            if !is_yaml_file(&entry) {
                continue;
            }
            let file = relative(self.root, entry.path());
            if self.report.compose_files.contains(&file) {
                continue;
            }
            budget += entry.metadata().map(|m| m.len() as usize).unwrap_or(0);
            if budget > 32_000_000 || documents.len() >= 2000 {
                diagnostic(
                    self.report,
                    &file,
                    None,
                    "Deployment discovery input limit reached.",
                );
                break;
            }
            for doc in yaml(self.root, &file, self.report) {
                push_document(&mut documents, &file, doc);
            }
        }
        documents
    }
    fn workloads(&mut self, documents: &[(String, Value)]) {
        let mut seen = BTreeMap::new();
        for (file, doc) in documents {
            if let Some(workload) = Workload::parse(file, doc) {
                self.workload(&workload, doc, documents, &mut seen);
            }
        }
    }
    fn workload(
        &mut self,
        workload: &Workload,
        doc: &Value,
        documents: &[(String, Value)],
        seen: &mut BTreeMap<String, Value>,
    ) {
        // Repeated manifests may be variants: only identical declarations are merged.
        if let Some(previous) = seen.get(&workload.id) {
            if previous != doc {
                diagnostic(
                    self.report,
                    workload.file,
                    None,
                    "Conflicting Kubernetes workload variants; environment from these variants is not merged.",
                );
                self.out.retain(|d| d.id != workload.id);
            }
            return;
        }
        seen.insert(workload.id.clone(), doc.clone());
        let containers = workload.pod["spec"]["containers"]
            .as_sequence()
            .cloned()
            .unwrap_or_default();
        self.register(workload, &containers);
        let env = self.workload_env(workload, &containers);
        self.out.push(Deployment {
            id: workload.id.clone(),
            scope: workload.scope.clone(),
            file: workload.file.to_owned(),
            env,
            aliases: workload.aliases(documents),
        });
    }
    /// Adds the workload as a service unless it is already known.
    fn register(&mut self, workload: &Workload, containers: &[Value]) {
        let project = match containers {
            [container] => project_for(
                self.report,
                container["image"].as_str().unwrap_or(""),
                workload.name,
            ),
            _ => None,
        };
        if !self.report.services.iter().any(|s| s.id == workload.id) {
            self.report.services.push(Service {
                id: workload.id.clone(),
                name: workload.name.into(),
                compose_file: workload.file.into(),
                build_context: None,
                networks: vec![workload.scope.clone()],
                profiles: vec![],
                project,
            });
        }
    }
    /// Literal, public endpoint values; unresolved references are diagnosed, never read.
    fn workload_env(
        &mut self,
        workload: &Workload,
        containers: &[Value],
    ) -> BTreeMap<String, String> {
        let (file, name) = (workload.file, Some(workload.name));
        let mut env = BTreeMap::new();
        for container in containers {
            if container["envFrom"].is_sequence() {
                diagnostic(
                    self.report,
                    file,
                    name,
                    "Kubernetes envFrom is unresolved; ConfigMap/Secret content is not loaded.",
                );
            }
            for e in container["env"].as_sequence().into_iter().flatten() {
                if !e["valueFrom"].is_null() {
                    diagnostic(
                        self.report,
                        file,
                        name,
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
        env
    }
    /// Removes project services that a deployment already maps to.
    fn drop_mapped_project_services(&mut self) {
        let mapped: BTreeSet<_> = self
            .report
            .services
            .iter()
            .filter(|s| s.id != s.project.as_deref().unwrap_or(""))
            .filter_map(|s| s.project.clone())
            .collect();
        self.report
            .services
            .retain(|s| !(s.id == s.project.as_deref().unwrap_or("") && mapped.contains(&s.id)));
    }
    /// An endpoint naming exactly one deployment in the same scope becomes a dependency.
    fn link(&mut self) {
        for d in &self.out {
            for (key, value) in &d.env {
                let Some(host) = host(value) else { continue };
                let targets: Vec<_> = self
                    .out
                    .iter()
                    .filter(|t| t.scope == d.scope && t.aliases.contains(&host))
                    .collect();
                let [target] = targets.as_slice() else {
                    continue;
                };
                let kind = if value.starts_with("http://") || value.starts_with("https://") {
                    Kind::ConfiguredHttp
                } else {
                    Kind::ConfiguredReference
                };
                let evidence = vec![proof(&d.file, &d.pointer(key), LINK_EVIDENCE)];
                add(self.report, &d.id, &target.id, kind, evidence);
            }
        }
    }
}
pub(super) fn augment(
    root: &Path,
    discovery: &super::support::Discovery,
    report: &mut ServiceTopology,
) -> Vec<Deployment> {
    let compose_files = report.compose_files.clone();
    let mut deployments = Deployments {
        root,
        report,
        out: Vec::new(),
    };
    for file in &compose_files {
        deployments.compose_file(file);
    }
    // Workloads and selectors, never Helm evaluation, Secret reads or cluster queries.
    let documents = deployments.manifests(discovery);
    deployments.workloads(&documents);
    deployments.drop_mapped_project_services();
    deployments.link();
    deployments.report.services.sort_by(|a, b| a.id.cmp(&b.id));
    deployments.out
}
