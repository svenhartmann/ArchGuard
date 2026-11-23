//! Index names a service's Java and C# source names literally, with the access the
//! surrounding call implies (ADR 0023, items 6 and 7). Names that are not literals
//! are reported as gaps; nothing is guessed from configuration.
use crate::project_files::read_repo_file;
use archguard_core::{SearchIndexUsage, ServiceEvidence, ServiceTopology};
use archguard_flow_common::support::{Discovery, diagnostic, owners, relative, test_path};
use std::collections::BTreeMap;
use std::path::Path;

const MAX_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 5_000;
const SOURCES: [&str; 2] = ["java", "cs"];
const IMPORTS: [&str; 5] = [
    "co.elastic.clients",
    "org.elasticsearch",
    "using Nest;",
    "using Nest.",
    "Elastic.Clients.Elasticsearch",
];
const GAP: &str =
    "Index name is not a literal (constant, variable or expression); the index stays unknown.";
const SAME_DIR: &str = "Several services build from the same source directory; indices named in this file are attributed to none of them.";
const UNREAD: &str = "Source file not read: larger than 1 MiB, not a regular file or not readable; its indices are unknown.";
const WRITE: [&str; 14] = [
    "IndexRequest",
    "IndexOperation",
    "BulkRequest",
    "BulkOperation",
    "CreateOperation",
    "UpdateRequest",
    "UpdateOperation",
    "DeleteRequest",
    "DeleteOperation",
    "DeleteByQueryRequest",
    "UpdateByQueryRequest",
    "Index",
    "IndexMany",
    "Bulk",
];
const READ: [&str; 9] = [
    "SearchRequest",
    "MsearchRequest",
    "MsearchTemplateRequest",
    "SearchTemplateRequest",
    "CountRequest",
    "GetRequest",
    "Search",
    "Count",
    "Get",
];

/// A call that names an index: the text that opens it, how many leading arguments
/// are not names, whether every later argument is one (Java varargs), whether the
/// call's own identifier counts for the access, and whether a non-literal argument
/// is always a name. Otherwise it is one only inside a builder or lambda, since
/// `client.index(request)` and `es.Index(doc, ...)` pass a request or a document.
struct Call {
    needle: &'static str,
    ext: &'static str,
    skip: usize,
    many: bool,
    own: bool,
    always: bool,
}

const fn call(needle: &'static str, ext: &'static str, skip: usize, flags: [bool; 3]) -> Call {
    Call {
        needle,
        ext,
        skip,
        many: flags[0],
        own: flags[1],
        always: flags[2],
    }
}

const CALLS: [Call; 7] = [
    call(".index(", "java", 0, [true, false, false]),
    call("new IndexRequest(", "java", 0, [false, true, true]),
    call("IndexCoordinates.of(", "java", 0, [false, false, true]),
    call(".Index(", "cs", 0, [false, false, false]),
    call("IndexMany(", "cs", 1, [false, true, true]),
    call("new IndexName(", "cs", 0, [false, true, true]),
    call(".DefaultIndex(", "cs", 0, [false, false, true]),
];
/// Arguments matched by their own call; the outer call ignores them.
const NESTED: [&str; 3] = ["Indices.Index(", "new IndexName(", "IndexCoordinates.of("];

/// One mention; `name` is `None` when the argument is not a literal.
struct Mention {
    name: Option<String>,
    access: &'static str,
    line: u32,
    pointer: &'static str,
}

pub(super) fn augment(root: &Path, discovery: &Discovery, report: &mut ServiceTopology) {
    let mut found: BTreeMap<(String, String, &str), Vec<ServiceEvidence>> = BTreeMap::new();
    let mut files: BTreeMap<String, usize> = BTreeMap::new();
    for entry in discovery.walk(root).flatten() {
        let rel = relative(root, entry.path());
        let ext = rel.rsplit('.').next().unwrap_or("");
        if !entry.file_type().is_some_and(|t| t.is_file())
            || !SOURCES.contains(&ext)
            || test_path(&rel)
        {
            continue;
        }
        let owners = owners(report, &rel);
        let below = owners
            .iter()
            .filter(|s| below_limit(&mut files, s, &rel, report))
            .count();
        if below == 0 {
            continue;
        }
        let Ok(bytes) = read_repo_file(root, Path::new(&rel), MAX_BYTES) else {
            owners
                .iter()
                .for_each(|s| diagnostic(report, &rel, Some(short(s)), UNREAD));
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        if !IMPORTS.iter().any(|i| text.contains(i)) {
            continue;
        }
        let [service] = owners.as_slice() else {
            owners
                .iter()
                .for_each(|s| diagnostic(report, &rel, Some(short(s)), SAME_DIR));
            continue;
        };
        for m in mentions(&text, ext) {
            let Some(name) = m.name else {
                diagnostic(report, &rel, Some(short(service)), GAP);
                continue;
            };
            let evidence = ServiceEvidence {
                file: rel.clone(),
                pointer: m.pointer.into(),
                line: Some(m.line),
                description: "Elasticsearch index named literally in source.".into(),
            };
            found
                .entry((service.clone(), name, m.access))
                .or_default()
                .push(evidence);
        }
    }
    report.elasticsearch.usages.extend(found.into_iter().map(
        |((service, index, access), mut evidence)| {
            evidence.sort();
            evidence.dedup();
            SearchIndexUsage {
                service,
                index,
                access: access.into(),
                evidence,
            }
        },
    ));
}

fn short(service: &str) -> &str {
    service.rsplit(':').next().unwrap_or(service)
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
        let message = format!(
            "Index scan of the service stopped after {MAX_FILES} files; indices in later files are unknown."
        );
        diagnostic(report, rel, Some(short(service)), message);
    }
    *n <= MAX_FILES
}

fn mentions(text: &str, ext: &str) -> Vec<Mention> {
    let mut out = Vec::new();
    for c in CALLS.iter().filter(|c| c.ext == ext) {
        for (at, _) in text.match_indices(c.needle) {
            let open = at + c.needle.len();
            let end = if c.own { open } else { at };
            let access = access(&text[statement_start(text, at)..end]);
            let line = text[..at].matches('\n').count() as u32 + 1;
            for name in names(c, text, at, open) {
                out.push(Mention {
                    name,
                    access,
                    line,
                    pointer: c.needle.trim_matches(|ch| ch == '.' || ch == '('),
                });
            }
        }
    }
    out
}

fn statement_start(text: &str, at: usize) -> usize {
    text[..at].rfind(';').map_or(0, |i| i + 1)
}

/// `write` or `read` by the nearest preceding identifier in the table, else `unknown`.
/// A C# `Index(` that takes a name is a naming call, not the write operation.
fn access(window: &str) -> &'static str {
    let mut window = window.to_owned();
    for name in std::iter::once("\"").chain(NESTED) {
        window = window.replace(&format!("Index({name}"), &format!("({name}"));
    }
    let tokens = window.split(|c: char| !(c.is_alphanumeric() || c == '_'));
    tokens
        .rev()
        .find_map(|t| {
            if WRITE.contains(&t) {
                Some("write")
            } else {
                READ.contains(&t).then_some("read")
            }
        })
        .unwrap_or("unknown")
}

/// The index names of one call: `Some(name)` for a literal, `None` for a gap.
/// Lambdas, nested name calls and request or document arguments are not names.
fn names(c: &Call, text: &str, at: usize, open: usize) -> Vec<Option<String>> {
    let window = &text[statement_start(text, at)..at];
    let builder = ["->", "=>", "Builder"].iter().any(|k| window.contains(k));
    let gaps = c.always || builder || window.ends_with("Indices");
    let mut args = split_args(&text[open..]);
    args.drain(..c.skip.min(args.len()));
    if !c.many {
        args.truncate(1);
    }
    let mut out = Vec::new();
    for arg in args {
        match arg
            .strip_prefix("List.of(")
            .or(arg.strip_prefix("Arrays.asList("))
        {
            Some(rest) => out.extend(split_args(rest).into_iter().filter_map(name)),
            None => out.extend(name(arg)),
        }
    }
    out.retain(|n| gaps || n.is_some());
    out
}

fn name(arg: &str) -> Option<Option<String>> {
    let skip = arg.is_empty()
        || arg.contains("->")
        || arg.contains("=>")
        || NESTED.iter().any(|n| arg.starts_with(n));
    if skip {
        return None;
    }
    let inner = arg.strip_prefix('"').and_then(|a| a.strip_suffix('"'));
    Some(
        inner
            .filter(|i| !i.is_empty() && !i.contains('"'))
            .map(str::to_owned),
    )
}

/// The top-level arguments of the call whose `(` just ended `s`, trimmed.
fn split_args(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut start, mut quote) = (Vec::new(), 0usize, 0, false);
    for (i, ch) in s.char_indices() {
        match ch {
            '"' => quote = !quote,
            _ if quote => {}
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => {
                out.push(s[start..i].trim());
                return out;
            }
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out
}
