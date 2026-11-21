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
//!   `@PermitAll`, `@DenyAll` (`never`). Authorities become `role` atoms too, stored as the runtime
//!   authority: `hasRole('A')` adds Spring's `ROLE_` (unless present), so
//!   `hasRole('A')`, `hasAuthority('ROLE_A')` and `@Secured("ROLE_A")` all
//!   read `role(ROLE_A)`; the convention (`role_prefix = ROLE_`) is emitted
//!   per application and shortens labels only.
//!
//! Each endpoint also gets its handler method and the request types it sends
//! (`handlers`, ADR 0022 decision 4).
//!
//! Also read: `[views.access]` of the root's configuration for attributes and
//! annotations (`index`), string constants of the scan in routes and roles,
//! attributes and `[Route]` of base controllers, conventional MVC routes
//! (`conventional`), CleanArchitecture endpoint groups (`minimal`), request
//! attributes a MediatR pipeline behaviour enforces (`index`), and Spring
//! `SecurityFilterChain` rules (`spring::chain`).
//!
//! A project is the nearest directory with a `.csproj`, `pom.xml` or
//! `build.gradle(.kts)`; its directory name is the endpoint's `application`.
//! Not recognized (left `undeclared`, or a diagnostic): `[AcceptVerbs]`,
//! `MapMethods`, `MapGroup` prefixes passed in from a call site, guards added
//! inside helper extension methods, `[Authorize<T>]`, `FallbackPolicy`, other
//! SpEL (`unknown`), non-constant routes or roles (diagnostic, the endpoint
//! is skipped or the condition is `unknown`), actions inherited from a base
//! controller under conventional routing, `MapIdentityApi`.
use crate::extract_ctx::{ExtractContext, ancestors, dir_name, read_text, walk, with_root_config};
use crate::services::support::test_path;
use archguard_core::{
    ANY_METHOD, AccessMeaning, AccessState, Condition, ConventionOrigin, DiagnosticSeverity,
    EndpointAccess, HandlerSite, RoleConvention, UiFlow, UiFlowDiagnostic,
};
/// Spring `GrantedAuthorityDefaults` default role prefix (framework default).
const ROLE_PREFIX: &str = "ROLE_";
use lex::{ident_before, literal_end, split_top};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

pub(crate) use handlers::{sends_in, simple_type};

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_FILES: usize = 20_000;
const BUILD_FILES: [&str; 3] = ["pom.xml", "build.gradle", "build.gradle.kts"];
const SOURCES: [&str; 3] = [".cs", ".java", ".kt"];

/// Endpoints with their declared access, plus diagnostics. `[views.access]`
/// comes from the root's configuration; an unreadable one is a note.
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] with the run's `[views.access]`.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut out = Out(UiFlow::default());
    if let Some(e) = &ctx.config_error {
        let message = format!("Configuration not read; [views.access] not applied: {e}");
        out.note(".", 0, message);
    }
    let projects = read_projects(ctx.root, &mut out);
    let access = ctx.config.views.access.clone();
    let index = Index::new(projects.values().flat_map(|(_, s)| s), access);
    for (application, sources) in projects.values() {
        let first = out.0.endpoints.len();
        let mut call_sites = HashMap::new();
        let routes = conventional::routes(sources, &index, &mut out);
        for src in sources {
            if src.rel.ends_with(".cs") {
                controllers::scan(src, &index, &routes, &mut out);
                minimal::scan(src, sources, &index, &mut call_sites, &mut out);
            } else {
                spring::scan(src, &index, &mut out);
            }
        }
        spring::central_rules(sources, &index, &mut out, first);
        for e in &mut out.0.endpoints[first..] {
            e.application = application.clone();
        }
        if out.0.endpoints.len() > first
            && sources
                .iter()
                .any(|s| s.text.contains("org.springframework"))
        {
            out.0.role_conventions.push(RoleConvention {
                application: application.clone(),
                role_prefix: Some(ROLE_PREFIX.into()),
                origins: vec![ConventionOrigin {
                    field: "role_prefix".into(),
                    file: String::new(),
                    line: 0,
                    source: "framework default spring GrantedAuthorityDefaults".into(),
                }],
                ..RoleConvention::default()
            });
        }
    }
    index.apply_request_access(&mut out);
    let flow = &mut out.0;
    flow.endpoints.sort();
    flow.endpoints.dedup();
    flow.diagnostics.sort();
    flow.diagnostics.dedup();
    flow.handler_sites.sort();
    flow.handler_sites.dedup();
    out.0
}

/// What one run reads: the projects with their sources.
mod projects {
    use super::*;

    /// Application name and source files by project directory; unreadable files
    /// become diagnostics.
    pub(super) fn read_projects(root: &Path, out: &mut Out) -> BTreeMap<String, Project> {
        let (projects, files) = discover(root, out);
        let mut by_project: BTreeMap<String, Project> = BTreeMap::new();
        for rel in files {
            match read_text(root, &rel, MAX_BYTES) {
                Ok(text) => by_project
                    .entry(project_of(&rel, &projects))
                    .or_insert_with_key(|dir| (application(root, dir, &projects), vec![]))
                    .1
                    .push(Source::new(rel, &text)),
                Err(e) => out.diag(&rel, 0, format!("Source file not read (limit 1 MiB): {e}")),
            }
        }
        by_project
    }

    /// The owning application of a project directory, and its sources.
    pub(super) type Project = (Option<String>, Vec<Source>);

    /// The project directory's name (the root's for a build file at the root);
    /// none for files outside every project.
    fn application(root: &Path, dir: &str, projects: &BTreeSet<String>) -> Option<String> {
        projects.contains(dir).then(|| dir_name(root, dir))
    }

    fn project_of(rel: &str, projects: &BTreeSet<String>) -> String {
        let dir = ancestors(rel).find(|d| d.is_empty() || projects.contains(*d));
        dir.unwrap_or_default().to_string()
    }

    /// Directories holding a build file, and the source files to scan.
    fn discover(root: &Path, out: &mut Out) -> (BTreeSet<String>, Vec<String>) {
        let (mut projects, mut files) = (BTreeSet::new(), Vec::new());
        let walk = match walk(root) {
            Ok(walk) => walk,
            Err(e) => {
                out.diag(".", 0, format!("Discovery settings not read: {e}"));
                return (projects, files);
            }
        };
        for e in walk.errors {
            out.diag(".", 0, format!("Directory entry not read: {e}"));
        }
        for rel in walk.files {
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
}
use projects::read_projects;

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
    /// Information only: the flow stays complete.
    fn note(&mut self, file: &str, line: u32, message: String) {
        self.0.diagnostics.push(UiFlowDiagnostic {
            file: file.into(),
            line,
            message,
            severity: DiagnosticSeverity::Note,
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
    /// Sets the handler and what it sends on the endpoints from index `from`
    /// on, and where the handler is declared.
    fn link(&mut self, from: usize, (handler, at, sends): handlers::Link) {
        let flow = &mut self.0;
        for e in &mut flow.endpoints[from..] {
            e.handler = handler.clone();
            e.sends = sends.clone();
            if let (Some(handler), Some((file, line))) = (&handler, &at) {
                flow.handler_sites.push(HandlerSite {
                    endpoint_file: e.file.clone(),
                    endpoint_line: e.line,
                    handler: handler.clone(),
                    file: file.clone(),
                    line: *line,
                });
            }
        }
    }
}

/// A source file with comments blanked (offsets and lines unchanged).
pub(crate) struct Source {
    rel: String,
    text: String,
    pairs: HashMap<usize, usize>,
    lines: Vec<usize>,
}
impl Source {
    pub(crate) fn new(rel: String, raw: &str) -> Self {
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

pub(crate) struct Attr {
    pub(crate) name: String,
    pub(crate) args: String,
    pub(crate) at: usize,
}

pub(crate) struct Decl {
    pub(crate) class: bool,
    pub(crate) name: String,
    pub(crate) attrs: Vec<Attr>,
    /// Offset of a method's parameter list; a class's header start.
    open: usize,
    /// A class's body end (`}`, or `;` without a body); 0 for a method.
    end: usize,
    /// A class's base list as simple names (`: Base<T>, IFoo`, `extends`,
    /// `implements`); empty for a method.
    pub(crate) bases: Vec<String>,
    /// `interface` for a class; `public` and not `static` for a method.
    flag: bool,
}

/// Calls `f(class, method)` for each method with the innermost class around it.
fn each_method(src: &Source, annotations: bool, mut f: impl FnMut(&Decl, &Decl)) {
    let mut open: Vec<Decl> = Vec::new();
    for d in decls(src, annotations) {
        while open.last().is_some_and(|c| c.end < d.open) {
            open.pop();
        }
        if d.class {
            open.push(d);
        } else if let Some(class) = open.last() {
            f(class, &d);
        }
    }
}

fn has(attrs: &[Attr], name: &str) -> bool {
    attrs.iter().any(|a| a.name == name)
}

/// Last segment of a qualified attribute name, without `Attribute` and a target.
pub(crate) fn simple(name: &str) -> String {
    let name = name.rsplit([':', '.']).next().unwrap_or(name).trim();
    name.strip_suffix("Attribute")
        .filter(|n| !n.is_empty())
        .unwrap_or(name)
        .to_string()
}

/// Class and annotated method declarations with their attributes, in order.
/// `annotations`: Java/Kotlin `@X(…)`; otherwise C# `[X(…), Y]`.
pub(crate) fn decls(src: &Source, annotations: bool) -> Vec<Decl> {
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

/// The attributes of a Razor `@attribute [A(…)] [B]` directive (text after
/// the keyword); `at` is the offset of the list in `rest`.
pub(crate) fn directive_attrs(rest: &str) -> Vec<Attr> {
    let src = Source::new(String::new(), rest);
    let (mut out, mut i) = (Vec::new(), 0);
    while let Some(at) = src.text[i..].find(|c: char| !c.is_whitespace()) {
        if src.bytes()[i + at] != b'[' {
            break;
        }
        let next = attribute_list(&src, i + at, &mut out);
        if next == i + at + 1 && src.pair(i + at).is_none() {
            break;
        }
        i = next;
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

/// A class, or a method with attributes; in C# also a method without any
/// (conventional routing needs every public action).
fn declaration(src: &Source, start: usize, end: usize, attrs: Vec<Attr>) -> Option<Decl> {
    let header = &src.text[start..end];
    if let Some((interface, name)) = class_name(header, src.rel.ends_with(".kt")) {
        let (bases, end) = class_shape(src, start);
        return Some(Decl {
            class: true,
            name,
            attrs,
            open: start,
            end,
            bases,
            flag: interface,
        });
    }
    let p = header.find('(')?;
    let modifiers: Vec<String> = (header[..p].split_whitespace())
        .map(str::to_owned)
        .collect();
    let has = |w: &str| modifiers.iter().any(|m| m == w);
    let public = has("public") && !has("static") && !header[..p].contains('=');
    if attrs.is_empty() && !(public && src.rel.ends_with(".cs")) {
        return None;
    }
    let (s, e) = ident_before(header.as_bytes(), p);
    (s < e).then(|| Decl {
        class: false,
        name: header[s..e].into(),
        attrs,
        open: start + p,
        end: 0,
        bases: vec![],
        flag: public,
    })
}

/// `(is an interface, name)` of a type declaration header.
fn class_name(header: &str, kotlin: bool) -> Option<(bool, String)> {
    let head = header.split('(').next().unwrap_or(header);
    let mut words = head
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty());
    while let Some(w) = words.next() {
        if matches!(w, "class" | "interface" | "record" | "struct") || kotlin && w == "object" {
            return words.next().map(|n| (w == "interface", n.to_owned()));
        }
    }
    None
}

/// The base list and body end of the type declared at `start`.
fn class_shape(src: &Source, start: usize) -> (Vec<String>, usize) {
    let b = src.bytes();
    let mut j = start;
    while j < b.len() && !matches!(b[j], b'{' | b';') {
        if let Some(end) = literal_end(b, j) {
            j = end;
            continue;
        }
        j = match b[j] {
            b'(' | b'[' => src.pair(j).map_or(j + 1, |c| c + 1),
            _ => j + 1,
        };
    }
    let end = match b.get(j) {
        Some(b'{') => src.pair(j).unwrap_or(b.len()),
        _ => j,
    };
    (base_names(&src.text[start..j.min(b.len())]), end)
}

/// Simple names in the base list of a type header (`: Base<T>, IFoo`,
/// `extends`, `implements`), primary constructors and generics left out.
fn base_names(header: &str) -> Vec<String> {
    let (mut head, mut depth) = (String::new(), 0i32);
    for c in header.chars() {
        match c {
            '(' | '[' | '<' => depth += 1,
            ')' | ']' | '>' => depth -= 1,
            _ if depth == 0 => head.push(c),
            _ => {}
        }
    }
    let head = head.split(" where ").next().unwrap_or("");
    let list = match head.find(':') {
        Some(p) => &head[p + 1..],
        None => match head.find(" extends ").or_else(|| head.find(" implements ")) {
            Some(p) => &head[p..],
            None => "",
        },
    };
    (list.replace(" implements ", ",").replace(" extends ", ","))
        .split(',')
        .filter_map(|item| {
            let item = item.split_whitespace().next()?;
            let name = item.rsplit('.').next().unwrap_or(item);
            let name: String = name
                .chars()
                .take_while(|c| lex::is_ident(*c as u8))
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

/// Literals, brackets, identifiers and argument lists on raw text.
mod lex {
    pub(super) use crate::lang_csharp::{code as strip_comments, literal, literal_end};

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
        split_by(s, b',')
    }

    /// Splits on `sep` outside brackets and literals; empty parts are dropped.
    pub(super) fn split_by(s: &str, sep: u8) -> Vec<&str> {
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
                c if c == sep && depth == 0 => {
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
    pub(super) fn named_part(part: &str) -> Option<(&str, &str)> {
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

/// `access` that must also satisfy `extra`: both checks run in turn.
fn require(access: &AccessState, extra: Condition) -> AccessState {
    let mut items = vec![extra];
    if let AccessState::Protected(c) = access {
        items.push(c.clone());
    }
    let flat = (items.into_iter())
        .flat_map(|c| match c {
            Condition::All(v) => v,
            c => vec![c],
        })
        .collect();
    combine(flat, Condition::All).map_or(AccessState::Undeclared, AccessState::Protected)
}

/// What the scans read across all projects of one run: `[views.access]`,
/// string constants, and types by simple name (base classes, request types).
///
/// - A constant is `const string X = …` (C#), `static final String X = …`
///   (Java) or `const val X = …` (Kotlin), referenced as `Class.X`; its value
///   is a literal, `nameof(Y)`, another constant or `+` of these. Two
///   different initializers for the same `Class.X` resolve to nothing.
/// - Bases are followed by simple name; a name declared twice is ambiguous
///   (a note) and stops the walk; a name outside the scan stops it silently.
/// - Requests: a class implementing `IPipelineBehavior<,>` whose file names
///   `AuthorizeAttribute` (or `<Listed>Attribute`) enforces that attribute on
///   every request type the endpoint sends, read as the CleanArchitecture
///   `AuthorizationBehaviour` does: signed in, any role of all attributes,
///   every policy; a listed attribute adds its meaning.
mod index {
    use super::lex::{literal, named, named_part, positional, split_by, split_top, words};
    use super::*;

    /// Bound for constant-to-constant and base-class hops.
    const MAX_DEPTH: usize = 8;

    pub(super) struct Index<'a> {
        access: BTreeMap<String, AccessMeaning>,
        /// `Class.Member` → the initializer texts found for it.
        constants: HashMap<String, BTreeSet<String>>,
        classes: HashMap<String, Vec<(&'a Source, Decl)>>,
        /// Attribute names a pipeline behaviour reads from requests.
        enforced: BTreeSet<String>,
    }

    impl<'a> Index<'a> {
        pub(super) fn new(
            sources: impl Iterator<Item = &'a Source>,
            access: BTreeMap<String, AccessMeaning>,
        ) -> Self {
            let mut index = Index {
                access,
                constants: HashMap::new(),
                classes: HashMap::new(),
                enforced: BTreeSet::new(),
            };
            for src in sources {
                let decls = decls(src, !src.rel.ends_with(".cs"));
                for (key, init) in constants(src, &decls) {
                    index.constants.entry(key).or_default().insert(init);
                }
                for d in decls.into_iter().filter(|d| d.class && !d.flag) {
                    if d.bases.iter().any(|b| b == "IPipelineBehavior") {
                        let names = std::iter::once("Authorize")
                            .chain(index.access.keys().map(String::as_str));
                        let read: Vec<String> = names
                            .filter(|n| words(&src.text, &format!("{n}Attribute")).next().is_some())
                            .map(str::to_owned)
                            .collect();
                        index.enforced.extend(read);
                    }
                    index
                        .classes
                        .entry(d.name.clone())
                        .or_default()
                        .push((src, d));
                }
            }
            index
        }

        /// A string value: a literal, `nameof(X)`, a constant of the scan, or `+` of these.
        pub(super) fn text(&self, raw: &str) -> Option<String> {
            self.text_at(raw, 0)
        }

        fn text_at(&self, raw: &str, depth: usize) -> Option<String> {
            let parts = split_by(raw, b'+');
            if depth > MAX_DEPTH || parts.is_empty() {
                return None;
            }
            (parts.iter().map(|p| self.atom(p, depth)))
                .collect::<Option<Vec<_>>>()
                .map(|v| v.concat())
        }

        fn atom(&self, p: &str, depth: usize) -> Option<String> {
            if let Some(s) = literal(p) {
                return Some(unescape(&s, p.starts_with('@')));
            }
            if let Some(inner) = p.strip_prefix("nameof(").and_then(|r| r.strip_suffix(')')) {
                let last = inner.rsplit('.').next().unwrap_or(inner).trim();
                return member_path(inner).then(|| last.to_string());
            }
            let mut segments = p.rsplit('.');
            let (member, class) = (segments.next()?, segments.next()?);
            let inits = self.constants.get(&format!("{class}.{member}"))?;
            match (member_path(p), inits.len()) {
                (true, 1) => self.text_at(inits.first()?, depth + 1),
                _ => None,
            }
        }

        /// The `[views.access]` meaning of attribute `a` at its use site.
        fn meaning(&self, a: &Attr) -> Option<&AccessMeaning> {
            let has_arguments = !split_top(&a.args).is_empty();
            Some(self.access.get(&a.name)?.at_use_site(has_arguments))
        }

        pub(super) fn is_open(&self, a: &Attr) -> bool {
            self.meaning(a) == Some(&AccessMeaning::Open)
        }

        /// A listed attribute's condition: `None` when not listed, `Some(None)` for open.
        pub(super) fn listed(
            &self,
            out: &mut Out,
            src: &Source,
            a: &Attr,
        ) -> Option<Option<Condition>> {
            use AccessMeaning as M;
            Some(match self.meaning(a)? {
                M::Open => None,
                M::Authenticated => Some(Condition::Authenticated),
                M::Never => Some(Condition::Never),
                M::Role(n) => Some(Condition::Role(n.clone())),
                M::Policy(n) => Some(Condition::Policy(n.clone())),
                M::RoleFromArguments => Some(self.arguments(out, src, a, Condition::Role)),
                M::PolicyFromArguments => Some(self.arguments(out, src, a, Condition::Policy)),
                // `at_use_site` resolved it; a fallback is never a `$arg` form.
                M::FromArgumentsOr(..) => Some(out.unknown(src, a.at, &a.name)),
            })
        }

        /// `$arg`: any of the arguments; a constant that is no literal keeps its member path.
        fn arguments(
            &self,
            out: &mut Out,
            src: &Source,
            a: &Attr,
            atom: fn(String) -> Condition,
        ) -> Condition {
            let args: Vec<&str> = (split_top(&a.args).into_iter())
                .map(|p| named_part(p).map_or(p, |(_, v)| v))
                .collect();
            if args.is_empty() {
                return out.unknown(src, a.at, &format!("{} without arguments", a.name));
            }
            let names: Option<Vec<Condition>> = (args.iter())
                .map(|p| {
                    let name = self
                        .text(p)
                        .or_else(|| member_path(p).then(|| p.trim().into()));
                    name.map(atom)
                })
                .collect();
            (names.and_then(|n| combine(n, Condition::Any)))
                .unwrap_or_else(|| out.unknown(src, a.at, &a.args))
        }

        /// Base classes of `class` (declared in `src`), nearest first.
        pub(super) fn ancestors<'s>(
            &'s self,
            out: &mut Out,
            src: &Source,
            class: &Decl,
        ) -> Vec<(&'a Source, &'s Decl)> {
            let mut found: Vec<(&'a Source, &'s Decl)> = Vec::new();
            let mut seen = BTreeSet::from([class.name.clone()]);
            let mut base = class.bases.first();
            while let Some(name) = base {
                let Some(candidates) = self.classes.get(name) else {
                    break;
                };
                if found.len() >= MAX_DEPTH || !seen.insert(name.clone()) {
                    break;
                }
                if candidates.len() > 1 {
                    let message = format!(
                        "Base class {name} is ambiguous ({} classes of that name); its \
                         attributes are not applied to {}.",
                        candidates.len(),
                        class.name
                    );
                    out.note(&src.rel, src.line(class.open), message);
                    break;
                }
                let (s, d) = &candidates[0];
                found.push((*s, d));
                base = d.bases.first();
            }
            found
        }

        /// The declaration of class `name` in `src`.
        pub(super) fn class_in(&self, src: &Source, name: &str) -> Option<&Decl> {
            (self.classes.get(name)?.iter())
                .find(|(s, _)| std::ptr::eq(*s, src))
                .map(|(_, d)| d)
        }

        /// Endpoints must also satisfy the enforced attributes of the requests they send.
        pub(super) fn apply_request_access(&self, out: &mut Out) {
            if self.enforced.is_empty() {
                return;
            }
            for i in 0..out.0.endpoints.len() {
                let sends = out.0.endpoints[i].sends.clone();
                let conditions = (sends.iter())
                    .filter_map(|t| self.request(out, simple_type(t)))
                    .collect();
                if let Some(c) = combine(conditions, Condition::All) {
                    let e = &mut out.0.endpoints[i];
                    e.access = require(&e.access, c);
                }
            }
        }

        fn request(&self, out: &mut Out, name: &str) -> Option<Condition> {
            let candidates = self.classes.get(name)?;
            let enforced = |d: &Decl| d.attrs.iter().any(|a| self.enforced.contains(&a.name));
            let [(src, d)] = candidates.as_slice() else {
                if let Some((s, d)) = candidates.iter().find(|(_, d)| enforced(d)) {
                    let message = format!(
                        "Request type {name} is declared {} times; its attributes are not applied.",
                        candidates.len()
                    );
                    out.note(&s.rel, s.line(d.open), message);
                }
                return None;
            };
            let mut attrs: Vec<(&Source, &Attr)> = d.attrs.iter().map(|a| (*src, a)).collect();
            for (s, base) in self.ancestors(out, src, d) {
                attrs.extend(base.attrs.iter().map(|a| (s, a)));
            }
            let (mut items, mut roles, mut authorize) = (Vec::new(), Vec::new(), false);
            for (s, a) in attrs
                .into_iter()
                .filter(|(_, a)| self.enforced.contains(&a.name))
            {
                if a.name != "Authorize" {
                    items.extend(self.listed(out, s, a).flatten());
                    continue;
                }
                authorize = true;
                if let Some(raw) = named(&a.args, "Roles") {
                    match self.text(raw) {
                        Some(r) => roles.extend(
                            (r.split(',').map(str::trim))
                                .filter(|r| !r.is_empty())
                                .map(|r| Condition::Role(r.into())),
                        ),
                        None => items.push(out.unknown(s, a.at, raw)),
                    }
                }
                if let Some(raw) = named(&a.args, "Policy").or_else(|| positional(&a.args)) {
                    let policy = self.text(raw).map(Condition::Policy);
                    items.push(policy.unwrap_or_else(|| out.unknown(s, a.at, raw)));
                }
            }
            if authorize {
                items.push(Condition::Authenticated);
            }
            items.extend(combine(roles, Condition::Any));
            combine(items, Condition::All)
        }
    }

    /// `A.B.C` or `X`: identifiers joined by dots.
    pub(super) fn member_path(p: &str) -> bool {
        let p = p.trim();
        !p.is_empty() && (p.split('.')).all(|s| !s.is_empty() && s.bytes().all(lex::is_ident))
    }

    fn unescape(s: &str, verbatim: bool) -> String {
        if verbatim {
            return s.replace("\"\"", "\"");
        }
        let (mut out, mut chars) = (String::new(), s.chars());
        while let Some(c) = chars.next() {
            match (c, chars.clone().next()) {
                ('\\', Some(n @ ('"' | '\\' | '\''))) => {
                    out.push(n);
                    chars.next();
                }
                _ => out.push(c),
            }
        }
        out
    }

    /// `(Class.Member, initializer)` of each string constant in `src`.
    fn constants(src: &Source, decls: &[Decl]) -> Vec<(String, String)> {
        const NOT_NAMES: [&str; 7] = [
            "static", "string", "String", "val", "final", "readonly", "new",
        ];
        let mut found = Vec::new();
        for kw in ["const", "final"] {
            for p in words(&src.text, kw) {
                let from = p + kw.len();
                let Some(eq) = src.text[from..].find('=').map(|e| from + e) else {
                    continue;
                };
                let head = &src.text[from..eq];
                let typed = (head.split_whitespace())
                    .any(|t| matches!(t.trim_end_matches(':'), "string" | "String" | "val"));
                let name = (head.split_whitespace().map(|t| t.trim_end_matches(':')))
                    .find(|t| !NOT_NAMES.contains(t));
                let class = (decls.iter())
                    .filter(|d| d.class && d.open < p && p < d.end)
                    .max_by_key(|d| d.open);
                let (Some(name), Some(class), true) = (name, class, typed) else {
                    continue;
                };
                if head.contains([';', '(', '{', '}', '\n'])
                    || src.bytes().get(eq + 1) == Some(&b'=')
                {
                    continue;
                }
                let end = if src.rel.ends_with(".kt") {
                    src.text[eq..]
                        .find(['\n', ';'])
                        .map_or(src.text.len(), |e| eq + e)
                } else {
                    minimal::statement_end(src, eq + 1)
                };
                found.push((
                    format!("{}.{name}", class.name),
                    src.text[eq + 1..end].trim().to_string(),
                ));
            }
        }
        found
    }
}
use index::Index;

/// ASP.NET Core controllers: attribute routes, else the project's
/// conventional routes ([`conventional`]). A class without its own `[Route]`
/// takes the one of its nearest base class; `[Authorize]`, `[AllowAnonymous]`,
/// `[Area]` and listed attributes of base classes apply too.
mod controllers {
    use super::conventional::Route;
    use super::lex::{named, positional};
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

    /// Attributes with the file they are written in.
    type Attrs<'a> = Vec<(&'a Source, &'a Attr)>;

    fn own<'a>(decl: &'a Decl, src: &'a Source) -> Attrs<'a> {
        decl.attrs.iter().map(|a| (src, a)).collect()
    }

    pub(super) fn scan(src: &Source, index: &Index, routes: &[Route], out: &mut Out) {
        each_method(src, false, |class, d| {
            let controller = class.name.ends_with("Controller")
                || has(&class.attrs, "ApiController")
                || has(&class.attrs, "Controller");
            if controller && !has(&class.attrs, "NonController") {
                let ancestors = index.ancestors(out, src, class);
                action(src, index, routes, out, (class, &ancestors), d);
            }
        });
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

    fn action(
        src: &Source,
        index: &Index,
        routes: &[Route],
        out: &mut Out,
        (class, ancestors): (&Decl, &[(&Source, &Decl)]),
        d: &Decl,
    ) {
        // `[ApiController]` demands attribute routes; conventional ones never reach it.
        let api = std::iter::once(class)
            .chain(ancestors.iter().map(|(_, b)| *b))
            .any(|c| has(&c.attrs, "ApiController"));
        let routes = if api { &[] } else { routes };
        let verbs = verbs(&d.attrs);
        let conventional = verbs.is_empty() && !has(&d.attrs, "Route") && !routes.is_empty();
        if d.name == class.name || has(&d.attrs, "NonAction") || verbs.is_empty() && !conventional {
            return;
        }
        let route_level = std::iter::once((src, class))
            .chain(ancestors.iter().copied())
            .find(|(_, c)| has(&c.attrs, "Route"));
        let class_attrs: Attrs = route_level.map_or_else(Vec::new, |(s, c)| own(c, s));
        let (Some(class_routes), Some(method_routes)) = (
            route_templates(index, out, &class_attrs),
            route_templates(index, out, &own(d, src)),
        ) else {
            return;
        };
        if verbs.is_empty() && !class_routes.is_empty() {
            return;
        }
        let mut levels = own(class, src);
        for (s, base) in ancestors {
            levels.extend(own(base, s));
        }
        let area = (levels.iter())
            .find(|(_, a)| a.name == "Area")
            .map(|(s, a)| {
                let name = positional(&a.args).and_then(|r| index.text(r));
                name.ok_or((*s, a.at, a.args.clone()))
            });
        levels.extend(own(d, src));
        let emit = Emit {
            src,
            routes,
            class_routes,
            method_routes,
            area: area.transpose(),
            access: access(index, out, &levels),
            class: &class.name,
            method: &d.name,
        };
        let first = out.0.endpoints.len();
        if verbs.is_empty() {
            emit.conventional(out, d.open, ANY_METHOD);
        }
        for (attr, method) in verbs {
            emit.verb(index, out, attr, method);
        }
        out.link(
            first,
            handlers::of_method(src, &class.name, &d.name, d.open),
        );
    }

    /// One action's routes and access, ready to give endpoints.
    struct Emit<'a> {
        src: &'a Source,
        routes: &'a [Route],
        class_routes: Vec<String>,
        method_routes: Vec<String>,
        /// `[Area]` name; `Err` with where it is when not a literal.
        area: Result<Option<String>, (&'a Source, usize, String)>,
        access: AccessState,
        class: &'a str,
        method: &'a str,
    }

    impl Emit<'_> {
        fn names(&self) -> (&str, &str) {
            let controller = self.class.strip_suffix("Controller").unwrap_or(self.class);
            (
                controller,
                self.method.strip_suffix("Async").unwrap_or(self.method),
            )
        }

        fn conventional(&self, out: &mut Out, at: usize, method: &str) {
            let area = match &self.area {
                Ok(area) => area.as_deref(),
                Err((s, a, args)) => {
                    return out.at(s, *a, format!("Area name is not a literal: {args}"));
                }
            };
            let (controller, action) = self.names();
            for p in conventional::paths(self.routes, area, controller, action) {
                out.endpoint(self.src, at, method, p, &self.access);
            }
        }

        fn verb(&self, index: &Index, out: &mut Out, attr: &Attr, method: &str) {
            let src = self.src;
            let templates = match template(index, attr) {
                Ok(own) => own.map_or_else(|| self.method_routes.clone(), |t| vec![t]),
                Err(raw) => {
                    let message = format!("Route template is not a literal: {raw}");
                    return out.at(src, attr.at, message);
                }
            };
            if templates.is_empty() && self.class_routes.is_empty() {
                if !self.routes.is_empty() {
                    return self.conventional(out, attr.at, method);
                }
                let message = format!(
                    "{}.{} has no route attribute (conventional routing); endpoint skipped.",
                    self.class, self.method
                );
                return out.at(src, attr.at, message);
            }
            let (controller, action) = self.names();
            let tokens = [("[controller]", controller), ("[action]", action)];
            for full in routes_of(&self.class_routes, &templates) {
                let full = tokens
                    .iter()
                    .fold(full, |p, (t, v)| replace_token(&p, t, v));
                out.endpoint(src, attr.at, method, full, &self.access);
            }
        }
    }

    /// Class × method templates; a method template from the root replaces the class one.
    fn routes_of(class: &[String], method: &[String]) -> Vec<String> {
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

    /// `Ok(None)` without a template; `Err(raw)` when it is not a literal or constant.
    fn template(index: &Index, attr: &Attr) -> Result<Option<String>, String> {
        match named(&attr.args, "Template").or_else(|| positional(&attr.args)) {
            None => Ok(None),
            Some(raw) => index.text(raw).map(Some).ok_or_else(|| raw.to_string()),
        }
    }

    fn route_templates(index: &Index, out: &mut Out, attrs: &Attrs) -> Option<Vec<String>> {
        let mut found = Vec::new();
        for (src, a) in attrs.iter().filter(|(_, a)| a.name == "Route") {
            match template(index, a) {
                Ok(t) => found.extend(t),
                Err(raw) => {
                    out.at(src, a.at, format!("Route template is not a literal: {raw}"));
                    return None;
                }
            }
        }
        Some(found)
    }

    /// `[AllowAnonymous]` or an `open` listed attribute anywhere wins; otherwise
    /// every `[Authorize]` and listed attribute must hold.
    fn access(index: &Index, out: &mut Out, levels: &Attrs) -> AccessState {
        let open = |a: &Attr| a.name == "AllowAnonymous" || index.is_open(a);
        if levels.iter().any(|(_, a)| open(a)) {
            return AccessState::Open;
        }
        let mut conditions = Vec::new();
        for (src, a) in levels {
            if a.name == "Authorize" {
                conditions.push(authorize(index, src, out, a));
            } else {
                conditions.extend(index.listed(out, src, a).flatten());
            }
        }
        combine(conditions, Condition::All).map_or(AccessState::Undeclared, AccessState::Protected)
    }

    fn authorize(index: &Index, src: &Source, out: &mut Out, a: &Attr) -> Condition {
        let mut parts = Vec::new();
        if let Some(raw) = named(&a.args, "Roles") {
            let roles = index.text(raw).map(|r| {
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
            let policy = index.text(raw).map(Condition::Policy);
            parts.push(policy.unwrap_or_else(|| out.unknown(src, a.at, raw)));
        }
        combine(parts, Condition::All).unwrap_or(Condition::Authenticated)
    }
}

/// Conventional routes of a project: `MapDefaultControllerRoute()` is
/// `{controller=Home}/{action=Index}/{id?}`; `MapControllerRoute(name,
/// pattern)` and `MapAreaControllerRoute(name, areaName, pattern)` count with
/// a literal (or constant) pattern. An action gets endpoints from every route
/// that reaches it (a route with `{area}` or an area name only for a
/// controller with that `[Area]`, others only without): the full path without
/// optional parameters, and each shorter path its default values allow
/// (`Home/Index` also at `/Home` and `/`). Other parameters stay `{name}`. A
/// non-literal pattern, `defaults:` or a segment mixing text and tokens is a
/// note, and that route is not used.
mod conventional {
    use super::lex::{named, named_part, split_top, words};
    use super::*;

    const DEFAULT_ROUTE: &str = "{controller=Home}/{action=Index}/{id?}";

    enum Segment {
        Literal(String),
        Area,
        Controller(Option<String>),
        Action(Option<String>),
        /// `{name}` text and whether it has a default value.
        Param(String, bool),
        Optional,
    }

    pub(super) struct Route {
        area: Option<String>,
        segments: Vec<Segment>,
    }

    pub(super) fn routes(sources: &[Source], index: &Index, out: &mut Out) -> Vec<Route> {
        let mut found = Vec::new();
        for src in sources.iter().filter(|s| s.rel.ends_with(".cs")) {
            let calls = [
                ("MapDefaultControllerRoute", None),
                ("MapControllerRoute", Some(1)),
                ("MapAreaControllerRoute", Some(2)),
            ];
            for (name, pattern_at) in calls {
                for p in words(&src.text, name) {
                    let Some(close) = minimal::call_close(src, p, name) else {
                        continue;
                    };
                    let open = lex::skip_ws(src.bytes(), p + name.len());
                    let args = &src.text[open + 1..close];
                    match pattern_at.map(|n| route(index, args, n)) {
                        None => found.extend(parse(DEFAULT_ROUTE, None)),
                        Some(Ok(route)) => found.push(route),
                        Some(Err(why)) => {
                            let message = format!(
                                "Conventional route not read ({why}); its endpoints are skipped."
                            );
                            out.note(&src.rel, src.line(p), message);
                        }
                    }
                }
            }
        }
        found
    }

    /// The route of `MapControllerRoute`/`MapAreaControllerRoute` arguments.
    fn route(index: &Index, args: &str, pattern_at: usize) -> Result<Route, String> {
        let positional: Vec<&str> = (split_top(args).into_iter())
            .filter(|p| named_part(p).is_none())
            .collect();
        if named(args, "defaults").is_some() || positional.len() > pattern_at + 1 {
            return Err(format!("default values in code: {}", args.trim()));
        }
        let raw = named(args, "pattern").or_else(|| positional.get(pattern_at).copied());
        let area =
            match pattern_at {
                2 => {
                    let raw = named(args, "areaName").or_else(|| positional.get(1).copied());
                    let name = raw.and_then(|r| index.text(r));
                    Some(name.ok_or_else(|| {
                        format!("area name is not a literal: {}", raw.unwrap_or(""))
                    })?)
                }
                _ => None,
            };
        let raw = raw.unwrap_or("");
        let pattern =
            (index.text(raw)).ok_or_else(|| format!("pattern is not a literal: {raw}"))?;
        parse(&pattern, area).ok_or_else(|| format!("segment mixes text and tokens: {pattern}"))
    }

    fn parse(pattern: &str, area: Option<String>) -> Option<Route> {
        let mut segments = Vec::new();
        for s in pattern.split('/').filter(|s| !s.is_empty()) {
            let Some(inner) = s.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
                if s.contains(['{', '}']) {
                    return None;
                }
                segments.push(Segment::Literal(s.into()));
                continue;
            };
            let name_end = inner.find([':', '=', '?']).unwrap_or(inner.len());
            let name = inner[..name_end].trim_start_matches('*');
            let default = inner
                .split_once('=')
                .map(|(_, d)| d.trim_end_matches('?').to_string());
            segments.push(match name {
                "area" => Segment::Area,
                "controller" => Segment::Controller(default),
                "action" => Segment::Action(default),
                _ if inner.ends_with('?') => Segment::Optional,
                _ => Segment::Param(format!("{{{name}}}"), default.is_some()),
            });
        }
        Some(Route { area, segments })
    }

    /// The paths at which `controller`/`action` (in `area`) answers.
    pub(super) fn paths(
        routes: &[Route],
        area: Option<&str>,
        controller: &str,
        action: &str,
    ) -> Vec<String> {
        let same = |default: &Option<String>, value: &str| {
            default
                .as_deref()
                .is_some_and(|d| d.eq_ignore_ascii_case(value))
        };
        let mut found = Vec::new();
        for r in routes {
            let area_route =
                r.area.is_some() || r.segments.iter().any(|s| matches!(s, Segment::Area));
            let other_area = (r.area.as_deref().zip(area))
                .is_some_and(|(fixed, a)| !fixed.eq_ignore_ascii_case(a));
            if area_route != area.is_some() || other_area {
                continue;
            }
            let mut parts: Vec<(&str, bool)> = Vec::new();
            for s in &r.segments {
                parts.push(match s {
                    Segment::Literal(l) => (l, false),
                    Segment::Area => (area.unwrap_or_default(), false),
                    Segment::Controller(d) => (controller, same(d, controller)),
                    Segment::Action(d) => (action, same(d, action)),
                    Segment::Param(text, default) => (text, *default),
                    Segment::Optional => continue,
                });
            }
            let texts = |n: usize| path(&parts[..n].iter().map(|(t, _)| *t).collect::<Vec<_>>());
            let mut k = parts.len();
            found.push(texts(k));
            while k > 0 && parts[k - 1].1 {
                k -= 1;
                found.push(texts(k));
            }
        }
        found.sort();
        found.dedup();
        found
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
        index: &Index,
        cache: &mut HashMap<String, Vec<Mod>>,
        out: &mut Out,
    ) {
        for verb in VERBS {
            for p in words(&src.text, verb.0) {
                endpoint((src, sources, index), cache, out, p, verb);
            }
        }
    }

    fn endpoint(
        (src, sources, index): (&Source, &[Source], &Index),
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
        let (pattern, delegate) = match index.text(first) {
            Some(pattern) => (pattern, args.get(1)),
            None => match handler_first(src, sources, name, &args) {
                Some(pattern) => (pattern, args.first()),
                None if forwarding(src, p, first) => return,
                None => {
                    out.at(src, p, format!("{name} pattern is not a literal: {first}"));
                    return;
                }
            },
        };
        let chain = match group(src, out, skip_ws_back(b, p) - 1) {
            Ok(chain) => chain,
            Err(message) => return out.at(src, p, message),
        };
        let group_prefix = match endpoint_group(src, sources, index, p, chain.vars.first()) {
            Some(Ok(prefix)) => Some(prefix),
            Some(Err(message)) => return out.at(src, p, message),
            None => None,
        };
        let mut mods = chain.mods;
        mods.extend(trailing(src, out, close, usize::MAX));
        if let Some(m) = returning_method(src, p, &chain.vars) {
            let found = cache
                .entry(m.to_string())
                .or_insert_with(|| call_site_mods(sources, out, m));
            mods.extend(found.iter().cloned());
        }
        let mut parts: Vec<&str> = (group_prefix.iter().chain(&chain.prefix))
            .map(String::as_str)
            .collect();
        parts.push(&pattern);
        let n = out.0.endpoints.len();
        out.endpoint(src, p, method, path(&parts), &access(&mods));
        if let Some(delegate) = delegate {
            out.link(n, handlers::of_delegate(src, sources, delegate));
        }
    }

    /// A project overload `MapGet(this IEndpointRouteBuilder b, Delegate
    /// handler, string pattern = "")` called with a method group of this file
    /// first: the pattern is the second argument, else the overload's default.
    fn handler_first(
        src: &Source,
        sources: &[Source],
        verb: &str,
        args: &[&str],
    ) -> Option<String> {
        let handler = args.first()?.trim();
        if !handler.bytes().all(lex::is_ident) || !handlers::declares(src, handler) {
            return None;
        }
        let params = sources
            .iter()
            .filter(|s| s.rel.ends_with(".cs"))
            .find_map(|s| {
                words(&s.text, verb).find_map(|p| {
                    let open = skip_ws(s.bytes(), p + verb.len());
                    let close = (s.bytes().get(open) == Some(&b'(')).then(|| s.pair(open))??;
                    let params = &s.text[open + 1..close];
                    (params.trim_start().starts_with("this ") && params.contains("Delegate "))
                        .then_some(params)
                })
            })?;
        match args.get(1) {
            Some(raw) => literal(raw),
            None => split_top(params)
                .last()?
                .split_once('=')
                .and_then(|(_, d)| literal(d)),
        }
    }

    /// Whether the call at `at` forwards `arg`, a parameter of the enclosing
    /// extension method that takes a `Delegate` (the overload itself).
    fn forwarding(src: &Source, at: usize, arg: &str) -> bool {
        let b = src.bytes();
        let paren = words(&src.text, "this")
            .filter(|p| *p < at)
            .map(|p| skip_ws_back(b, p))
            .filter(|k| *k > 0 && b[k - 1] == b'(')
            .last();
        let Some((open, close)) = paren.and_then(|k| Some((k - 1, src.pair(k - 1)?))) else {
            return false;
        };
        let params = &src.text[open + 1..close];
        let body = src.text[close..]
            .find('{')
            .and_then(|o| src.pair(close + o));
        params.contains("Delegate")
            && words(params, arg.trim()).next().is_some()
            && body.is_some_and(|end| end > at)
    }

    /// The prefix of an `IEndpointGroup` class (CleanArchitecture template)
    /// whose `Map(RouteGroupBuilder root)` contains `at`: its own literal
    /// `RoutePrefix`, else the default the project's registration builds (the
    /// `?? $"/api/{groupName}"` after `IEndpointGroup.RoutePrefix`, where
    /// `groupName = type.Name`). `None` outside such a class; `Err` when
    /// neither is readable.
    fn endpoint_group(
        src: &Source,
        sources: &[Source],
        index: &Index,
        at: usize,
        root: Option<&String>,
    ) -> Option<Result<String, String>> {
        let class = handlers::enclosing_class(src, at)?;
        let decl = index.class_in(src, &class)?;
        let body = &src.text[decl.open..decl.end.min(src.text.len())];
        let parameter = format!("RouteGroupBuilder {}", root?);
        if !decl.bases.iter().any(|b| b == "IEndpointGroup") || !body.contains(&parameter) {
            return None;
        }
        if let Some(p) = body.find("RoutePrefix") {
            let value = body[p..]
                .split_once("=>")
                .map(|(_, v)| v.split(';').next().unwrap_or(""));
            let prefix = value.and_then(|v| index.text(v));
            return Some(prefix.ok_or_else(|| format!("{class}.RoutePrefix is not a literal")));
        }
        let rule = sources.iter().find_map(registration);
        Some(rule.map(|(before, after)| format!("{before}{class}{after}")).ok_or_else(|| {
            format!("{class}: no readable IEndpointGroup prefix rule in the project; endpoints skipped")
        }))
    }

    /// Text before and after the class-name hole of the default group prefix.
    fn registration(src: &Source) -> Option<(String, String)> {
        let rest = &src.text[src.text.find("IEndpointGroup.RoutePrefix")?..];
        let start = rest.find("?? $\"")? + 4;
        let end = literal_end(rest.as_bytes(), start)?;
        let (before, hole) = rest.get(start + 1..end - 1)?.split_once('{')?;
        let (hole, after) = hole.split_once('}')?;
        let assigned = (src.text.find(&format!("{hole} ="))).map(|p| {
            src.text[p..]
                .split(';')
                .next()
                .unwrap_or("")
                .trim_end()
                .ends_with(".Name")
        });
        (assigned == Some(true) && !after.contains('{')).then(|| (before.into(), after.into()))
    }

    /// The innermost `{…}` block around `at`.
    fn enclosing_block(src: &Source, at: usize) -> Option<(usize, usize)> {
        let b = src.bytes();
        (0..at)
            .rev()
            .filter(|&k| b[k] == b'{')
            .find_map(|k| src.pair(k).filter(|c| *c > at).map(|c| (k, c)))
    }

    /// Modifiers of `var.X(…);` statements in `(from, until)`.
    fn statement_mods(
        src: &Source,
        out: &mut Out,
        var: &str,
        from: usize,
        until: usize,
    ) -> Vec<Mod> {
        let b = src.bytes();
        let mut mods = Vec::new();
        for p in words(&src.text, var).filter(|p| *p > from && *p < until) {
            let k = skip_ws_back(b, p);
            if k == 0 || b[k - 1] != b'.' {
                mods.extend(trailing(src, out, p + var.len() - 1, 1));
            }
        }
        mods
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
    /// statements until the next assignment. Unassigned (parameters, `app`) is
    /// the root, with the `var.RequireAuthorization();` statements of its block.
    fn var_group(src: &Source, out: &mut Out, var: &str, at: usize) -> Result<Chain, String> {
        let assignments = assignments(src, var);
        let Some(&(start, semi)) = assignments.iter().rev().find(|(_, semi)| *semi < at) else {
            let mods = enclosing_block(src, at).map_or_else(Vec::new, |(open, close)| {
                statement_mods(src, out, var, open, close)
            });
            return Ok(Chain {
                vars: vec![var.into()],
                mods,
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
        chain
            .mods
            .extend(statement_mods(src, out, var, semi, until));
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
    pub(super) fn call_close(src: &Source, p: usize, name: &str) -> Option<usize> {
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
    use super::lex::{literals, named, positional};
    use super::*;

    const VERBS: [(&str, &str); 5] = [
        ("GetMapping", "GET"),
        ("PostMapping", "POST"),
        ("PutMapping", "PUT"),
        ("DeleteMapping", "DELETE"),
        ("PatchMapping", "PATCH"),
    ];

    pub(super) fn scan(src: &Source, index: &Index, out: &mut Out) {
        each_method(src, true, |class, d| {
            if has(&class.attrs, "RestController") || has(&class.attrs, "Controller") {
                method(src, index, out, class, d);
            }
        });
    }

    pub(super) use chain::central_rules;

    /// Which scoping-name calls are rules of the authorization block.
    mod rule_calls {
        use super::super::lex::{skip_ws, skip_ws_back};
        use super::super::*;
        use super::chain_span;

        /// `(parameter, open, close)` of every authorization lambda.
        pub(super) type Lambdas = [(String, usize, usize)];

        fn ident_before(src: &Source, end: usize) -> (usize, &str) {
            let b = src.bytes();
            let start = end
                - b[..end]
                    .iter()
                    .rev()
                    .take_while(|c| lex::is_ident(**c))
                    .count();
            (start, &src.text[start..end])
        }

        /// The link of a receiver chain that ends before `dot`: `(start, name,
        /// is a call)`; `None` when it is no plain `name` / `name(…)`.
        fn link_before(src: &Source, dot: usize) -> Option<(usize, &str, bool)> {
            let b = src.bytes();
            let end = skip_ws_back(b, dot);
            let call = end > 0 && b[end - 1] == b')';
            let name_end = if call {
                let open = (0..end).rev().find(|&k| src.pair(k) == Some(end - 1));
                skip_ws_back(b, open.filter(|&k| b[k] == b'(')?)
            } else {
                end
            };
            let (start, name) = ident_before(src, name_end);
            (!name.is_empty()).then_some((start, name, call))
        }

        /// Whether the receiver chain ending before the `.` at `dot` starts at an
        /// authorization lambda's parameter or at an `authorizeRequests()` /
        /// `authorizeHttpRequests()` call, with no `.and()` in between. Anything
        /// unclear (cast, index, `?.`, unbalanced) is not rooted.
        fn rooted_in_rules(src: &Source, mut dot: usize, lambdas: &Lambdas) -> bool {
            let b = src.bytes();
            while let Some((start, name, call)) = link_before(src, dot) {
                // Only the empty-parentheses form returns the rule registry; the
                // lambda form returns the HttpSecurity.
                let open = skip_ws(b, start + name.len());
                let empty = src
                    .pair(open)
                    .is_some_and(|c| src.text[open + 1..c].trim().is_empty());
                match (call, name) {
                    (true, "and") => return false,
                    (true, "authorizeRequests" | "authorizeHttpRequests") if empty => return true,
                    _ => {}
                }
                let before = skip_ws_back(b, start);
                if before == 0 || b[before - 1] != b'.' {
                    let param = |&(ref p, f, t): &(String, usize, usize)| {
                        p == name && f <= start && start < t
                    };
                    return !call && lambdas.iter().any(param);
                }
                dot = before - 1;
            }
            false
        }

        /// A bare call (static factory) is legitimate only inside the arguments of
        /// a rule call.
        fn in_rule_args(src: &Source, s: usize, lambdas: &Lambdas) -> bool {
            let b = src.bytes();
            let mut at = s;
            while let Some(open) = chain_span::enclosing_open(src, at) {
                let (start, name) = ident_before(src, skip_ws_back(b, open));
                let before = skip_ws_back(b, start);
                let rule = chain_span::SCOPING.contains(&name)
                    && !name.starts_with("securityMatcher")
                    && before > 0
                    && b[before - 1] == b'.'
                    && rooted_in_rules(src, before - 1, lambdas);
                if rule {
                    return true;
                }
                at = open;
            }
            false
        }

        /// Whether the scoping-name call starting at `s` is a rule call.
        /// `securityMatcher(s)` never is.
        pub(super) fn is_rule_call(src: &Source, s: usize, lambdas: &Lambdas) -> bool {
            let b = src.bytes();
            if src.text[s..].starts_with("securityMatcher") {
                return false;
            }
            let q = skip_ws_back(b, s);
            if q > 0 && b[q - 1] == b'.' {
                return rooted_in_rules(src, q - 1, lambdas);
            }
            in_rule_args(src, s, lambdas)
        }
    }

    /// Where a call chain ends in the source text.
    mod chain_span {
        use super::super::lex::{skip_ws, skip_ws_back};
        use super::super::*;

        /// The identifier word ending at `q` (exclusive), possibly empty.
        pub(super) fn word_before(src: &Source, q: usize) -> &str {
            let b = src.bytes();
            let start = q - b[..q]
                .iter()
                .rev()
                .take_while(|c| lex::is_ident(**c))
                .count();
            &src.text[start..q]
        }

        /// Whether the name at `s..e` is a call or method reference. Only a
        /// declaration or a constructor is not: Kotlin `fun [Recv.]name(`, Java
        /// `name(…)` followed by a body (`{` or `throws`), and `new name(`. When
        /// unsure it is a call (conservative).
        pub(super) fn is_call(src: &Source, s: usize, e: usize) -> bool {
            let b = src.bytes();
            let q = skip_ws_back(b, s);
            if q >= 2 && &b[q - 2..q] == b"::" {
                return true;
            }
            let open = skip_ws(b, e);
            if b.get(open) != Some(&b'(') || word_before(src, q) == "new" {
                return false;
            }
            if src.rel.ends_with(".kt") {
                let mut r = q;
                if q > 0 && b[q - 1] == b'.' {
                    // the receiver type of `fun Foo.name(`
                    while r > 0 && !b[r - 1].is_ascii_whitespace() {
                        r -= 1;
                    }
                }
                return word_before(src, skip_ws_back(b, r)) != "fun";
            }
            let body = src.pair(open).map(|c| skip_ws(b, c + 1));
            !body.is_some_and(|k| b.get(k) == Some(&b'{') || src.text[k..].starts_with("throws"))
        }

        /// The innermost `(` around `at` that is still open, if any.
        pub(super) fn enclosing_open(src: &Source, at: usize) -> Option<usize> {
            let b = src.bytes();
            let mut depth = 0;
            for k in (0..at).rev() {
                match b[k] {
                    b')' => depth += 1,
                    b'(' if depth == 0 => return Some(k),
                    b'(' => depth -= 1,
                    b';' | b'{' | b'}' => return None,
                    _ => {}
                }
            }
            None
        }

        /// Names of the `HttpSecurity` parameters (`HttpSecurity http`, `http: HttpSecurity`).
        pub(super) fn http_vars(src: &Source) -> Vec<String> {
            let mut vars = Vec::new();
            for p in lex::words(&src.text, "HttpSecurity") {
                let Some(open) = enclosing_open(src, p) else {
                    continue;
                };
                let Some(close) = src.pair(open) else {
                    continue;
                };
                for part in lex::split_top(&src.text[open + 1..close]) {
                    if lex::words(part, "HttpSecurity").next().is_none() {
                        continue;
                    }
                    let name = match part.split_once(':') {
                        Some((n, _)) => n.split_whitespace().last(),
                        None => part.split_whitespace().last(),
                    };
                    vars.extend(name.map(str::to_owned));
                }
            }
            vars.sort();
            vars.dedup();
            vars
        }

        /// Whole identifiers that scope a security chain when called outside the
        /// authorization block (`ignoringAntMatchers` and the like do not).
        pub(super) const SCOPING: [&str; 10] = [
            "antMatcher",
            "antMatchers",
            "mvcMatcher",
            "mvcMatchers",
            "regexMatcher",
            "regexMatchers",
            "requestMatcher",
            "requestMatchers",
            "securityMatcher",
            "securityMatchers",
        ];
        /// Kotlin scope functions and Spring configurer calls: they take or
        /// provide the `HttpSecurity` as `this`/`it`/an argument.
        const SCOPE_FUNS: [&str; 5] = ["apply", "with", "also", "let", "run"];

        fn in_ranges(ranges: &[(usize, usize)], at: usize) -> bool {
            ranges.iter().any(|&(from, to)| from <= at && at < to)
        }

        pub(super) fn ident_end(src: &Source, s: usize) -> usize {
            s + src.bytes()[s..]
                .iter()
                .take_while(|c| lex::is_ident(**c))
                .count()
        }

        /// Offset of the first call of a [`SCOPING`] name that is no rule call.
        pub(super) fn scoping_call(
            src: &Source,
            lambdas: &super::rule_calls::Lambdas,
        ) -> Option<usize> {
            let b = src.bytes();
            (0..b.len()).find(|&s| {
                let e = ident_end(src, s);
                (s == 0 || !lex::is_ident(b[s - 1]))
                    && SCOPING.contains(&&src.text[s..e])
                    && is_call(src, s, e)
                    && !super::rule_calls::is_rule_call(src, s, lambdas)
            })
        }

        /// `name.ident` / `name?.ident` right after `end`: the value is only a receiver.
        fn is_receiver(src: &Source, end: usize) -> bool {
            let b = src.bytes();
            let mut k = skip_ws(b, end);
            if b.get(k) == Some(&b'?') {
                k += 1;
            }
            b.get(k) == Some(&b'.') && b.get(skip_ws(b, k + 1)).is_some_and(|c| lex::is_ident(*c))
        }

        /// The parameter declaration itself: `HttpSecurity http` or `http: HttpSecurity`.
        fn is_declaration(src: &Source, p: usize, end: usize) -> bool {
            let b = src.bytes();
            let typed = b.get(skip_ws(b, end)) == Some(&b':')
                && src.text[skip_ws(b, end) + 1..]
                    .trim_start()
                    .starts_with("HttpSecurity");
            typed || word_before(src, skip_ws_back(b, p)) == "HttpSecurity"
        }

        /// The `{ … }` of `with(http) { … }` or `http.apply { … }` (any [`SCOPE_FUNS`]).
        fn scope_block(src: &Source, p: usize, end: usize) -> Option<(usize, usize)> {
            let b = src.bytes();
            let q = skip_ws_back(b, p);
            let brace = if q > 0 && b[q - 1] == b'(' {
                let close = skip_ws(b, end);
                let named = word_before(src, skip_ws_back(b, q - 1)) == "with";
                (named && b.get(close) == Some(&b')')).then(|| skip_ws(b, close + 1))?
            } else {
                let dot = skip_ws(b, end);
                let s = skip_ws(b, dot + 1);
                let e = ident_end(src, s);
                let known = b.get(dot) == Some(&b'.') && SCOPE_FUNS.contains(&&src.text[s..e]);
                known.then(|| skip_ws(b, e))?
            };
            let close = (b.get(brace) == Some(&b'{'))
                .then(|| src.pair(brace))
                .flatten()?;
            Some((brace, close))
        }

        /// Spans of string and char literals.
        fn literal_spans(src: &Source) -> Vec<(usize, usize)> {
            let b = src.bytes();
            let (mut spans, mut j) = (Vec::new(), 0);
            while j < b.len() {
                match literal_end(b, j) {
                    Some(e) => {
                        spans.push((j, e));
                        j = e.max(j + 1);
                    }
                    None => j += 1,
                }
            }
            spans
        }

        /// Whether `p` lies in a string literal that cannot use the variable: a
        /// Kotlin string with a `$` template (`"$x"`, `"${…}"`) is not plain.
        fn in_plain_string(src: &Source, strings: &[(usize, usize)], p: usize) -> bool {
            let template = |t: &str| {
                t.as_bytes()
                    .windows(2)
                    .any(|w| w[0] == b'$' && (w[1] == b'{' || lex::is_ident(w[1])))
            };
            let kotlin = src.rel.ends_with(".kt");
            (strings.iter().find(|&&(f, t)| f <= p && p < t))
                .is_some_and(|&(f, t)| !(kotlin && template(&src.text[f..t])))
        }

        /// An occurrence of the `HttpSecurity` variable that is not a receiver, a
        /// declaration or the `with(http) { }` scope: it is passed, aliased or cast.
        /// Occurrences in string literals (`"http://…"`) are no uses.
        fn var_hand_off(src: &Source, inside: &[(usize, usize)], vars: &[String]) -> Option<usize> {
            let strings = literal_spans(src);
            vars.iter().find_map(|var| {
                lex::words(&src.text, var).find(|&p| {
                    let end = p + var.len();
                    !in_ranges(inside, p)
                        && !in_plain_string(src, &strings, p)
                        && !is_declaration(src, p, end)
                        && !is_receiver(src, end)
                        && scope_block(src, p, end).is_none()
                })
            })
        }

        /// A `.apply(…)`, `.with(…)`, `.also(…)`, `.let(…)` or `.run(…)` call with
        /// parentheses: the configurer or function is unknown here.
        fn call_hand_off(src: &Source, inside: &[(usize, usize)]) -> Option<usize> {
            let b = src.bytes();
            (0..b.len()).filter(|&i| b[i] == b'.').find_map(|i| {
                let s = skip_ws(b, i + 1);
                let e = ident_end(src, s);
                let call =
                    SCOPE_FUNS.contains(&&src.text[s..e]) && b.get(skip_ws(b, e)) == Some(&b'(');
                (call && !in_ranges(inside, s)).then_some(s)
            })
        }

        /// `this`/`it` inside a scope block of the variable that is not a receiver.
        fn block_hand_off(
            src: &Source,
            inside: &[(usize, usize)],
            blocks: &[(usize, usize)],
        ) -> Option<usize> {
            blocks.iter().find_map(|&(from, to)| {
                ["this", "it"].iter().find_map(|kw| {
                    lex::words(&src.text[from..to], kw)
                        .map(|p| p + from)
                        .find(|&at| !in_ranges(inside, at) && !is_receiver(src, at + kw.len()))
                })
            })
        }

        /// Offset where the `HttpSecurity` leaves the configure method / bean
        /// outside `inside`, so rules elsewhere cannot be seen.
        fn hand_off(src: &Source, inside: &[(usize, usize)]) -> Option<usize> {
            let vars = http_vars(src);
            let blocks: Vec<(usize, usize)> = (vars.iter())
                .flat_map(|v| lex::words(&src.text, v).map(move |p| (p, v.len())))
                .filter_map(|(p, len)| scope_block(src, p, p + len))
                .collect();
            var_hand_off(src, inside, &vars)
                .or_else(|| call_hand_off(src, inside))
                .or_else(|| block_hand_off(src, inside, &blocks))
        }

        /// Why the project's chain cannot be read as global, if so: several
        /// configurations; a scoping call in any file that mentions `HttpSecurity`
        /// (configurers, extensions, helpers); or the `HttpSecurity` handed out of
        /// the configure method / bean of `files`.
        pub(super) fn why_not_evaluated(
            sources: &[Source],
            files: &[&Source],
            chains: usize,
        ) -> Option<String> {
            if chains > 1 {
                return Some("Several security configurations".into());
            }
            let lambdas = |s: &Source| super::chain::auth_lambdas(s);
            let at = |s: &Source, p: usize| format!("{}:{}", s.rel, s.line(p));
            let mentions = |s: &&Source| lex::words(&s.text, "HttpSecurity").next().is_some();
            let scoped = (sources.iter())
                .filter(|s| !s.rel.ends_with(".cs"))
                .filter(mentions)
                .find_map(|s| scoping_call(s, &lambdas(s)).map(|p| at(s, p)));
            if let Some(site) = scoped {
                return Some(format!("A scoping matcher at {site}"));
            }
            let handed = files
                .iter()
                .find_map(|s| hand_off(s, &[]).map(|p| at(s, p)));
            handed.map(|site| format!("Code at {site} hands the HttpSecurity out"))
        }

        /// End of the call chain starting at `from`: the `;`, or (Kotlin has none) a
        /// line end outside brackets whose next code line does not continue the
        /// chain with a leading `.` (comment lines skipped; a trailing `.` continues).
        pub(super) fn chain_end(src: &Source, from: usize) -> usize {
            let b = src.bytes();
            let end = minimal::statement_end(src, from);
            let mut j = from;
            while j < end {
                if let Some(e) = literal_end(b, j) {
                    j = e;
                    continue;
                }
                match b[j] {
                    b'(' | b'[' | b'{' => j = src.pair(j).map_or(j + 1, |c| c + 1),
                    b'\n' if !continues(src, j) => return j,
                    _ => j += 1,
                }
            }
            end
        }

        /// Whether the chain goes on after the line end at `nl`.
        fn continues(src: &Source, nl: usize) -> bool {
            let b = src.bytes();
            let last = skip_ws_back(b, nl);
            if last > 0 && b[last - 1] == b'.' {
                return true;
            }
            let mut k = skip_ws(b, nl);
            loop {
                let rest = &src.text[k.min(b.len())..];
                if rest.starts_with("//") {
                    k = skip_ws(b, k + rest.find('\n').unwrap_or(rest.len()));
                } else if rest.starts_with("/*") {
                    k = skip_ws(b, k + rest.find("*/").map_or(rest.len(), |e| e + 2));
                } else {
                    return b.get(k) == Some(&b'.') || k >= b.len();
                }
            }
        }
    }

    /// Central rules of a `SecurityFilterChain` (`authorizeHttpRequests` /
    /// `authorizeRequests`), applied to the project's endpoints from index
    /// `first` on.
    ///
    /// Rules are read in text order: `requestMatchers`, `mvcMatchers`,
    /// `antMatchers` (literal or constant paths, `HttpMethod.X`) and
    /// `anyRequest()`, each followed by `permitAll`, `denyAll`,
    /// `authenticated`, `fullyAuthenticated`, `anonymous`, `hasRole`,
    /// `hasAnyRole`, `hasAuthority` or `hasAnyAuthority`. The first rule that
    /// matches decides (Spring semantics; Ant `*` within a segment, `**` across
    /// segments). The result combines with the endpoint's annotation as both
    /// layers check in turn: the filter chain runs before method security, so
    /// a protected rule and a protected annotation must both hold, a
    /// protected rule also binds a `@PermitAll` method, and a `permitAll`
    /// rule leaves the annotation as it is. The endpoint keeps its access,
    /// with a note, when the first rule that may match is not readable
    /// (non-literal matcher or other grant), lies inside `if`/`else`, or
    /// matches only some requests (`/api/users/me` against
    /// `/api/users/{login}`, a method rule against `ANY`). No matching rule:
    /// unchanged. Legacy `configure(HttpSecurity)` of classes extending
    /// `WebSecurityConfigurerAdapter` / `ResourceServerConfigurerAdapter` is read
    /// the same way (a class without readable rules is a gap).
    ///
    /// Not evaluated (one note, endpoints keep their annotation access):
    /// - several chains or adapters;
    /// - a call or method reference of `antMatcher(s)`, `mvcMatcher(s)`,
    ///   `regexMatcher(s)`, `requestMatcher(s)` or `securityMatcher(s)` outside
    ///   the authorization block (and outside `configure(WebSecurity)`) in ANY
    ///   file that mentions `HttpSecurity` (configurers, extensions, helpers);
    ///   declarations and `new` are no calls;
    /// - the `HttpSecurity` handed out of the configure method / bean: its
    ///   variable used other than as a receiver (`http.x`), declaration or
    ///   `with(http) { }`; `this`/`it` in `with(http)`, `http.apply/let/also/run
    ///   { }` blocks used other than as a receiver; any `.apply(…)`, `.with(…)`,
    ///   `.also(…)`, `.let(…)` or `.run(…)` call with parentheses (a configurer).
    ///
    /// A scoping-name call is a rule only if its dotted receiver chain (balanced
    /// calls, newlines allowed) is rooted at the parameter of an authorization
    /// lambda or at an `authorizeRequests()`/`authorizeHttpRequests()` call, with
    /// no `.and()` in between; a bare call (static factory) only inside the
    /// arguments of such a rule call; `securityMatcher(s)` never. Unclear receiver
    /// chains are not rules. There are no ranges or statement ends involved.
    /// A scoping name or hand-off anywhere else counts, also in
    /// `configure(WebSecurity)`, a `WebSecurityCustomizer` or the arguments of
    /// `ignoring…Matcher(s)(…)`. Accepted conservative limit: apps using
    /// `web.ignoring().requestMatchers(..)`, `WebSecurityCustomizer`,
    /// `csrf().ignoring…Matchers(..)` with matcher calls, or `configure(WebSecurity)`
    /// with matchers are "not evaluated". Other statements of an authorization
    /// block still count as uses of the variable. Occurrences of the variable in
    /// plain string literals are no uses; a Kotlin string with a `$` template is.
    ///
    /// Limits: scoping in a file that never mentions `HttpSecurity` is not seen;
    /// `.apply(`/`.with(`/… count whatever their receiver (conservative).
    mod chain {
        use super::super::lex::{skip_ws, skip_ws_back, split_top, words};
        use super::super::*;
        use super::{prefixed_role, role, spel};

        const MATCHERS: [&str; 4] = [
            "requestMatchers",
            "mvcMatchers",
            "antMatchers",
            "anyRequest",
        ];

        struct Rule {
            line: u32,
            file: String,
            text: String,
            /// Empty: any method.
            methods: Vec<String>,
            /// `None`: not readable, may match anything.
            patterns: Option<Vec<String>>,
            /// `None`: not readable.
            grant: Option<AccessState>,
            conditional: bool,
        }

        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum Hit {
            No,
            Part,
            Full,
        }

        pub(in super::super) fn central_rules(
            sources: &[Source],
            index: &Index,
            out: &mut Out,
            first: usize,
        ) {
            let found: Vec<(&Source, Vec<(String, usize)>)> = (sources.iter())
                .filter(|s| !s.rel.ends_with(".cs"))
                .map(|s| (s, adapters(s)))
                .filter(|(s, a)| !a.is_empty() || s.text.contains("SecurityFilterChain"))
                .collect();
            let files: Vec<&Source> = found.iter().map(|(s, _)| *s).collect();
            let Some((home, home_adapters)) = found.first() else {
                return;
            };
            let at = (home_adapters.first().map(|a| a.1))
                .or_else(|| home.text.find("SecurityFilterChain"))
                .unwrap_or(0);
            let chains: usize = (found.iter())
                .map(|(s, a)| {
                    let beans = words(&s.text, "SecurityFilterChain").filter(|p| bean(s, *p));
                    beans.count() + a.len()
                })
                .sum();
            let why = super::chain_span::why_not_evaluated(sources, &files, chains);
            if let Some(why) = why {
                let message = format!(
                    "{why}: central rules are not evaluated; endpoints marked undeclared may be protected there."
                );
                return out.note(&home.rel, home.line(at), message);
            }
            let rules: Vec<Rule> = (files.iter()).flat_map(|s| read(s, index)).collect();
            unreadable_classes(&found, index, out);
            rule_notes(&rules, out);
            let (changed, partial) = apply(&rules, &mut out.0.endpoints[first..]);
            for (i, n) in partial {
                let r = &rules[i];
                let message = format!(
                    "Filter chain rule `{}` matches only some requests of {n} endpoint(s) (a path parameter or ANY method); they keep their declared access.",
                    r.text
                );
                out.note(&r.file, r.line, message);
            }
            if rules.is_empty() {
                return;
            }
            let source = if found.iter().all(|(_, a)| a.is_empty()) {
                "Central SecurityFilterChain"
            } else {
                "Security configuration"
            };
            let message = format!(
                "{source} rules evaluated: {} rule(s), access of {changed} endpoint(s) changed.",
                rules.len()
            );
            out.note(&home.rel, home.line(at), message);
        }

        /// A gap per adapter class in a file without any readable rule.
        fn unreadable_classes(
            found: &[(&Source, Vec<(String, usize)>)],
            index: &Index,
            out: &mut Out,
        ) {
            for (s, classes) in found {
                if classes.is_empty() || !read(s, index).is_empty() {
                    continue;
                }
                for (name, at) in classes {
                    let message = format!(
                        "Security configuration `{name}` has no readable `authorizeRequests()` rules; endpoints it protects stay undeclared."
                    );
                    out.at(s, *at, message);
                }
            }
        }

        /// A note per rule that is not read completely.
        fn rule_notes(rules: &[Rule], out: &mut Out) {
            for r in rules {
                let why = match (&r.patterns, &r.grant) {
                    (None, _) => "its matcher is not a literal",
                    (_, None) => "its grant is not read",
                    _ if r.conditional => "it is conditional (inside if/else)",
                    _ => continue,
                };
                let message = format!(
                    "Filter chain rule `{}`: {why}; endpoints it may match keep their declared access.",
                    r.text
                );
                out.note(&r.file, r.line, message);
            }
        }

        /// Applies the first matching rule to each endpoint; returns how many
        /// changed and, per rule index, how many it matched only partly.
        fn apply(
            rules: &[Rule],
            endpoints: &mut [EndpointAccess],
        ) -> (usize, BTreeMap<usize, usize>) {
            let (mut changed, mut partial) = (0, BTreeMap::new());
            for e in endpoints {
                let Some((i, hit)) = (rules.iter().enumerate())
                    .map(|(i, r)| (i, matches(r, e)))
                    .find(|(_, h)| *h != Hit::No)
                else {
                    continue;
                };
                let rule = &rules[i];
                match (&rule.grant, hit, rule.conditional, &rule.patterns) {
                    (Some(grant), Hit::Full, false, Some(_)) => {
                        let before = e.access.clone();
                        e.access = match grant {
                            AccessState::Protected(c) => require(&e.access, c.clone()),
                            _ if e.access == AccessState::Undeclared => AccessState::Open,
                            _ => e.access.clone(),
                        };
                        changed += usize::from(e.access != before);
                    }
                    (Some(_), Hit::Part, false, Some(_)) => *partial.entry(i).or_default() += 1,
                    _ => {}
                }
            }
            (changed, partial)
        }

        /// Classes extending the legacy Spring Security configurer adapters, as
        /// `(name, offset)`.
        fn adapters(src: &Source) -> Vec<(String, usize)> {
            (decls(src, true).into_iter())
                .filter(|d| d.class)
                .filter(|d| {
                    (d.bases.iter()).any(|b| {
                        matches!(
                            b.as_str(),
                            "WebSecurityConfigurerAdapter" | "ResourceServerConfigurerAdapter"
                        )
                    })
                })
                .map(|d| (d.name, d.open))
                .collect()
        }

        /// A `SecurityFilterChain name(` method declaration at `p`.
        fn bean(src: &Source, p: usize) -> bool {
            let b = src.bytes();
            let s = skip_ws(b, p + "SecurityFilterChain".len());
            let e = s + b[s..].iter().take_while(|c| lex::is_ident(**c)).count();
            e > s && b.get(skip_ws(b, e)) == Some(&b'(')
        }

        /// The `(` of every `authorizeHttpRequests`/`authorizeRequests` call, in order.
        fn authorize_sites(src: &Source) -> Vec<usize> {
            let b = src.bytes();
            let mut sites: Vec<usize> = ["authorizeHttpRequests", "authorizeRequests"]
                .iter()
                .flat_map(|kw| words(&src.text, kw).map(move |p| (p, kw.len())))
                .filter_map(|(p, len)| {
                    let open = skip_ws(b, p + len);
                    (b.get(open) == Some(&b'(')).then_some(open)
                })
                .collect();
            sites.sort();
            sites
        }

        /// The text range holding the rules of the `authorize…(` call opening at
        /// `open`, and whether it is the lambda argument; otherwise the chain
        /// `authorizeRequests().antMatchers(…)…` up to `.and()` or the statement end.
        fn block(src: &Source, open: usize) -> Option<(usize, usize, bool)> {
            let b = src.bytes();
            let close = src.pair(open)?;
            let (from, to, lambda) = match src.text[open + 1..close].find("->") {
                None => (
                    close + 1,
                    super::chain_span::chain_end(src, close + 1),
                    false,
                ),
                Some(arrow) => {
                    let body = skip_ws(b, open + 1 + arrow + 2);
                    match (b.get(body), src.pair(body)) {
                        (Some(b'{'), Some(end)) => (body + 1, end, true),
                        _ => (body, close, true),
                    }
                }
            };
            // `.and()` returns to the HttpSecurity: what follows is no rule.
            let end = (calls(src, from, to).into_iter())
                .find(|c| c.1 == "and")
                .map_or(to, |c| c.0);
            Some((from, end, lambda))
        }

        /// `(parameter, open, close)` of every authorization lambda: its rule calls
        /// are rooted at the parameter.
        pub(in super::super) fn auth_lambdas(src: &Source) -> Vec<(String, usize, usize)> {
            (authorize_sites(src).into_iter())
                .filter_map(|open| {
                    let close = src.pair(open)?;
                    let text = &src.text[open + 1..close];
                    let head = text[..text.find("->")?].trim().trim_matches(['(', ')']);
                    Some((head.split_whitespace().last()?.to_owned(), open, close))
                })
                .collect()
        }

        /// The rules of every `authorizeHttpRequests`/`authorizeRequests` call in `src`.
        fn read(src: &Source, index: &Index) -> Vec<Rule> {
            let mut rules = Vec::new();
            for open in authorize_sites(src) {
                site(src, index, open, &mut rules);
            }
            rules
        }

        /// The rules of the `authorize…(` call whose parenthesis opens at `open`.
        fn site(src: &Source, index: &Index, open: usize, rules: &mut Vec<Rule>) {
            let Some((from, to, lambda)) = block(src, open) else {
                return;
            };
            let conditional = conditional(src, open);
            let mut pending: Option<(usize, Vec<String>, Option<Vec<String>>)> = None;
            for (at, name, args) in calls(src, from, to) {
                if name == "and" && !lambda {
                    break;
                }
                if MATCHERS.contains(&name) || pending.is_none() {
                    pending = Some(matcher(index, at, name, args));
                    continue;
                }
                let Some((start, methods, patterns)) = pending.take() else {
                    continue;
                };
                let end = (at + name.len() + args.len() + 2).min(to);
                let text = src.text[start..end].split_whitespace().collect();
                rules.push(Rule {
                    line: src.line(start),
                    file: src.rel.clone(),
                    text,
                    methods,
                    patterns,
                    grant: grant(index, name, args),
                    conditional,
                });
            }
        }

        /// `(position, name, arguments)` of each `.name(args)` call in `from..to`,
        /// skipping nested brackets.
        fn calls(src: &Source, from: usize, to: usize) -> Vec<(usize, &str, &str)> {
            let b = src.bytes();
            let (mut found, mut j) = (Vec::new(), from);
            while j < to {
                if let Some(end) = literal_end(b, j) {
                    j = end;
                    continue;
                }
                match b[j] {
                    b'(' | b'[' | b'{' => j = src.pair(j).map_or(j + 1, |c| c + 1),
                    b'.' => {
                        let s = skip_ws(b, j + 1);
                        let e = s + b[s..].iter().take_while(|c| lex::is_ident(**c)).count();
                        let open = skip_ws(b, e);
                        match (b.get(open) == Some(&b'('))
                            .then(|| src.pair(open))
                            .flatten()
                        {
                            Some(close) if e > s => {
                                found.push((s, &src.text[s..e], &src.text[open + 1..close]));
                                j = close + 1;
                            }
                            _ => j = e.max(j + 1),
                        }
                    }
                    _ => j += 1,
                }
            }
            found
        }

        /// Whether the call at `at` sits in a block opened by `if (…)` or `else`.
        fn conditional(src: &Source, at: usize) -> bool {
            let b = src.bytes();
            let Some(open) = (0..at)
                .rev()
                .find(|&k| b[k] == b'{' && src.pair(k).is_some_and(|c| c > at))
            else {
                return false;
            };
            let k = skip_ws_back(b, open);
            let head = if k > 0 && b[k - 1] == b')' {
                src.pair(k - 1).unwrap_or(k)
            } else {
                k
            };
            let (s, e) = ident_before(b, head);
            matches!(&src.text[s..e], "if" | "else")
        }

        fn matcher(
            index: &Index,
            at: usize,
            name: &str,
            args: &str,
        ) -> (usize, Vec<String>, Option<Vec<String>>) {
            match name {
                "anyRequest" => return (at, vec![], Some(vec!["/**".into()])),
                // Forwards and error dispatches, never a controller request.
                "dispatcherTypeMatchers" => return (at, vec![], Some(vec![])),
                _ => {}
            }
            if !MATCHERS.contains(&name) {
                return (at, vec![], None);
            }
            let (mut methods, mut patterns) = (Vec::new(), Some(Vec::new()));
            for part in split_top(args) {
                if let Some(m) = part.trim().strip_prefix("HttpMethod.") {
                    methods.push(m.to_string());
                    continue;
                }
                match (index.text(part), patterns.as_mut()) {
                    (Some(p), Some(list)) => list.push(p),
                    _ => patterns = None,
                }
            }
            if patterns.as_ref().is_some_and(Vec::is_empty) {
                patterns = Some(vec!["/**".into()]);
            }
            (at, methods, patterns)
        }

        fn grant(index: &Index, name: &str, args: &str) -> Option<AccessState> {
            let roles = |make: fn(&str) -> Condition| -> Option<Condition> {
                let names: Option<Vec<Condition>> = split_top(args)
                    .into_iter()
                    .map(|a| index.text(a).map(|n| make(&n)))
                    .collect();
                combine(names?, Condition::Any)
            };
            let protected = |c: Condition| Some(AccessState::Protected(c));
            match name {
                "permitAll" => Some(AccessState::Open),
                "denyAll" => protected(Condition::Never),
                "authenticated" | "fullyAuthenticated" => protected(Condition::Authenticated),
                "anonymous" => protected(Condition::Not(Box::new(Condition::Authenticated))),
                "hasRole" | "hasAnyRole" => roles(prefixed_role).map(AccessState::Protected),
                "hasAuthority" | "hasAnyAuthority" => roles(role).map(AccessState::Protected),
                "access" => {
                    let expr = index.text(args)?;
                    match expr.trim() {
                        "permitAll" | "permitAll()" => Some(AccessState::Open),
                        _ => spel(&expr).map(AccessState::Protected),
                    }
                }
                _ => None,
            }
        }

        fn matches(rule: &Rule, e: &EndpointAccess) -> Hit {
            let method = match (rule.methods.is_empty(), e.method.as_str()) {
                (true, _) => Hit::Full,
                (false, ANY_METHOD) => Hit::Part,
                (false, m) if rule.methods.iter().any(|r| r == m) => Hit::Full,
                _ => Hit::No,
            };
            let Some(patterns) = &rule.patterns else {
                return method.min(Hit::Part);
            };
            let segments = |p: &str| -> Vec<String> {
                p.split('/')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            };
            let path = segments(&e.path);
            let best = (patterns.iter())
                .map(|p| ant(&segments(p), &path))
                .max()
                .unwrap_or(Hit::No);
            method.min(best)
        }

        /// An Ant pattern against an endpoint path whose `{x}` segments stand
        /// for any value.
        fn ant(rule: &[String], path: &[String]) -> Hit {
            match rule.split_first() {
                None if path.is_empty() => Hit::Full,
                None => Hit::No,
                Some((r, rest)) if r == "**" => (0..=path.len())
                    .map(|k| ant(rest, &path[k..]))
                    .max()
                    .unwrap_or(Hit::No),
                Some((r, rest)) => match path.split_first() {
                    None => Hit::No,
                    Some((p, tail)) => segment(r, p).min(ant(rest, tail)),
                },
            }
        }

        fn segment(rule: &str, path: &str) -> Hit {
            let param = |s: &str| s.starts_with('{') && s.ends_with('}');
            match (param(rule) || rule == "*", param(path)) {
                (true, _) => Hit::Full,
                (false, true) => Hit::Part,
                (false, false) if glob(rule.as_bytes(), path.as_bytes()) => Hit::Full,
                _ => Hit::No,
            }
        }

        fn glob(p: &[u8], s: &[u8]) -> bool {
            match p.split_first() {
                None => s.is_empty(),
                Some((b'*', rest)) => (0..=s.len()).any(|k| glob(rest, &s[k..])),
                Some((b'?', rest)) => !s.is_empty() && glob(rest, &s[1..]),
                Some((c, rest)) => s.first() == Some(c) && glob(rest, &s[1..]),
            }
        }
    }

    fn method(src: &Source, index: &Index, out: &mut Out, class: &Decl, d: &Decl) {
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
            let access = level(src, index, out, &d.attrs)
                .or_else(|| level(src, index, out, &class.attrs))
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

    /// The authority as written (`hasAuthority`, `@Secured`).
    fn role(name: &str) -> Condition {
        Condition::Role(name.into())
    }

    /// `hasRole('A')`: Spring adds the default role prefix unless present.
    fn prefixed_role(name: &str) -> Condition {
        role(&if name.starts_with(crate::ui_flow_backend::ROLE_PREFIX) {
            name.to_string()
        } else {
            format!("{}{name}", crate::ui_flow_backend::ROLE_PREFIX)
        })
    }

    /// Access declared by one level's annotations; `None` when there is none.
    fn level(src: &Source, index: &Index, out: &mut Out, attrs: &[Attr]) -> Option<AccessState> {
        let items: Vec<Option<Condition>> = attrs
            .iter()
            .filter_map(|a| item(src, index, out, a))
            .collect();
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
    fn item(src: &Source, index: &Index, out: &mut Out, a: &Attr) -> Option<Option<Condition>> {
        let raw = named(&a.args, "value")
            .or_else(|| positional(&a.args))
            .unwrap_or("");
        match a.name.as_str() {
            "PreAuthorize" => Some(pre_authorize(src, index, out, a, raw)),
            "Secured" | "RolesAllowed" => {
                let roles = literals(raw)
                    .and_then(|r| combine(r.iter().map(|x| role(x)).collect(), Condition::Any));
                Some(Some(roles.unwrap_or_else(|| out.unknown(src, a.at, raw))))
            }
            "PermitAll" => Some(None),
            "DenyAll" => Some(Some(Condition::Never)),
            _ => index.listed(out, src, a),
        }
    }

    /// `None` for `permitAll()`.
    fn pre_authorize(
        src: &Source,
        index: &Index,
        out: &mut Out,
        a: &Attr,
        raw: &str,
    ) -> Option<Condition> {
        let Some(expr) = index.text(raw) else {
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
                // `#oauth2.hasScope('x')`: the OAuth2 expression root; scopes are roles.
                b'#' if expr[i..].starts_with("#oauth2.") => (None, "#oauth2.".len()),
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
            let make = if matches!(name.as_str(), "hasRole" | "hasAnyRole") {
                prefixed_role
            } else {
                role
            };
            let mut args = if self.eat(&Tok::Open) {
                self.args(make)?
            } else {
                vec![]
            };
            match (name.as_str(), args.len()) {
                ("isAuthenticated" | "isFullyAuthenticated", 0) => Some(Condition::Authenticated),
                ("hasRole" | "hasAuthority" | "hasScope", 1) => args.pop(),
                ("hasAnyRole" | "hasAnyAuthority" | "hasAnyScope", 1..) => {
                    combine(args, Condition::Any)
                }
                _ => None,
            }
        }
        /// Quoted arguments up to the closing parenthesis, as roles.
        fn args(&mut self, make: fn(&str) -> Condition) -> Option<Vec<Condition>> {
            let mut args = Vec::new();
            if self.eat(&Tok::Close) {
                return Some(args);
            }
            loop {
                let Some(Tok::Str(s)) = self.toks.get(self.at) else {
                    return None;
                };
                args.push(make(s));
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

    /// Handler method id, where it is declared and the types it sends.
    pub(super) type Link = (Option<String>, Option<(String, u32)>, Vec<String>);

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
        let at = (src.rel.clone(), src.line(open));
        (Some(format!("{class}.{name}")), Some(at), sends)
    }

    /// A minimal-API delegate argument: a method group (searched in `src`, then
    /// the project) or a lambda.
    pub(super) fn of_delegate(src: &Source, sources: &[Source], arg: &str) -> Link {
        let at = arg.as_ptr() as usize - src.text.as_ptr() as usize;
        if arg.contains("=>") {
            return (None, None, sends(src, (at, at + arg.len())));
        }
        let name = arg.rsplit('.').next().unwrap_or(arg).trim();
        if name.is_empty() || !name.bytes().all(is_ident) {
            return (None, None, vec![]);
        }
        let found = std::iter::once(src)
            .chain(sources)
            .find_map(|s| definition(s, name).map(|open| (s, open)));
        match found {
            Some((s, open)) => {
                let class = enclosing_class(s, open).unwrap_or_default();
                of_method(s, &class, name, open)
            }
            None => (None, None, vec![]),
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

    /// Whether `src` declares a method named `name` with a body.
    pub(super) fn declares(src: &Source, name: &str) -> bool {
        definition(src, name).is_some()
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
    pub(super) fn enclosing_class(src: &Source, at: usize) -> Option<String> {
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

        /// Where the handler of the endpoint is declared.
        fn site(flow: &UiFlow, method: &str, path: &str) -> Option<(String, u32)> {
            let s = flow.handler_site(endpoint(flow, method, path))?;
            Some((s.file.clone(), s.line))
        }

        /// The 1-based line of the first `needle` in `text`.
        fn line_of(text: &str, needle: &str) -> u32 {
            text.lines().position(|l| l.contains(needle)).expect(needle) as u32 + 1
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
            let api = "Ordering.API/Apis/OrdersApi.cs".to_string();
            assert_eq!(
                site(&flow, "POST", "/api/orders"),
                Some((api.clone(), line_of(ORDERS_API, "CreateOrderAsync(")))
            );
            assert_eq!(
                site(&flow, "PUT", "/api/orders/ship"),
                Some((api, line_of(ORDERS_API, "ShipAsync(")))
            );
            // A method group of another file is found there.
            assert_eq!(
                site(&flow, "GET", "/api/orders/elsewhere"),
                Some((
                    "Ordering.API/Apis/Handlers.cs".into(),
                    line_of(HANDLERS, "Elsewhere(")
                ))
            );
            assert_eq!(site(&flow, "POST", "/api/orders/quick"), None, "lambda");
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
            let file = "Api/PaymentsController.cs".to_string();
            assert_eq!(
                site(&flow, "POST", "/api/Payments/ship"),
                Some((file.clone(), line_of(CONTROLLER, "Ship(int id)")))
            );
            assert_eq!(
                site(&flow, "GET", "/api/Payments"),
                Some((file, line_of(CONTROLLER, "List()")))
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
            assert_eq!(
                site(&flow, "PUT", "/orders/{id}/pay"),
                Some((
                    "orders/src/main/java/shop/OrderController.java".into(),
                    line_of(SPRING, "void pay(")
                ))
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
        // The filter chain's `/api/**` sign-in holds next to every annotation.
        let signed_in = |c: Condition| protected(Condition::All(vec![Condition::Authenticated, c]));
        assert_eq!(
            access(&flow, "GET", base),
            &protected(Condition::Authenticated)
        );
        let admin_read = protected(Condition::All(vec![
            Condition::Any(vec![role("SCOPE_x"), role("owner:read")]),
            Condition::Authenticated,
            role("ROLE_ADMIN"),
        ]));
        assert_eq!(access(&flow, "GET", "/api/owners/{id}"), &admin_read);
        assert_eq!(access(&flow, "GET", "/api/owners/by-id/{id}"), &admin_read);
        assert_eq!(
            access(&flow, "POST", "/api/owners/new"),
            &protected(Condition::All(vec![
                Condition::Any(vec![
                    Condition::Not(Box::new(Condition::All(vec![
                        Condition::Authenticated,
                        role("BLOCKED"),
                    ]))),
                    role("ROLE_ADMIN"),
                ]),
                Condition::Authenticated,
            ]))
        );
        // `permitAll()` on the method does not lift the chain's sign-in.
        assert_eq!(
            access(&flow, "PUT", "/api/owners/legacy"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(
            access(&flow, "PATCH", "/api/owners/legacy"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(
            access(&flow, "DELETE", "/api/owners/{id}"),
            &signed_in(Condition::Unknown("@guard.isOwner(#id)".into()))
        );
        assert_eq!(
            access(&flow, "PUT", "/api/owners/{id}/editor"),
            &protected(Condition::All(vec![
                Condition::Any(vec![role("ROLE_ADMIN"), role("ROLE_EDITOR")]),
                Condition::Authenticated,
            ]))
        );
        assert_eq!(
            access(&flow, "PATCH", "/api/owners/{id}/audit"),
            &signed_in(role("AUDITOR"))
        );
        assert_eq!(access(&flow, "GET", "/plain"), &AccessState::Undeclared);
        assert_eq!(access(&flow, "GET", "/k/a"), &protected(role("ROLE_USER")));
        // the read default prefix is emitted; names stay the runtime authority
        assert!(
            flow.role_conventions
                .iter()
                .any(|c| c.role_prefix.as_deref() == Some("ROLE_")),
            "{:?}",
            flow.role_conventions
        );
        assert_eq!(
            access(&flow, "ANY", "/api/owners/closed"),
            &signed_in(Condition::Never)
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

    fn notes(flow: &UiFlow, part: &str) -> usize {
        (flow.diagnostics.iter())
            .filter(|d| d.message.contains(part))
            .count()
    }

    const PERMISSION_IDS: &str = r#"
namespace Squidex.Shared;
public static class PermissionIds
{
    public const string AppContentsRead = "squidex.apps.{app}.contents.{schema}.read";
    public const string Computed = Prefix + ".x";
}
"#;
    const SQUIDEX: &str = r#"
[SignedIn]
public sealed class ContentsController : ControllerBase
{
    [HttpGet("api/content")]
    [ApiPermission(PermissionIds.AppContentsRead)]
    public IActionResult Read() => Ok();

    [HttpGet("api/computed")]
    [ApiPermission(PermissionIds.Computed, "squidex.other")]
    public IActionResult Computed() => Ok();

    [HttpGet("api/bare")]
    [ApiPermission]
    public IActionResult Bare() => Ok();

    [HttpGet("api/public")]
    [Public(PermissionIds.AppContentsRead)]
    public IActionResult Public() => Ok();

    [HttpGet("api/plain")]
    public IActionResult Plain() => Ok();
}

public sealed class OtherController : ControllerBase
{
    [HttpGet("api/custom")]
    [Custom("x")]
    public IActionResult Custom() => Ok();
}
"#;

    #[test]
    fn views_access_maps_listed_attributes_and_constants() {
        let config = "[views.access]\nApiPermission = \"policy:$arg\"\nPublic = \"open\"\n\
                      SignedIn = \"authenticated\"\n";
        let dir = project(&[
            (".archguard/archguard.toml", config),
            ("Web/Web.csproj", "<Project />"),
            ("Web/PermissionIds.cs", PERMISSION_IDS),
            ("Web/ContentsController.cs", SQUIDEX),
        ]);
        let flow = extract(dir.path());
        let signed_in = |c: Condition| protected(Condition::All(vec![Condition::Authenticated, c]));
        assert_eq!(
            access(&flow, "GET", "/api/content"),
            &signed_in(policy("squidex.apps.{app}.contents.{schema}.read"))
        );
        // A constant that is no literal keeps its member path as the name.
        assert_eq!(
            access(&flow, "GET", "/api/computed"),
            &protected(Condition::All(vec![
                Condition::Any(vec![
                    policy("PermissionIds.Computed"),
                    policy("squidex.other")
                ]),
                Condition::Authenticated,
            ]))
        );
        assert_eq!(
            access(&flow, "GET", "/api/bare"),
            &signed_in(Condition::Unknown("ApiPermission without arguments".into()))
        );
        assert_eq!(access(&flow, "GET", "/api/public"), &AccessState::Open);
        assert_eq!(
            access(&flow, "GET", "/api/plain"),
            &protected(Condition::Authenticated)
        );
        // Not listed: never guessed from the name.
        assert_eq!(
            access(&flow, "GET", "/api/custom"),
            &AccessState::Undeclared
        );
        assert_eq!(notes(&flow, "ApiPermission without arguments"), 1);
        assert_eq!(flow.diagnostics.len(), 1, "{:#?}", flow.diagnostics);

        // An invalid configuration: no access entries, one note, no panic.
        let invalid = "[views.access]\nx = 1\n";
        std::fs::write(dir.path().join(".archguard/archguard.toml"), invalid).unwrap();
        let flow = extract(dir.path());
        assert_eq!(
            access(&flow, "GET", "/api/content"),
            &AccessState::Undeclared
        );
        let note = (flow.diagnostics.iter())
            .find(|d| d.message.contains("Configuration not read"))
            .unwrap();
        assert_eq!(note.severity, DiagnosticSeverity::Note);
    }

    // Squidex: `[ApiPermission]` without arguments only needs a signed-in user,
    // `[ApiPermissionOrAnonymous]` without arguments lets everyone in.
    const FALLBACK: &str = r#"
public sealed class AppsController : ControllerBase
{
    [HttpGet("api/apps")]
    [ApiPermission]
    public IActionResult Apps() => Ok();

    [HttpGet("api/apps/{app}")]
    [ApiPermission(PermissionIds.AppContentsRead)]
    public IActionResult App() => Ok();

    [HttpGet("api/ping")]
    [ApiPermissionOrAnonymous]
    public IActionResult Ping() => Ok();

    [HttpGet("api/assets")]
    [ApiPermissionOrAnonymous("squidex.assets")]
    public IActionResult Assets() => Ok();
}
"#;

    #[test]
    fn a_fallback_meaning_applies_to_attributes_without_arguments() {
        let dir = project(&[
            ("Web/Web.csproj", "<Project />"),
            ("Web/PermissionIds.cs", PERMISSION_IDS),
            ("Web/AppsController.cs", FALLBACK),
        ]);
        // Given by the caller (`--config`), not read from the project.
        let config: archguard_core::Config = toml::from_str(
            "[views.access]\nApiPermission = \"policy:$arg|authenticated\"\n\
             ApiPermissionOrAnonymous = \"policy:$arg|open\"\n",
        )
        .unwrap();
        let flow = extract_with(&ExtractContext::new(dir.path(), &config));
        assert_eq!(
            access(&flow, "GET", "/api/apps"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(
            access(&flow, "GET", "/api/apps/{app}"),
            &protected(policy("squidex.apps.{app}.contents.{schema}.read"))
        );
        assert_eq!(access(&flow, "GET", "/api/ping"), &AccessState::Open);
        assert_eq!(
            access(&flow, "GET", "/api/assets"),
            &protected(policy("squidex.assets"))
        );
        assert!(flow.diagnostics.is_empty(), "{:#?}", flow.diagnostics);
    }

    const BASE: &str = r#"
public static class Constants
{
    public const string PrefixApi = "/api";
}

[ApiController]
[Route(Constants.PrefixApi)]
[Authorize]
public abstract class ApiController : Controller
{
}

public abstract class Twin : Controller { }
"#;
    const DERIVED: &str = r#"
public sealed class CommentsController(ICommandBus commandBus)
    : ApiController(commandBus)
{
    private sealed class Helper
    {
        public int X { get; set; }
    }

    [HttpGet]
    [Route("users/collaboration")]
    public IActionResult Collaboration() => Ok();

    [HttpPost("users/open"), AllowAnonymous]
    public IActionResult Open() => Ok();
}

[Authorize(Roles = "Admin")]
public abstract class Twin : Controller { }

[Route("twins")]
public class TwinsController : Twin
{
    [HttpGet]
    public IActionResult List() => Ok();
}
"#;

    #[test]
    fn base_controller_attributes_and_routes_are_inherited() {
        let dir = project(&[
            ("Web/Web.csproj", "<Project />"),
            ("Web/ApiController.cs", BASE),
            ("Api/Api.csproj", "<Project />"),
            ("Api/CommentsController.cs", DERIVED),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            access(&flow, "GET", "/api/users/collaboration"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(access(&flow, "POST", "/api/users/open"), &AccessState::Open);
        // Two classes named Twin: the base is ambiguous, nothing is inherited.
        assert_eq!(access(&flow, "GET", "/twins"), &AccessState::Undeclared);
        assert_eq!(
            notes(&flow, "Base class Twin is ambiguous"),
            1,
            "{:#?}",
            flow.diagnostics
        );
        assert_eq!(flow.endpoints.len(), 3, "{:#?}", flow.endpoints);
    }

    const GROUP_INFRA: &str = r#"
public interface IEndpointGroup
{
    static virtual string? RoutePrefix => null;
    static abstract void Map(RouteGroupBuilder groupBuilder);
}

public static class WebApplicationExtensions
{
    public static WebApplication MapEndpoints(this WebApplication app, Assembly assembly)
    {
        foreach (var type in endpointGroupTypes)
        {
            var groupName = type.Name;
            var routePrefix = type.GetProperty(nameof(IEndpointGroup.RoutePrefix))
                ?.GetValue(null) as string ?? $"/api/{groupName}";
            var group = app.MapGroup(routePrefix).WithTags(groupName);
        }
        return app;
    }
}

public static class EndpointRouteBuilderExtensions
{
    public static RouteHandlerBuilder MapGet(this IEndpointRouteBuilder builder, Delegate handler, [StringSyntax("Route")] string pattern = "")
    {
        return builder.MapGet(pattern, handler).WithName(handler.Method.Name);
    }
    public static RouteHandlerBuilder MapPost(this IEndpointRouteBuilder builder, Delegate handler, [StringSyntax("Route")] string pattern = "")
    {
        return builder.MapPost(pattern, handler).WithName(handler.Method.Name);
    }
    public static RouteHandlerBuilder MapDelete(this IEndpointRouteBuilder builder, Delegate handler, [StringSyntax("Route")] string pattern)
    {
        return builder.MapDelete(pattern, handler).WithName(handler.Method.Name);
    }
}
"#;
    const TODO_LISTS: &str = r#"
public class TodoLists : IEndpointGroup
{
    public static void Map(RouteGroupBuilder groupBuilder)
    {
        groupBuilder.RequireAuthorization();

        groupBuilder.MapGet(GetTodoLists);
        groupBuilder.MapDelete(DeleteTodoList, "{id}");
        groupBuilder.MapPost(PurgeTodoLists, "purge");
    }

    public static async Task<Ok<TodosVm>> GetTodoLists(ISender sender)
    {
        var vm = await sender.Send(new GetTodosQuery());
        return TypedResults.Ok(vm);
    }

    public static async Task<NoContent> DeleteTodoList(ISender sender, int id)
    {
        await sender.Send(new DeleteTodoListCommand(id));
        return TypedResults.NoContent();
    }

    public static async Task<NoContent> PurgeTodoLists(ISender sender)
    {
        await sender.Send(new PurgeTodoListsCommand());
        return TypedResults.NoContent();
    }
}

public class Users : IEndpointGroup
{
    public static string? RoutePrefix => "/api/people";

    public static void Map(RouteGroupBuilder groupBuilder)
    {
        groupBuilder.MapPost(Logout, "logout").RequireAuthorization();
        groupBuilder.MapGet(Info);
    }

    public static Task<Ok> Logout(SignInManager<ApplicationUser> signInManager) => Task.FromResult(TypedResults.Ok());
    public static Ok Info() => TypedResults.Ok();
}
"#;
    const REQUESTS: &str = r#"
public abstract class Roles
{
    public const string Administrator = nameof(Administrator);
}
public abstract class Policies
{
    public const string CanPurge = "CanPurge";
}

[Authorize]
public record GetTodosQuery : IRequest<TodosVm>;

[Authorize(Roles = Roles.Administrator)]
[Authorize(Policy = Policies.CanPurge)]
public record PurgeTodoListsCommand : IRequest;

public record DeleteTodoListCommand(int Id) : IRequest;
"#;
    const BEHAVIOUR: &str = r#"
public class AuthorizationBehaviour<TRequest, TResponse> : IPipelineBehavior<TRequest, TResponse>
    where TRequest : notnull
{
    public async Task<TResponse> Handle(TRequest request, RequestHandlerDelegate<TResponse> next, CancellationToken cancellationToken)
    {
        var authorizeAttributes = request.GetType().GetCustomAttributes<AuthorizeAttribute>();
        return await next();
    }
}
"#;

    #[test]
    fn endpoint_groups_and_roles_on_enforced_requests() {
        let files = [
            ("src/Web/Web.csproj", "<Project />"),
            ("src/Web/Infrastructure/Groups.cs", GROUP_INFRA),
            ("src/Web/Endpoints/TodoLists.cs", TODO_LISTS),
            ("src/Application/Application.csproj", "<Project />"),
            ("src/Application/Requests.cs", REQUESTS),
        ];
        let dir = project(&files);
        let flow = extract(dir.path());
        let authenticated = protected(Condition::Authenticated);
        // Without a pipeline behaviour the request attributes are not enforced.
        assert_eq!(access(&flow, "GET", "/api/TodoLists"), &authenticated);
        assert_eq!(
            access(&flow, "POST", "/api/TodoLists/purge"),
            &authenticated
        );
        assert_eq!(
            access(&flow, "DELETE", "/api/TodoLists/{id}"),
            &authenticated
        );
        assert_eq!(access(&flow, "POST", "/api/people/logout"), &authenticated);
        assert_eq!(
            access(&flow, "GET", "/api/people"),
            &AccessState::Undeclared
        );
        assert_eq!(flow.endpoints.len(), 5, "{:#?}", flow.endpoints);
        let purge = (flow.endpoints.iter())
            .find(|e| e.path == "/api/TodoLists/purge")
            .unwrap();
        assert_eq!(purge.handler.as_deref(), Some("TodoLists.PurgeTodoLists"));
        assert_eq!(purge.sends, vec!["PurgeTodoListsCommand".to_string()]);
        assert!(flow.diagnostics.is_empty(), "{:#?}", flow.diagnostics);

        let mut with_behaviour = files.to_vec();
        with_behaviour.push(("src/Application/AuthorizationBehaviour.cs", BEHAVIOUR));
        let dir = project(&with_behaviour);
        let flow = extract(dir.path());
        assert_eq!(access(&flow, "GET", "/api/TodoLists"), &authenticated);
        assert_eq!(
            access(&flow, "POST", "/api/TodoLists/purge"),
            &protected(Condition::All(vec![
                Condition::Authenticated,
                role("Administrator"),
                policy("CanPurge"),
            ]))
        );
        assert_eq!(
            access(&flow, "DELETE", "/api/TodoLists/{id}"),
            &authenticated
        );
    }

    const AUTHORITIES: &str = r#"
public final class AuthoritiesConstants {
    public static final String ADMIN = "ROLE_ADMIN";
}
"#;
    const FILTER_CHAIN: &str = r#"
@Configuration
public class SecurityConfiguration {
    @Bean
    public SecurityFilterChain filterChain(HttpSecurity http) {
        http.authorizeHttpRequests(authz ->
                authz
                    .requestMatchers("/index.html", "/*.js").permitAll()
                    .requestMatchers(HttpMethod.POST, "/api/authenticate").permitAll()
                    .requestMatchers("/api/register").permitAll()
                    .requestMatchers("/api/users/me").permitAll()
                    .requestMatchers(props.getOpenPath()).permitAll()
                    .requestMatchers("/api/admin/**").hasAuthority(AuthoritiesConstants.ADMIN)
                    .requestMatchers("/api/**").authenticated()
                    .requestMatchers("/management/info").permitAll()
                    .requestMatchers("/management/**").access(custom)
            );
        if (dev) {
            http.authorizeHttpRequests(authz -> authz.requestMatchers("/h2/**").permitAll());
        }
        return http.build();
    }
}
"#;
    const RESOURCES: &str = r#"
@RestController
@RequestMapping("/api")
public class AccountResource {
    private static class AccountResourceException extends RuntimeException {
        AccountResourceException(String message) { super(message); }
    }

    @PostMapping("/register")
    public void register() {}

    @GetMapping("/account")
    public void account() {}

    @PostMapping("/authenticate")
    public void login() {}

    @GetMapping("/authenticate")
    public void check() {}

    @GetMapping("/users/{login}")
    public void user() {}

    @GetMapping("/admin/users")
    @PreAuthorize("hasAuthority(\"" + AuthoritiesConstants.ADMIN + "\")")
    public void users() {}

    @GetMapping("/authorities")
    @PreAuthorize("hasRole('AUDITOR')")
    public void authorities() {}

    @PostMapping("/public")
    @PermitAll
    public void open() {}
}

@RestController
public class ManagementResource {
    @GetMapping("/management/info")
    public void info() {}

    @GetMapping("/management/env")
    public void env() {}

    @GetMapping("/h2/console")
    public void h2() {}

    @GetMapping("/other")
    public void other() {}
}
"#;

    #[test]
    fn spring_filter_chain_stops_at_an_unreadable_rule() {
        let dir = project(&[
            ("pom.xml", "<project/>"),
            ("src/main/java/app/AuthoritiesConstants.java", AUTHORITIES),
            ("src/main/java/app/SecurityConfiguration.java", FILTER_CHAIN),
            ("src/main/java/app/AccountResource.java", RESOURCES),
        ]);
        let flow = extract(dir.path());
        assert_eq!(access(&flow, "POST", "/api/register"), &AccessState::Open);
        assert_eq!(
            access(&flow, "POST", "/api/authenticate"),
            &AccessState::Open
        );
        // The POST rule does not match GET; the non-literal matcher after it
        // may match anything, so every later rule decides nothing.
        assert_eq!(
            access(&flow, "GET", "/api/authenticate"),
            &AccessState::Undeclared
        );
        assert_eq!(
            access(&flow, "GET", "/api/account"),
            &AccessState::Undeclared
        );
        assert_eq!(
            access(&flow, "GET", "/management/info"),
            &AccessState::Undeclared
        );
        // The annotation keeps its own reading; the constant resolves.
        assert_eq!(
            access(&flow, "GET", "/api/admin/users"),
            &protected(role("ROLE_ADMIN"))
        );
        assert_eq!(flow.endpoints.len(), 12, "{:#?}", flow.endpoints);
        assert_eq!(notes(&flow, "not a literal"), 1, "{:#?}", flow.diagnostics);
        assert_eq!(notes(&flow, "conditional"), 1, "{:#?}", flow.diagnostics);
        assert_eq!(notes(&flow, "access(custom)"), 1, "{:#?}", flow.diagnostics);
        assert!(
            (flow.diagnostics.iter())
                .filter(|d| d.message.contains("ilter chain"))
                .all(|d| d.severity == DiagnosticSeverity::Note)
        );
    }

    const LEGACY_RESOURCES: &str = r##"
@RestController
@RequestMapping("/api/v1")
public class TicketController {
    @PostMapping("/admin/tickets")
    public void create() {}
    @GetMapping("/admin/tickets")
    public void list() {}
    @GetMapping("/open/ping")
    public void ping() {}
    @GetMapping("/other")
    public void other() {}
    @GetMapping("/hidden/x")
    public void hidden() {}
    @PreAuthorize("#oauth2.hasScope('server')")
    @GetMapping("/internal")
    public void internal() {}
    @PreAuthorize("#oauth2.hasAnyScope('a', 'b') or hasRole('ROOT')")
    @GetMapping("/either")
    public void either() {}
}
"##;

    fn legacy(rules: &str) -> UiFlow {
        let config = format!(
            "@Configuration\npublic class SecurityConfig extends WebSecurityConfigurerAdapter {{\n    @Override\n    protected void configure(HttpSecurity http) throws Exception {{\n        http.csrf().disable().sessionManagement().and()\n            .authorizeRequests()\n{rules}\n            .and().addFilterBefore(new JWTFilter(), X.class);\n    }}\n}}\n"
        );
        let dir = project(&[
            ("pom.xml", "<project/>"),
            ("src/main/java/app/SecurityConfig.java", &config),
            ("src/main/java/app/TicketController.java", LEGACY_RESOURCES),
        ]);
        extract(dir.path())
    }

    #[test]
    fn legacy_adapter_reads_methods_roles_and_any_request() {
        let flow = legacy(
            r#"            .antMatchers(HttpMethod.POST, "/api/v1/admin/**").hasAnyRole("ADMIN", "ROLE_ROOT")
            .antMatchers("/api/v1/open/**", "/swagger/**").permitAll()
            .anyRequest().authenticated()"#,
        );
        let admin = Condition::Any(vec![role("ROLE_ADMIN"), role("ROLE_ROOT")]);
        assert_eq!(
            access(&flow, "POST", "/api/v1/admin/tickets"),
            &protected(admin)
        );
        // The POST rule skips GET; anyRequest decides.
        let signed_in = protected(Condition::Authenticated);
        assert_eq!(access(&flow, "GET", "/api/v1/admin/tickets"), &signed_in);
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
        assert_eq!(access(&flow, "GET", "/api/v1/other"), &signed_in);
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn legacy_adapter_first_match_wins() {
        let flow = legacy(
            r#"            .antMatchers("/api/v1/**").permitAll()
            .antMatchers("/api/v1/admin/**").hasRole("ADMIN")
            .anyRequest().denyAll()"#,
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/admin/tickets"),
            &AccessState::Open
        );
        let flow = legacy(
            r#"            .antMatchers("/api/v1/admin/**").hasRole("ADMIN")
            .antMatchers("/api/v1/**").permitAll()"#,
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/admin/tickets"),
            &protected(role("ROLE_ADMIN"))
        );
        assert_eq!(access(&flow, "GET", "/api/v1/other"), &AccessState::Open);
    }

    #[test]
    fn legacy_regex_matchers_stay_undeclared_with_a_note() {
        let flow = legacy(
            r#"            .regexMatchers("/api/v1/hidden/.*").denyAll()
            .anyRequest().authenticated()"#,
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/hidden/x"),
            &AccessState::Undeclared
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/other"),
            &AccessState::Undeclared
        );
        assert_eq!(notes(&flow, "not a literal"), 1, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn legacy_access_expression_and_oauth2_scopes() {
        let flow = legacy(
            r##"            .antMatchers("/api/v1/admin/**").access("#oauth2.hasScope('server') and hasRole('ADMIN')")
            .anyRequest().authenticated()"##,
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/admin/tickets"),
            &protected(Condition::All(vec![role("ROLE_ADMIN"), role("server")]))
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/internal"),
            &protected(Condition::All(vec![
                Condition::Authenticated,
                role("server")
            ]))
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/either"),
            &protected(Condition::All(vec![
                Condition::Any(vec![
                    Condition::Any(vec![role("a"), role("b")]),
                    role("ROLE_ROOT")
                ]),
                Condition::Authenticated
            ]))
        );
    }

    fn configured(files: &[(&str, &str)]) -> UiFlow {
        let mut all = vec![
            ("pom.xml", "<project/>"),
            ("src/main/java/app/TicketController.java", LEGACY_RESOURCES),
        ];
        all.extend_from_slice(files);
        let dir = project(&all);
        extract(dir.path())
    }

    fn adapter(body: &str) -> String {
        format!(
            "public class SecurityConfig extends WebSecurityConfigurerAdapter {{\n    protected void configure(HttpSecurity http) throws Exception {{\n        {body}\n    }}\n}}\n"
        )
    }

    /// Scoped or several configurations: nothing evaluated, one note.
    fn assert_not_evaluated(flow: &UiFlow) {
        assert_eq!(
            access(flow, "GET", "/api/v1/other"),
            &AccessState::Undeclared
        );
        assert_eq!(notes(flow, "not evaluated"), 1, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn security_matchers_scope_a_filter_chain() {
        let chain = "public class C { @Bean SecurityFilterChain api(HttpSecurity http) { http.securityMatchers(m -> m.requestMatchers(\"/api/**\")).authorizeHttpRequests(a -> a.anyRequest().authenticated()); return http.build(); } }";
        let flow = configured(&[("src/main/java/app/C.java", chain)]);
        assert_not_evaluated(&flow);
    }

    #[test]
    fn legacy_scoping_forms_are_not_evaluated() {
        for scope in [
            "http.requestMatchers().antMatchers(\"/api/**\").and().authorizeRequests().anyRequest().authenticated();",
            "http.antMatcher(\"/api/**\").authorizeRequests().anyRequest().authenticated();",
        ] {
            let config = adapter(scope);
            let flow = configured(&[("src/main/java/app/SecurityConfig.java", &config)]);
            assert_not_evaluated(&flow);
        }
    }

    fn kotlin(body: &str) -> UiFlow {
        let config = format!(
            "class SecurityConfig : WebSecurityConfigurerAdapter() {{\n    override fun configure(http: HttpSecurity) {{\n{body}\n    }}\n}}\n"
        );
        configured(&[("src/main/java/app/SecurityConfig.kt", &config)])
    }

    #[test]
    fn a_kotlin_chain_ends_at_its_last_line() {
        // No `;`: a later scoping call is not part of the authorization block.
        for body in [
            "        http.authorizeRequests().antMatchers(\"/api/**\").authenticated().anyRequest().permitAll()\n        http.antMatcher(\"/api/**\")",
            "        http.authorizeRequests()\n            .antMatchers(\"/api/**\").authenticated()\n            // comment between rules\n            .anyRequest().permitAll()\n\n        http.requestMatchers().antMatchers(\"/api/**\")",
        ] {
            let flow = kotlin(body);
            assert_not_evaluated(&flow);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Undeclared
            );
        }
        let flow = kotlin(
            "        http.csrf().disable()\n        http.authorizeRequests()\n            .antMatchers(\"/api/v1/open/**\").permitAll()\n            .anyRequest().authenticated()\n            .and().httpBasic()",
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/other"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(notes(&flow, "not evaluated"), 0, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn kotlin_receiver_blocks_call_scoping_matchers_without_a_dot() {
        let chain = "authorizeRequests()\n            .antMatchers(\"/api/**\").authenticated()\n            .anyRequest().permitAll()";
        for body in [
            format!(
                "        http.apply {{\n            {chain}\n            antMatcher(\"/api/**\")\n        }}"
            ),
            format!(
                "        http.apply {{\n            antMatcher(\"/api/**\")\n            {chain}\n        }}"
            ),
            format!("        with(http) {{ antMatcher(\"/api/**\"); {chain} }}"),
            format!(
                "        http.run {{\n            {chain}\n            mvcMatcher(\"/api/**\")\n        }}"
            ),
        ] {
            let flow = kotlin(&body);
            assert_not_evaluated(&flow);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Undeclared,
                "{body}"
            );
        }
        let flow = kotlin(
            "        http.apply {\n            authorizeRequests()\n                .antMatchers(\"/api/v1/open/**\").permitAll()\n                .anyRequest().authenticated()\n        }",
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/other"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(notes(&flow, "not evaluated"), 0, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn kotlin_scoping_matchers_in_any_expression_position() {
        let chain = "authorizeRequests().antMatchers(\"/api/v1/open/**\").permitAll().anyRequest().authenticated()";
        for scope in [
            "if (api) antMatcher(\"/api/**\")",
            "if (api) antMatcher(\"/api/**\") else antMatcher(\"/x/**\")",
            "val scoped = antMatcher(\"/api/**\")",
            "when (mode) { \"api\" -> antMatcher(\"/api/**\") }",
            "requireNotNull(antMatcher(\"/api/**\"))",
        ] {
            let flow = kotlin(&format!(
                "        http.apply {{\n            {scope}\n            {chain}\n        }}"
            ));
            assert_not_evaluated(&flow);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Undeclared,
                "{scope}"
            );
            let flow = kotlin(&format!("        http.apply {{ {chain}; {scope} }}"));
            assert_not_evaluated(&flow);
        }
    }

    #[test]
    fn scoping_calls_are_found_whatever_precedes_the_name() {
        let chain = "authorizeRequests().antMatchers(\"/api/v1/open/**\").permitAll().anyRequest().authenticated()";
        for scope in [
            "val api = true\n            antMatcher(\"/api/**\")",
            "val h = http\n            antMatcher(\"/api/**\")",
            "val m = \"a\" to antMatcher(\"/api/**\")",
            "val f = http::antMatcher",
        ] {
            let body = format!(
                "        http.apply {{\n            {scope}\n            {chain}\n        }}"
            );
            let flow = kotlin(&body);
            assert_not_evaluated(&flow);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Undeclared,
                "{scope}"
            );
        }
        let flow = kotlin(&format!(
            "        http.apply {{\n            {chain}\n            return@apply antMatcher(\"/api/**\")\n        }}"
        ));
        assert_not_evaluated(&flow);
        for scope in [
            "http.<HttpSecurity>antMatcher(\"/api/**\").authorizeRequests().anyRequest().authenticated();",
            "Function<String, HttpSecurity> f = http::antMatcher; http.authorizeRequests().anyRequest().authenticated();",
        ] {
            let config = adapter(scope);
            let flow = configured(&[("src/main/java/app/SecurityConfig.java", &config)]);
            assert_not_evaluated(&flow);
        }
    }

    #[test]
    fn a_helper_receiving_the_http_security_hides_its_scoping() {
        for body in [
            "ApiScope.scope(http); http.authorizeRequests().anyRequest().authenticated();",
            "customize(http); http.authorizeRequests().anyRequest().authenticated();",
            "run(a, http); http.authorizeRequests().anyRequest().authenticated();",
        ] {
            let config = adapter(body);
            let flow = configured(&[("src/main/java/app/SecurityConfig.java", &config)]);
            assert_not_evaluated(&flow);
            assert_eq!(
                notes(&flow, "hands the HttpSecurity out"),
                1,
                "{:#?}",
                flow.diagnostics
            );
        }
        let flow = kotlin(
            "        scope(http)\n        http.authorizeRequests().anyRequest().authenticated()",
        );
        assert_not_evaluated(&flow);
        // The DSL itself is no helper: with(http), http.apply(..), http.x(y), assignments.
        let flow =
            kotlin("        with(http) { authorizeRequests().anyRequest().authenticated() }");
        assert_eq!(
            access(&flow, "GET", "/api/v1/other"),
            &protected(Condition::Authenticated)
        );
    }

    const OPEN_CHAIN_JAVA: &str = "http.authorizeRequests().antMatchers(\"/api/v1/open/**\").permitAll().anyRequest().authenticated();";
    const OPEN_CHAIN_KT: &str = "http.authorizeRequests().antMatchers(\"/api/v1/open/**\").permitAll().anyRequest().authenticated()";

    #[test]
    fn handing_the_http_security_out_is_not_evaluated() {
        let java = [
            "HttpSecurity h = http; ApiScope.scope(h);",
            "this.h = http; scope(this.h);",
            "ApiScope.scope((HttpSecurity) http);",
            "http.apply(new ApiScopeConfigurer());",
            "http.csrf().disable().and().apply(cfg);",
            "http.with(cfg, Customizer.withDefaults());",
        ];
        for hand_off in java {
            let config = adapter(&format!("{hand_off} {OPEN_CHAIN_JAVA}"));
            let flow = configured(&[("src/main/java/app/SecurityConfig.java", &config)]);
            assert_not_evaluated(&flow);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Undeclared,
                "{hand_off}"
            );
            assert_eq!(notes(&flow, "hands the HttpSecurity out"), 1, "{hand_off}");
        }
        let kotlin_forms = [
            "val h = http\n        ApiScope.scope(h)",
            "http.let { ApiScope.scope(it) }",
            "with(http) { ApiScope.scope(this) }",
            "http.apply { ApiScope.scope(this) }",
            "ApiScope.scope(h = http)",
            "http.also(ApiScope::scope)",
        ];
        for hand_off in kotlin_forms {
            let flow = kotlin(&format!("        {hand_off}\n        {OPEN_CHAIN_KT}"));
            assert_not_evaluated(&flow);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Undeclared,
                "{hand_off}"
            );
        }
    }

    #[test]
    fn scoping_in_any_file_mentioning_http_security_scopes_every_chain() {
        let extension = "fun HttpSecurity.scopeApi() = antMatcher(\"/api/**\")\n";
        let configurer = "class ApiScopeConfigurer : AbstractHttpConfigurer<ApiScopeConfigurer, HttpSecurity>() {\n    override fun init(http: HttpSecurity) {\n        http.antMatcher(\"/api/**\")\n    }\n}\n";
        for (file, text, call) in [
            ("ApiScope.kt", extension, "http.scopeApi()"),
            ("ApiScope.kt", extension, "http.csrf().disable()"),
            ("ApiScopeConfigurer.kt", configurer, "http.csrf().disable()"),
        ] {
            let config = adapter(&format!("{call}; {OPEN_CHAIN_JAVA}"));
            let path = format!("src/main/java/app/{file}");
            let flow = configured(&[
                ("src/main/java/app/SecurityConfig.java", &config),
                (path.as_str(), text),
            ]);
            assert_not_evaluated(&flow);
            assert_eq!(
                notes(&flow, &format!("{file}:")),
                1,
                "{:#?}",
                flow.diagnostics
            );
        }
        // A file without the word HttpSecurity is not inspected (documented limit).
        let other = "object Unrelated { fun f(x: Any) = x.antMatcher(\"/api/**\") }\n";
        let config = adapter(OPEN_CHAIN_JAVA);
        let flow = configured(&[
            ("src/main/java/app/SecurityConfig.java", &config),
            ("src/main/java/app/Unrelated.kt", other),
        ]);
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
    }

    #[test]
    fn calls_after_and_inside_the_authorization_lambda_are_not_rules() {
        let rules =
            "a.requestMatchers(\"/api/v1/open/**\").permitAll().anyRequest().authenticated()";
        let beans = [
            format!("http.authorizeHttpRequests(a -> {rules}.and().securityMatcher(\"/api/**\"));"),
            format!(
                "http.authorizeHttpRequests(a -> {{ {rules}; a.and().securityMatcher(\"/api/**\"); }});"
            ),
            format!("http.authorizeHttpRequests(a -> {rules}.and().apply(cfg));"),
            format!(
                "http.authorizeHttpRequests(a -> {rules}.and().with(cfg, Customizer.withDefaults()));"
            ),
        ];
        for body in beans {
            let text = format!(
                "public class C {{ @Bean SecurityFilterChain api(HttpSecurity http) throws Exception {{ {body} return http.build(); }} }}"
            );
            let flow = configured(&[("src/main/java/app/C.java", &text)]);
            assert_not_evaluated(&flow);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Undeclared,
                "{body}"
            );
        }
        for tail in [
            ".and().antMatcher(\"/api/**\")",
            ".and().requestMatchers().antMatchers(\"/api/**\")",
        ] {
            let config = adapter(&format!(
                "http.authorizeRequests(a -> a.antMatchers(\"/api/v1/open/**\").permitAll().anyRequest().authenticated(){tail});"
            ));
            let flow = configured(&[("src/main/java/app/SecurityConfig.java", &config)]);
            assert_not_evaluated(&flow);
        }
    }

    #[test]
    fn strings_are_no_uses_of_the_http_security_variable() {
        let java = format!(
            "public class SecurityConfig extends WebSecurityConfigurerAdapter {{\n    void cors() {{ c.setAllowedOrigins(List.of(\"http://localhost:4200\")); log.debug(\"configuring http security\"); String t = \"\"\"\n        http text\n        \"\"\"; }}\n    protected void configure(HttpSecurity http) throws Exception {{ {OPEN_CHAIN_JAVA} }}\n}}\n"
        );
        let flow = configured(&[("src/main/java/app/SecurityConfig.java", &java)]);
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
        assert_eq!(notes(&flow, "not evaluated"), 0, "{:#?}", flow.diagnostics);
        let flow = kotlin(&format!(
            "        val s = \"http security\"\n        {OPEN_CHAIN_KT}"
        ));
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
        for template in ["\"${http}\"", "\"$http\""] {
            let flow = kotlin(&format!(
                "        val s = {template}\n        {OPEN_CHAIN_KT}"
            ));
            assert_not_evaluated(&flow);
        }
    }

    #[test]
    fn ignoring_matchers_and_customizers_make_the_chain_not_evaluated() {
        let config = adapter(&format!(
            "http.csrf(c -> c.ignoringRequestMatchers(antMatcher(\"/h2-console/**\"))); {OPEN_CHAIN_JAVA}"
        ));
        let flow = configured(&[("src/main/java/app/SecurityConfig.java", &config)]);
        assert_not_evaluated(&flow);
        let customizer = "public class SecurityConfig extends WebSecurityConfigurerAdapter {\n    @Bean public WebSecurityCustomizer customizer() { return web -> web.ignoring().requestMatchers(\"/static/**\"); }\n    protected void configure(HttpSecurity http) throws Exception { http.authorizeRequests().antMatchers(\"/api/v1/open/**\").permitAll(); }\n}\n";
        let flow = configured(&[("src/main/java/app/SecurityConfig.java", customizer)]);
        assert_not_evaluated(&flow);
        let config = adapter(OPEN_CHAIN_JAVA);
        let other_java = "class Other { void f(HttpSecurity h) {} @Bean WebSecurityCustomizer c() { return web -> web.ignoring().requestMatchers(\"/x\"); } }";
        let other_kt = "class Other { fun f(h: HttpSecurity) {}\n fun c() = WebSecurityCustomizer { web -> web.ignoring().requestMatchers(\"/x\") } }";
        for (file, text) in [("Other.java", other_java), ("Other.kt", other_kt)] {
            let path = format!("src/main/java/app/{file}");
            let flow = configured(&[
                ("src/main/java/app/SecurityConfig.java", &config),
                (path.as_str(), text),
            ]);
            assert_not_evaluated(&flow);
        }
    }

    #[test]
    fn skips_never_hide_scoping_or_hand_offs() {
        let open =
            "a.requestMatchers(\"/api/v1/open/**\").permitAll().anyRequest().authenticated()";
        let bean_chain = format!("http.authorizeHttpRequests(a -> {open});");
        // F-A: WebSecurityCustomizer next to the chain is not a skip.
        let scope = "http.securityMatcher(\"/api/**\");";
        let java = [
            format!(
                "public class C implements WebSecurityCustomizer {{ public void customize(WebSecurity w) {{}} @Bean SecurityFilterChain api(HttpSecurity http) throws Exception {{ {scope} {bean_chain} return http.build(); }} }}"
            ),
            format!(
                "public class C {{ @ConditionalOnMissingBean(WebSecurityCustomizer.class) @Bean SecurityFilterChain api(HttpSecurity http) throws Exception {{ {scope} {bean_chain} return http.build(); }} }}"
            ),
            format!(
                "public class C {{ @Bean SecurityFilterChain api(HttpSecurity http, WebSecurityCustomizer c) throws Exception {{ {scope} {bean_chain} return http.build(); }} }}"
            ),
            format!(
                "public class C {{ @Bean WebSecurityCustomizer c(HttpSecurity http) throws Exception {{ {scope} return web -> web.ignoring().requestMatchers(\"/x\"); }} @Bean SecurityFilterChain api(HttpSecurity http) throws Exception {{ {bean_chain} return http.build(); }} }}"
            ),
            // F-B: the argument of ignoring…Matchers names the HttpSecurity.
            format!(
                "public class C {{ @Bean SecurityFilterChain api(HttpSecurity http) throws Exception {{ http.csrf(c -> c.ignoringRequestMatchers(http.antMatcher(\"/api/**\"))); {bean_chain} return http.build(); }} }}"
            ),
            format!(
                "public class C {{ @Bean SecurityFilterChain api(HttpSecurity http) throws Exception {{ http.csrf(c -> c.ignoringRequestMatchers(ApiScope.scope(http))); {bean_chain} return http.build(); }} }}"
            ),
            // F-D: a statement in the authorization block that uses the variable.
            format!(
                "public class C {{ @Bean SecurityFilterChain api(HttpSecurity http) throws Exception {{ http.authorizeHttpRequests(a -> {{ {open}; ApiScope.scope(http); }}); return http.build(); }} }}"
            ),
        ];
        for text in &java {
            let flow = configured(&[("src/main/java/app/C.java", text)]);
            assert_not_evaluated(&flow);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Undeclared,
                "{text}"
            );
        }
        let kt = "class C : WebSecurityConfigurerAdapter(), WebSecurityCustomizer {\n    override fun configure(http: HttpSecurity) {\n        http.antMatcher(\"/api/**\")\n        http.authorizeRequests().anyRequest().authenticated()\n    }\n}\n";
        assert_not_evaluated(&configured(&[("src/main/java/app/C.kt", kt)]));
        // F-C: Kotlin templates that may use the variable.
        for template in [
            "\"${describe(http)}\"",
            "\"${ http }\"",
            "\"\"\"${scope(http)}\"\"\"",
        ] {
            let flow = kotlin(&format!(
                "        val s = {template}\n        {OPEN_CHAIN_KT}"
            ));
            assert_not_evaluated(&flow);
        }
    }

    #[test]
    fn nothing_after_ignoring_hides_scoping() {
        let chain = OPEN_CHAIN_JAVA;
        let bean = format!(
            "@Bean SecurityFilterChain api(HttpSecurity http) throws Exception {{ {chain} return http.build(); }}"
        );
        let adapter_cfg =
            format!("protected void configure(HttpSecurity http) throws Exception {{ {chain} }}");
        let java = [
            format!(
                "public class C {{ HttpSecurity otherHttp; @Bean WebSecurityCustomizer c() {{ return web -> {{ web.ignoring().requestMatchers(\"/x\"); otherHttp.securityMatcher(\"/api/**\"); }}; }} {bean} }}"
            ),
            format!(
                "public class C {{ final HttpSecurity otherHttp = null; @Bean WebSecurityCustomizer c() {{ return web -> {{ web.ignoring().requestMatchers(\"/x\"); otherHttp.securityMatcher(\"/api/**\"); }}; }} {bean} }}"
            ),
            format!(
                "public class C {{ @Bean WebSecurityCustomizer c() {{ return web -> {{ web.ignoring().requestMatchers(\"/x\"); Scopes.chain().securityMatcher(\"/api/**\"); }}; }} {bean} }}"
            ),
            format!(
                "public class C extends WebSecurityConfigurerAdapter {{ @Bean WebSecurityCustomizer c() {{ return web -> {{ web.ignoring().requestMatchers(\"/x\"); getHttp().antMatcher(\"/api/**\"); }}; }} {adapter_cfg} }}"
            ),
            format!(
                "public class C extends WebSecurityConfigurerAdapter {{ public void configure(WebSecurity web) {{ web.ignoring().antMatchers(\"/x\"); getHttp().antMatcher(\"/api/**\"); }} {adapter_cfg} }}"
            ),
            format!(
                "public class C extends WebSecurityConfigurerAdapter {{ HttpSecurity h; public void configure(WebSecurity web) {{ web.ignoring().antMatchers(\"/x\"); h.antMatcher(\"/api/**\"); }} {adapter_cfg} }}"
            ),
        ];
        for text in &java {
            let flow = configured(&[("src/main/java/app/C.java", text)]);
            assert_not_evaluated(&flow);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Undeclared,
                "{text}"
            );
        }
        // Braced authorization lambda: only statements on the lambda parameter are rules.
        let rules =
            "a.requestMatchers(\"/api/v1/open/**\").permitAll(); a.anyRequest().authenticated();";
        for x in [
            "otherHttp.securityMatcher(\"/api/**\");",
            "getHttp().antMatcher(\"/api/**\");",
            "http.requestMatchers(\"/api/**\");",
            "a.securityMatcher(\"/api/**\");",
        ] {
            let text = format!(
                "public class C {{ HttpSecurity otherHttp; @Bean SecurityFilterChain api(HttpSecurity http) throws Exception {{ http.authorizeHttpRequests(a -> {{ {rules} {x} }}); return http.build(); }} }}"
            );
            let flow = configured(&[("src/main/java/app/C.java", &text)]);
            assert_not_evaluated(&flow);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Undeclared,
                "{x}"
            );
        }
        let kt = "class C : WebSecurityConfigurerAdapter() {\n    override fun configure(http: HttpSecurity) {\n        http.authorizeRequests({ a -> a.antMatchers(\"/api/v1/open/**\").permitAll(); a.anyRequest().authenticated(); getHttp().antMatcher(\"/api/**\") })\n    }\n}\n";
        assert_not_evaluated(&configured(&[("src/main/java/app/C.kt", kt)]));
        // The lambda form of authorizeHttpRequests returns the HttpSecurity, not the rules.
        let text = "public class C { @Bean SecurityFilterChain api(HttpSecurity http) throws Exception { http.authorizeHttpRequests(a -> a.anyRequest().permitAll()).antMatcher(\"/public/**\"); return http.build(); } }";
        assert_not_evaluated(&configured(&[("src/main/java/app/C.java", text)]));
        let kt = "class C : WebSecurityConfigurerAdapter() {\n    override fun configure(http: HttpSecurity) {\n        http.authorizeRequests { a -> a.anyRequest().permitAll() }.antMatcher(\"/public/**\")\n    }\n}\n";
        assert_not_evaluated(&configured(&[("src/main/java/app/C.kt", kt)]));
        // Kotlin statements separated by newlines only.
        for x in [
            "http.antMatcher(\"/admin/**\")",
            "http.antMatcher(\"/api/**\")",
        ] {
            let kt = format!(
                "class C : WebSecurityConfigurerAdapter() {{\n    override fun configure(http: HttpSecurity) {{\n        http.authorizeRequests({{ a ->\n            a.antMatchers(\"/api/v1/open/**\").permitAll()\n            a.anyRequest().authenticated()\n            {x}\n        }})\n    }}\n}}\n"
            );
            assert_not_evaluated(&configured(&[("src/main/java/app/C.kt", &kt)]));
        }
        // A multi-line rule chain on the lambda parameter stays evaluated.
        let text = "public class C { @Bean SecurityFilterChain api(HttpSecurity http) throws Exception { http.authorizeHttpRequests(a -> a\n  .requestMatchers(\"/api/v1/open/**\").permitAll()\n  .anyRequest().authenticated()); return http.build(); } }";
        let flow = configured(&[("src/main/java/app/C.java", text)]);
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
        // Static factories inside a rule's arguments stay legitimate.
        let text = "public class C { @Bean SecurityFilterChain api(HttpSecurity http) throws Exception { http.authorizeHttpRequests(a -> { a.requestMatchers(antMatcher(\"/api/v1/open/**\")).permitAll(); a.anyRequest().authenticated(); }); return http.build(); } }";
        let flow = configured(&[("src/main/java/app/C.java", text)]);
        assert_eq!(notes(&flow, "not evaluated"), 0, "{:#?}", flow.diagnostics);
        let kt_head = "class C : WebSecurityConfigurerAdapter() {\n";
        let kt_cfg = "    override fun configure(http: HttpSecurity) {\n        http.authorizeRequests().antMatchers(\"/api/v1/open/**\").permitAll().anyRequest().authenticated()\n    }\n}\n";
        let kotlin_bodies = [
            "    fun c() = WebSecurityCustomizer { web -> web.ignoring().requestMatchers(\"/x\"); getHttp().antMatcher(\"/api/**\") }\n",
            "    override fun configure(http: HttpSecurity) {\n        http.apply { csrf().ignoringRequestMatchers(securityMatcher(\"/api/**\").let { it }) }\n    }\n",
            "    override fun configure(http: HttpSecurity) {\n        with(http) { csrf().ignoringRequestMatchers(run { antMatcher(\"/api/**\"); x }) }\n    }\n",
        ];
        for body in kotlin_bodies {
            let text = format!("{kt_head}{body}{kt_cfg}");
            let flow = configured(&[("src/main/java/app/C.kt", &text)]);
            assert_not_evaluated(&flow);
        }
    }

    #[test]
    fn receivers_and_scope_blocks_stay_evaluated() {
        let flow = kotlin(
            "        with(http) { authorizeRequests().antMatchers(\"/api/v1/open/**\").permitAll().anyRequest().authenticated() }",
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
        let flow = kotlin(&format!(
            "        http.apply {{ this.csrf().disable() }}\n        http.let {{ it.csrf() }}\n        {OPEN_CHAIN_KT}"
        ));
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
        assert_eq!(notes(&flow, "not evaluated"), 0, "{:#?}", flow.diagnostics);
        let config = adapter(&format!(
            "http\n            .csrf().disable(); {OPEN_CHAIN_JAVA}"
        ));
        let flow = configured(&[("src/main/java/app/SecurityConfig.java", &config)]);
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
    }

    #[test]
    fn declarations_of_scoping_names_are_not_calls() {
        let kt = "class SecurityConfig : WebSecurityConfigurerAdapter() {\n    fun requestMatcher(): RequestMatcher = TODO()\n    fun Foo.antMatcher(x: String) = 1\n    override fun configure(http: HttpSecurity) {\n        http.authorizeRequests().antMatchers(\"/api/v1/open/**\").permitAll()\n    }\n}\n";
        let flow = configured(&[("src/main/java/app/SecurityConfig.kt", kt)]);
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
        assert_eq!(notes(&flow, "not evaluated"), 0, "{:#?}", flow.diagnostics);
        let java = "public class SecurityConfig extends WebSecurityConfigurerAdapter {\n    private RequestMatcher requestMatcher() { return null; }\n    RequestMatcher[] requestMatchers() throws Exception {\n        return null;\n    }\n    protected void configure(HttpSecurity http) throws Exception {\n        http.authorizeRequests().antMatchers(\"/api/v1/open/**\").permitAll();\n    }\n}\n";
        let flow = configured(&[("src/main/java/app/SecurityConfig.java", java)]);
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
        assert_eq!(notes(&flow, "not evaluated"), 0, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn web_security_matchers_make_the_chain_not_evaluated_csrf_ignoring_names_do_not() {
        let chain = "http.csrf().ignoringAntMatchers(\"/api/v1/x\").and().csrf(c -> c.ignoringRequestMatchers(\"/y\")).authorizeRequests().antMatchers(\"/api/v1/open/**\").permitAll().anyRequest().authenticated();";
        let config = format!(
            "public class SecurityConfig extends WebSecurityConfigurerAdapter {{\n    @Override\n    public void configure(WebSecurity web) throws Exception {{\n        web.ignoring().antMatchers(\"/static/**\");\n    }}\n    protected void configure(HttpSecurity http) throws Exception {{\n        {chain}\n    }}\n    private RequestMatcher requestMatcher() {{ return null; }}\n}}\n"
        );
        let flow = configured(&[("src/main/java/app/SecurityConfig.java", &config)]);
        assert_not_evaluated(&flow);
        let kt = "class SecurityConfig : WebSecurityConfigurerAdapter() {\n    override fun configure(web: WebSecurity) {\n        web.ignoring().antMatchers(\"/static/**\")\n    }\n    override fun configure(http: HttpSecurity) {\n        http.authorizeRequests().antMatchers(\"/api/v1/open/**\").permitAll()\n    }\n}\n";
        let flow = configured(&[("src/main/java/app/SecurityConfig.kt", kt)]);
        assert_not_evaluated(&flow);
    }

    #[test]
    fn any_matcher_call_outside_the_authorization_block_scopes_the_chain() {
        for scope in [
            "http.requestMatchers(m -> m.antMatchers(\"/api/**\")).authorizeRequests(a -> a.anyRequest().authenticated());",
            "http.mvcMatcher(\"/api/**\").authorizeRequests().anyRequest().authenticated();",
            "http.regexMatcher(\"/api/.*\").authorizeRequests().anyRequest().authenticated();",
            "http.requestMatchers().antMatchers(\"/api/**\").and().authorizeRequests().anyRequest().authenticated();",
        ] {
            let config = adapter(scope);
            let flow = configured(&[("src/main/java/app/SecurityConfig.java", &config)]);
            assert_not_evaluated(&flow);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Undeclared,
                "{scope}"
            );
        }
        // Matchers inside the authorization block stay rules; a variable is no call.
        let config = adapter(
            "RequestMatcher requestMatcher = null; http.csrf().disable().authorizeRequests().antMatchers(\"/api/v1/open/**\").permitAll().regexMatchers(\"/x\").denyAll().anyRequest().authenticated().and().httpBasic();",
        );
        let flow = configured(&[("src/main/java/app/SecurityConfig.java", &config)]);
        assert_eq!(
            access(&flow, "GET", "/api/v1/open/ping"),
            &AccessState::Open
        );
        assert_eq!(notes(&flow, "not evaluated"), 0, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn several_adapters_are_not_evaluated() {
        let one = adapter("http.authorizeRequests().anyRequest().authenticated();");
        let two = one.replace("class SecurityConfig", "class OtherConfig");
        let flow = configured(&[
            ("src/main/java/app/SecurityConfig.java", &one),
            ("src/main/java/app/OtherConfig.java", &two),
        ]);
        assert_not_evaluated(&flow);
    }

    #[test]
    fn access_permit_all_with_and_without_parentheses() {
        for call in ["permitAll", "permitAll()"] {
            let config = adapter(&format!(
                "http.authorizeRequests().antMatchers(\"/api/v1/open/**\").access(\"{call}\").anyRequest().authenticated();"
            ));
            let flow = configured(&[("src/main/java/app/SecurityConfig.java", &config)]);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Open,
                "{call}"
            );
        }
    }

    #[test]
    fn kotlin_and_qualified_adapters_are_recognised() {
        let body = "http.authorizeRequests().anyRequest().authenticated()";
        let kotlin = format!(
            "class SecurityConfig : WebSecurityConfigurerAdapter() {{\n    override fun configure(http: HttpSecurity) {{\n        {body}\n    }}\n}}\n"
        );
        let qualified = adapter(body).replace(
            "extends WebSecurityConfigurerAdapter",
            "extends org.springframework.security.config.annotation.web.configuration.WebSecurityConfigurerAdapter",
        );
        for (file, text) in [
            ("SecurityConfig.kt", &kotlin),
            ("SecurityConfig.java", &qualified),
        ] {
            let path = format!("src/main/java/app/{file}");
            let flow = configured(&[(path.as_str(), text)]);
            assert_eq!(
                access(&flow, "GET", "/api/v1/other"),
                &protected(Condition::Authenticated),
                "{file}: {:#?}",
                flow.diagnostics
            );
        }
    }

    #[test]
    fn notes_name_the_source_and_skip_zero_rule_counts() {
        let config = adapter("http.authorizeRequests().anyRequest().authenticated();");
        let flow = configured(&[("src/main/java/app/SecurityConfig.java", &config)]);
        assert_eq!(notes(&flow, "Security configuration rules evaluated: 1"), 1);
        let empty = "public class ResourceServerConfig extends ResourceServerConfigurerAdapter {}";
        let flow = configured(&[("src/main/java/app/ResourceServerConfig.java", empty)]);
        assert_eq!(notes(&flow, "0 rule"), 0, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn an_unreadable_security_class_is_a_gap() {
        let dir = project(&[
            ("pom.xml", "<project/>"),
            (
                "src/main/java/app/ResourceServerConfig.java",
                "@Configuration\npublic class ResourceServerConfig extends ResourceServerConfigurerAdapter {}\n",
            ),
            ("src/main/java/app/TicketController.java", LEGACY_RESOURCES),
        ]);
        let flow = extract(dir.path());
        assert!(!flow.complete);
        assert_eq!(
            (flow.diagnostics.iter())
                .filter(|d| d.severity == DiagnosticSeverity::Gap
                    && d.message.contains("ResourceServerConfig"))
                .count(),
            1,
            "{:#?}",
            flow.diagnostics
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/other"),
            &AccessState::Undeclared
        );
    }

    #[test]
    fn spring_filter_chain_combines_with_annotations() {
        let chain = FILTER_CHAIN.replace(".requestMatchers(props.getOpenPath()).permitAll()\n", "");
        let dir = project(&[
            ("pom.xml", "<project/>"),
            ("src/main/java/app/AuthoritiesConstants.java", AUTHORITIES),
            ("src/main/java/app/SecurityConfiguration.java", &chain),
            ("src/main/java/app/AccountResource.java", RESOURCES),
        ]);
        let flow = extract(dir.path());
        let authenticated = protected(Condition::Authenticated);
        assert_eq!(access(&flow, "GET", "/api/account"), &authenticated);
        assert_eq!(access(&flow, "GET", "/api/authenticate"), &authenticated);
        // `/api/users/me` matches only some values of `{login}`: undecided.
        assert_eq!(
            access(&flow, "GET", "/api/users/{login}"),
            &AccessState::Undeclared
        );
        assert_eq!(notes(&flow, "/api/users/me"), 1, "{:#?}", flow.diagnostics);
        assert_eq!(
            access(&flow, "GET", "/api/authorities"),
            &protected(Condition::All(vec![
                Condition::Authenticated,
                role("ROLE_AUDITOR")
            ]))
        );
        // `@PermitAll` does not lift the chain's sign-in.
        assert_eq!(access(&flow, "POST", "/api/public"), &authenticated);
        assert_eq!(
            access(&flow, "GET", "/api/admin/users"),
            &protected(role("ROLE_ADMIN"))
        );
        assert_eq!(access(&flow, "GET", "/management/info"), &AccessState::Open);
        // `access(custom)` and the rule inside `if` are not read.
        assert_eq!(
            access(&flow, "GET", "/management/env"),
            &AccessState::Undeclared
        );
        assert_eq!(
            access(&flow, "GET", "/h2/console"),
            &AccessState::Undeclared
        );
        // No rule matches: unchanged.
        assert_eq!(access(&flow, "GET", "/other"), &AccessState::Undeclared);
    }

    const MVC_PROGRAM: &str = r#"
var app = builder.Build();
app.MapDefaultControllerRoute();
app.MapControllerRoute(name: "areas", pattern: "{area:exists}/{controller=Home}/{action=Index}/{id?}");
app.MapControllerRoute("dynamic", Routes.Dynamic);
app.Run();
"#;
    const MVC: &str = r#"
public class HomeController : Controller
{
    public HomeController(ILogger logger) { }

    public IActionResult Index() => View();

    [Authorize]
    public IActionResult Privacy() => View();

    [NonAction]
    public string Helper() => "";

    public static string Version() => "1";

    private IActionResult Hidden() => View();
}

[AllowAnonymous]
public class AccountController : Controller
{
    [HttpGet]
    public IActionResult Login(string returnUrl) => View();

    [HttpPost]
    [ValidateAntiForgeryToken]
    public IActionResult Login(LoginInputModel model) => View();

    public async Task<IActionResult> LogoutAsync() => View();
}

[Area("Admin")]
public class UsersController : Controller
{
    public IActionResult Index() => View();
}

[ApiController]
public class ApiOnlyController : Controller
{
    public IActionResult Hidden() => Ok();
}
"#;

    #[test]
    fn conventional_routes_give_endpoints_with_defaults() {
        let dir = project(&[
            ("Identity/Identity.csproj", "<Project />"),
            ("Identity/Program.cs", MVC_PROGRAM),
            ("Identity/Controllers/Controllers.cs", MVC),
        ]);
        let flow = extract(dir.path());
        let paths: Vec<String> = (flow.endpoints.iter())
            .map(|e| format!("{} {}", e.method, e.path))
            .collect();
        assert_eq!(
            paths,
            vec![
                "ANY /",
                "ANY /Account/Logout",
                "ANY /Admin/Users",
                "ANY /Admin/Users/Index",
                "ANY /Home",
                "ANY /Home/Index",
                "ANY /Home/Privacy",
                "GET /Account/Login",
                "POST /Account/Login",
            ]
        );
        assert_eq!(access(&flow, "ANY", "/"), &AccessState::Undeclared);
        assert_eq!(
            access(&flow, "ANY", "/Home/Privacy"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(access(&flow, "POST", "/Account/Login"), &AccessState::Open);
        let login = (flow.endpoints.iter()).find(|e| e.method == "GET").unwrap();
        assert_eq!(login.handler.as_deref(), Some("AccountController.Login"));
        assert_eq!(notes(&flow, "Routes.Dynamic"), 1, "{:#?}", flow.diagnostics);
        assert_eq!(flow.diagnostics.len(), 1, "{:#?}", flow.diagnostics);
    }
}
