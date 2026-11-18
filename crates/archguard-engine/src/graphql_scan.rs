//! Finds GraphQL documents (ADR 0014): schema files and `gql`/`graphql`
//! tagged templates in TS/JS, each with the service whose scope holds it.
use crate::graphql::{self, Document};
use crate::services::support::{Discovery, relative};
use archguard_core::ServiceTopology;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

pub(crate) const MAX_BYTES: u64 = 1 << 20;
const MAX_ENTRIES: usize = 50_000;
const SCHEMA_FILES: [&str; 3] = ["graphql", "graphqls", "gql"];
const SCRIPT_FILES: [&str; 6] = ["ts", "tsx", "js", "jsx", "mjs", "cjs"];

/// One parsed document: a `.graphql` file or one tagged template.
pub(crate) struct Source {
    pub file: String,
    pub owner: Option<String>,
    pub text_hash: String,
    /// Line of the document start in its file (templates start mid-file).
    line_at: Vec<usize>,
    base: usize,
    pub doc: Document,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn test_path(rel: &str) -> bool {
    rel.split('/')
        .any(|p| matches!(p, "test" | "tests" | "__tests__" | "__mocks__"))
        || rel.contains(".test.")
        || rel.contains(".spec.")
}
/// The service whose source scope (build context or project path) is the
/// deepest one containing `rel`; `None` when none or several tie.
fn owner(topology: &ServiceTopology, rel: &str) -> Option<String> {
    let mut best: Option<(usize, BTreeSet<&str>)> = None;
    for s in &topology.services {
        let project = s
            .project
            .as_ref()
            .and_then(|id| topology.projects.iter().find(|p| p.id == *id))
            .map(|p| p.path.as_str());
        for dir in s.build_context.as_deref().into_iter().chain(project) {
            let dir = dir.trim_end_matches('/');
            let depth = if dir == "." || dir.is_empty() {
                0
            } else if rel.starts_with(&format!("{dir}/")) {
                dir.split('/').count()
            } else {
                continue;
            };
            match &mut best {
                Some((d, ids)) if *d == depth => {
                    ids.insert(&s.id);
                }
                Some((d, _)) if *d > depth => {}
                _ => best = Some((depth, BTreeSet::from([s.id.as_str()]))),
            }
        }
    }
    best.filter(|(_, ids)| ids.len() == 1)
        .and_then(|(_, ids)| ids.first().map(|s| s.to_string()))
}
fn tag_start(text: &str, at: usize) -> bool {
    !text[..at]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$' || c == '.')
}
/// Replaces a `${…}` interpolation starting at `i` by spaces; returns the end.
fn skip_interpolation(bytes: &[u8], mut i: usize, body: &mut String) -> usize {
    let mut depth = 0;
    while i < bytes.len() {
        depth += match bytes[i] {
            b'{' => 1,
            b'}' => -1,
            _ => 0,
        };
        body.push(if bytes[i] == b'\n' { '\n' } else { ' ' });
        i += 1;
        if depth == 0 && bytes[i - 1] == b'}' {
            break;
        }
    }
    i
}
/// The template text from `start` to the closing backtick, if any.
fn template_body(text: &str, start: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let mut body = String::new();
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            return Some(body);
        }
        if bytes[i] == b'\\' {
            body.push_str("  ");
            i += 2;
        } else if bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'{') {
            i = skip_interpolation(bytes, i, &mut body);
        } else {
            let c = text[i..].chars().next().unwrap_or(' ');
            body.push(c);
            i += c.len_utf8();
        }
    }
    None
}
/// Bodies of `gql` / `graphql` tagged templates with their byte offsets;
/// `${…}` interpolations become spaces.
fn templates(text: &str) -> Vec<(usize, String)> {
    let mut out = vec![];
    for tag in ["gql`", "graphql`", "gql(`", "graphql(`"] {
        for (at, _) in text.match_indices(tag) {
            let start = at + tag.len();
            if let Some(body) = template_body(text, start).filter(|_| tag_start(text, at)) {
                out.push((start, body));
            }
        }
    }
    out
}
fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}
impl Source {
    pub(crate) fn line(&self, at: usize) -> u32 {
        self.line_at.partition_point(|&s| s <= self.base + at) as u32
    }
}

pub(crate) struct Scan {
    pub sources: Vec<Source>,
    pub diagnostics: Vec<String>,
}
/// GraphQL documents of one file: the whole schema file or each template.
fn documents(
    root: &Path,
    entry: &ignore::DirEntry,
) -> Result<(String, String, Vec<(usize, String)>), String> {
    let path = entry.path();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let schema = SCHEMA_FILES.contains(&ext);
    let rel = relative(root, path);
    if !(schema || SCRIPT_FILES.contains(&ext))
        || !entry.file_type().is_some_and(|t| t.is_file())
        || test_path(&rel)
    {
        return Ok((rel, String::new(), vec![]));
    }
    let text = crate::project_files::read_repo_file(root, Path::new(&rel), MAX_BYTES)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    match text {
        Ok(text) if schema => Ok((rel, text.clone(), vec![(0, text)])),
        Ok(text) if text.contains("gql") || text.contains("graphql") => {
            let found = templates(&text);
            Ok((rel, text, found))
        }
        Ok(_) => Ok((rel, String::new(), vec![])),
        Err(e) if schema => Err(format!("{rel}: not read: {e}")),
        Err(_) => Ok((rel, String::new(), vec![])),
    }
}
impl Scan {
    fn add(&mut self, owner: Option<String>, rel: String, text: &str, found: Vec<(usize, String)>) {
        let text_hash = hash(text.as_bytes());
        let line_at = line_starts(text);
        for (base, body) in found {
            match graphql::parse(&body) {
                Ok(doc) => self.sources.push(Source {
                    file: rel.clone(),
                    owner: owner.clone(),
                    text_hash: text_hash.clone(),
                    line_at: line_at.clone(),
                    base,
                    doc,
                }),
                Err((at, message)) => {
                    let line = line_at.partition_point(|&s| s <= base + at);
                    self.diagnostics
                        .push(format!("{rel}:{line}: GraphQL not parsed: {message}"));
                }
            }
        }
    }
}
pub(crate) fn scan(root: &Path, discovery: &Discovery, topology: &ServiceTopology) -> Scan {
    let mut scan = Scan {
        sources: vec![],
        diagnostics: vec![],
    };
    for (index, entry) in discovery.walk(root).flatten().enumerate() {
        if index >= MAX_ENTRIES {
            scan.diagnostics.push(format!(
                "GraphQL scan stopped after {MAX_ENTRIES} entries; later files are not read."
            ));
            break;
        }
        match documents(root, &entry) {
            Ok((rel, text, found)) if !found.is_empty() => {
                scan.add(owner(topology, &rel), rel, &text, found)
            }
            Ok(_) => {}
            Err(e) => scan.diagnostics.push(e),
        }
    }
    scan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_skip_interpolations_and_other_tags() {
        let found =
            templates("const a = gql`query A { a }\n${F}`; const b = mygql`x`; graphql(`{ b }`)");
        assert_eq!(found.len(), 2);
        assert!(found[0].1.starts_with("query A { a }\n    "));
        assert_eq!(found[1].1, "{ b }");
    }

    #[test]
    fn deepest_unique_scope_owns_a_file() {
        let topology: ServiceTopology = serde_json::from_value(serde_json::json!({
            "schema_version":1,"root":"","monorepo":false,"monorepo_reason":"","compose_files":[],
            "services":[
                {"id":"a","name":"a","compose_file":"c","build_context":".","networks":[],"profiles":[]},
                {"id":"b","name":"b","compose_file":"c","build_context":"svc/b","networks":[],"profiles":[]},
                {"id":"c","name":"c","compose_file":"c","build_context":"svc/c","networks":[],"profiles":[]},
                {"id":"d","name":"d","compose_file":"c","build_context":"svc/c","networks":[],"profiles":[]}
            ],
            "dependencies":[],"cycles":[],"diagnostics":[],"complete":true
        }))
        .unwrap();
        assert_eq!(
            owner(&topology, "svc/b/schema.graphql").as_deref(),
            Some("b")
        );
        assert_eq!(
            owner(&topology, "svc/bx/schema.graphql").as_deref(),
            Some("a")
        );
        assert_eq!(
            owner(&topology, "svc/c/schema.graphql"),
            None,
            "two services share the scope"
        );
    }
}
