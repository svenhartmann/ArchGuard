//! Index names a service's Java, C#, Python and TypeScript source names literally, with the access the
//! surrounding call implies (ADR 0023, items 6 and 7). Names that are not literals
//! are reported as gaps; nothing is guessed from configuration.
use crate::project_files::read_repo_file;
use archguard_core::{SearchIndexUsage, ServiceEvidence, ServiceTopology};
use archguard_flow_common::support::{Discovery, diagnostic, owners, relative, test_path};
#[path = "service_elasticsearch_constants.rs"]
pub(super) mod constants;

use constants::{Constants, Name, gap, literal, short, site};

pub(super) const SCAN_STOP_HEAD: &str = "Index scan of the service stopped after ";
pub(super) const SCAN_STOP_TAIL: &str = " files; indices in later files are unknown.";
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

const MAX_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 5_000;
const SOURCES: [&str; 5] = ["java", "cs", "py", "ts", "tsx"];
const IMPORTS: [&str; 5] = [
    "co.elastic.clients",
    "org.elasticsearch",
    "using Nest;",
    "using Nest.",
    "Elastic.Clients.Elasticsearch",
];
const LITERAL: &str = "Elasticsearch index named literally in source.";
pub(super) const SAME_DIR: &str = "Several services build from the same source directory; indices named in this file are attributed to none of them.";
pub(super) const UNREAD: &str = "Source file not read: larger than 1 MiB, not a regular file or not readable; its indices are unknown.";
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
const READ: [&str; 10] = [
    "header", // `header(h -> h.index(X))` exists in the Java client only on Msearch request items
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

/// One mention.
struct Mention {
    name: Name,
    access: &'static str,
    line: u32,
    pointer: &'static str,
}

pub(super) fn augment(root: &Path, discovery: &Discovery, report: &mut ServiceTopology) {
    let mut found: BTreeMap<(String, String, &str), Vec<ServiceEvidence>> = BTreeMap::new();
    let mut files: BTreeMap<String, usize> = BTreeMap::new();
    let mut constants = Constants::default();
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
        let [service] = owners.as_slice() else {
            if imports_client(&text, ext) {
                owners
                    .iter()
                    .for_each(|s| diagnostic(report, &rel, Some(short(s)), SAME_DIR));
            }
            continue;
        };
        constants.collect(service, ext, &rel, &text);
        if !imports_client(&text, ext) {
            continue;
        }
        let found_here = match ext {
            "py" => script_mentions(&text, true),
            "ts" | "tsx" => script_mentions(&text, false),
            _ => mentions(&text, ext),
        };
        for m in found_here {
            let sink = Sink {
                found: &mut found,
                constants: &mut constants,
                report,
            };
            sink.take(m, service, ext, &rel);
        }
    }
    constants.settle(&mut found, report);
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

/// Where mentions go: literal usages, references waiting for constants, or gaps.
struct Sink<'a> {
    found: &'a mut BTreeMap<(String, String, &'static str), Vec<ServiceEvidence>>,
    constants: &'a mut Constants,
    report: &'a mut ServiceTopology,
}

impl Sink<'_> {
    fn take(self, m: Mention, service: &str, ext: &str, rel: &str) {
        match m.name {
            Name::Lit(name) => {
                let evidence = site(rel, m.line, m.pointer, LITERAL);
                let key = (service.to_owned(), name, m.access);
                self.found.entry(key).or_default().push(evidence);
            }
            Name::Ref(r) => {
                self.constants
                    .defer(service, ext, rel, r, (m.access, m.line, m.pointer))
            }
            Name::Gap => diagnostic(self.report, rel, Some(short(service)), gap(m.line)),
        }
    }
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
        let message = format!("{SCAN_STOP_HEAD}{MAX_FILES}{SCAN_STOP_TAIL}");
        diagnostic(report, rel, Some(short(service)), message);
    }
    *n <= MAX_FILES
}

fn mentions(text: &str, ext: &str) -> Vec<Mention> {
    let objects = object_variables(text);
    let lines: Vec<usize> = text.match_indices('\n').map(|(i, _)| i).collect();
    let mut out = Vec::new();
    for c in CALLS.iter().filter(|c| c.ext == ext) {
        for (at, _) in text.match_indices(c.needle) {
            let open = at + c.needle.len();
            let end = if c.own { open } else { at };
            let access = access(&text[statement_start(text, at)..end]);
            let line = line_of(&lines, at);
            for name in names(c, text, at, open, &objects) {
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

/// Line of byte offset `at`, given the newline offsets built once per file (binary search per hit).
fn line_of(newlines: &[usize], at: usize) -> u32 {
    newlines.partition_point(|&n| n < at) as u32 + 1
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
fn names(c: &Call, text: &str, at: usize, open: usize, objects: &HashSet<&str>) -> Vec<Name> {
    let window = &text[statement_start(text, at)..at];
    let builder = ["->", "=>", "Builder"].iter().any(|k| window.contains(k));
    let gaps = c.always || builder || window.ends_with("Indices") || builder_variable(text, at);
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
    out.retain(|n| gaps || matches!(n, Name::Lit(_)));
    out.retain(|n| !matches!(n, Name::Ref(r) if objects.contains(r.as_str())));
    out
}

/// The names declared once per file as request or operation objects (`IndexRequest<Doc> x`,
/// `var x = new IndexOperation...`); `index(x)` passes such an object whole instead of naming an index.
// shortcut: matched by name over the whole file, not by scope; upgrade if a shadowed name misleads.
fn object_variables(text: &str) -> HashSet<&str> {
    /// The type created by `= new X...` or by the factory `= X.of(` / `= X.<T>of(`.
    fn created_type(after: &str) -> Option<&str> {
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let a = after.strip_prefix('=')?.trim_start();
        let factory = {
            let rest = a.trim_start_matches(word);
            rest.starts_with(".of(") || rest.starts_with(".<")
        };
        let a = a.strip_prefix("new ").or(factory.then_some(a))?;
        a.split(|c: char| !word(c)).next()
    }
    let object = |t: &str| t.ends_with("Request") || t.ends_with("Operation");
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = HashSet::new();
    let mut start = None;
    for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (start, word(c)) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                start = None;
                let (before, after) = (text[..s].trim_end(), text[i..].trim_start());
                // Only the text next to the word matters; a lambda `->` must not scan back to a `<`.
                let near = before.char_indices().rev().nth(200).map_or(0, |(k, _)| k);
                let before = &before[near..];
                let ty = match before.strip_suffix('>') {
                    Some(g) => g.rsplit_once('<').map_or("", |(t, _)| t),
                    None => before,
                };
                let ty = ty.rsplit(|c: char| !word(c)).next().unwrap_or("");
                let created = created_type(after);
                if object(ty) || (ty == "var" && created.is_some_and(object)) {
                    out.insert(&text[s..i]);
                }
            }
            _ => {}
        }
    }
    out
}

fn name(arg: &str) -> Option<Name> {
    let skip = arg.is_empty()
        || arg.contains("->")
        || arg.contains("=>")
        || NESTED.iter().any(|n| arg.starts_with(n));
    if skip {
        return None;
    }
    let inner = arg.strip_prefix('"').and_then(|a| a.strip_suffix('"'));
    Some(match inner.filter(|i| !i.is_empty() && !i.contains('"')) {
        Some(i) => Name::Lit(i.to_owned()),
        None => constants::reference(arg),
    })
}

/// The top-level arguments of the call whose `(` just ended `s`, trimmed.
fn split_args(s: &str) -> Vec<&str> {
    split_quoted(s, &['"'])
}

/// `split_args` where each of `quotes` opens a string that runs to the same character.
fn split_quoted<'a>(s: &'a str, quotes: &[char]) -> Vec<&'a str> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0usize, 0);
    for ((i, ch), _) in s
        .char_indices()
        .zip(string_mask(s, quotes))
        .filter(|(_, in_str)| !*in_str)
    {
        match ch {
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

/// Lazily, per character: whether it belongs to a string, quotes included; `\` escapes.
fn string_mask<'a>(s: &'a str, quotes: &'a [char]) -> impl Iterator<Item = bool> + 'a {
    let (mut quote, mut escaped) = (None, false);
    s.chars().map(move |ch| match quote {
        None if quotes.contains(&ch) => {
            quote = Some(ch);
            true
        }
        None => false,
        Some(q) => {
            if !escaped && ch == q {
                quote = None;
            }
            escaped = !escaped && ch == '\\';
            true
        }
    })
}

/// Whether the receiver of the call at `at` is a local declared with a builder type,
/// e.g. `var b = new IndexRequest.Builder<Doc>();`; its `index(x)` takes a name.
// shortcut: declarations are matched per statement by name, not by scope; upgrade if a shadowed name misleads.
fn builder_variable(text: &str, at: usize) -> bool {
    let word = |s: &str| {
        let s = s.trim_end();
        let start = s
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map_or(0, |i| i + 1);
        s[start..].to_owned()
    };
    let receiver = word(&text[..at]);
    !receiver.is_empty()
        && text
            .split(';')
            .filter(|stmt| stmt.contains("Builder"))
            .any(|stmt| word(stmt.split('=').next().unwrap_or("")) == receiver)
}

/// Whether the file imports an Elasticsearch client; only import lines count.
fn imports_client(text: &str, ext: &str) -> bool {
    let mut lines = text.lines().map(str::trim_start);
    match ext {
        // elasticsearch_dsl and similar siblings count: they wrap the same client.
        "py" => lines
            .any(|l| l.starts_with("from elasticsearch") || l.starts_with("import elasticsearch")),
        "ts" | "tsx" => lines.any(|l| {
            (l.starts_with("import ") || l.contains("require("))
                && l.contains("@elastic/elasticsearch")
        }),
        _ => IMPORTS.iter().any(|i| text.contains(i)),
    }
}

/// Method names of the Python and TypeScript clients and the access they imply.
const METHODS: [(&str, &str); 9] = [
    ("index", "write"),
    ("create", "write"),
    ("update", "write"),
    ("delete", "write"),
    ("bulk", "write"),
    ("search", "read"),
    ("count", "read"),
    ("get", "read"),
    ("msearch", "read"),
];

/// Python (`index="x"`, `"_index": "x"` for bulk helpers) and TypeScript (`{ index: 'x' }`).
fn script_mentions(text: &str, python: bool) -> Vec<Mention> {
    let lines: Vec<usize> = text.match_indices('\n').map(|(i, _)| i).collect();
    let mut out = Vec::new();
    for (method, access) in METHODS {
        let needle = format!(".{method}(");
        for (at, _) in text.match_indices(&needle) {
            let args = split_quoted(&text[at + needle.len()..], &SCRIPT_QUOTES);
            let value = index_value(&args, python);
            out.extend(value.into_iter().flat_map(literals).map(|name| Mention {
                name,
                access,
                line: line_of(&lines, at),
                pointer: method,
            }));
        }
    }
    if python && text.contains("bulk(") {
        out.extend(bulk_helper_mentions(text));
    }
    out
}

const SCRIPT_QUOTES: [char; 3] = ['"', '\'', '`'];

/// The `index` argument: a keyword in Python, a property of the request object literal in
/// TypeScript.
fn index_value<'a>(args: &[&'a str], python: bool) -> Option<&'a str> {
    if python {
        return args.iter().find_map(|a| keyed(a, "index", '='));
    }
    let body = args.first().and_then(|a| a.strip_prefix('{'))?;
    split_quoted(body, &SCRIPT_QUOTES)
        .into_iter()
        .find_map(|a| keyed(a, "index", ':'))
}

/// `"_index": "x"` entries of bulk helper actions.
fn bulk_helper_mentions(text: &str) -> Vec<Mention> {
    let lines: Vec<usize> = text.match_indices('\n').map(|(i, _)| i).collect();
    let mut out = Vec::new();
    for (at, _) in text.match_indices("_index") {
        let rest = text[at + 6..].trim_start_matches(['"', '\'']).trim_start();
        let Some(value) = rest
            .strip_prefix(':')
            .filter(|_| text[..at].ends_with(['"', '\'']))
        else {
            continue;
        };
        let value = split_quoted(value, &['"', '\''])
            .first()
            .copied()
            .unwrap_or("");
        out.extend(literals(value).into_iter().map(|name| Mention {
            name,
            access: "write",
            line: line_of(&lines, at),
            pointer: "_index",
        }));
    }
    out
}

/// The value of `key<sep>value` (key optionally quoted), or a bare `key` in TypeScript shorthand.
fn keyed<'a>(arg: &'a str, key: &'a str, sep: char) -> Option<&'a str> {
    let rest = arg.trim_start_matches(['"', '\'']).strip_prefix(key)?;
    let rest = rest.trim_start_matches(['"', '\'']).trim_start();
    match rest.strip_prefix(sep) {
        Some(v) if !v.starts_with('=') => Some(v.trim()),
        None if rest.is_empty() && sep == ':' => Some(key),
        _ => None,
    }
}

/// A literal string or a list of them; every non-literal element is a gap (`None`).
fn literals(value: &str) -> Vec<Name> {
    let name_of = |v: &str| literal(v).map_or_else(|| constants::reference(v), Name::Lit);
    match value.strip_prefix('[') {
        Some(list) => split_quoted(list, &['"', '\'', '`'])
            .into_iter()
            .filter(|a| !a.is_empty())
            .map(name_of)
            .collect(),
        None => vec![name_of(value)],
    }
}
