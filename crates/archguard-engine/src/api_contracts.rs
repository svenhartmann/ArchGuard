//! Explicit OpenAPI imports. Configured callers do not prove operation usage.
use archguard_core::{ArchguardError, ServiceDependencyKind, ServiceTopology};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const SNAPSHOT: &str = ".archguard/contracts.json";
const MAX_BYTES: u64 = 8 * 1024 * 1024;
const METHODS: &[&str] = &[
    "get", "post", "put", "patch", "delete", "head", "options", "trace", "query",
];
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiContractReport {
    pub schema_version: u32,
    pub status: String,
    pub contracts: Vec<ApiContract>,
    pub diagnostics: Vec<String>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiContract {
    pub provider: String,
    pub mapped_service: bool,
    pub source_path: String,
    pub source_sha256: String,
    pub source_status: String,
    pub openapi: String,
    pub imported_at_unix: u64,
    pub operations: Vec<ApiOperation>,
    pub usages: Vec<ApiUsage>,
    pub usage_path: Option<String>,
    pub usage_sha256: Option<String>,
    pub usage_status: String,
    pub configured_callers: Vec<String>,
    pub comparison_status: String,
    pub previous_imported_at_unix: Option<u64>,
    pub changes: Vec<ApiChange>,
    pub diagnostics: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiOperation {
    pub id: String,
    pub operation_id: Option<String>,
    pub pointer: String,
    pub required_parameters: Vec<String>,
    pub request_fingerprint: String,
    pub response_fingerprint: String,
    #[serde(default)]
    pub request_references: Vec<String>,
    #[serde(default)]
    pub response_references: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiUsage {
    pub consumer: String,
    pub operation: String,
    pub mapped_service: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiChange {
    pub operation: String,
    pub kind: String,
    pub severity: String,
    pub message: String,
    pub consumers: Vec<String>,
}
fn error(s: impl Into<String>) -> ArchguardError {
    ArchguardError::Analysis(s.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn empty() -> ApiContractReport {
    ApiContractReport {schema_version:1,status:"not_imported".into(),contracts:vec![],diagnostics:vec![],limitations:vec![
        "OpenAPI 3.x HTTP operation/parameter/request/response projection, not a complete compatibility proof. Schema changes need review; authentication, callbacks, webhooks and runtime behavior are not compared.".into(),
        "Consumer usage is explicitly declared. Configured HTTP callers do not prove use of an operation. Comparison uses the previous explicit import, not a Git merge base.".into(),
        "Document hashes detect changes since import; producer/source revision is unverified. Local references only; no network access.".into(),
    ]}
}
fn input(root: &Path, path: &Path) -> Result<(String, Vec<u8>), ArchguardError> {
    let canonical = std::fs::canonicalize(root)?;
    let relative = if path.is_absolute() {
        // Accept aliases of the chosen root (for example /var -> /private/var),
        // but retain every component below that root for the NOFOLLOW read.
        // Prefer its lexical prefix; otherwise choose the outermost matching
        // ancestor so an inner symlink back to the root cannot be stripped away.
        let prefix = if path.starts_with(&canonical) {
            canonical.as_path()
        } else {
            path.ancestors()
                .skip(1)
                .filter(|ancestor| std::fs::canonicalize(ancestor).is_ok_and(|p| p == canonical))
                .last()
                .ok_or_else(|| error("Contract input must be inside the project root"))?
        };
        path.strip_prefix(prefix)
            .map_err(|_| error("Contract input must be inside the project root"))?
    } else {
        path
    };
    let bytes = crate::project_files::read_repo_file(&canonical, relative, MAX_BYTES)?;
    let name = relative
        .to_str()
        .ok_or_else(|| error("Contract input path must be UTF-8"))?
        .to_owned();
    Ok((name, bytes))
}
fn refresh(report: &mut ApiContractReport, root: &Path, topology: &ServiceTopology) {
    let ids: BTreeSet<_> = topology.services.iter().map(|s| s.id.as_str()).collect();
    let mut stale = false;
    let mut partial = false;
    report.diagnostics.clear();
    for c in &mut report.contracts {
        c.mapped_service = ids.contains(c.provider.as_str());
        if !c.mapped_service {
            partial = true;
            report.diagnostics.push(format!(
                "Provider {} is not mapped to a discovered service.",
                c.provider
            ));
        }
        c.source_status = match crate::project_files::read_repo_file(
            root,
            Path::new(&c.source_path),
            MAX_BYTES,
        ) {
            Ok(b) if hash(&b) == c.source_sha256 => "unchanged_since_import",
            _ => {
                stale = true;
                "changed_since_import"
            }
        }
        .into();
        if let Some(path) = &c.usage_path {
            c.usage_status =
                match crate::project_files::read_repo_file(root, Path::new(path), MAX_BYTES) {
                    Ok(b) if c.usage_sha256.as_ref() == Some(&hash(&b)) => "unchanged_since_import",
                    _ => {
                        stale = true;
                        "changed_since_import"
                    }
                }
                .into();
        }
        for u in &mut c.usages {
            u.mapped_service = ids.contains(u.consumer.as_str());
            if !u.mapped_service {
                partial = true;
                report.diagnostics.push(format!(
                    "Declared consumer {} is not mapped to a discovered service.",
                    u.consumer
                ));
            }
        }
        c.configured_callers = topology
            .dependencies
            .iter()
            .filter(|d| d.kind == ServiceDependencyKind::ConfiguredHttp && d.to == c.provider)
            .map(|d| d.from.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if !c.diagnostics.is_empty() {
            partial = true;
            report
                .diagnostics
                .extend(c.diagnostics.iter().map(|d| format!("{}: {d}", c.provider)));
        }
    }
    report.diagnostics.truncate(200);
    report.status = if stale {
        "stale"
    } else if partial {
        "partial"
    } else if report.contracts.is_empty() {
        "not_imported"
    } else {
        "ready"
    }
    .into();
}
fn valid_snapshot(report: &ApiContractReport) -> bool {
    let mut providers = BTreeSet::new();
    report.schema_version == 1
        && report.contracts.len() <= 200
        && report.contracts.iter().all(|c| {
            let mut operations = BTreeSet::new();
            !c.provider.is_empty()
                && providers.insert(c.provider.as_str())
                && c.operations.len() <= 2048
                && c.usages.len() <= 10000
                && c.source_sha256.len() == 64
                && c.source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                && c.operations
                    .iter()
                    .all(|op| !op.id.is_empty() && operations.insert(op.id.as_str()))
        })
}
pub fn read_api_contracts(root: &Path, topology: &ServiceTopology) -> ApiContractReport {
    let mut report =
        match crate::project_files::read_repo_file(root, Path::new(SNAPSHOT), MAX_BYTES) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return empty(),
            Ok(bytes) => match serde_json::from_slice::<ApiContractReport>(&bytes) {
                Ok(r) if valid_snapshot(&r) => r,
                _ => {
                    let mut r = empty();
                    r.status = "error".into();
                    r.diagnostics
                        .push("Invalid contract snapshot; import OpenAPI again.".into());
                    return r;
                }
            },
            Err(_) => {
                let mut r = empty();
                r.status = "error".into();
                r.diagnostics
                    .push("Contract snapshot cannot be read safely.".into());
                return r;
            }
        };
    refresh(&mut report, root, topology);
    report
}
pub fn api_contracts_fingerprint(root: &Path) -> String {
    match crate::project_files::read_repo_file(root, Path::new(SNAPSHOT), MAX_BYTES) {
        Ok(bytes) => {
            let mut h = Sha256::new();
            h.update(&bytes);
            if let Ok(r) = serde_json::from_slice::<ApiContractReport>(&bytes) {
                for c in r.contracts.iter().take(200) {
                    for p in [Some(c.source_path.as_str()), c.usage_path.as_deref()]
                        .into_iter()
                        .flatten()
                    {
                        match crate::project_files::read_repo_file(root, Path::new(p), MAX_BYTES) {
                            Ok(b) => h.update(hash(&b)),
                            Err(_) => h.update(b"unavailable"),
                        }
                    }
                }
            }
            format!("{:x}", h.finalize())
        }
        Err(e) => format!("unavailable:{:?}", e.kind()),
    }
}
fn local_ref<'a>(
    doc: &'a Value,
    value: &'a Value,
    notes: &mut BTreeSet<String>,
) -> Option<&'a Value> {
    let mut current = value;
    let mut seen = BTreeSet::new();
    for _ in 0..64 {
        let Some(raw_reference) = current.get("$ref") else {
            return Some(current);
        };
        let Some(reference) = raw_reference.as_str() else {
            notes.insert("Invalid non-string object reference omitted.".into());
            return None;
        };
        if current.as_object().is_some_and(|m| m.len() > 1) {
            notes.insert(format!(
                "Object reference sibling fields are not analyzed: {}",
                reference.chars().take(240).collect::<String>()
            ));
        }
        if !seen.insert(reference) {
            notes.insert("Cyclic object reference omitted.".into());
            return None;
        }
        if let Some(pointer) = reference.strip_prefix('#').filter(|p| p.starts_with('/'))
            && let Some(target) = doc.pointer(pointer)
        {
            current = target;
            continue;
        }
        notes.insert(format!(
            "Unresolved or external reference omitted: {}",
            reference.chars().take(240).collect::<String>()
        ));
        return None;
    }
    notes.insert("Reference chain exceeds analysis limit.".into());
    None
}
// Include each local reference once; never expand recursive/shared schemas.
fn signature(
    doc: &Value,
    value: &Value,
    notes: &mut BTreeSet<String>,
) -> Result<(String, Vec<String>), ArchguardError> {
    let mut refs = BTreeMap::new();
    let mut pending = vec![value];
    let mut visited = 0;
    while let Some(v) = pending.pop() {
        visited += 1;
        if visited > 100_000 {
            return Err(error("Contract schema traversal exceeds limit"));
        }
        match v {
            Value::Object(m) => {
                if let Some(reference) = m
                    .get("$ref")
                    .and_then(Value::as_str)
                    .filter(|r| !refs.contains_key(*r))
                {
                    if refs.len() >= 2048 {
                        return Err(error("Too many schema references"));
                    }
                    if let Some(target) = reference
                        .strip_prefix('#')
                        .filter(|p| p.starts_with('/'))
                        .and_then(|p| doc.pointer(p))
                    {
                        refs.insert(reference.to_owned(), target);
                        pending.push(target);
                    } else {
                        notes.insert(format!(
                            "Unresolved or external reference: {}",
                            reference.chars().take(240).collect::<String>()
                        ));
                    }
                }
                pending.extend(m.values());
            }
            Value::Array(a) => pending.extend(a),
            _ => {}
        }
    }
    let mut h = Sha256::new();
    let mut size = 0;
    for (name, v) in std::iter::once(("", value)).chain(refs.iter().map(|(k, v)| (k.as_str(), *v)))
    {
        let bytes = serde_json::to_vec(v).map_err(|e| error(e.to_string()))?;
        size += bytes.len();
        if size > MAX_BYTES as usize {
            return Err(error("Resolved schema exceeds analysis budget"));
        }
        h.update(name.as_bytes());
        h.update([0]);
        h.update(bytes);
    }
    Ok((
        format!("{:x}", h.finalize()),
        refs.keys().cloned().collect(),
    ))
}
fn check_budget(doc: &Value) -> Result<(), ArchguardError> {
    let mut pending = vec![(doc, 0)];
    let mut count = 0;
    while let Some((v, d)) = pending.pop() {
        count += 1;
        if count > 100_000 || d > 64 {
            return Err(error(
                "Contract document nesting/size exceeds analysis limits",
            ));
        }
        match v {
            Value::Array(a) => pending.extend(a.iter().map(|v| (v, d + 1))),
            Value::Object(m) => pending.extend(m.values().map(|v| (v, d + 1))),
            _ => {}
        }
    }
    Ok(())
}
fn validate_path_item(item: &Value, path: &str, openapi_32: bool) -> Result<(), ArchguardError> {
    let fields = item
        .as_object()
        .ok_or_else(|| error(format!("OpenAPI path item must be an object: {path}")))?;
    for (name, value) in fields {
        let valid = match name.as_str() {
            "$ref" | "summary" | "description" => value.is_string(),
            "servers" | "parameters" => value.is_array(),
            "additionalOperations" if openapi_32 => value.is_object(),
            method if METHODS.contains(&method) && (method != "query" || openapi_32) => {
                value.is_object()
            }
            extension if extension.starts_with("x-") => true,
            _ => {
                return Err(error(format!(
                    "Unsupported OpenAPI path item field {name}: {path}"
                )));
            }
        };
        if !valid {
            return Err(error(format!(
                "Invalid OpenAPI path item field {name}: {path}"
            )));
        }
    }
    Ok(())
}
fn operations(
    doc: &Value,
    notes: &mut BTreeSet<String>,
) -> Result<Vec<ApiOperation>, ArchguardError> {
    let mut result = vec![];
    let openapi_32 = doc
        .get("openapi")
        .and_then(Value::as_str)
        .is_some_and(|v| v.starts_with("3.2."));
    let paths = doc
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| error("OpenAPI paths must be an object"))?;
    for (path, item) in paths {
        if path.starts_with("x-") {
            continue;
        }
        if !path.starts_with('/') {
            return Err(error("OpenAPI path must start with /"));
        }
        validate_path_item(item, path, openapi_32)?;
        let Some(item) = local_ref(doc, item, notes) else {
            continue;
        };
        validate_path_item(item, path, openapi_32)?;
        if item.get("additionalOperations").is_some() {
            notes.insert(format!(
                "OpenAPI additionalOperations are not analyzed: {path}"
            ));
        }
        for method in METHODS {
            let Some(op) = item.get(*method) else {
                continue;
            };
            if !op.is_object() {
                return Err(error("HTTP operation must be an object"));
            }
            if result.len() >= 2048 {
                return Err(error("Contract has more than 2048 operations"));
            }
            let mut parameters = BTreeMap::new();
            for owner in [item, op] {
                if let Some(values) = owner.get("parameters") {
                    for raw in values
                        .as_array()
                        .ok_or_else(|| error("Operation parameters must be an array"))?
                    {
                        let Some(p) = local_ref(doc, raw, notes) else {
                            continue;
                        };
                        let name = p
                            .get("name")
                            .and_then(Value::as_str)
                            .ok_or_else(|| error("Parameter name missing"))?;
                        let location = p
                            .get("in")
                            .and_then(Value::as_str)
                            .ok_or_else(|| error("Parameter location missing"))?;
                        if !matches!(location, "query" | "header" | "path" | "cookie") {
                            return Err(error("Unsupported parameter location"));
                        }
                        parameters.insert(format!("{location}:{name}"), p);
                    }
                }
            }
            let required_parameters = parameters
                .iter()
                .filter(|(k, p)| {
                    k.starts_with("path:")
                        || p.get("required").and_then(Value::as_bool) == Some(true)
                })
                .map(|(k, _)| k.clone())
                .collect();
            let request = json!({"parameters":parameters,"body":op.get("requestBody")});
            let response = op
                .get("responses")
                .filter(|v| v.is_object())
                .ok_or_else(|| error("Operation responses object missing"))?;
            let (request_fingerprint, request_references) = signature(doc, &request, notes)?;
            let (response_fingerprint, response_references) = signature(doc, response, notes)?;
            result.push(ApiOperation {
                id: format!("{} {path}", method.to_uppercase()),
                operation_id: op
                    .get("operationId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                pointer: format!(
                    "/paths/{}/{}",
                    path.replace('~', "~0").replace('/', "~1"),
                    method
                ),
                required_parameters,
                request_fingerprint,
                response_fingerprint,
                request_references,
                response_references,
            });
        }
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}
fn changes(previous: &ApiContract, current: &ApiContract) -> Vec<ApiChange> {
    let old: BTreeMap<_, _> = previous
        .operations
        .iter()
        .map(|v| (v.id.as_str(), v))
        .collect();
    let new: BTreeMap<_, _> = current
        .operations
        .iter()
        .map(|v| (v.id.as_str(), v))
        .collect();
    let mut changes = vec![];
    let mut add = |id: &str, kind: &str, severity: &str, message: String| {
        let consumers = previous
            .usages
            .iter()
            .chain(&current.usages)
            .filter(|u| u.operation == id || u.operation == "*")
            .map(|u| u.consumer.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        changes.push(ApiChange {
            operation: id.into(),
            kind: kind.into(),
            severity: severity.into(),
            message,
            consumers,
        });
    };
    for (id, before) in &old {
        let Some(after) = new.get(id) else {
            add(
                id,
                "operation_removed",
                "breaking",
                "Previously declared HTTP operation was removed.".into(),
            );
            continue;
        };
        for p in &after.required_parameters {
            if !before.required_parameters.contains(p) {
                add(
                    id,
                    "required_parameter_added",
                    "breaking",
                    format!("New required parameter: {p}"),
                );
            }
        }
        if before.request_fingerprint != after.request_fingerprint {
            add(
                id,
                "request_schema_changed",
                "review",
                format!(
                    "Request contract changed (including metadata); compatibility needs review. References: {}",
                    after.request_references.join(", ")
                ),
            );
        }
        if before.response_fingerprint != after.response_fingerprint {
            add(
                id,
                "response_schema_changed",
                "review",
                format!(
                    "Response contract changed (including metadata); compatibility needs review. References: {}",
                    after.response_references.join(", ")
                ),
            );
        }
    }
    for id in new.keys() {
        if !old.contains_key(id) {
            add(
                id,
                "operation_added",
                "info",
                "New HTTP operation declared.".into(),
            );
        }
    }
    changes
}
pub fn import_api_contract(
    root: &Path,
    topology: &ServiceTopology,
    report: &Path,
    provider: &str,
    usage: Option<&Path>,
) -> Result<ApiContractReport, ArchguardError> {
    if provider.trim().is_empty() || provider.len() > 1024 || provider.chars().any(char::is_control)
    {
        return Err(error("A nonempty service identity is required"));
    }
    let (source_path, bytes) = input(root, report)?;
    let doc: Value = serde_yaml_ng::from_slice(&bytes)
        .map_err(|_| error("Invalid JSON/YAML OpenAPI document"))?;
    check_budget(&doc)?;
    let version = doc
        .get("openapi")
        .and_then(Value::as_str)
        .filter(|s| s.starts_with("3.0.") || s.starts_with("3.1.") || s.starts_with("3.2."))
        .ok_or_else(|| error("Only OpenAPI 3.x documents are supported"))?;
    if !doc.get("info").is_some_and(Value::is_object) {
        return Err(error("OpenAPI info object missing"));
    }
    let mut notes = BTreeSet::new();
    if doc.get("webhooks").is_some() {
        notes.insert("Webhook contracts are not analyzed.".into());
    }
    let operations = operations(&doc, &mut notes)?;
    let mut saved = read_api_contracts(root, topology);
    if saved.status == "error" {
        return Err(error(
            "Existing contract snapshot is invalid; it was preserved",
        ));
    }
    let previous = saved.contracts.iter().find(|c| c.provider == provider);
    let (usages, usage_path, usage_sha256) = if let Some(path) = usage {
        let (name, bytes) = input(root, path)?;
        let doc: Value =
            serde_json::from_slice(&bytes).map_err(|_| error("Invalid API usage JSON"))?;
        if doc.get("schema_version") != Some(&json!(1)) {
            return Err(error("API usage requires schema_version 1"));
        }
        let rows = doc
            .get("usages")
            .and_then(Value::as_array)
            .filter(|a| a.len() <= 10000)
            .ok_or_else(|| error("API usages must be an array of at most 10000 records"))?;
        let mut result = vec![];
        let mut seen = BTreeSet::new();
        for row in rows {
            let consumer = row
                .get("consumer")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty() && s.len() <= 1024)
                .ok_or_else(|| error("Usage consumer missing"))?;
            let operation = row
                .get("operation")
                .and_then(Value::as_str)
                .filter(|s| {
                    *s == "*"
                        || operations.iter().any(|o| o.id == *s)
                        || previous.is_some_and(|p| p.operations.iter().any(|o| o.id == *s))
                })
                .ok_or_else(|| {
                    error("Usage operation must match a current/previous METHOD /path or *")
                })?;
            if seen.insert((consumer, operation)) {
                result.push(ApiUsage {
                    consumer: consumer.into(),
                    operation: operation.into(),
                    mapped_service: false,
                });
            }
        }
        (result, Some(name), Some(hash(&bytes)))
    } else {
        previous
            .map(|c| {
                (
                    c.usages.clone(),
                    c.usage_path.clone(),
                    c.usage_sha256.clone(),
                )
            })
            .unwrap_or_default()
    };
    let mut c = ApiContract {
        provider: provider.into(),
        mapped_service: false,
        source_path,
        source_sha256: hash(&bytes),
        source_status: "unchanged_since_import".into(),
        openapi: version.into(),
        imported_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        operations,
        usages,
        usage_path,
        usage_sha256,
        usage_status: "not_imported".into(),
        configured_callers: vec![],
        comparison_status: "no_baseline".into(),
        previous_imported_at_unix: previous.map(|p| p.imported_at_unix),
        changes: vec![],
        diagnostics: notes.into_iter().collect(),
    };
    if let Some(previous) = previous {
        c.changes = changes(previous, &c);
        c.comparison_status = if previous.diagnostics.is_empty() && c.diagnostics.is_empty() {
            "compared"
        } else {
            "partial"
        }
        .into();
    }
    saved.contracts.retain(|c| c.provider != provider);
    saved.contracts.push(c);
    saved.contracts.sort_by(|a, b| a.provider.cmp(&b.provider));
    if saved.contracts.len() > 200 {
        return Err(error("Contract inventory exceeds 200 services"));
    }
    refresh(&mut saved, root, topology);
    let bytes = serde_json::to_vec_pretty(&saved).map_err(|e| error(e.to_string()))?;
    if bytes.len() > MAX_BYTES as usize {
        return Err(error("Contract snapshot exceeds 8 MiB"));
    }
    crate::project_files::write_project_file(root, Path::new("contracts.json"), &bytes, true)?;
    Ok(saved)
}
