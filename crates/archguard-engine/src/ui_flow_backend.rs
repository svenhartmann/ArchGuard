//! Backend endpoint access extractor (ASP.NET, Spring) for ui-flow/v1 (ADR 0021).
//!
//! A text-level scanner (no parser): comments are blanked, string literals are
//! skipped, brackets are matched. It fills only `endpoints` and `diagnostics`.
//!
//! Recognized:
//! - ASP.NET Core controllers (`.cs`, class named `*Controller` or marked
//!   `[ApiController]`/`[Controller]`): `[Route]` on the class with the
//!   `[controller]` and `[action]` tokens (the action name drops `Async`), and
//!   `[HttpGet]`…`[HttpOptions]` (or a method `[Route]`) on methods; a method
//!   template starting with `/` or `~/` replaces the class route. Access follows
//!   ASP.NET: `[AllowAnonymous]` on class or method wins (`open`), otherwise all
//!   `[Authorize]` of class and method must hold (`Roles` = any of the roles,
//!   `Policy` or the positional argument = a policy, bare = authenticated).
//! - ASP.NET minimal APIs: `MapGet/MapPost/MapPut/MapDelete/MapPatch("…")` on
//!   `MapGroup("…")` chains and on variables assigned from them in the same
//!   file; `.RequireAuthorization(…)`/`.AllowAnonymous()` on the endpoint, the
//!   group chain, as a statement on the group variable, or at the call sites of
//!   an extension method that returns the group
//!   (`app.MapOrdersApi().RequireAuthorization()` anywhere in the same project).
//! - Spring (`.java`, `.kt`) in `@RestController`/`@Controller` classes:
//!   `@RequestMapping` on the class with `@Get/Post/Put/Delete/PatchMapping` or
//!   `@RequestMapping(method = …)` on methods (`value`, `path` or positional,
//!   arrays give one endpoint each; `@RequestMapping` without `method` gives
//!   method `ANY`, which the join matches with every method). Access from the method, else the class: `@PreAuthorize` with
//!   `hasRole`, `hasAnyRole`, `hasAuthority`, `hasAnyAuthority`,
//!   `isAuthenticated()`, `isFullyAuthenticated()`, `and`/`or`/`!`/`not`/`&&`/`||`
//!   and parentheses, `permitAll()` alone = `open`; `@Secured`, `@RolesAllowed`,
//!   `@PermitAll`, `@DenyAll` (`never`). Authorities become `role` atoms too, and a `ROLE_` prefix is
//!   dropped everywhere, so `hasAuthority('ROLE_A')`, `hasRole('A')` and
//!   `@Secured("ROLE_A")` all read `role(A)`.
//!
//! Each endpoint also gets its handler method and the request types it sends
//! (`handlers`, ADR 0022 decision 4).
//!
//! A project is the nearest directory with a `.csproj`, `pom.xml` or
//! `build.gradle(.kts)`; its directory name is the endpoint's `application`.
//! Not recognized (left `undeclared`, or a diagnostic):
//! conventional routing (actions skipped with a diagnostic), `[AcceptVerbs]`,
//! `MapMethods`, `MapGroup` prefixes passed in from a call site, central rules
//! (`FallbackPolicy`; Spring `SecurityFilterChain` only gets one `note` per
//! project), other SpEL (`unknown`), constants as routes or roles (diagnostic,
//! the endpoint is skipped or the condition is `unknown`).
use crate::project_files::read_repo_file;
use crate::services::support::{Discovery, relative, test_path};
use archguard_core::{
    ANY_METHOD, AccessState, Condition, DiagnosticSeverity, EndpointAccess, ServiceTopology,
    UiFlow, UiFlowDiagnostic,
};
use lex::{ident_before, literal_end, split_top};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

pub(crate) use handlers::{sends_in, simple_type};

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_FILES: usize = 20_000;
const BUILD_FILES: [&str; 3] = ["pom.xml", "build.gradle", "build.gradle.kts"];
const SOURCES: [&str; 3] = [".cs", ".java", ".kt"];

/// Endpoints with their declared access, plus diagnostics.
pub fn extract(root: &Path) -> UiFlow {
    let mut out = Out(UiFlow::default());
    for (application, sources) in read_projects(root, &mut out).values() {
        let first = out.0.endpoints.len();
        let mut call_sites = HashMap::new();
        for src in sources {
            if src.rel.ends_with(".cs") {
                controllers::scan(src, &mut out);
                minimal::scan(src, sources, &mut call_sites, &mut out);
            } else {
                spring::scan(src, &mut out);
            }
        }
        spring::central_rules(sources, &mut out);
        for e in &mut out.0.endpoints[first..] {
            e.application = application.clone();
        }
    }
    let flow = &mut out.0;
    flow.endpoints.sort();
    flow.endpoints.dedup();
    flow.diagnostics.sort();
    flow.diagnostics.dedup();
    out.0
}

/// Application name and source files by project directory; unreadable files
/// become diagnostics.
fn read_projects(root: &Path, out: &mut Out) -> BTreeMap<String, Project> {
    let (projects, files) = discover(root, out);
    let mut by_project: BTreeMap<String, Project> = BTreeMap::new();
    for rel in files {
        match read_repo_file(root, Path::new(&rel), MAX_BYTES) {
            Ok(bytes) => by_project
                .entry(project_of(&rel, &projects))
                .or_insert_with_key(|dir| (application(root, dir, &projects), vec![]))
                .1
                .push(Source::new(rel, &String::from_utf8_lossy(&bytes))),
            Err(e) => out.diag(&rel, 0, format!("Source file not read (limit 1 MiB): {e}")),
        }
    }
    by_project
}

/// The owning application of a project directory, and its sources.
type Project = (Option<String>, Vec<Source>);

/// The project directory's name (the root's for a build file at the root);
/// none for files outside every project.
fn application(root: &Path, dir: &str, projects: &BTreeSet<String>) -> Option<String> {
    if !projects.contains(dir) {
        return None;
    }
    match dir.rsplit_once('/') {
        Some((_, name)) => Some(name.into()),
        None if dir.is_empty() => root.file_name().map(|n| n.to_string_lossy().into_owned()),
        None => Some(dir.into()),
    }
}

fn project_of(rel: &str, projects: &BTreeSet<String>) -> String {
    let mut dir = rel;
    loop {
        dir = dir.rsplit_once('/').map_or("", |(d, _)| d);
        if dir.is_empty() || projects.contains(dir) {
            return dir.to_string();
        }
    }
}

/// Directories holding a build file, and the source files to scan.
fn discover(root: &Path, out: &mut Out) -> (BTreeSet<String>, Vec<String>) {
    let (mut projects, mut files) = (BTreeSet::new(), Vec::new());
    let discovery = match Discovery::new(root, &mut ServiceTopology::default()) {
        Ok(d) => d,
        Err(e) => {
            out.diag(".", 0, format!("Discovery settings not read: {e}"));
            return (projects, files);
        }
    };
    for entry in discovery.walk(root) {
        let entry = match entry {
            Ok(entry) if entry.file_type().is_some_and(|t| t.is_file()) => entry,
            Ok(_) => continue,
            Err(e) => {
                out.diag(".", 0, format!("Directory entry not read: {e}"));
                continue;
            }
        };
        let rel = relative(root, entry.path());
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", &rel));
        if name.ends_with(".csproj") || BUILD_FILES.contains(&name) {
            projects.insert(dir.to_string());
        } else if SOURCES.iter().any(|e| name.ends_with(e)) && !test_path(&rel) {
            files.push(rel);
        }
    }
    files.sort();
    if files.len() > MAX_FILES {
        let message = format!("More than {MAX_FILES} source files; the rest were not scanned.");
        out.diag(".", 0, message);
        files.truncate(MAX_FILES);
    }
    (projects, files)
}

struct Out(UiFlow);
impl Out {
    /// A diagnostic for something not read literally: the flow is incomplete.
    fn diag(&mut self, file: &str, line: u32, message: String) {
        self.0.complete = false;
        self.0.diagnostics.push(UiFlowDiagnostic {
            file: file.into(),
            line,
            message,
            severity: DiagnosticSeverity::Gap,
        });
    }
    fn at(&mut self, src: &Source, at: usize, message: String) {
        self.diag(&src.rel, src.line(at), message);
    }
    fn unknown(&mut self, src: &Source, at: usize, text: &str) -> Condition {
        self.at(src, at, format!("Access condition not readable: {text}"));
        Condition::Unknown(text.into())
    }
    fn endpoint(
        &mut self,
        src: &Source,
        at: usize,
        method: &str,
        path: String,
        access: &AccessState,
    ) {
        let source = if src.rel.ends_with(".cs") {
            "aspnet"
        } else {
            "spring"
        };
        self.0.endpoints.push(EndpointAccess {
            method: method.into(),
            path,
            access: access.clone(),
            application: None, // set per project in `extract`
            file: src.rel.clone(),
            line: src.line(at),
            source: source.into(),
            handler: None,
            sends: vec![],
        });
    }
    /// Sets the handler and what it sends on the endpoints from index `from` on.
    fn link(&mut self, from: usize, (handler, sends): handlers::Link) {
        for e in &mut self.0.endpoints[from..] {
            e.handler = handler.clone();
            e.sends = sends.clone();
        }
    }
}

/// A source file with comments blanked (offsets and lines unchanged).
struct Source {
    rel: String,
    text: String,
    pairs: HashMap<usize, usize>,
    lines: Vec<usize>,
}
impl Source {
    fn new(rel: String, raw: &str) -> Self {
        let text = lex::strip_comments(raw);
        let b = text.as_bytes();
        let lines = std::iter::once(0)
            .chain(
                b.iter()
                    .enumerate()
                    .filter(|(_, c)| **c == b'\n')
                    .map(|(i, _)| i + 1),
            )
            .collect();
        Self {
            rel,
            pairs: bracket_pairs(b),
            text,
            lines,
        }
    }
    fn line(&self, at: usize) -> u32 {
        self.lines.partition_point(|&s| s <= at) as u32
    }
    fn pair(&self, at: usize) -> Option<usize> {
        self.pairs.get(&at).copied()
    }
    fn bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }
}

/// Matching bracket positions, both ways, outside literals.
fn bracket_pairs(b: &[u8]) -> HashMap<usize, usize> {
    let (mut pairs, mut stack, mut i) = (HashMap::new(), Vec::new(), 0);
    while i < b.len() {
        if let Some(end) = literal_end(b, i) {
            i = end;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => stack.push(i),
            b')' | b']' | b'}' => {
                if let Some(open) = stack.pop() {
                    pairs.insert(open, i);
                    pairs.insert(i, open);
                }
            }
            _ => {}
        }
        i += 1;
    }
    pairs
}

fn combine(mut items: Vec<Condition>, many: fn(Vec<Condition>) -> Condition) -> Option<Condition> {
    items.sort();
    items.dedup();
    match items.len() {
        0 => None,
        1 => items.pop(),
        _ => Some(many(items)),
    }
}

fn path(parts: &[&str]) -> String {
    let segments: Vec<&str> = parts
        .iter()
        .flat_map(|p| p.split('/'))
        .filter(|s| !s.is_empty())
        .collect();
    format!("/{}", segments.join("/"))
}

struct Attr {
    name: String,
    args: String,
    at: usize,
}

struct Decl {
    class: bool,
    name: String,
    attrs: Vec<Attr>,
    /// Offset of a method's parameter list; 0 for a class.
    open: usize,
}

fn has(attrs: &[Attr], name: &str) -> bool {
    attrs.iter().any(|a| a.name == name)
}

/// Last segment of a qualified attribute name, without `Attribute` and a target.
fn simple(name: &str) -> String {
    let name = name.rsplit([':', '.']).next().unwrap_or(name).trim();
    name.strip_suffix("Attribute")
        .filter(|n| !n.is_empty())
        .unwrap_or(name)
        .to_string()
}

/// Class and annotated method declarations with their attributes, in order.
/// `annotations`: Java/Kotlin `@X(…)`; otherwise C# `[X(…), Y]`.
fn decls(src: &Source, annotations: bool) -> Vec<Decl> {
    let b = src.bytes();
    let (mut out, mut pending, mut i) = (Vec::new(), Vec::new(), 0);
    while i < b.len() {
        i = match b[i] {
            c if c.is_ascii_whitespace() => i + 1,
            b'[' if !annotations => attribute_list(src, i, &mut pending),
            b'@' if annotations => annotation(src, i, &mut pending),
            _ => {
                let end = header_end(src, i);
                out.extend(declaration(src, i, end, std::mem::take(&mut pending)));
                end + 1
            }
        };
    }
    out
}

/// A C# `[A(…), B]` list at `i`; returns the position after it.
fn attribute_list(src: &Source, i: usize, pending: &mut Vec<Attr>) -> usize {
    let Some(close) = src.pair(i) else {
        return i + 1;
    };
    for part in split_top(&src.text[i + 1..close]) {
        let (name, args) = match part.find('(') {
            Some(p) if part.ends_with(')') => (&part[..p], &part[p + 1..part.len() - 1]),
            _ => (part, ""),
        };
        pending.push(Attr {
            name: simple(name),
            args: args.into(),
            at: i,
        });
    }
    close + 1
}

/// A Java/Kotlin `@A(…)` at `i`; returns the position after it.
fn annotation(src: &Source, i: usize, pending: &mut Vec<Attr>) -> usize {
    let b = src.bytes();
    let j = i
        + 1
        + b[i + 1..]
            .iter()
            .take_while(|c| lex::is_ident(**c) || matches!(c, b'.' | b':'))
            .count();
    let k = j + b[j..]
        .iter()
        .take_while(|c| matches!(c, b' ' | b'\t'))
        .count();
    let close = (b.get(k) == Some(&b'(')).then(|| src.pair(k)).flatten();
    pending.push(Attr {
        name: simple(&src.text[i + 1..j]),
        args: close.map_or("", |c| &src.text[k + 1..c]).into(),
        at: i,
    });
    close.map_or(j, |c| c + 1).max(i + 1)
}

/// End of a declaration or statement head: `{`, `;`, `}` or a line end outside brackets.
fn header_end(src: &Source, mut j: usize) -> usize {
    let b = src.bytes();
    while j < b.len() {
        if let Some(end) = literal_end(b, j) {
            j = end;
            continue;
        }
        match b[j] {
            b'(' | b'[' => j = src.pair(j).map_or(j + 1, |c| c + 1),
            b'{' | b';' | b'}' | b'\n' => return j,
            _ => j += 1,
        }
    }
    b.len()
}

fn declaration(src: &Source, start: usize, end: usize, attrs: Vec<Attr>) -> Option<Decl> {
    let header = &src.text[start..end];
    if let Some(name) = class_name(header, src.rel.ends_with(".kt")) {
        return Some(Decl {
            class: true,
            name,
            attrs,
            open: 0,
        });
    }
    let p = header.find('(').filter(|_| !attrs.is_empty())?;
    let (s, e) = ident_before(header.as_bytes(), p);
    (s < e).then(|| Decl {
        class: false,
        name: header[s..e].into(),
        attrs,
        open: start + p,
    })
}

fn class_name(header: &str, kotlin: bool) -> Option<String> {
    let head = header.split('(').next().unwrap_or(header);
    let mut words = head
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty());
    while let Some(w) = words.next() {
        if matches!(w, "class" | "interface" | "record" | "struct") || kotlin && w == "object" {
            return words.next().map(str::to_owned);
        }
    }
    None
}

/// Literals, brackets, identifiers and argument lists on raw text.
mod lex {
    /// End (exclusive) of the string or char literal starting at `i`, if one does.
    pub(super) fn literal_end(b: &[u8], i: usize) -> Option<usize> {
        match b[i] {
            b'"' if b[i..].starts_with(b"\"\"\"") => {
                Some(find(b, i + 3, b"\"\"\"").map_or(b.len(), |e| e + 3))
            }
            b'"' => Some(string_end(b, i)),
            b'\'' => char_end(b, i),
            _ => None,
        }
    }

    fn string_end(b: &[u8], i: usize) -> usize {
        let verbatim = i > 0 && b[i - 1] == b'@' || i > 1 && b[i - 1] == b'$' && b[i - 2] == b'@';
        let mut j = i + 1;
        while j < b.len() {
            match b[j] {
                b'\\' if !verbatim => j += 2,
                b'"' if verbatim && b.get(j + 1) == Some(&b'"') => j += 2,
                b'"' => return j + 1,
                b'\n' if !verbatim => return j,
                _ => j += 1,
            }
        }
        b.len()
    }

    fn char_end(b: &[u8], i: usize) -> Option<usize> {
        let mut j = i + 1;
        while j < b.len().min(i + 10) {
            match b[j] {
                b'\\' => j += 2,
                b'\'' => return Some(j + 1),
                b'\n' => return None,
                _ => j += 1,
            }
        }
        None
    }

    fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
        b.get(from..)?
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|p| p + from)
    }

    pub(super) fn strip_comments(text: &str) -> String {
        let mut b = text.as_bytes().to_vec();
        let mut i = 0;
        while i < b.len() {
            if let Some(end) = literal_end(&b, i) {
                i = end;
                continue;
            }
            let end = if b[i..].starts_with(b"//") {
                find(&b, i, b"\n").unwrap_or(b.len())
            } else if b[i..].starts_with(b"/*") {
                find(&b, i + 2, b"*/").map_or(b.len(), |e| e + 2)
            } else {
                i += 1;
                continue;
            };
            b[i..end]
                .iter_mut()
                .filter(|c| **c != b'\n')
                .for_each(|c| *c = b' ');
            i = end;
        }
        String::from_utf8_lossy(&b).into_owned()
    }

    pub(super) fn is_ident(c: u8) -> bool {
        c.is_ascii_alphanumeric() || c == b'_'
    }

    pub(super) fn skip_ws(b: &[u8], i: usize) -> usize {
        i + b[i.min(b.len())..]
            .iter()
            .take_while(|c| c.is_ascii_whitespace())
            .count()
    }

    pub(super) fn skip_ws_back(b: &[u8], mut i: usize) -> usize {
        while i > 0 && b[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        i
    }

    /// The identifier right before `end` (exclusive), skipping blanks and one `<…>`.
    pub(super) fn ident_before(b: &[u8], end: usize) -> (usize, usize) {
        let mut k = skip_ws_back(b, end);
        if k > 0 && b[k - 1] == b'>' {
            let mut depth = 0;
            while k > 0 {
                k -= 1;
                depth += match b[k] {
                    b'>' => 1,
                    b'<' => -1,
                    _ => 0,
                };
                if depth == 0 {
                    break;
                }
            }
            k = skip_ws_back(b, k);
        }
        let start = k - b[..k].iter().rev().take_while(|c| is_ident(**c)).count();
        (start, k)
    }

    /// Whole-word occurrences of `word`.
    pub(super) fn words<'a>(text: &'a str, word: &'a str) -> impl Iterator<Item = usize> + 'a {
        let b = text.as_bytes();
        text.match_indices(word).map(|(i, _)| i).filter(move |&i| {
            (i == 0 || !is_ident(b[i - 1])) && b.get(i + word.len()).is_none_or(|c| !is_ident(*c))
        })
    }

    /// Splits on commas outside brackets and literals.
    pub(super) fn split_top(s: &str) -> Vec<&str> {
        let b = s.as_bytes();
        let (mut parts, mut depth, mut start, mut i) = (Vec::new(), 0i32, 0, 0);
        while i < b.len() {
            if let Some(end) = literal_end(b, i) {
                i = end;
                continue;
            }
            match b[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    parts.push(s[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        parts.push(s[start..].trim());
        parts.retain(|p| !p.is_empty());
        parts
    }

    /// `name = value` (Java/Kotlin/C# attribute) or `name: value` (C# argument).
    fn named_part(part: &str) -> Option<(&str, &str)> {
        let b = part.as_bytes();
        let end = b.iter().take_while(|c| is_ident(**c)).count();
        let k = skip_ws(b, end);
        let pair = (*b.get(k)?, b.get(k + 1).copied());
        let named = matches!(pair, (b'=', next) if next != Some(b'='))
            || matches!(pair, (b':', next) if next != Some(b':'));
        (end > 0 && named).then(|| (&part[..end], part[k + 1..].trim()))
    }

    pub(super) fn named<'a>(args: &'a str, key: &str) -> Option<&'a str> {
        split_top(args)
            .into_iter()
            .find_map(|p| named_part(p).filter(|(k, _)| *k == key).map(|(_, v)| v))
    }

    pub(super) fn positional(args: &str) -> Option<&str> {
        split_top(args)
            .into_iter()
            .find(|p| named_part(p).is_none())
    }

    /// The content of a plain string literal; `None` for anything computed.
    pub(super) fn literal(s: &str) -> Option<String> {
        let s = s.trim();
        let s = s.strip_prefix('@').unwrap_or(s);
        let b = s.as_bytes();
        let plain = b.len() >= 2 && b[0] == b'"' && !s.starts_with("\"\"\"");
        (plain && literal_end(b, 0) == Some(b.len())).then(|| s[1..s.len() - 1].to_string())
    }

    /// A literal or an array (`{…}`, `[…]`, `arrayOf(…)`) of literals.
    pub(super) fn literals(s: &str) -> Option<Vec<String>> {
        let s = s.trim();
        if let Some(one) = literal(s) {
            return Some(vec![one]);
        }
        let inner = [("arrayOf(", ')'), ("{", '}'), ("[", ']')]
            .iter()
            .find_map(|(open, close)| s.strip_prefix(open)?.strip_suffix(*close))?;
        split_top(inner).into_iter().map(literal).collect()
    }
}

/// ASP.NET Core attribute-routed controllers.
mod controllers {
    use super::lex::{literal, named, positional};
    use super::*;

    const VERBS: [(&str, &str); 7] = [
        ("HttpGet", "GET"),
        ("HttpPost", "POST"),
        ("HttpPut", "PUT"),
        ("HttpDelete", "DELETE"),
        ("HttpPatch", "PATCH"),
        ("HttpHead", "HEAD"),
        ("HttpOptions", "OPTIONS"),
    ];

    pub(super) fn scan(src: &Source, out: &mut Out) {
        let mut class: Option<Decl> = None;
        for d in decls(src, false) {
            if d.class {
                let controller = d.name.ends_with("Controller")
                    || has(&d.attrs, "ApiController")
                    || has(&d.attrs, "Controller");
                class = (controller && !has(&d.attrs, "NonController")).then_some(d);
            } else if let Some(class) = &class {
                action(src, out, class, &d);
            }
        }
    }

    fn verbs(attrs: &[Attr]) -> Vec<(&Attr, &'static str)> {
        attrs
            .iter()
            .filter_map(|a| {
                VERBS
                    .iter()
                    .find(|(n, _)| *n == a.name)
                    .map(|(_, m)| (a, *m))
            })
            .collect()
    }

    fn action(src: &Source, out: &mut Out, class: &Decl, d: &Decl) {
        let verbs = verbs(&d.attrs);
        if verbs.is_empty() {
            return;
        }
        let class_routes = route_templates(src, out, &class.attrs);
        let (Some(class_routes), Some(method_routes)) =
            (class_routes, route_templates(src, out, &d.attrs))
        else {
            return;
        };
        let access = access(src, out, &class.attrs, &d.attrs);
        let first = out.0.endpoints.len();
        let tokens = [
            (
                "[controller]",
                class.name.strip_suffix("Controller").unwrap_or(&class.name),
            ),
            ("[action]", d.name.strip_suffix("Async").unwrap_or(&d.name)),
        ];
        for (attr, method) in verbs {
            let templates = match template(attr) {
                Ok(own) => own.map_or_else(|| method_routes.clone(), |t| vec![t]),
                Err(raw) => {
                    out.at(
                        src,
                        attr.at,
                        format!("Route template is not a literal: {raw}"),
                    );
                    continue;
                }
            };
            if templates.is_empty() && class_routes.is_empty() {
                let message = format!(
                    "{}.{} has no route attribute (conventional routing); endpoint skipped.",
                    class.name, d.name
                );
                out.at(src, attr.at, message);
                continue;
            }
            for full in routes(&class_routes, &templates) {
                let full = tokens
                    .iter()
                    .fold(full, |p, (t, v)| replace_token(&p, t, v));
                out.endpoint(src, attr.at, method, full, &access);
            }
        }
        out.link(
            first,
            handlers::of_method(src, &class.name, &d.name, d.open),
        );
    }

    /// Class × method templates; a method template from the root replaces the class one.
    fn routes(class: &[String], method: &[String]) -> Vec<String> {
        let or_root = |v: &[String]| {
            if v.is_empty() {
                vec![String::new()]
            } else {
                v.to_vec()
            }
        };
        let mut found = Vec::new();
        for m in or_root(method) {
            let rooted = m.strip_prefix('~').unwrap_or(&m);
            for c in or_root(class) {
                found.push(if rooted.starts_with('/') {
                    path(&[rooted])
                } else {
                    path(&[&c, &m])
                });
            }
        }
        found
    }

    fn replace_token(path: &str, token: &str, value: &str) -> String {
        match path.to_ascii_lowercase().find(token) {
            Some(p) => format!("{}{value}{}", &path[..p], &path[p + token.len()..]),
            None => path.to_string(),
        }
    }

    /// `Ok(None)` without a template; `Err(raw)` when it is not a literal.
    fn template(attr: &Attr) -> Result<Option<String>, String> {
        match named(&attr.args, "Template").or_else(|| positional(&attr.args)) {
            None => Ok(None),
            Some(raw) => literal(raw).map(Some).ok_or_else(|| raw.to_string()),
        }
    }

    fn route_templates(src: &Source, out: &mut Out, attrs: &[Attr]) -> Option<Vec<String>> {
        let mut found = Vec::new();
        for a in attrs.iter().filter(|a| a.name == "Route") {
            match template(a) {
                Ok(t) => found.extend(t),
                Err(raw) => {
                    out.at(src, a.at, format!("Route template is not a literal: {raw}"));
                    return None;
                }
            }
        }
        Some(found)
    }

    fn access(src: &Source, out: &mut Out, class: &[Attr], method: &[Attr]) -> AccessState {
        let all = || class.iter().chain(method);
        if all().any(|a| a.name == "AllowAnonymous") {
            return AccessState::Open;
        }
        let conditions = all()
            .filter(|a| a.name == "Authorize")
            .map(|a| authorize(src, out, a))
            .collect();
        combine(conditions, Condition::All).map_or(AccessState::Undeclared, AccessState::Protected)
    }

    fn authorize(src: &Source, out: &mut Out, a: &Attr) -> Condition {
        let mut parts = Vec::new();
        if let Some(raw) = named(&a.args, "Roles") {
            let roles = literal(raw).map(|r| {
                r.split(',')
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                    .map(|r| Condition::Role(r.into()))
                    .collect()
            });
            let any = roles.and_then(|r| combine(r, Condition::Any));
            parts.push(any.unwrap_or_else(|| out.unknown(src, a.at, raw)));
        }
        if let Some(raw) = named(&a.args, "Policy").or_else(|| positional(&a.args)) {
            let policy = literal(raw).map(Condition::Policy);
            parts.push(policy.unwrap_or_else(|| out.unknown(src, a.at, raw)));
        }
        combine(parts, Condition::All).unwrap_or(Condition::Authenticated)
    }
}

/// ASP.NET Core minimal APIs: `Map*` calls, route groups and their authorization.
mod minimal {
    use super::lex::{literal, skip_ws, skip_ws_back, words};
    use super::*;

    const VERBS: [(&str, &str); 5] = [
        ("MapGet", "GET"),
        ("MapPost", "POST"),
        ("MapPut", "PUT"),
        ("MapDelete", "DELETE"),
        ("MapPatch", "PATCH"),
    ];

    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub(super) enum Mod {
        Auth(Condition),
        Anonymous,
    }

    #[derive(Default)]
    struct Chain {
        prefix: Vec<String>,
        mods: Vec<Mod>,
        vars: Vec<String>,
    }

    pub(super) fn scan(
        src: &Source,
        sources: &[Source],
        cache: &mut HashMap<String, Vec<Mod>>,
        out: &mut Out,
    ) {
        for verb in VERBS {
            for p in words(&src.text, verb.0) {
                endpoint(src, sources, cache, out, p, verb);
            }
        }
    }

    fn endpoint(
        src: &Source,
        sources: &[Source],
        cache: &mut HashMap<String, Vec<Mod>>,
        out: &mut Out,
        p: usize,
        (name, method): (&str, &str),
    ) {
        let Some(close) = call_close(src, p, name) else {
            return;
        };
        let b = src.bytes();
        let open = skip_ws(b, p + name.len());
        let args = split_top(&src.text[open + 1..close]);
        let first = args.first().copied().unwrap_or("");
        let Some(pattern) = literal(first) else {
            out.at(src, p, format!("{name} pattern is not a literal: {first}"));
            return;
        };
        let chain = match group(src, out, skip_ws_back(b, p) - 1) {
            Ok(chain) => chain,
            Err(message) => return out.at(src, p, message),
        };
        let mut mods = chain.mods;
        mods.extend(trailing(src, out, close, usize::MAX));
        if let Some(m) = returning_method(src, p, &chain.vars) {
            let found = cache
                .entry(m.to_string())
                .or_insert_with(|| call_site_mods(sources, out, m));
            mods.extend(found.iter().cloned());
        }
        let mut parts: Vec<&str> = chain.prefix.iter().map(String::as_str).collect();
        parts.push(&pattern);
        let n = out.0.endpoints.len();
        out.endpoint(src, p, method, path(&parts), &access(&mods));
        if let Some(delegate) = args.get(1) {
            out.link(n, handlers::of_delegate(src, sources, delegate));
        }
    }

    fn access(mods: &[Mod]) -> AccessState {
        if mods.contains(&Mod::Anonymous) {
            return AccessState::Open;
        }
        let conditions = mods
            .iter()
            .filter_map(|m| match m {
                Mod::Auth(c) => Some(c.clone()),
                Mod::Anonymous => None,
            })
            .collect();
        combine(conditions, Condition::All).map_or(AccessState::Undeclared, AccessState::Protected)
    }

    fn modifier(src: &Source, out: &mut Out, at: usize, name: &str, args: &str) -> Vec<Mod> {
        match (name, split_top(args)) {
            ("AllowAnonymous", _) => vec![Mod::Anonymous],
            ("RequireAuthorization", parts) if parts.is_empty() => {
                vec![Mod::Auth(Condition::Authenticated)]
            }
            ("RequireAuthorization", parts) => parts
                .into_iter()
                .map(|p| {
                    let policy = literal(p).map(Condition::Policy);
                    Mod::Auth(policy.unwrap_or_else(|| out.unknown(src, at, p)))
                })
                .collect(),
            _ => vec![],
        }
    }

    /// Modifiers in up to `max` `.X(…)` calls chained after position `close`.
    fn trailing(src: &Source, out: &mut Out, close: usize, max: usize) -> Vec<Mod> {
        let b = src.bytes();
        let mut mods = Vec::new();
        let mut j = skip_ws(b, close + 1);
        for _ in 0..max {
            if b.get(j) != Some(&b'.') {
                break;
            }
            let s = skip_ws(b, j + 1);
            let e = s + b[s..].iter().take_while(|c| lex::is_ident(**c)).count();
            let open = skip_ws(b, e);
            let Some(c) = (b.get(open) == Some(&b'('))
                .then(|| src.pair(open))
                .flatten()
            else {
                break;
            };
            mods.extend(modifier(
                src,
                out,
                s,
                &src.text[s..e],
                &src.text[open + 1..c],
            ));
            j = skip_ws(b, c + 1);
        }
        mods
    }

    /// The route group of the expression ending at `end` (exclusive). Every step
    /// moves left, so it ends. `Err` for a group prefix that is not a literal.
    fn group(src: &Source, out: &mut Out, end: usize) -> Result<Chain, String> {
        let b = src.bytes();
        let j = skip_ws_back(b, end);
        if j > 0 && b[j - 1] == b')' {
            return call_group(src, out, j - 1);
        }
        let (s, e) = ident_before(b, j);
        let k = skip_ws_back(b, s);
        if s == e || k > 0 && b[k - 1] == b'.' {
            return Ok(Chain::default());
        }
        var_group(src, out, &src.text[s..e], s)
    }

    fn call_group(src: &Source, out: &mut Out, close: usize) -> Result<Chain, String> {
        let b = src.bytes();
        let Some(open) = src.pair(close) else {
            return Ok(Chain::default());
        };
        let (s, e) = ident_before(b, open);
        let k = skip_ws_back(b, s);
        let mut chain = if k > 0 && b[k - 1] == b'.' {
            group(src, out, k - 1)?
        } else {
            Chain::default()
        };
        let args = &src.text[open + 1..close];
        match &src.text[s..e] {
            "MapGroup" => {
                let prefix = split_top(args).first().and_then(|a| literal(a));
                let error = || format!("MapGroup prefix is not a literal: {}", args.trim());
                chain.prefix.push(prefix.ok_or_else(error)?);
            }
            name => chain.mods.extend(modifier(src, out, s, name, args)),
        }
        Ok(chain)
    }

    /// A variable: its last assignment before `at`, plus `var.RequireAuthorization();`
    /// statements until the next assignment. Unassigned (parameters, `app`) is the root.
    fn var_group(src: &Source, out: &mut Out, var: &str, at: usize) -> Result<Chain, String> {
        let assignments = assignments(src, var);
        let Some(&(start, semi)) = assignments.iter().rev().find(|(_, semi)| *semi < at) else {
            return Ok(Chain {
                vars: vec![var.into()],
                ..Chain::default()
            });
        };
        let mut chain = group(src, out, semi)?;
        chain.vars.push(var.into());
        let until = assignments
            .iter()
            .map(|(a, _)| *a)
            .find(|a| *a > start)
            .unwrap_or(src.text.len());
        let b = src.bytes();
        for p in words(&src.text, var).filter(|p| *p > semi && *p < until) {
            let k = skip_ws_back(b, p);
            if k == 0 || b[k - 1] != b'.' {
                chain.mods.extend(trailing(src, out, p + var.len() - 1, 1));
            }
        }
        Ok(chain)
    }

    /// `(start, end of statement)` of each `var = …;` in the file.
    fn assignments(src: &Source, var: &str) -> Vec<(usize, usize)> {
        let b = src.bytes();
        words(&src.text, var)
            .filter(|&p| {
                let eq = skip_ws(b, p + var.len());
                let prev = skip_ws_back(b, p);
                b.get(eq) == Some(&b'=')
                    && !matches!(b.get(eq + 1), Some(b'=' | b'>'))
                    && (prev == 0 || b[prev - 1] != b'.')
            })
            .map(|p| (p, statement_end(src, skip_ws(b, p + var.len()) + 1)))
            .collect()
    }

    pub(super) fn statement_end(src: &Source, mut j: usize) -> usize {
        let b = src.bytes();
        while j < b.len() {
            if let Some(end) = literal_end(b, j) {
                j = end;
                continue;
            }
            match b[j] {
                b'(' | b'[' | b'{' => j = src.pair(j).map_or(j + 1, |c| c + 1),
                b';' => return j,
                _ => j += 1,
            }
        }
        b.len()
    }

    /// The extension method around `at` (`Name(this …)`) if it returns one of `vars`.
    fn returning_method<'a>(src: &'a Source, at: usize, vars: &[String]) -> Option<&'a str> {
        let b = src.bytes();
        let paren = words(&src.text, "this")
            .filter(|p| *p < at)
            .map(|p| skip_ws_back(b, p))
            .filter(|k| *k > 0 && b[k - 1] == b'(')
            .last()?
            - 1;
        let (s, e) = ident_before(b, paren);
        let body_open = paren + src.text[paren..].find('{')?;
        let body = &src.text[body_open..src.pair(body_open).filter(|c| *c > at)?];
        let returns = words(body, "return").any(|r| {
            let value = body[r + 6..].split(';').next().unwrap_or("").trim();
            vars.iter().any(|v| v == value)
        });
        (s < e && returns).then(|| &src.text[s..e])
    }

    /// The closing parenthesis of a member call `.name(…)` whose name starts at `p`.
    fn call_close(src: &Source, p: usize, name: &str) -> Option<usize> {
        let b = src.bytes();
        let prev = skip_ws_back(b, p);
        let open = skip_ws(b, p + name.len());
        let call = prev > 0 && b[prev - 1] == b'.' && b.get(open) == Some(&b'(');
        call.then(|| src.pair(open)).flatten()
    }

    /// The modifiers chained at the call sites `.method(…)` in the project.
    fn call_site_mods(sources: &[Source], out: &mut Out, method: &str) -> Vec<Mod> {
        let mut seen = BTreeSet::new();
        let mut first = None;
        for src in sources.iter().filter(|s| s.rel.ends_with(".cs")) {
            for p in words(&src.text, method) {
                let Some(close) = call_close(src, p, method) else {
                    continue;
                };
                let mut mods = trailing(src, out, close, usize::MAX);
                mods.sort();
                first.get_or_insert((src.rel.clone(), src.line(p)));
                seen.insert(mods);
            }
        }
        if seen.len() <= 1 {
            return seen.pop_first().unwrap_or_default();
        }
        let (file, line) = first.unwrap_or_default();
        let text = format!("call sites of {method} with different authorization");
        out.diag(
            &file,
            line,
            format!("Access condition not readable: {text}"),
        );
        vec![Mod::Auth(Condition::Unknown(text))]
    }
}

/// Spring MVC controllers and Spring Security / JSR-250 annotations.
mod spring {
    use super::lex::{literal, literals, named, positional};
    use super::*;

    const VERBS: [(&str, &str); 5] = [
        ("GetMapping", "GET"),
        ("PostMapping", "POST"),
        ("PutMapping", "PUT"),
        ("DeleteMapping", "DELETE"),
        ("PatchMapping", "PATCH"),
    ];

    pub(super) fn scan(src: &Source, out: &mut Out) {
        let mut class: Option<Decl> = None;
        for d in decls(src, true) {
            if d.class {
                let controller = has(&d.attrs, "RestController") || has(&d.attrs, "Controller");
                class = controller.then_some(d);
            } else if let Some(class) = &class {
                method(src, out, class, &d);
            }
        }
    }

    /// One diagnostic per project with a central `SecurityFilterChain`.
    pub(super) fn central_rules(sources: &[Source], out: &mut Out) {
        let found = sources
            .iter()
            .filter(|s| !s.rel.ends_with(".cs"))
            .find_map(|s| s.text.find("SecurityFilterChain").map(|at| (s, at)));
        if let Some((src, at)) = found {
            out.0.diagnostics.push(UiFlowDiagnostic {
                file: src.rel.clone(),
                line: src.line(at),
                message: "Central SecurityFilterChain rules exist and are not evaluated; endpoints marked undeclared may be protected there.".into(),
                severity: DiagnosticSeverity::Note,
            });
        }
    }

    fn method(src: &Source, out: &mut Out, class: &Decl, d: &Decl) {
        let bases = match class.attrs.iter().find(|a| a.name == "RequestMapping") {
            Some(a) => match paths(src, out, a) {
                Some(p) => p,
                None => return,
            },
            None => vec![String::new()],
        };
        let first = out.0.endpoints.len();
        for a in &d.attrs {
            let Some(methods) = methods(a) else { continue };
            let Some(paths) = paths(src, out, a) else {
                continue;
            };
            let access = level(src, out, &d.attrs)
                .or_else(|| level(src, out, &class.attrs))
                .unwrap_or(AccessState::Undeclared);
            let full: Vec<String> = bases
                .iter()
                .flat_map(|b| paths.iter().map(move |p| path(&[b, p])))
                .collect();
            for (p, m) in full
                .iter()
                .flat_map(|p| methods.iter().map(move |m| (p, m)))
            {
                out.endpoint(src, a.at, m, p.clone(), &access);
            }
        }
        out.link(
            first,
            handlers::of_method(src, &class.name, &d.name, d.open),
        );
    }

    /// HTTP methods of a mapping annotation; `None` for other annotations.
    fn methods(a: &Attr) -> Option<Vec<String>> {
        if let Some((_, m)) = VERBS.iter().find(|(n, _)| *n == a.name) {
            return Some(vec![m.to_string()]);
        }
        if a.name != "RequestMapping" {
            return None;
        }
        let found: Vec<String> = named(&a.args, "method")
            .unwrap_or("")
            .split("RequestMethod.")
            .skip(1)
            .map(|r| r.chars().take_while(char::is_ascii_alphabetic).collect())
            .collect();
        Some(if found.is_empty() {
            vec![ANY_METHOD.into()]
        } else {
            found
        })
    }

    fn paths(src: &Source, out: &mut Out, a: &Attr) -> Option<Vec<String>> {
        let raw = named(&a.args, "value")
            .or_else(|| named(&a.args, "path"))
            .or_else(|| positional(&a.args));
        let Some(raw) = raw else {
            return Some(vec![String::new()]);
        };
        match literals(raw) {
            Some(p) if p.is_empty() => Some(vec![String::new()]),
            Some(p) => Some(p),
            None => {
                out.at(src, a.at, format!("Mapping path is not a literal: {raw}"));
                None
            }
        }
    }

    fn role(name: &str) -> Condition {
        Condition::Role(name.strip_prefix("ROLE_").unwrap_or(name).into())
    }

    /// Access declared by one level's annotations; `None` when there is none.
    fn level(src: &Source, out: &mut Out, attrs: &[Attr]) -> Option<AccessState> {
        let items: Vec<Option<Condition>> =
            attrs.iter().filter_map(|a| item(src, out, a)).collect();
        if items.is_empty() {
            return None;
        }
        let open = items.contains(&None);
        Some(
            match combine(items.into_iter().flatten().collect(), Condition::All) {
                Some(c) => AccessState::Protected(c),
                None if open => AccessState::Open,
                None => AccessState::Undeclared,
            },
        )
    }

    /// One security annotation: `Some(None)` declares open access.
    fn item(src: &Source, out: &mut Out, a: &Attr) -> Option<Option<Condition>> {
        let raw = named(&a.args, "value")
            .or_else(|| positional(&a.args))
            .unwrap_or("");
        match a.name.as_str() {
            "PreAuthorize" => Some(pre_authorize(src, out, a, raw)),
            "Secured" | "RolesAllowed" => {
                let roles = literals(raw)
                    .and_then(|r| combine(r.iter().map(|x| role(x)).collect(), Condition::Any));
                Some(Some(roles.unwrap_or_else(|| out.unknown(src, a.at, raw))))
            }
            "PermitAll" => Some(None),
            "DenyAll" => Some(Some(Condition::Never)),
            _ => None,
        }
    }

    /// `None` for `permitAll()`.
    fn pre_authorize(src: &Source, out: &mut Out, a: &Attr, raw: &str) -> Option<Condition> {
        let Some(expr) = literal(raw) else {
            return Some(out.unknown(src, a.at, raw));
        };
        if matches!(expr.trim(), "permitAll" | "permitAll()") {
            return None;
        }
        Some(spel(&expr).unwrap_or_else(|| out.unknown(src, a.at, &expr)))
    }

    #[derive(Clone, PartialEq)]
    enum Tok {
        Id(String),
        Str(String),
        Open,
        Close,
        Comma,
        Not,
        And,
        Or,
    }

    /// The supported subset of Spring Security SpEL; `None` for anything else.
    fn spel(expr: &str) -> Option<Condition> {
        let mut parser = Spel {
            toks: tokens(expr)?,
            at: 0,
        };
        let condition = parser.or()?;
        (parser.at == parser.toks.len()).then_some(condition)
    }

    fn tokens(expr: &str) -> Option<Vec<Tok>> {
        let b = expr.as_bytes();
        let (mut toks, mut i) = (Vec::new(), 0);
        while i < b.len() {
            let (tok, len) = match b[i] {
                c if c.is_ascii_whitespace() => (None, 1),
                q @ (b'\'' | b'"') => {
                    let end = i + 1 + b[i + 1..].iter().position(|x| *x == q)?;
                    (Some(Tok::Str(expr[i + 1..end].into())), end + 1 - i)
                }
                c if lex::is_ident(c) => {
                    let len = b[i..].iter().take_while(|x| lex::is_ident(**x)).count();
                    (Some(word(&expr[i..i + len])), len)
                }
                _ => symbol(&b[i..]).map(|(t, len)| (Some(t), len))?,
            };
            toks.extend(tok);
            i += len;
        }
        Some(toks)
    }

    fn symbol(b: &[u8]) -> Option<(Tok, usize)> {
        Some(match b {
            [b'(', ..] => (Tok::Open, 1),
            [b')', ..] => (Tok::Close, 1),
            [b',', ..] => (Tok::Comma, 1),
            [b'!', ..] => (Tok::Not, 1),
            [b'&', b'&', ..] => (Tok::And, 2),
            [b'|', b'|', ..] => (Tok::Or, 2),
            _ => return None,
        })
    }

    fn word(word: &str) -> Tok {
        match word.to_ascii_lowercase().as_str() {
            "and" => Tok::And,
            "or" => Tok::Or,
            "not" => Tok::Not,
            _ => Tok::Id(word.into()),
        }
    }

    struct Spel {
        toks: Vec<Tok>,
        at: usize,
    }
    impl Spel {
        fn eat(&mut self, tok: &Tok) -> bool {
            let hit = self.toks.get(self.at) == Some(tok);
            self.at += usize::from(hit);
            hit
        }
        fn or(&mut self) -> Option<Condition> {
            let mut items = vec![self.and()?];
            while self.eat(&Tok::Or) {
                items.push(self.and()?);
            }
            combine(items, Condition::Any)
        }
        fn and(&mut self) -> Option<Condition> {
            let mut items = vec![self.unary()?];
            while self.eat(&Tok::And) {
                items.push(self.unary()?);
            }
            combine(items, Condition::All)
        }
        fn unary(&mut self) -> Option<Condition> {
            if self.eat(&Tok::Not) {
                return Some(Condition::Not(Box::new(self.unary()?)));
            }
            if self.eat(&Tok::Open) {
                let inner = self.or()?;
                return self.eat(&Tok::Close).then_some(inner);
            }
            self.call()
        }
        fn call(&mut self) -> Option<Condition> {
            let Some(Tok::Id(name)) = self.toks.get(self.at).cloned() else {
                return None;
            };
            self.at += 1;
            let mut args = if self.eat(&Tok::Open) {
                self.args()?
            } else {
                vec![]
            };
            match (name.as_str(), args.len()) {
                ("isAuthenticated" | "isFullyAuthenticated", 0) => Some(Condition::Authenticated),
                ("hasRole" | "hasAuthority", 1) => args.pop(),
                ("hasAnyRole" | "hasAnyAuthority", 1..) => combine(args, Condition::Any),
                _ => None,
            }
        }
        /// Quoted arguments up to the closing parenthesis, as roles.
        fn args(&mut self) -> Option<Vec<Condition>> {
            let mut args = Vec::new();
            if self.eat(&Tok::Close) {
                return Some(args);
            }
            loop {
                let Some(Tok::Str(s)) = self.toks.get(self.at) else {
                    return None;
                };
                args.push(role(s));
                self.at += 1;
                if self.eat(&Tok::Close) {
                    return Some(args);
                }
                if !self.eat(&Tok::Comma) {
                    return None;
                }
            }
        }
    }
}

/// The handler of an endpoint and the requests it sends (ADR 0022, decision 4).
///
/// In the handler body (method or lambda, parameters included):
/// - `x.Send(arg)`, `x.SendAsync(arg)`, `x.send(arg)`, `x.sendAndWait(arg)`
///   where the receiver name contains `mediator`, `sender`, `gateway` or `bus`
///   (MediatR, `services.Mediator`, Axon `commandGateway`). `arg` is
///   `new T(…)`, or a variable assigned `new T(…)` or declared with type `T`
///   (local, parameter or field). A generic `new W<A, B>(x, …)` also yields the
///   argument types that are one of `A`, `B`: the wrapped request.
/// - `x.m(…)` where `x` is declared with a type named `*UseCase`,
///   `*CommandHandler` or `*QueryHandler`: that class, called directly.
///
/// Anything else (a computed argument, a factory call, Kotlin constructor
/// calls without `new`) adds nothing; nothing is guessed.
mod handlers {
    use super::Source;
    use super::lex::{ident_before, is_ident, skip_ws, skip_ws_back, split_top, words};
    use super::minimal::statement_end;

    /// Handler method id and the types it sends.
    pub(super) type Link = (Option<String>, Vec<String>);

    const SEND: [&str; 4] = ["Send", "SendAsync", "send", "sendAndWait"];
    const RECEIVERS: [&str; 4] = ["mediator", "sender", "gateway", "bus"];
    const USE_CASES: [&str; 3] = ["UseCase", "CommandHandler", "QueryHandler"];
    const NOT_TYPES: [&str; 12] = [
        "return", "new", "await", "in", "out", "ref", "is", "as", "case", "else", "throw", "yield",
    ];
    /// Bound for `a = b; b = new T()` hops.
    const MAX_DEPTH: usize = 3;

    /// The method of `class` whose parameter list opens at `open`.
    pub(super) fn of_method(src: &Source, class: &str, name: &str, open: usize) -> Link {
        let sends = method_span(src, open).map_or_else(Vec::new, |span| sends(src, span));
        (Some(format!("{class}.{name}")), sends)
    }

    /// A minimal-API delegate argument: a method group (searched in `src`, then
    /// the project) or a lambda.
    pub(super) fn of_delegate(src: &Source, sources: &[Source], arg: &str) -> Link {
        let at = arg.as_ptr() as usize - src.text.as_ptr() as usize;
        if arg.contains("=>") {
            return (None, sends(src, (at, at + arg.len())));
        }
        let name = arg.rsplit('.').next().unwrap_or(arg).trim();
        if name.is_empty() || !name.bytes().all(is_ident) {
            return (None, vec![]);
        }
        let found = std::iter::once(src)
            .chain(sources)
            .find_map(|s| definition(s, name).map(|open| (s, open)));
        match found {
            Some((s, open)) => {
                let class = enclosing_class(s, open).unwrap_or_default();
                of_method(s, &class, name, open)
            }
            None => (None, vec![]),
        }
    }

    /// From the parameter list at `open` to the end of the body; none without one.
    fn method_span(src: &Source, open: usize) -> Option<(usize, usize)> {
        let b = src.bytes();
        let mut j = src.pair(open)? + 1;
        while j < b.len() {
            match b[j] {
                b'=' if b.get(j + 1) == Some(&b'>') => return Some((open, statement_end(src, j))),
                b'{' => return Some((open, src.pair(j)?)),
                b';' | b'}' => return None,
                _ => j += 1,
            }
        }
        None
    }

    /// The parameter list of a method named `name` declared in `src`.
    fn definition(src: &Source, name: &str) -> Option<usize> {
        let b = src.bytes();
        words(&src.text, name).find_map(|p| {
            let prev = skip_ws_back(b, p);
            let typed =
                prev > 0 && (is_ident(b[prev - 1]) || matches!(b[prev - 1], b'>' | b']' | b'?'));
            let open = skip_ws(b, p + name.len());
            (typed && b.get(open) == Some(&b'(') && method_span(src, open).is_some())
                .then_some(open)
        })
    }

    /// The innermost class, record or struct whose body contains `at`.
    fn enclosing_class(src: &Source, at: usize) -> Option<String> {
        let b = src.bytes();
        let mut found = None;
        for kw in ["class", "record", "struct"] {
            for p in words(&src.text, kw).filter(|p| *p < at) {
                let s = skip_ws(b, p + kw.len());
                let e = s + b[s..].iter().take_while(|c| is_ident(**c)).count();
                let Some(open) = src.text[e..at].find('{').map(|o| o + e) else {
                    continue;
                };
                if src.text[e..open].contains(';') || src.pair(open).is_none_or(|c| c < at) {
                    continue;
                }
                if found.as_ref().is_none_or(|(o, _)| *o < open) {
                    found = Some((open, src.text[s..e].to_string()));
                }
            }
        }
        found.map(|(_, name)| name)
    }

    /// [`sends`] for other extractors (message handlers, ADR 0022): `text` is
    /// a C#, Java or Kotlin file, `span` a byte range of it.
    pub(crate) fn sends_in(text: &str, span: (usize, usize)) -> Vec<String> {
        sends(&Source::new(String::new(), text), span)
    }

    /// Sent request types and directly called use cases in `span`, sorted.
    fn sends(src: &Source, span: (usize, usize)) -> Vec<String> {
        let b = src.bytes();
        let mut found = Vec::new();
        for dot in (span.0..span.1).filter(|&i| b[i] == b'.') {
            let (s, e) = ident_before(b, dot);
            let m = skip_ws(b, dot + 1);
            let m_end = m + b[m..].iter().take_while(|c| is_ident(**c)).count();
            let open = skip_ws(b, m_end);
            let close = (b.get(open) == Some(&b'('))
                .then(|| src.pair(open))
                .flatten();
            let (Some(close), true) = (close, s < e) else {
                continue;
            };
            let receiver = &src.text[s..e];
            let lower = receiver.to_ascii_lowercase();
            if SEND.contains(&&src.text[m..m_end]) && RECEIVERS.iter().any(|r| lower.contains(r)) {
                if let Some(arg) = arguments(&src.text[open + 1..close]).first() {
                    found.extend(resolve(src, span, arg, 0));
                }
            } else if let Some(t) = declared(src, span, receiver).filter(|t| use_case(t)) {
                found.push(t);
            }
        }
        found.sort();
        found.dedup();
        found
    }

    /// `split_top` that keeps `W<A, B>` in one argument.
    fn arguments(s: &str) -> Vec<&str> {
        let angles = |p: &str| {
            let b = p.as_bytes();
            let opened = b.iter().filter(|c| **c == b'<').count();
            let closed = (b.iter().enumerate())
                .filter(|(i, c)| **c == b'>' && (*i == 0 || b[i - 1] != b'='))
                .count();
            opened > closed
        };
        let mut out: Vec<&str> = Vec::new();
        for part in split_top(s) {
            match out.last_mut() {
                Some(last) if angles(last) => {
                    let start = last.as_ptr() as usize - s.as_ptr() as usize;
                    let end = part.as_ptr() as usize - s.as_ptr() as usize + part.len();
                    *last = &s[start..end];
                }
                _ => out.push(part),
            }
        }
        out
    }

    /// `IdentifiedCommand` of `Ns.IdentifiedCommand<A, B>`.
    pub(crate) fn simple_type(t: &str) -> &str {
        let outer = t.split('<').next().unwrap_or(t).trim();
        outer.rsplit('.').next().unwrap_or(outer)
    }

    fn use_case(type_text: &str) -> bool {
        USE_CASES
            .iter()
            .any(|x| simple_type(type_text).ends_with(x))
    }

    /// Types the expression `expr` (a slice of `src.text`) stands for.
    fn resolve(src: &Source, span: (usize, usize), expr: &str, depth: usize) -> Vec<String> {
        let expr = expr.trim();
        if depth > MAX_DEPTH {
            return vec![];
        }
        if let Some(rest) = expr.strip_prefix("new ") {
            return constructed(src, span, rest.trim_start(), depth);
        }
        if !expr.is_empty() && expr.bytes().all(is_ident) {
            return variable(src, span, expr, depth);
        }
        vec![]
    }

    /// `T(…)` or `W<A, B>(x, …)` after `new`: `T`, or `W<A, B>` and the
    /// arguments whose type is one of `A`, `B`.
    fn constructed(src: &Source, span: (usize, usize), text: &str, depth: usize) -> Vec<String> {
        let end = text.find(['(', '{']).unwrap_or(text.len());
        let ty = text[..end].trim();
        if ty.is_empty() {
            return vec![];
        }
        let mut found = vec![ty.to_string()];
        let generics: Vec<&str> = ty
            .split_once('<')
            .map(|(_, g)| g.trim_end_matches('>'))
            .map_or_else(Vec::new, |g| g.split(',').map(simple_type).collect());
        let at = text.as_ptr() as usize - src.text.as_ptr() as usize + end;
        if let (false, Some(close)) = (generics.is_empty(), src.pair(at)) {
            for arg in arguments(&src.text[at + 1..close]) {
                let inner = resolve(src, span, arg, depth + 1);
                found.extend(
                    inner
                        .into_iter()
                        .filter(|t| generics.contains(&simple_type(t))),
                );
            }
        }
        found
    }

    /// The variable's initializer if it is `new …`, else its declared type: first
    /// in `span`, then anywhere in the file (fields).
    fn variable(src: &Source, span: (usize, usize), var: &str, depth: usize) -> Vec<String> {
        for scope in [&src.text[span.0..span.1], &src.text[..]] {
            let base = scope.as_ptr() as usize - src.text.as_ptr() as usize;
            for p in words(scope, var).map(|p| p + base) {
                if let Some(found) = declaration(src, span, p, var, depth) {
                    return found;
                }
            }
        }
        vec![]
    }

    /// `T var`, `T var = init` or `var var = init` at `p`.
    fn declaration(
        src: &Source,
        span: (usize, usize),
        p: usize,
        var: &str,
        depth: usize,
    ) -> Option<Vec<String>> {
        let b = src.bytes();
        let ty = type_before(src, p)?;
        let eq = skip_ws(b, p + var.len());
        if b.get(eq) == Some(&b'=') && !matches!(b.get(eq + 1), Some(b'=' | b'>')) {
            let init = &src.text[eq + 1..statement_end(src, eq + 1)];
            let found = resolve(src, span, init, depth + 1);
            if !found.is_empty() {
                return Some(found);
            }
        }
        (!matches!(ty, "var" | "val")).then(|| vec![ty.to_string()])
    }

    /// The declared type right before the variable at `p`, if one is written.
    fn type_before(src: &Source, p: usize) -> Option<&str> {
        let b = src.bytes();
        let end = skip_ws_back(b, p);
        let (s, e) = ident_before(b, end);
        let before = skip_ws_back(b, s);
        let member = before > 0 && b[before - 1] == b'.';
        let ty = &src.text[s..end];
        (s < e && !member && !NOT_TYPES.contains(&&src.text[s..e])).then_some(ty)
    }

    /// The type text of the variable `var` in `span` or the file.
    fn declared(src: &Source, span: (usize, usize), var: &str) -> Option<String> {
        for scope in [&src.text[span.0..span.1], &src.text[..]] {
            let base = scope.as_ptr() as usize - src.text.as_ptr() as usize;
            let found = words(scope, var)
                .map(|p| p + base)
                .find_map(|p| type_before(src, p).filter(|t| !matches!(*t, "var" | "val")));
            if let Some(t) = found {
                return Some(t.to_string());
            }
        }
        None
    }

    #[cfg(test)]
    mod tests {
        use super::super::extract;
        use archguard_core::{EndpointAccess, UiFlow};

        fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
            let dir = tempfile::tempdir().unwrap();
            for (path, text) in files {
                let path = dir.path().join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            dir
        }

        fn endpoint<'a>(flow: &'a UiFlow, method: &str, path: &str) -> &'a EndpointAccess {
            (flow.endpoints.iter())
                .find(|e| e.method == method && e.path == path)
                .unwrap_or_else(|| panic!("{method} {path} in {:#?}", flow.endpoints))
        }

        fn link(flow: &UiFlow, method: &str, path: &str) -> (Option<String>, Vec<String>) {
            let e = endpoint(flow, method, path);
            (e.handler.clone(), e.sends.clone())
        }

        fn s(items: &[&str]) -> Vec<String> {
            items.iter().map(|i| i.to_string()).collect()
        }

        const ORDERS_API: &str = r#"
    public static class OrdersApi
    {
        public static RouteGroupBuilder MapOrdersApiV1(this IEndpointRouteBuilder app)
        {
            var api = app.MapGroup("api/orders");
            api.MapPut("/cancel", CancelOrderAsync);
            api.MapPost("/draft", CreateOrderDraftAsync);
            api.MapPost("/", OrdersApi.CreateOrderAsync);
            api.MapPost("/quick", async (QuickOrderCommand cmd, IMediator mediator) => await mediator.Send(cmd));
            api.MapPost("/built", BuiltAsync);
            api.MapPost("/mail", MailAsync);
            api.MapPut("/ship", ShipAsync);
            api.MapGet("/elsewhere", Handlers.Elsewhere);
            return api;
        }

        public static async Task<Results<Ok, BadRequest<string>>> CancelOrderAsync(
            [FromHeader(Name = "x-requestid")] Guid requestId,
            CancelOrderCommand command,
            [AsParameters] OrderServices services)
        {
            var requestCancelOrder = new IdentifiedCommand<CancelOrderCommand, bool>(command, requestId);
            var commandResult = await services.Mediator.Send(requestCancelOrder);
            return TypedResults.Ok();
        }

        public static async Task<OrderDraftDTO> CreateOrderDraftAsync(CreateOrderDraftCommand command, [AsParameters] OrderServices services)
            => await services.Mediator.Send(command);

        public static async Task<Results<Ok, BadRequest<string>>> CreateOrderAsync(
            [FromHeader(Name = "x-requestid")] Guid requestId,
            CreateOrderRequest request,
            [AsParameters] OrderServices services)
        {
            using (services.Logger.BeginScope(new List<KeyValuePair<string, object>> { new("IdentifiedCommandId", requestId) }))
            {
                var createOrderCommand = new CreateOrderCommand(request.Items, request.UserId);
                var requestCreateOrder = new IdentifiedCommand<CreateOrderCommand, bool>(createOrderCommand, requestId);
                var result = await services.Mediator.Send(requestCreateOrder);
                return TypedResults.Ok();
            }
        }

        public static async Task<Ok> BuiltAsync([AsParameters] OrderServices services)
        {
            await services.Mediator.Send(Build());
            return TypedResults.Ok();
        }

        public static Task<bool> ShipAsync(ShipOrderCommand command, Guid requestId, IMediator mediator)
            => mediator.Send(new IdentifiedCommand<ShipOrderCommand, bool>(command, requestId));

        public static async Task<Ok> MailAsync(IEmailService email)
        {
            await email.Send(new WelcomeMail());
            return TypedResults.Ok();
        }
    }
    "#;

        const HANDLERS: &str = r#"
    public static class Handlers
    {
        public static Task<int> Elsewhere(ISender sender) => sender.Send(new CountOrdersQuery());
    }
    "#;

        #[test]
        fn minimal_api_method_groups_and_lambdas_send_wrapped_commands() {
            let dir = project(&[
                ("Ordering.API/Ordering.API.csproj", "<Project />"),
                ("Ordering.API/Apis/OrdersApi.cs", ORDERS_API),
                ("Ordering.API/Apis/Handlers.cs", HANDLERS),
            ]);
            let flow = extract(dir.path());
            let wrapped = |inner: &str| format!("IdentifiedCommand<{inner}, bool>");
            assert_eq!(
                link(&flow, "POST", "/api/orders"),
                (
                    Some("OrdersApi.CreateOrderAsync".into()),
                    vec!["CreateOrderCommand".into(), wrapped("CreateOrderCommand")]
                )
            );
            assert_eq!(
                link(&flow, "PUT", "/api/orders/cancel"),
                (
                    Some("OrdersApi.CancelOrderAsync".into()),
                    vec!["CancelOrderCommand".into(), wrapped("CancelOrderCommand")]
                )
            );
            assert_eq!(
                link(&flow, "POST", "/api/orders/draft"),
                (
                    Some("OrdersApi.CreateOrderDraftAsync".into()),
                    s(&["CreateOrderDraftCommand"])
                )
            );
            assert_eq!(
                link(&flow, "POST", "/api/orders/quick"),
                (None, s(&["QuickOrderCommand"]))
            );
            // A factory call and a mail service are not requests.
            assert_eq!(
                link(&flow, "POST", "/api/orders/built"),
                (Some("OrdersApi.BuiltAsync".into()), vec![])
            );
            assert_eq!(
                link(&flow, "POST", "/api/orders/mail").1,
                Vec::<String>::new()
            );
            assert_eq!(
                link(&flow, "PUT", "/api/orders/ship").1,
                vec![
                    "IdentifiedCommand<ShipOrderCommand, bool>".to_string(),
                    "ShipOrderCommand".into()
                ]
            );
            assert_eq!(
                link(&flow, "GET", "/api/orders/elsewhere"),
                (Some("Handlers.Elsewhere".into()), s(&["CountOrdersQuery"]))
            );
        }

        const CONTROLLER: &str = r#"
    [ApiController]
    [Route("api/[controller]")]
    public class PaymentsController : ControllerBase
    {
        private readonly IMediator _mediator;
        private readonly ShipOrderUseCase _ship;

        [HttpPost]
        public async Task<IActionResult> Pay([FromBody] PayOrderCommand command)
        {
            await _mediator.Send(command);
            return Ok();
        }

        [HttpPost("ship")]
        public IActionResult Ship(int id)
        {
            _ship.Execute(id);
            return Ok();
        }

        [HttpGet]
        public IActionResult List() => Ok();
    }
    "#;

        #[test]
        fn controller_actions_send_parameters_and_call_use_case_fields() {
            let dir = project(&[
                ("Api/Api.csproj", "<Project />"),
                ("Api/PaymentsController.cs", CONTROLLER),
            ]);
            let flow = extract(dir.path());
            assert_eq!(
                link(&flow, "POST", "/api/Payments"),
                (
                    Some("PaymentsController.Pay".into()),
                    s(&["PayOrderCommand"])
                )
            );
            assert_eq!(
                link(&flow, "POST", "/api/Payments/ship"),
                (
                    Some("PaymentsController.Ship".into()),
                    s(&["ShipOrderUseCase"])
                )
            );
            assert_eq!(
                link(&flow, "GET", "/api/Payments"),
                (Some("PaymentsController.List".into()), vec![])
            );
        }

        const SPRING: &str = r#"
    @RestController
    @RequestMapping("/orders")
    public class OrderController {
        private final CommandGateway commandGateway;
        private final PayOrderUseCase payOrderUseCase;

        @PostMapping
        public CompletableFuture<String> create(@RequestBody CreateOrder body) {
            return commandGateway.send(new PlaceOrderCommand(body.id(), body.items()));
        }

        @PutMapping("/{id}/pay")
        public void pay(@PathVariable String id) throws IOException {
            payOrderUseCase.execute(id);
        }
    }
    "#;

        #[test]
        fn spring_methods_send_axon_commands_and_call_use_cases() {
            let dir = project(&[
                ("orders/pom.xml", "<project/>"),
                ("orders/src/main/java/shop/OrderController.java", SPRING),
            ]);
            let flow = extract(dir.path());
            assert_eq!(
                link(&flow, "POST", "/orders"),
                (
                    Some("OrderController.create".into()),
                    s(&["PlaceOrderCommand"])
                )
            );
            assert_eq!(
                link(&flow, "PUT", "/orders/{id}/pay"),
                (Some("OrderController.pay".into()), s(&["PayOrderUseCase"]))
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }
    fn access<'a>(flow: &'a UiFlow, method: &str, path: &str) -> &'a AccessState {
        let hits: Vec<_> = flow
            .endpoints
            .iter()
            .filter(|e| e.method == method && e.path == path)
            .collect();
        assert_eq!(hits.len(), 1, "{method} {path} in {:#?}", flow.endpoints);
        &hits[0].access
    }
    fn role(name: &str) -> Condition {
        Condition::Role(name.into())
    }
    fn policy(name: &str) -> Condition {
        Condition::Policy(name.into())
    }
    fn protected(condition: Condition) -> AccessState {
        AccessState::Protected(condition)
    }

    const CONTROLLERS: &str = r#"
using Microsoft.AspNetCore.Mvc;

namespace Shop;

[ApiController]
[Route("api/[controller]")]
[Authorize]
public class OrdersController : ControllerBase
{
    [HttpGet]
    public IActionResult List() => Ok("[HttpGet(\"fake\")]");

    [HttpGet("{id:int}")]
    [AllowAnonymous]
    public IActionResult Get(int id) => Ok();

    [HttpPost("/admin/orders")]
    [Authorize(Roles = "Admin, Clerk")]
    public IActionResult Create([FromBody] Order order)
    {
        return Ok(new { id = 1 });
    }

    [HttpDelete("[action]/{id}"), Authorize(Policy = "CanDelete")]
    public async Task<IActionResult> RemoveAsync(int id) => Ok();

    [HttpGet(Routes.Detail)]
    public IActionResult Detail() => Ok();
}

public class HealthController : Controller
{
    // [HttpGet("commented")]
    /* [HttpGet("block")] */
    [HttpGet("~/health")]
    public string Ping() => "ok";

    [Route("only-route")]
    public string NoVerb() => "x";

    [HttpGet]
    public string Conventional() => "";
}

[Route("not-a-controller")]
public class Helper
{
    [HttpGet("x")]
    public string X() => "";
}
"#;

    #[test]
    fn controllers_combine_routes_and_attributes() {
        let dir = project(&[
            ("Shop/Shop.csproj", "<Project />"),
            ("Shop/Controllers/OrdersController.cs", CONTROLLERS),
            (
                "Shop.Tests/OrdersTests.cs",
                "public class XController { [HttpGet(\"t\")] public void T() {} }",
            ),
            (
                "Shop/bin/Gen.cs",
                "public class GenController { [HttpGet(\"g\")] public void G() {} }",
            ),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            access(&flow, "GET", "/api/Orders"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(
            access(&flow, "GET", "/api/Orders/{id:int}"),
            &AccessState::Open
        );
        assert_eq!(
            access(&flow, "POST", "/admin/orders"),
            &protected(Condition::All(vec![
                Condition::Any(vec![role("Admin"), role("Clerk")]),
                Condition::Authenticated,
            ]))
        );
        assert_eq!(
            access(&flow, "DELETE", "/api/Orders/Remove/{id}"),
            &protected(Condition::All(vec![
                Condition::Authenticated,
                policy("CanDelete")
            ]))
        );
        assert_eq!(access(&flow, "GET", "/health"), &AccessState::Undeclared);
        assert_eq!(flow.endpoints.len(), 5, "{:#?}", flow.endpoints);
        let e = &flow.endpoints.iter().find(|e| e.path == "/health").unwrap();
        assert_eq!(
            (e.file.as_str(), e.line, e.source.as_str()),
            ("Shop/Controllers/OrdersController.cs", 36, "aspnet")
        );
        assert!(!flow.complete);
        assert_eq!(flow.diagnostics.len(), 2, "{:#?}", flow.diagnostics);
        assert_eq!(flow.diagnostics[0].line, 28);
        assert!(flow.diagnostics[0].message.contains("Routes.Detail"));
        assert_eq!(flow.diagnostics[1].line, 42);
        assert!(
            flow.diagnostics[1]
                .message
                .contains("HealthController.Conventional")
        );
    }

    const PROGRAM: &str = r#"
var app = builder.Build();
app.MapGet("/health", () => "ok").AllowAnonymous();
var api = app.MapGroup("/api");
var admin = api.MapGroup("admin")
    .RequireAuthorization("AdminPolicy");
admin.MapGet("/users", () => 1);
api.MapPost("items", () => 1).WithName("x").RequireAuthorization();
api.MapGet("items", () => 1);
app.MapGet(PathConstants.X, () => 1);
var open = app.MapGroup("open");
open.AllowAnonymous();
open.MapGet("ping", () => 1).RequireAuthorization();
var orders = app.NewVersionedApi("Orders");
orders.MapOrdersApiV1()
      .RequireAuthorization();
app.Run();
"#;
    const ORDERS_API: &str = r#"
public static class OrdersApi
{
    public static RouteGroupBuilder MapOrdersApiV1(this IEndpointRouteBuilder app)
    {
        var api = app.MapGroup("api/orders").HasApiVersion(1.0);
        api.MapPost("/", CreateOrderAsync);
        api.MapGet("{orderId:int}", GetOrderAsync);
        return api;
    }

    public static IEndpointRouteBuilder MapPublic(this IEndpointRouteBuilder app)
    {
        var api = app.MapGroup("public");
        api.MapGet("/info", () => 1);
        return app;
    }
}
"#;

    #[test]
    fn minimal_apis_follow_groups_and_call_sites() {
        let dir = project(&[
            ("Ordering.API/Ordering.API.csproj", "<Project />"),
            ("Ordering.API/Program.cs", PROGRAM),
            ("Ordering.API/Apis/OrdersApi.cs", ORDERS_API),
        ]);
        let flow = extract(dir.path());
        let authenticated = protected(Condition::Authenticated);
        assert_eq!(access(&flow, "GET", "/health"), &AccessState::Open);
        assert_eq!(
            access(&flow, "GET", "/api/admin/users"),
            &protected(policy("AdminPolicy"))
        );
        assert_eq!(access(&flow, "POST", "/api/items"), &authenticated);
        assert_eq!(access(&flow, "GET", "/api/items"), &AccessState::Undeclared);
        assert_eq!(access(&flow, "GET", "/open/ping"), &AccessState::Open);
        assert_eq!(access(&flow, "POST", "/api/orders"), &authenticated);
        assert_eq!(
            access(&flow, "GET", "/api/orders/{orderId:int}"),
            &authenticated
        );
        assert_eq!(
            access(&flow, "GET", "/public/info"),
            &AccessState::Undeclared
        );
        assert_eq!(flow.endpoints.len(), 8, "{:#?}", flow.endpoints);
        assert!(flow.endpoints.iter().all(|e| e.source == "aspnet"));
        assert!(!flow.complete);
        assert_eq!(flow.diagnostics.len(), 1, "{:#?}", flow.diagnostics);
        assert_eq!(
            (flow.diagnostics[0].file.as_str(), flow.diagnostics[0].line),
            ("Ordering.API/Program.cs", 10)
        );
    }

    const SPRING: &str = r#"
package shop;

@RestController
@RequestMapping("/api/owners")
@PreAuthorize("isAuthenticated()")
public class OwnerController {
    @Autowired
    private OwnerRepository repo;

    @GetMapping
    public List<Owner> list() { return repo.findAll(); }

    @GetMapping({"/{id}", "/by-id/{id}"})
    @PreAuthorize("hasRole('ROLE_ADMIN') and hasAnyAuthority('owner:read', 'SCOPE_x')")
    public Owner get(@PathVariable("id") int id) { return null; }

    @PostMapping(value = "/new")
    @PreAuthorize("hasRole('ADMIN') or !(hasAuthority('BLOCKED') && isAuthenticated())")
    public Owner create() { return null; }

    @RequestMapping(path = "/legacy", method = {RequestMethod.PUT, RequestMethod.PATCH})
    @PreAuthorize("permitAll()")
    public void legacy() {}

    @DeleteMapping(path = "/{id}")
    @PreAuthorize("@guard.isOwner(#id)")
    public void delete(@PathVariable int id) {}

    @PutMapping("/{id}/editor") @Secured({"ROLE_EDITOR", "ROLE_ADMIN"})
    public void edit() {}

    @org.springframework.web.bind.annotation.PatchMapping("/{id}/audit")
    @RolesAllowed("AUDITOR")
    public void audit() {}

    @RequestMapping("/closed")
    @DenyAll
    public void closed() {}
}

@FeignClient("owners")
interface OwnerClient {
    @GetMapping("/feign")
    Owner get();
}

@Controller
class PlainController {
    @GetMapping("/plain")
    String plain() { return "plain"; }
}
"#;
    const SECURITY: &str = r#"
@Configuration
public class SecurityConfig {
    @Bean
    SecurityFilterChain filterChain(HttpSecurity http) throws Exception {
        return http.authorizeHttpRequests(a -> a.requestMatchers("/api/**").authenticated()).build();
    }
}
"#;
    const KOTLIN: &str = r#"
@RestController
@RequestMapping(["/k"])
class KController(private val repo: Repo) {
    @GetMapping("/a")
    @PreAuthorize("hasRole('USER')")
    fun a(): String = "a"
}
"#;

    #[test]
    fn spring_mappings_and_security_annotations() {
        let dir = project(&[
            ("owners/pom.xml", "<project/>"),
            ("owners/src/main/java/shop/OwnerController.java", SPRING),
            ("owners/src/main/java/shop/SecurityConfig.java", SECURITY),
            ("owners/src/main/kotlin/shop/KController.kt", KOTLIN),
            (
                "owners/src/test/java/shop/T.java",
                "@RestController class T { @GetMapping(\"/t\") void t() {} }",
            ),
        ]);
        let flow = extract(dir.path());
        let base = "/api/owners";
        assert_eq!(
            access(&flow, "GET", base),
            &protected(Condition::Authenticated)
        );
        let admin_read = protected(Condition::All(vec![
            Condition::Any(vec![role("SCOPE_x"), role("owner:read")]),
            role("ADMIN"),
        ]));
        assert_eq!(access(&flow, "GET", "/api/owners/{id}"), &admin_read);
        assert_eq!(access(&flow, "GET", "/api/owners/by-id/{id}"), &admin_read);
        assert_eq!(
            access(&flow, "POST", "/api/owners/new"),
            &protected(Condition::Any(vec![
                Condition::Not(Box::new(Condition::All(vec![
                    Condition::Authenticated,
                    role("BLOCKED"),
                ]))),
                role("ADMIN"),
            ]))
        );
        assert_eq!(
            access(&flow, "PUT", "/api/owners/legacy"),
            &AccessState::Open
        );
        assert_eq!(
            access(&flow, "PATCH", "/api/owners/legacy"),
            &AccessState::Open
        );
        assert_eq!(
            access(&flow, "DELETE", "/api/owners/{id}"),
            &protected(Condition::Unknown("@guard.isOwner(#id)".into()))
        );
        assert_eq!(
            access(&flow, "PUT", "/api/owners/{id}/editor"),
            &protected(Condition::Any(vec![role("ADMIN"), role("EDITOR")]))
        );
        assert_eq!(
            access(&flow, "PATCH", "/api/owners/{id}/audit"),
            &protected(role("AUDITOR"))
        );
        assert_eq!(access(&flow, "GET", "/plain"), &AccessState::Undeclared);
        assert_eq!(access(&flow, "GET", "/k/a"), &protected(role("USER")));
        assert_eq!(
            access(&flow, "ANY", "/api/owners/closed"),
            &protected(Condition::Never)
        );
        assert_eq!(flow.endpoints.len(), 12, "{:#?}", flow.endpoints);
        assert!(flow.endpoints.iter().all(|e| e.source == "spring"));
        assert!(
            flow.endpoints
                .iter()
                .all(|e| e.application.as_deref() == Some("owners"))
        );
        let messages: Vec<_> = flow
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect();
        assert_eq!(messages.len(), 2, "{messages:#?}");
        assert!(messages.iter().any(|m| m.contains("@guard.isOwner")));
        let central = flow
            .diagnostics
            .iter()
            .find(|d| d.message.contains("SecurityFilterChain"))
            .unwrap();
        assert_eq!(
            (central.file.as_str(), central.line),
            ("owners/src/main/java/shop/SecurityConfig.java", 5)
        );
        assert_eq!(central.severity, DiagnosticSeverity::Note);
        assert!(!flow.complete);
    }

    #[test]
    fn unreadable_files_become_diagnostics() {
        let dir = project(&[(
            "A.java",
            "@RestController class A { @GetMapping(\"/a\") void a() {} }",
        )]);
        std::fs::write(dir.path().join("Big.java"), vec![b' '; 1024 * 1024 + 1]).unwrap();
        let flow = extract(dir.path());
        assert_eq!(access(&flow, "GET", "/a"), &AccessState::Undeclared);
        assert_eq!(flow.diagnostics.len(), 1, "{:#?}", flow.diagnostics);
        assert_eq!(flow.diagnostics[0].file, "Big.java");
        assert_eq!(flow.diagnostics[0].severity, DiagnosticSeverity::Gap);
        // Outside every project there is no application.
        assert_eq!(flow.endpoints[0].application, None);
        assert!(!flow.complete);
        assert!(extract(&dir.path().join("missing")).endpoints.is_empty());
    }
}
