//! GraphQL contracts (ADR 0014). Schemas (`*.graphql`, `*.graphqls`, `*.gql`
//! and `gql`/`graphql` tagged templates in TS/JS) belong to the service whose
//! source scope contains them. An operation document links its service to
//! the one provider whose schema declares all its root fields; anything else
//! stays unresolved with a reason. Changes compare against an explicitly
//! saved baseline, like the OpenAPI import compares against the previous one.
use crate::graphql::{self, Field};
use crate::graphql_model::{
    GraphqlArgument, GraphqlContract, GraphqlField, GraphqlLink, GraphqlReport, GraphqlType,
    GraphqlUnresolved, key,
};
use crate::graphql_scan::{MAX_BYTES, Scan, Source, scan};
use crate::services::support::Discovery;
use archguard_core::{
    ArchguardError, ServiceDependency, ServiceDependencyKind, ServiceEvidence, ServiceTopology,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const BASELINE: &str = ".archguard/graphql-contracts.json";

#[derive(Serialize, Deserialize)]
struct Baseline {
    schema_version: u32,
    schemas: Vec<GraphqlContract>,
}

fn field(f: &Field) -> GraphqlField {
    GraphqlField {
        ty: f.ty.clone(),
        arguments: f
            .args
            .iter()
            .map(|(n, (ty, d))| {
                let arg = GraphqlArgument {
                    ty: ty.clone(),
                    has_default: *d,
                };
                (n.clone(), arg)
            })
            .collect(),
    }
}
fn contract(sources: &[&Source]) -> GraphqlContract {
    let mut types: BTreeMap<String, GraphqlType> = BTreeMap::new();
    let mut roots = BTreeMap::new();
    let mut directives = BTreeSet::new();
    let mut files = BTreeMap::new();
    for s in sources {
        files.insert(s.file.clone(), s.text_hash.clone());
        roots.extend(s.doc.roots.clone());
        directives.extend(s.doc.directives.iter().cloned());
        for t in &s.doc.types {
            let entry = types.entry(t.name.clone()).or_insert_with(|| GraphqlType {
                kind: t.kind.clone(),
                fields: BTreeMap::new(),
            });
            entry
                .fields
                .extend(t.fields.iter().map(|(n, f)| (n.clone(), field(f))));
        }
    }
    let mut operations = vec![];
    for kind in ["query", "mutation", "subscription"] {
        let root = root_type(&roots, kind);
        if let Some(t) = types.get(&root) {
            operations.extend(t.fields.keys().map(|f| format!("{root}.{f}")));
        }
    }
    let mut h = Sha256::new();
    for (file, digest) in &files {
        h.update(file.as_bytes());
        h.update([0]);
        h.update(digest.as_bytes());
    }
    GraphqlContract {
        provider: sources[0].owner.clone(),
        files: files.into_keys().collect(),
        sha256: format!("{:x}", h.finalize()),
        operations,
        types,
        directives: directives.into_iter().collect(),
    }
}
fn contracts(scan: &Scan) -> Vec<GraphqlContract> {
    // Unowned files stay separate contracts: no service joins them.
    let mut groups: BTreeMap<String, Vec<&Source>> = BTreeMap::new();
    for s in scan.sources.iter().filter(|s| !s.doc.types.is_empty()) {
        let group = s
            .owner
            .clone()
            .unwrap_or_else(|| format!("file:{}", s.file));
        groups.entry(group).or_default().push(s);
    }
    groups.values().map(|g| contract(g)).collect()
}
fn root_type(roots: &BTreeMap<String, String>, kind: &str) -> String {
    roots.get(kind).cloned().unwrap_or_else(|| {
        let mut c = kind.chars();
        c.next()
            .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
            .unwrap_or_default()
    })
}
/// The root type name a contract uses for `kind`, from its `schema` block
/// or the default name.
fn contract_root(c: &GraphqlContract, scan: &Scan, kind: &str) -> String {
    let roots: BTreeMap<String, String> = scan
        .sources
        .iter()
        .filter(|s| c.files.contains(&s.file) && s.owner == c.provider)
        .flat_map(|s| s.doc.roots.clone())
        .collect();
    root_type(&roots, kind)
}

/// The link for one operation, `Ok(None)` for a service's use of its own
/// schema, or the reason it stays unresolved.
fn link_one(
    scan: &Scan,
    schemas: &[GraphqlContract],
    s: &Source,
    op: &graphql::Operation,
) -> Result<Option<GraphqlLink>, String> {
    if !op.unknown_spreads.is_empty() {
        return Err(format!(
            "Root selection spreads fragments not defined in the same document: {}.",
            op.unknown_spreads.join(", ")
        ));
    }
    if op.root_fields.is_empty() {
        return Err("Operation selects no root field.".into());
    }
    let fields: Vec<_> = op.root_fields.iter().cloned().collect();
    let candidates: Vec<(&GraphqlContract, String)> = schemas
        .iter()
        .filter_map(|c| {
            let root = contract_root(c, scan, &op.kind);
            let t = c.types.get(&root)?;
            fields
                .iter()
                .all(|f| t.fields.contains_key(f))
                .then_some((c, root))
        })
        .collect();
    let names = candidates
        .iter()
        .map(|(c, _)| key(c))
        .collect::<Vec<_>>()
        .join(", ");
    let [(c, root)] = candidates.as_slice() else {
        return Err(if candidates.is_empty() {
            format!(
                "No GraphQL schema in the repository declares the {} root fields {}.",
                op.kind,
                fields.join(", ")
            )
        } else {
            format!(
                "The root fields match {} schemas ({names}); no provider was guessed.",
                candidates.len()
            )
        });
    };
    match (&s.owner, &c.provider) {
        (Some(consumer), Some(provider)) if consumer == provider => Ok(None),
        (Some(consumer), Some(provider)) => Ok(Some(GraphqlLink {
            consumer: consumer.clone(),
            provider: provider.clone(),
            operation: label(op),
            root_fields: fields.iter().map(|f| format!("{root}.{f}")).collect(),
            file: s.file.clone(),
            line: Some(s.line(op.at)),
            evidence: "operation_document".into(),
        })),
        (None, _) => Err(format!(
            "The document lies outside every service scope; the matching schema is {names}."
        )),
        (_, None) => Err(format!(
            "The only matching schema ({}) lies outside every service scope.",
            c.files.join(", ")
        )),
    }
}
fn label(op: &graphql::Operation) -> String {
    format!(
        "{} {}",
        op.kind,
        op.name.as_deref().unwrap_or("(anonymous)")
    )
}
fn link(scan: &Scan, schemas: &[GraphqlContract]) -> (Vec<GraphqlLink>, Vec<GraphqlUnresolved>) {
    let (mut links, mut unresolved) = (vec![], vec![]);
    for s in &scan.sources {
        for op in &s.doc.operations {
            match link_one(scan, schemas, s, op) {
                Ok(found) => links.extend(found),
                Err(reason) => unresolved.push(GraphqlUnresolved {
                    consumer: s.owner.clone(),
                    operation: label(op),
                    file: s.file.clone(),
                    line: Some(s.line(op.at)),
                    reason,
                }),
            }
        }
    }
    (links, unresolved)
}
/// The variable name when a configured endpoint's name or value says GraphQL.
fn graphql_endpoint(
    root: &Path,
    documents: &mut BTreeMap<String, Option<Value>>,
    e: &ServiceEvidence,
) -> Option<String> {
    let doc = documents.entry(e.file.clone()).or_insert_with(|| {
        crate::project_files::read_repo_file(root, Path::new(&e.file), MAX_BYTES)
            .ok()
            .and_then(|b| serde_yaml_ng::from_slice(&b).ok())
    });
    // The value is only searched, never reported.
    let value = doc
        .as_ref()
        .and_then(|v| v.pointer(&e.pointer))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let name = e.pointer.rsplit('/').next().unwrap_or_default();
    format!("{name} {value}")
        .to_ascii_lowercase()
        .contains("graphql")
        .then(|| name.to_owned())
}
/// Configured HTTP endpoints whose variable name or value names GraphQL.
fn endpoints(
    root: &Path,
    topology: &ServiceTopology,
    schemas: &[GraphqlContract],
    links: &mut Vec<GraphqlLink>,
    unresolved: &mut Vec<GraphqlUnresolved>,
) {
    let mut documents = BTreeMap::new();
    let configured = topology
        .dependencies
        .iter()
        .filter(|d| d.kind == ServiceDependencyKind::ConfiguredHttp)
        .flat_map(|d| d.evidence.iter().map(move |e| (d, e)));
    for (d, e) in configured {
        let Some(name) = graphql_endpoint(root, &mut documents, e) else {
            continue;
        };
        if !schemas.iter().any(|c| c.provider.as_ref() == Some(&d.to)) {
            unresolved.push(GraphqlUnresolved {
                consumer: Some(d.from.clone()),
                operation: format!("endpoint {name}"),
                file: e.file.clone(),
                line: e.line,
                reason: format!(
                    "Configured GraphQL endpoint points at {}, but no GraphQL schema lies in its scope.",
                    d.to
                ),
            });
            continue;
        }
        let link = GraphqlLink {
            consumer: d.from.clone(),
            provider: d.to.clone(),
            operation: String::new(),
            root_fields: vec![],
            file: e.file.clone(),
            line: e.line,
            evidence: "configured_endpoint".into(),
        };
        // Several readers may record the same variable.
        if !links.contains(&link) {
            links.push(link);
        }
    }
}

fn limitations() -> Vec<String> {
    vec![
        "GraphQL SDL is read for types, fields, arguments, enum values and union members; directives (including federation @key) are recorded as text, not interpreted. Subgraphs are not composed into a supergraph.".into(),
        "Operations are matched by their root fields only. Nested selections, variables and fragments outside the same document are not checked against the schema; consumers of a change are known only for root fields.".into(),
        "Documents come from .graphql/.graphqls/.gql files and gql/graphql tagged templates in TS/JS. Generated clients, persisted queries and runtime traffic are not read; a link is static evidence, not an observed call.".into(),
        "Changes compare against the baseline saved with `archguard contracts --graphql-baseline`, not a Git merge base.".into(),
    ]
}
fn report(root: &Path, discovery: &Discovery, topology: &ServiceTopology) -> Option<GraphqlReport> {
    let scan = scan(root, discovery, topology);
    if scan.sources.is_empty() && scan.diagnostics.is_empty() {
        return None;
    }
    let schemas = contracts(&scan);
    let (mut links, mut unresolved) = link(&scan, &schemas);
    endpoints(root, topology, &schemas, &mut links, &mut unresolved);
    let mut diagnostics = scan.diagnostics;
    let (comparison_status, changes) =
        match crate::project_files::read_repo_file(root, Path::new(BASELINE), 8 * MAX_BYTES) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ("no_baseline", vec![]),
            Ok(bytes) => match serde_json::from_slice::<Baseline>(&bytes) {
                Ok(b) if b.schema_version == 1 => (
                    "compared",
                    crate::graphql_changes::changes(&b.schemas, &schemas, &links),
                ),
                _ => {
                    diagnostics.push(format!("{BASELINE} is invalid; save the baseline again."));
                    ("baseline_invalid", vec![])
                }
            },
            Err(e) => {
                diagnostics.push(format!("{BASELINE} cannot be read safely: {e}"));
                ("baseline_invalid", vec![])
            }
        };
    diagnostics.truncate(200);
    Some(GraphqlReport {
        status: if diagnostics.is_empty() {
            "ready"
        } else {
            "partial"
        }
        .into(),
        schemas,
        links,
        unresolved,
        comparison_status: comparison_status.into(),
        changes,
        diagnostics,
        limitations: limitations(),
    })
}
fn discovery(root: &Path) -> Result<Discovery, ArchguardError> {
    Discovery::new(root, &mut ServiceTopology::default())
}
/// The GraphQL part of the contract report; `None` without any GraphQL document.
pub(crate) fn read(root: &Path, topology: &ServiceTopology) -> Option<GraphqlReport> {
    match discovery(root) {
        Ok(d) => report(root, &d, topology),
        Err(e) => Some(GraphqlReport {
            status: "error".into(),
            schemas: vec![],
            links: vec![],
            unresolved: vec![],
            comparison_status: "no_baseline".into(),
            changes: vec![],
            diagnostics: vec![e.to_string()],
            limitations: limitations(),
        }),
    }
}
/// Saves the current GraphQL schemas as the comparison baseline.
pub fn save_graphql_baseline(
    root: &Path,
    topology: &ServiceTopology,
) -> Result<usize, ArchguardError> {
    let d = discovery(root)?;
    let schemas = contracts(&scan(root, &d, topology));
    let count = schemas.len();
    let bytes = serde_json::to_vec_pretty(&Baseline {
        schema_version: 1,
        schemas,
    })
    .map_err(|e| ArchguardError::Analysis(e.to_string()))?;
    crate::project_files::write_project_file(
        root,
        Path::new("graphql-contracts.json"),
        &bytes,
        true,
    )?;
    Ok(count)
}
/// Adds `HttpCall` edges for operation documents linked to exactly one provider.
pub(crate) fn augment(root: &Path, discovery: &Discovery, topology: &mut ServiceTopology) {
    let scan = scan(root, discovery, topology);
    let schemas = contracts(&scan);
    for l in link(&scan, &schemas).0 {
        let evidence = ServiceEvidence {
            file: l.file.clone(),
            pointer: format!("graphql:{}", l.operation),
            line: l.line,
            description: format!(
                "GraphQL {} selects root fields {}, which only the schema of {} declares. Static evidence; no runtime call is proven.",
                l.operation,
                l.root_fields.join(", "),
                l.provider
            ),
        };
        match topology.dependencies.iter_mut().find(|d| {
            d.from == l.consumer && d.to == l.provider && d.kind == ServiceDependencyKind::HttpCall
        }) {
            Some(d) if !d.evidence.contains(&evidence) => d.evidence.push(evidence),
            Some(_) => {}
            None => topology.dependencies.push(ServiceDependency {
                from: l.consumer,
                to: l.provider,
                kind: ServiceDependencyKind::HttpCall,
                evidence: vec![evidence],
            }),
        }
    }
}
