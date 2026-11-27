//! JSON strings that Java, C#, Python and TypeScript code passes as the body of an index or template
//! call (ADR 0023; ticket 27). Rules of this format:
//! - A call counts only at the name position of the client API: Python `.indices.create(index=, body=)`
//!   (also `put_template`, `put_index_template`, `put_component_template`, `put_mapping`,
//!   `ingest.put_pipeline`, `ilm.put_lifecycle`), TypeScript the camelCase methods with an object
//!   literal, Java `new CreateIndexRequest|PutIndexTemplateRequest|PutMappingRequest("x").source(B, ..)`
//!   (chained or on the variable it is assigned to) and `.indices().create(b -> b.index("x").withJson(new
//!   StringReader(B)))`, C# low-level `.Indices.Create<R>("x", PostData.String(B))`. No client import is needed.
//! - The name is a literal or a string constant of the service (`constants.rs`); the body is a string
//!   literal or a constant (declaration rules of `constants.rs`; a variable is a gap) whose whole value is one literal that starts a JSON object,
//!   found by its name (qualified: the file stem). Two declarations of one name are a gap; two calls with one name in one file keep both (`:line` suffix).
//! - Composed strings (`+`, `%`, `format`, interpolation), unknown escapes, files, variables, invalid
//!   JSON and spans over 2 000 tokens are a gap (`BodyNotLiteral`); object literals are not string bodies.
//!   A TypeScript `body: NAME` where `NAME = {` in a client-importing file is left to ts_bodies.rs.
//! - The kind comes from the shape of the JSON (`search_definition`); a body without ES shape yields nothing.
//!   `scope` = `code:<service>`; `flavor` is `opensearch` when the file imports an OpenSearch client.
//! - Trust boundary: files below the root through the safe reader (1 MiB, no symlinks), at most 5 000 files
//!   per service, no test paths; contents are data, nothing is executed.
// shortcut: constants are matched by name over the service, not by scope; `Class.NAME` compares the file stem.
use super::constants::{Constants, Name, declared_lines, lang, short, site};
use super::json::search_definition;
use super::string_bodies_calls::{Arg, Found, find, json_constants, leave_object_bodies};
use super::string_bodies_lex::{Lang, lex};
use crate::context::{SearchContext, SourceOutput};
use crate::findings::{EsGap, es_gap};
use archguard_core::{SearchArtifact, ServiceEvidence, ServiceTopology};
use archguard_flow_common::repo_file::read_repo_file;
use archguard_flow_common::support::{owners, relative, test_path};
use std::collections::BTreeMap;
use std::path::Path;

const MAX_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 5_000;
const SOURCES: [&str; 5] = ["java", "cs", "py", "ts", "tsx"];

/// A recognized call waiting for the constants of its whole service.
struct Pending {
    service: String,
    rel: String,
    ext: String,
    flavor: &'static str,
    found: Found,
}

/// One JSON string constant: the declaring file stem, the text and where it is declared.
struct Body {
    owner: String,
    text: String,
    evidence: ServiceEvidence,
}

type Bodies = BTreeMap<(String, &'static str, String), Vec<Body>>;

pub(crate) fn extract_with(ctx: &SearchContext) -> SourceOutput {
    let mut report = ServiceTopology::default();
    let (mut names, mut bodies, mut pending) = (Constants::default(), Bodies::new(), vec![]);
    let mut files: BTreeMap<String, usize> = BTreeMap::new();
    for entry in ctx.discovery.walk(ctx.root).flatten() {
        let rel = relative(ctx.root, entry.path());
        let ext = rel.rsplit('.').next().unwrap_or("");
        if !entry.file_type().is_some_and(|t| t.is_file())
            || !SOURCES.contains(&ext)
            || test_path(&rel)
        {
            continue;
        }
        let owners = owners(ctx.topology, &rel);
        let within: Vec<_> = owners
            .iter()
            .filter(|s| below_limit(&mut files, s, &rel, &mut report))
            .collect();
        if within.is_empty() {
            continue;
        }
        let Ok(bytes) = read_repo_file(ctx.root, Path::new(&rel), MAX_BYTES) else {
            for s in &owners {
                es_gap(&mut report, EsGap::Unread, &rel, Some(short(s)), &[]);
            }
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let toks = lex(&text, ext);
        let mut found = find(&toks, Lang::of(ext));
        leave_object_bodies(&toks, &mut found, ext, &text);
        let [service] = owners.as_slice() else {
            for s in owners.iter().filter(|_| !found.is_empty()) {
                es_gap(&mut report, EsGap::SameDir, &rel, Some(short(s)), &[]);
            }
            continue;
        };
        names.collect(service, ext, &rel, &text);
        let declared = declared_lines(&text, ext, &rel);
        for (name, text, line) in json_constants(&toks) {
            if !declared.contains(&line) {
                continue; // a local variable, field or argument is no constant
            }
            let key = (service.clone(), lang(ext), name.clone());
            let owner = rel.rsplit('/').next().unwrap_or(&rel);
            let owner = owner.rsplit_once('.').map_or(owner, |(s, _)| s);
            let evidence = site(&rel, line, &name, "JSON body constant declared here.");
            let body = Body {
                owner: owner.to_owned(),
                text,
                evidence,
            };
            bodies.entry(key).or_default().push(body);
        }
        let flavor = flavor_of(&text);
        pending.extend(found.into_iter().map(|found| Pending {
            service: service.clone(),
            rel: rel.clone(),
            ext: ext.to_owned(),
            flavor,
            found,
        }));
    }
    let mut artifacts = BTreeMap::new();
    for p in pending {
        settle(p, &names, &bodies, &mut artifacts, &mut report);
    }
    report
        .elasticsearch
        .artifacts
        .extend(artifacts.into_values());
    SourceOutput::from_scratch(report)
}

/// Counts a file against a service's limit; reports the first file past it.
fn below_limit(
    files: &mut BTreeMap<String, usize>,
    service: &str,
    rel: &str,
    report: &mut ServiceTopology,
) -> bool {
    let n = files.entry(service.to_owned()).or_default();
    *n += 1;
    if *n == MAX_FILES + 1 {
        let n = MAX_FILES.to_string();
        es_gap(report, EsGap::ScanStop, rel, Some(short(service)), &[&n]);
    }
    *n <= MAX_FILES
}

/// `opensearch` when an import line names an OpenSearch client.
fn flavor_of(text: &str) -> &'static str {
    let imports = |l: &str| {
        ["import ", "using ", "from "]
            .iter()
            .any(|p| l.starts_with(p))
            || l.contains("require(")
    };
    let hit = text
        .lines()
        .map(str::trim_start)
        .any(|l| imports(l) && l.to_ascii_lowercase().contains("opensearch"));
    if hit { "opensearch" } else { "" }
}

/// The text of the one constant `reference` names in `service`; any other count is no answer.
fn resolve_body<'a>(
    bodies: &'a Bodies,
    service: &str,
    ext: &str,
    reference: &str,
) -> Option<&'a Body> {
    let (qualifier, name) = match reference.rsplit_once('.') {
        Some((q, n)) => (Some(q.rsplit('.').next().unwrap_or(q)), n),
        None => (None, reference),
    };
    let key = (service.to_owned(), lang(ext), name.to_owned());
    let mut same = bodies
        .get(&key)?
        .iter()
        .filter(|b| qualifier.is_none_or(|q| q == b.owner));
    let (Some(b), None) = (same.next(), same.next()) else {
        return None;
    };
    Some(b)
}

/// Turns one call into an artifact, or into a gap when its name or body cannot be read.
fn settle(
    p: Pending,
    names: &Constants,
    bodies: &Bodies,
    artifacts: &mut BTreeMap<(String, String), SearchArtifact>,
    report: &mut ServiceTopology,
) {
    let line = p.found.line;
    let mut evidence = vec![site(
        &p.rel,
        line,
        &p.found.api,
        "Index or template definition passed as a JSON string; read as data, no cluster access.",
    )];
    let name = match &p.found.name {
        Name::Lit(n) => Some(n.clone()),
        Name::Ref(r) => names.resolve(&p.service, lang(&p.ext), r).map(|(v, at)| {
            evidence.push(at);
            v
        }),
        Name::Gap => None,
    };
    let text = match &p.found.body {
        Arg::Text(t) => t.clone(),
        Arg::Ref(r) => resolve_body(bodies, &p.service, &p.ext, r).map(|b| {
            evidence.push(b.evidence.clone());
            b.text.clone()
        }),
        Arg::Gap | Arg::Other => None,
    };
    let value = text.and_then(|t| serde_json::from_str(&t).ok());
    let (Some(name), Some(value)) = (name, value) else {
        let service = Some(short(&p.service));
        es_gap(
            report,
            EsGap::BodyNotLiteral,
            &p.rel,
            service,
            &[&line.to_string()],
        );
        return;
    };
    let Some(def) = search_definition(&value, None) else {
        return;
    };
    let flavor = if def.flavor.is_empty() {
        p.flavor
    } else {
        def.flavor
    };
    let mut address = format!("{}#{name}", p.rel);
    let scope = format!("code:{}", short(&p.service));
    if artifacts.contains_key(&(scope.clone(), address.clone())) {
        address = format!("{address}:{line}"); // a second call with the same name keeps its own body
    }
    let artifact = SearchArtifact {
        flavor: flavor.into(),
        kind: def.kind.into(),
        name: Some(name),
        address: address.clone(),
        scope: scope.clone(),
        patterns: def.patterns,
        references: def.references,
        evidence,
        origins: vec![],
        detail: def.detail,
    };
    artifacts.insert((scope, address), artifact);
}
