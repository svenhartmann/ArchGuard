//! Object literals in calls of the TypeScript/JavaScript Elasticsearch and OpenSearch clients
//! become artifacts (ticket 26). Rules: a file counts only when it imports `@elastic/elasticsearch`
//! (flavor empty) or `@opensearch-project/opensearch` (flavor `opensearch`) by `import` or
//! `require`; the receiver of a call is a client when its last name was assigned `new <Client>(`
//! or is typed `: <Client>` in that file, so a same-named method of another object does not
//! match. Calls (name position, API of the 8.x client; the 7.x `body: {…}` is flattened in):
//! `indices.create` (`index`), `indices.putIndexTemplate` and `cluster.putComponentTemplate`
//! (`name`), `indices.putMapping` (`index`), `ilm.putLifecycle` (`name`, or the string `policy`
//! of 7.x) and `ingest.putPipeline` (`id`). The name must be a string literal or a module
//! constant of the same file, else the call is a gap. Each listed attribute (`mappings`,
//! `settings`, `template`, `policy`, `processors`, ...) is read only when it is entirely literal:
//! strings (a template string with `${}` is none), numbers, booleans, null, arrays, objects,
//! shorthand and identifiers naming one top-level `const` of the file; a spread, call, other
//! expression or a constant that is not unique is a gap for that attribute, the rest is kept.
//! `scope` = `ts:<folder of the nearest package.json>`, evidence = the call line. Trust boundary:
//! files below the root through the safe reader, at most 5 000 files, no test paths, no
//! network, nothing executed; contents are data.
// shortcut: constants of other files, class members and `let` are not resolved (gap); the
// receiver check is per file by name, not by scope; lift when a corpus needs it.
use super::json::search_definition;
use crate::context::{SearchContext, SourceOutput};
use crate::findings::{EsGap, es_gap};
use archguard_core::{SearchArtifact, ServiceEvidence, ServiceTopology};
use archguard_flow_common::support::{relative, test_path};
use serde_json::{Map, Number, Value};
use std::cell::Cell;
use std::collections::HashMap;

const MAX_FILES: usize = 5_000;
/// Nesting of one literal, constants followed per chain, and work (values built plus constants
/// followed) per call: a fat constant used many times runs the budget out and becomes a gap.
const MAX_NEST: u8 = 64;
const MAX_CHAIN: u8 = 8;
const MAX_WORK: u32 = 100_000;
const EXTS: [&str; 8] = ["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];
const SKIPPED_DIRS: [&str; 3] = [".git", "node_modules", "dist"];
const PACKAGES: [(&str, &str); 2] = [
    ("@elastic/elasticsearch", ""),
    ("@opensearch-project/opensearch", "opensearch"),
];
const MAPPING_KEYS: &[&str] = &[
    "properties",
    "dynamic",
    "dynamic_templates",
    "_source",
    "_meta",
    "_routing",
    "runtime",
    "date_detection",
    "numeric_detection",
];

/// A client method: its namespace, the artifact kind, the keys that may hold the name, the
/// attributes read from the request, and the key the attributes go below (`putMapping`).
struct Api {
    ns: &'static str,
    method: &'static str,
    kind: &'static str,
    names: &'static [&'static str],
    attrs: &'static [&'static str],
    wrap: Option<&'static str>,
}

const APIS: [Api; 6] = [
    Api {
        ns: "indices",
        method: "create",
        kind: "index",
        names: &["index"],
        attrs: &["mappings", "settings", "aliases"],
        wrap: None,
    },
    Api {
        ns: "indices",
        method: "putIndexTemplate",
        kind: "index_template",
        names: &["name"],
        attrs: &["index_patterns", "composed_of", "template"],
        wrap: None,
    },
    Api {
        ns: "cluster",
        method: "putComponentTemplate",
        kind: "component_template",
        names: &["name"],
        attrs: &["template"],
        wrap: None,
    },
    Api {
        ns: "indices",
        method: "putMapping",
        kind: "index",
        names: &["index"],
        attrs: MAPPING_KEYS,
        wrap: Some("mappings"),
    },
    Api {
        ns: "ilm",
        method: "putLifecycle",
        kind: "ilm_policy",
        names: &["name", "policy"],
        attrs: &["policy"],
        wrap: None,
    },
    Api {
        ns: "ingest",
        method: "putPipeline",
        kind: "ingest_pipeline",
        names: &["id"],
        attrs: &["processors", "on_failure"],
        wrap: None,
    },
];

pub(crate) fn extract_with(ctx: &SearchContext) -> SourceOutput {
    let mut out = ServiceTopology::default();
    let mut files = candidates(ctx);
    if files.len() > MAX_FILES {
        files.truncate(MAX_FILES);
        let n = MAX_FILES.to_string();
        es_gap(&mut out, EsGap::ScanStop, ".", None, &[&n]);
    }
    let mut scopes = HashMap::new();
    for rel in &files {
        let Some(text) = (ctx.read)(ctx.root, &ctx.root.join(rel)) else {
            es_gap(&mut out, EsGap::Unread, rel, None, &[]);
            continue;
        };
        if PACKAGES.iter().any(|(p, _)| text.contains(p)) {
            let scope = package_scope(ctx, rel, &mut scopes);
            FileScan::new(&text, rel, scope, &mut out).run();
        }
    }
    SourceOutput::from_scratch(out)
}

fn candidates(ctx: &SearchContext) -> Vec<String> {
    let mut files: Vec<String> = ctx
        .discovery
        .walk(ctx.root)
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| relative(ctx.root, e.path()))
        .filter(|rel| EXTS.contains(&rel.rsplit('.').next().unwrap_or("")))
        .filter(|rel| !test_path(rel) && !rel.split('/').any(|p| SKIPPED_DIRS.contains(&p)))
        .collect();
    files.sort();
    files
}

/// `ts:<folder of the nearest package.json>`, `ts:.` without one; cached per folder.
fn package_scope(ctx: &SearchContext, rel: &str, cache: &mut HashMap<String, String>) -> String {
    let mut dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
    let start = dir.to_owned();
    let found = loop {
        if let Some(s) = cache.get(dir) {
            break s.clone();
        }
        let manifest = ctx.root.join(dir).join("package.json");
        if (ctx.read)(ctx.root, &manifest).is_some() {
            break if dir.is_empty() { "." } else { dir }.to_owned();
        }
        match dir.rsplit_once('/') {
            Some((up, _)) => dir = up,
            None if dir.is_empty() => break ".".to_owned(),
            None => dir = "",
        }
    };
    cache.insert(start, found.clone());
    format!("ts:{found}")
}

/// One file: its client variables, module constants and the calls on them.
struct FileScan<'a> {
    text: &'a str,
    rel: &'a str,
    scope: String,
    newlines: Vec<usize>,
    code: Vec<bool>,
    consts: HashMap<String, Option<usize>>,
    out: &'a mut ServiceTopology,
}

impl<'a> FileScan<'a> {
    fn new(text: &'a str, rel: &'a str, scope: String, out: &'a mut ServiceTopology) -> Self {
        Self {
            text,
            rel,
            scope,
            newlines: text.match_indices('\n').map(|(i, _)| i).collect(),
            code: code_mask(text),
            consts: constants(text),
            out,
        }
    }

    fn run(&mut self) {
        let clients = clients(self.text);
        for api in &APIS {
            for (p, _) in self.text.match_indices(api.method) {
                if !self.code[p] {
                    continue;
                }
                let Some(receiver) = receiver_of(self.text, p, api) else {
                    continue;
                };
                if let Some(flavor) = clients.get(receiver) {
                    self.call(api, p, flavor);
                }
            }
        }
    }

    fn line(&self, at: usize) -> u32 {
        self.newlines.partition_point(|&n| n < at) as u32 + 1
    }

    fn call(&mut self, api: &Api, at: usize, flavor: &str) {
        let line = self.line(at);
        let open = self.text[at + api.method.len()..].trim_start();
        let from = self.text.len() - open.len() + 1;
        let steps = Cell::new(MAX_WORK);
        let mut lit = Lit::new(self.text, from, &self.consts, &steps);
        let entries = lit.object_at().and_then(|()| lit.entries(true));
        let Some(mut entries) = entries else {
            return self.not_a_name(line);
        };
        let Some(name) = take_name(&mut entries, api.names) else {
            return self.not_a_name(line);
        };
        let address = format!("{}:{line}", self.rel);
        let attrs = self.attrs(api, entries, &address);
        let body = match api.wrap {
            Some(w) => Map::from_iter([(w.to_owned(), Value::Object(attrs))]),
            None => attrs,
        };
        let Some(def) = search_definition(&Value::Object(body), Some(api.kind)) else {
            return;
        };
        let flavor = if def.flavor.is_empty() {
            flavor
        } else {
            def.flavor
        };
        self.out.elasticsearch.artifacts.push(SearchArtifact {
            flavor: flavor.into(),
            kind: api.kind.into(),
            address,
            scope: self.scope.clone(),
            patterns: def.patterns,
            references: def.references,
            evidence: vec![ServiceEvidence {
                file: self.rel.into(),
                pointer: format!("{}.{}", api.ns, api.method),
                line: Some(line),
                description: format!(
                    "{} {name} declared by client call {}.{}; read as data, no cluster access.",
                    api.kind, api.ns, api.method
                ),
            }],
            origins: vec![],
            detail: def.detail,
            name: Some(name),
        });
    }

    /// The listed attributes that are entirely literal; each other one is a gap.
    fn attrs(&mut self, api: &Api, entries: Entries, address: &str) -> Map<String, Value> {
        let mut attrs = Map::new();
        for (key, value) in entries {
            if !api.attrs.contains(&key.as_str()) && key != "..." {
                continue;
            }
            match value {
                Some(v) => drop(attrs.insert(key, v)),
                None => es_gap(
                    self.out,
                    EsGap::DetailNotLiteral,
                    self.rel,
                    None,
                    &[address, &key],
                ),
            }
        }
        attrs
    }

    fn not_a_name(&mut self, line: u32) {
        es_gap(
            self.out,
            EsGap::IndexNotLiteral,
            self.rel,
            None,
            &[&line.to_string()],
        );
    }
}

/// The first of `keys` (in this order) holding a string literal; its entry is removed.
fn take_name(entries: &mut Vec<(String, Option<Value>)>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| {
        let at = entries
            .iter()
            .position(|(key, v)| key == k && matches!(v, Some(Value::String(_))))?;
        match entries.remove(at).1 {
            Some(Value::String(s)) => Some(s),
            _ => None,
        }
    })
}

/// The last name before `.<ns>.<method>` at `at` (`?.` and line breaks allowed), if the text
/// there is that call.
fn receiver_of<'t>(text: &'t str, at: usize, api: &Api) -> Option<&'t str> {
    text[at + api.method.len()..]
        .trim_start()
        .strip_prefix('(')?;
    let dot = |s: &'t str| {
        s.trim_end()
            .strip_suffix('.')
            .map(|s| s.trim_end_matches('?').trim_end())
    };
    let head = dot(&text[..at])?.strip_suffix(api.ns)?;
    let name = last_ident(dot(head)?);
    (!name.is_empty()).then_some(name)
}

/// Per byte: true in code, false inside a comment or a string (quotes included), built in one
/// pass. shortcut: regex literals and `${}` in template strings are not lexed; lift when a
/// corpus needs it.
fn code_mask(text: &str) -> Vec<bool> {
    let b = text.as_bytes();
    let mut mask = vec![true; b.len()];
    let mut i = 0;
    while i < b.len() {
        let end = match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => text[i..].find('\n').map_or(b.len(), |n| i + n),
            b'/' if b.get(i + 1) == Some(&b'*') => {
                text[i + 2..].find("*/").map_or(b.len(), |n| i + n + 4)
            }
            q @ (b'\'' | b'"' | b'`') => {
                let mut j = i + 1;
                while j < b.len() && b[j] != q && (q == b'`' || b[j] != b'\n') {
                    j += 1 + usize::from(b[j] == b'\\');
                }
                (j + 1).min(b.len())
            }
            _ => {
                i += 1;
                continue;
            }
        };
        mask[i..end].fill(false);
        i = end;
    }
    mask
}

fn last_ident(s: &str) -> &str {
    let s = s.trim_end();
    let start = s.rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'));
    &s[start.map_or(0, |i| i + 1)..]
}

/// Names that hold a client: assigned `new <Client>(`, or typed `: <Client>`; with the flavor of
/// the package the class came from.
fn clients(text: &str) -> HashMap<&str, &'static str> {
    let mut out = HashMap::new();
    for (class, flavor) in client_classes(text) {
        for (p, _) in text.match_indices(&format!("new {class}(")) {
            let left = text[..p].trim_end().strip_suffix('=').unwrap_or("");
            let name = last_ident(left);
            let typed = left
                .trim_end()
                .strip_suffix(name)
                .and_then(|l| l.trim_end().strip_suffix(':'));
            out.insert(typed.map_or(name, last_ident), flavor);
        }
        let needle = format!(": {class}");
        for (p, _) in text.match_indices(&needle) {
            let next = text[p + needle.len()..].chars().next();
            if !next.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                out.insert(last_ident(&text[..p]), flavor);
            }
        }
    }
    out.retain(|k, _| !k.is_empty());
    out
}

/// How the file names the client class: `Client`, an alias, or `<namespace>.Client`.
fn client_classes(text: &str) -> Vec<(String, &'static str)> {
    let mut out = vec![];
    for (pkg, flavor) in PACKAGES {
        for (p, _) in text.match_indices(pkg) {
            let head = &text[..p];
            let from = ["\nimport ", "\nconst ", "\nlet ", "\nvar ", "\nexport "]
                .iter()
                .filter_map(|k| head.rfind(k))
                .max()
                .map_or(0, |i| i + 1);
            let stmt = head[from..]
                .trim_end()
                .trim_end_matches(['\'', '"', '('])
                .trim_end();
            let bound = stmt
                .strip_suffix("from")
                .or_else(|| stmt.strip_suffix("require"));
            let Some(bound) = bound.map(|b| b.trim_end().trim_end_matches('=')) else {
                continue;
            };
            out.extend(bindings(bound).into_iter().map(|c| (c, flavor)));
        }
    }
    out
}

/// `import type { Client as Es } from` / `const { Client: Es } =` / `* as es` / `const es =`.
fn bindings(bound: &str) -> Vec<String> {
    let bound = bound
        .split_once(char::is_whitespace)
        .map_or("", |(_, r)| r.trim());
    let bound = bound.strip_prefix("type ").unwrap_or(bound);
    if let Some((_, rest)) = bound.split_once('{') {
        let inner = rest.split('}').next().unwrap_or("");
        return inner
            .split(',')
            .filter_map(|item| {
                let item = item.trim().strip_prefix("type ").unwrap_or(item.trim());
                let (orig, local) = item.split_once(" as ").or_else(|| item.split_once(':'))?;
                (orig.trim() == "Client").then(|| local.trim().to_owned())
            })
            .chain(
                inner
                    .split(',')
                    .map(str::trim)
                    .filter(|i| *i == "Client")
                    .map(str::to_owned),
            )
            .collect();
    }
    let ns = bound.strip_prefix("* as ").unwrap_or(bound).trim();
    vec![format!("{ns}.Client")]
}

/// Top-level `const NAME = value` declarations: the offset of the value, `None` when the name
/// is declared twice.
fn constants(text: &str) -> HashMap<String, Option<usize>> {
    let mut out = HashMap::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let rest = line.strip_prefix("export ").unwrap_or(line);
        if let Some(decl) = rest.strip_prefix("const ") {
            let name = decl
                .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                .next();
            let eq = decl.find('=').filter(|_| !decl.contains('('));
            if let (Some(name), Some(eq)) = (name.filter(|n| !n.is_empty()), eq) {
                let value = at + line.len() - decl.len() + eq + 1;
                let first = !out.contains_key(name);
                out.insert(name.to_owned(), first.then_some(value));
            }
        }
        at += line.len();
    }
    out
}

type Entries = Vec<(String, Option<Value>)>;

/// A strict reader of JavaScript literals over `text` from offset `i`.
struct Lit<'a> {
    text: &'a str,
    i: usize,
    consts: &'a HashMap<String, Option<usize>>,
    steps: &'a Cell<u32>,
    depth: u8,
    nest: u8,
}

impl<'a> Lit<'a> {
    fn new(
        text: &'a str,
        i: usize,
        consts: &'a HashMap<String, Option<usize>>,
        steps: &'a Cell<u32>,
    ) -> Self {
        Self {
            text,
            i,
            consts,
            steps,
            depth: 0,
            nest: 0,
        }
    }

    fn at(&self) -> Option<u8> {
        self.text.as_bytes().get(self.i).copied()
    }

    /// Skips white space and comments.
    fn ws(&mut self) {
        loop {
            let rest = &self.text[self.i..];
            let t = rest.trim_start();
            self.i += rest.len() - t.len();
            if t.starts_with("//") {
                self.i += t.find('\n').unwrap_or(t.len());
            } else if t.starts_with("/*") {
                self.i += t.find("*/").map_or(t.len(), |e| e + 2);
            } else {
                return;
            }
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        let hit = self.at() == Some(c);
        self.i += usize::from(hit);
        hit
    }

    /// At the `{` that opens the first argument.
    fn object_at(&mut self) -> Option<()> {
        self.ws();
        (self.at() == Some(b'{')).then_some(())
    }

    fn string(&mut self) -> Option<String> {
        let quote = self.at()?;
        let start = self.i + 1;
        let mut out = String::new();
        let mut chars = self.text[start..].char_indices();
        while let Some((k, c)) = chars.next() {
            match c {
                '\\' => out.push(match chars.next()?.1 {
                    'n' => '\n',
                    't' => '\t',
                    c @ ('\\' | '\'' | '"' | '`') => c,
                    _ => return None,
                }),
                '\n' if quote != b'`' => return None,
                c if c == quote as char => {
                    self.i = start + k + 1;
                    let raw = &self.text[start..start + k];
                    return (quote != b'`' || !raw.contains("${")).then_some(out);
                }
                c => out.push(c),
            }
        }
        None
    }

    fn number(&mut self) -> Option<Value> {
        let rest = &self.text[self.i..];
        let len = rest
            .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')))
            .unwrap_or(rest.len());
        self.i += len;
        rest[..len].parse::<Number>().ok().map(Value::Number)
    }

    fn ident(&mut self) -> Option<&'a str> {
        let rest = &self.text[self.i..];
        let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
        let len = rest.find(|c: char| !word(c)).unwrap_or(rest.len());
        self.i += len;
        (len > 0 && !rest.starts_with(|c: char| c.is_ascii_digit())).then(|| &rest[..len])
    }

    /// Nothing after the value but a delimiter or `as const`.
    fn finished(&mut self) -> bool {
        self.ws();
        self.at().is_none_or(|c| b",}]);".contains(&c)) || self.text[self.i..].starts_with("as ")
    }

    fn value(&mut self) -> Option<Value> {
        self.charge()?;
        self.ws();
        match self.at()? {
            b'\'' | b'"' | b'`' => self.string().map(Value::String),
            b'{' => self.nested(Self::object),
            b'[' => self.nested(Self::array),
            b'-' | b'0'..=b'9' => self.number(),
            _ => {
                let word = self.ident()?;
                match word {
                    "true" | "false" | "null" => serde_json::from_str(word).ok(),
                    _ => self.lookup(word).filter(|_| self.finished()),
                }
            }
        }
    }

    fn charge(&self) -> Option<()> {
        self.steps.set(self.steps.get().checked_sub(1)?);
        Some(())
    }

    /// The value of the one top-level constant `name`.
    fn lookup(&self, name: &str) -> Option<Value> {
        let at = (*self.consts.get(name)?)?;
        self.charge()?;
        if self.depth >= MAX_CHAIN {
            return None;
        }
        let mut sub = Lit {
            depth: self.depth + 1,
            ..Lit::new(self.text, at, self.consts, self.steps)
        };
        sub.value()
            .filter(|_| sub.statement_ends() || sub.finished())
    }

    /// ASI style: the value is followed by a line break and the next statement.
    fn statement_ends(&self) -> bool {
        let rest = &self.text.as_bytes()[self.i..];
        let gap = rest.iter().take_while(|c| c.is_ascii_whitespace()).count();
        let next = rest.get(gap);
        rest[..gap].contains(&b'\n')
            && next.is_some_and(|c| {
                c.is_ascii_alphabetic() || b"_$".contains(c) || rest[gap..].starts_with(b"//")
            })
    }

    /// `read` with the nesting counted; deeper than `MAX_NEST` is a gap, not a stack overflow.
    fn nested(&mut self, read: fn(&mut Self) -> Option<Value>) -> Option<Value> {
        if self.nest >= MAX_NEST {
            return None;
        }
        self.nest += 1;
        let value = read(self);
        self.nest -= 1;
        value
    }

    fn key(&mut self) -> Option<String> {
        self.ws();
        match self.at()? {
            b'\'' | b'"' => self.string(),
            b'0'..=b'9' => self.number().map(|n| n.to_string()),
            _ => self.ident().map(str::to_owned),
        }
    }

    fn object(&mut self) -> Option<Value> {
        self.i += 1;
        let mut map = Map::new();
        while !self.eat(b'}') {
            let key = self.key()?;
            let value = if self.eat(b':') {
                self.value()?
            } else {
                self.lookup(&key)?
            };
            map.insert(key, value);
            if !self.eat(b',') && self.at() != Some(b'}') {
                return None;
            }
        }
        Some(Value::Object(map))
    }

    fn array(&mut self) -> Option<Value> {
        self.i += 1;
        let mut items = vec![];
        while !self.eat(b']') {
            items.push(self.value()?);
            if !self.eat(b',') && self.at() != Some(b']') {
                return None;
            }
        }
        Some(Value::Array(items))
    }

    /// The members of the object at `i`, each read on its own: a member that is not entirely
    /// literal is `None` and the next one is still read. `body: {…}` members are lifted.
    fn entries(&mut self, lift_body: bool) -> Option<Entries> {
        self.i += 1;
        let mut out = vec![];
        while !self.eat(b'}') {
            let start = self.i;
            let key = self.key().filter(|_| self.at() != Some(b'.'));
            let Some(key) = key else {
                self.i = start;
                self.skip_member();
                out.push(("...".to_owned(), None));
                if !self.eat(b',') && self.at() != Some(b'}') {
                    return None;
                }
                continue;
            };
            let value_at = self.i;
            if lift_body && key == "body" && self.eat(b':') && self.eat_peek(b'{') {
                out.extend(self.entries(false)?);
            } else {
                self.i = value_at;
                let value = if self.eat(b':') {
                    self.value()
                } else {
                    self.lookup(&key)
                };
                let value = value.filter(|_| self.finished());
                if value.is_none() {
                    self.i = value_at;
                    self.skip_member();
                }
                out.push((key, value));
            }
            if !self.eat(b',') && self.at() != Some(b'}') {
                return None;
            }
        }
        Some(out)
    }

    fn eat_peek(&mut self, c: u8) -> bool {
        self.ws();
        self.at() == Some(c)
    }

    /// Moves to the `,` or `}` that ends the member at `i`.
    fn skip_member(&mut self) {
        let (mut depth, mut quote) = (0usize, None);
        while let Some(c) = self.at() {
            let rest = &self.text.as_bytes()[self.i..]; // bytes: self.i may sit inside a multibyte char
            match (quote, c) {
                (Some(_), b'\\') => self.i += 1,
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, b'\'' | b'"' | b'`') => quote = Some(c),
                (None, b'/') if rest.starts_with(b"//") || rest.starts_with(b"/*") => {
                    self.ws();
                    continue;
                }
                (None, b'{' | b'[' | b'(') => depth += 1,
                (None, b'}' | b']' | b')' | b',') if depth == 0 => return,
                (None, b'}' | b']' | b')') => depth -= 1,
                _ => {}
            }
            self.i += 1;
        }
    }
}
