//! Micronaut Security extractor for ui-flow/v1 (ADR 0021): controller routes
//! and who may call them, read as text from Java/Kotlin sources and the
//! `application*` configuration; no compiler and no run.
//!
//! - Routes: a class with `@Controller("/p")` (import `io.micronaut.http.annotation`)
//!   and methods with `@Get/@Post/@Put/@Delete/@Patch/@Head/@Options/@Trace`
//!   (`value`, `uri` or `uris`, default `/`); a non-literal path is a gap.
//! - Rules, first decision wins (no AND between them): the annotation of the
//!   method, else of the class (`@Secured`: `IS_ANONYMOUS`/`isAnonymous()` =
//!   open, `IS_AUTHENTICATED`, `DENY_ALL`, role names = any of; `#{…}` = unknown;
//!   JSR-250 `@PermitAll/@DenyAll/@RolesAllowed` only with
//!   `micronaut-security-processor`/`-annotations` in the build file; both on
//!   one element = unknown) -> `micronaut.security.intercept-url-map` (YAML or
//!   properties; a method-matching entry before method-less ones, first match;
//!   a pattern that may match a parameter route is unknown) -> a class
//!   implementing `SecurityRule` = unknown -> default deny (`never` + note).
//! - Without `micronaut-security` in the project's build file, or with
//!   `micronaut.security.enabled: false`, endpoints are `undeclared`. Keys of
//!   `application-<env>` files are conditional: not applied, a gap, and no
//!   default deny is claimed.
//!
//! Not read: annotations inherited from parent classes or interfaces,
//! `ip-patterns` (a gap), `SensitiveEndpointRule`, `RolesFinder`.
use crate::ui_flow_backend::{Attr, Decl, Source, class_header, decls, each_method};
use archguard_core::{
    AccessState, Condition, DiagnosticSeverity, EndpointAccess, UiFlow, UiFlowDiagnostic,
};
use archguard_flow_common::extract_ctx::with_root_config;
use archguard_flow_common::extract_ctx::{
    ExtractContext, ancestors, dir_name, finish, gap, read_text, walk,
};
use archguard_flow_common::lang_csharp::literal;
use archguard_flow_common::support::test_path;
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

const SOURCE: &str = "micronaut";
const MAX_BYTES: u64 = 1024 * 1024;
const BUILD_FILES: [&str; 3] = ["pom.xml", "build.gradle", "build.gradle.kts"];
const VERBS: [(&str, &str); 8] = [
    ("Get", "GET"),
    ("Post", "POST"),
    ("Put", "PUT"),
    ("Delete", "DELETE"),
    ("Patch", "PATCH"),
    ("Head", "HEAD"),
    ("Options", "OPTIONS"),
    ("Trace", "TRACE"),
];
const MAP: &str = "micronaut.security.intercept-url-map";

pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    let Ok(walk) = walk(ctx.root) else {
        return flow;
    };
    let (projects, sources, configs) = classify(&walk.files);
    let project_of = |rel: &str| {
        let dir = ancestors(rel).find(|d| d.is_empty() || projects.contains(*d));
        dir.unwrap_or_default().to_string()
    };
    let mut by_project: BTreeMap<String, Project> = BTreeMap::new();
    for rel in sources {
        match read_text(ctx.root, rel, MAX_BYTES) {
            Ok(text) if text.contains("io.micronaut") => by_project
                .entry(project_of(rel))
                .or_default()
                .sources
                .push(Source::new(rel.clone(), &text)),
            Ok(_) => {}
            Err(e) => gap(
                &mut flow.diagnostics,
                rel,
                0,
                format!("Source file not read: {e}"),
            ),
        }
    }
    for (dir, project) in &mut by_project {
        project.read_build(ctx.root, dir);
    }
    for rel in configs {
        let Some(project) = by_project.get_mut(&project_of(rel)) else {
            continue;
        };
        let text = read_text(ctx.root, rel, MAX_BYTES);
        project.config.read(rel, text, &mut flow.diagnostics);
    }
    for (dir, project) in &by_project {
        let app = projects.contains(dir).then(|| dir_name(ctx.root, dir));
        project.run(app, &mut flow);
    }
    finish(&mut flow);
    flow
}

type Classified<'a> = (BTreeSet<String>, Vec<&'a String>, Vec<&'a String>);

/// Build-file directories, source files and configuration files.
fn classify(files: &[String]) -> Classified<'_> {
    let mut projects = BTreeSet::new();
    let (mut sources, mut configs) = (Vec::new(), Vec::new());
    for rel in files {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        if BUILD_FILES.contains(&name) {
            projects.insert(rel.rsplit_once('/').map_or("", |(d, _)| d).to_string());
        } else if test_path(rel) {
        } else if name.ends_with(".java") || name.ends_with(".kt") {
            sources.push(rel);
        } else if is_config(name) {
            configs.push(rel);
        }
    }
    (projects, sources, configs)
}

fn is_config(name: &str) -> bool {
    let stem = name.strip_prefix("application");
    let ext = [".yml", ".yaml", ".properties"]
        .iter()
        .find_map(|e| stem?.strip_suffix(e));
    ext.is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
}

struct Rule {
    name: String,
    /// Overrides `getOrder`: it may run before the annotation rule.
    ordered: bool,
}

struct Env {
    rule: Option<Rule>,
    types: Types,
    app: Option<String>,
    /// Files that already carry the default-deny note.
    denied: RefCell<BTreeSet<String>>,
}

struct TypeInfo {
    name: String,
    attrs: Vec<Attr>,
    bases: Vec<String>,
    /// Type parameters of the class.
    vars: Vec<String>,
    /// `(name, annotations, parameter types, type parameters in scope)`.
    methods: Vec<(String, Vec<Attr>, Sig, Vec<String>)>,
}

/// Overload matching by parameter types.
mod sig {
    use super::*;

    /// Normalised parameter types of a method; `None` when not readable.
    pub(super) type Sig = Option<Vec<String>>;

    /// The parameter types of the method whose parameter list opens at `open`.
    pub(super) fn sig_of(src: &Source, open: usize) -> Sig {
        let close = src.pair(open)?;
        let text = &src.text[open + 1..close];
        if text.trim().is_empty() {
            return Some(vec![]);
        }
        let (mut parts, mut depth, mut from) = (vec![], 0i32, 0);
        let mut it = text.char_indices();
        while let Some((i, c)) = it.next() {
            match c {
                '(' | '<' | '[' | '{' => depth += 1,
                ')' | '>' | ']' | '}' => depth -= 1,
                '"' | '\'' => skip_quoted(&mut it, c),
                ',' if depth == 0 => {
                    parts.push(&text[from..i]);
                    from = i + 1;
                }
                _ => {}
            }
        }
        parts.push(&text[from..]);
        parts
            .into_iter()
            .map(|p| param_type(p).map(|t| qualify(src, &t)))
            .collect()
    }

    /// Names that need no import in Java or Kotlin (never qualified).
    const IMPLICIT: &[&str] = &[
        "String",
        "Object",
        "Any",
        "Unit",
        "List",
        "Map",
        "Set",
        "Collection",
        "Iterable",
        "Number",
        "CharSequence",
        "Array",
        "Nothing",
        "Throwable",
        "Void",
        "Class",
        "Enum",
        "Comparable",
        "Iterator",
        "Runnable",
    ];

    /// `Dto` -> `x.Dto` by the file's explicit import, else its package; left
    /// plain when a wildcard import could supply it (`same_type` then reports
    /// the package as unknown).
    fn qualify(src: &Source, ty: &str) -> String {
        let base = ty.trim_end_matches("[]");
        let dims = &ty[base.len()..];
        if base.contains('.')
            || !base.starts_with(|c: char| c.is_ascii_uppercase())
            || IMPLICIT.contains(&canon(base).as_str())
            || is_primitive(&canon(base))
        {
            return ty.to_string();
        }
        let (mut package, mut wildcard) = (None, false);
        for line in src.text.lines() {
            let l = line.trim().trim_end_matches(';').trim();
            if let Some(p) = l.strip_prefix("package ") {
                package = Some(p.trim());
            } else if let Some(i) = l.strip_prefix("import ") {
                let i = i.trim();
                if i.ends_with(".*") {
                    wildcard = true;
                } else if i.rsplit('.').next() == Some(base) {
                    return format!("{i}{dims}");
                }
            }
        }
        match package {
            Some(p) if !wildcard => format!("{p}.{base}{dims}"),
            _ => ty.to_string(),
        }
    }

    /// `x.Dto[]` -> `Dto[]`.
    fn unq(t: &str) -> String {
        let base = t.trim_end_matches("[]");
        let dims = &t[base.len()..];
        format!("{}{dims}", base.rsplit('.').next().unwrap_or(base))
    }

    fn is_primitive(w: &str) -> bool {
        [
            "int", "long", "short", "byte", "char", "boolean", "float", "double",
        ]
        .contains(&w.trim_end_matches("[]"))
    }

    /// `final @A String name`, `name: String = x` -> `String`; generics and
    /// package prefixes are dropped (erased comparison).
    pub(super) fn param_type(p: &str) -> Option<String> {
        let p = strip_annotations(p.trim())?;
        let (ty, vararg) = if let Some((name, t)) = p.split_once(':') {
            let vararg = name.split_whitespace().any(|w| w == "vararg");
            (t.split('=').next().unwrap_or(t).to_string(), vararg)
        } else {
            let mut words: Vec<&str> = p.split_whitespace().collect();
            // `String ids[]`: the brackets belong to the type.
            let dims = words.pop()?.matches("[]").count();
            (format!("{}{}", words.join(" "), "[]".repeat(dims)), false)
        };
        let ty = erase(ty.trim().trim_end_matches('?'))?;
        Some(if vararg { ty + "[]" } else { ty })
    }

    type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

    /// `p` without `@A` / `@A(…)`; parentheses inside string and char literals
    /// do not count. `None` when an argument list is not closed.
    fn strip_annotations(p: &str) -> Option<String> {
        let (mut out, mut it) = (String::new(), p.chars().peekable());
        while let Some(c) = it.next() {
            out.push(if c == '@' { ' ' } else { c });
            if c != '@' {
                continue;
            }
            while it
                .next_if(|c| c.is_alphanumeric() || matches!(c, '_' | '.'))
                .is_some()
            {}
            while it.next_if(|c| c.is_whitespace()).is_some() {}
            if it.next_if_eq(&'(').is_some() {
                skip_arguments(&mut it)?;
            }
        }
        Some(out)
    }

    /// Consumes up to the `)` that closes an already opened argument list.
    fn skip_arguments(it: &mut Chars) -> Option<()> {
        let mut depth = 1;
        while depth > 0 {
            match it.next()? {
                '(' => depth += 1,
                ')' => depth -= 1,
                q @ ('"' | '\'') => skip_literal(it, q),
                _ => {}
            }
        }
        Some(())
    }

    /// Consumes the rest of a string or char literal opened with `quote`.
    fn skip_literal(it: &mut Chars, quote: char) {
        while let Some(c) = it.next() {
            if c == '\\' {
                it.next();
            } else if c == quote || c == '\n' {
                break;
            }
        }
    }

    /// `Array<out String>` -> `String[]`; other generics and packages dropped.
    fn erase(ty: &str) -> Option<String> {
        let ty = ty.trim().trim_end_matches('?');
        if let Some(inner) = ty.strip_prefix("Array<").and_then(|r| r.strip_suffix('>')) {
            let inner = inner
                .trim()
                .trim_start_matches("out ")
                .trim_start_matches("in ");
            return Some(erase(inner)? + "[]");
        }
        let (mut out, mut depth) = (String::new(), 0);
        for c in ty.chars() {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                _ if depth == 0 => out.push(c),
                _ => {}
            }
        }
        let out = out.replace("...", "[]").replace('?', "");
        let words: Vec<&str> = (out.split_whitespace())
            .filter(|w| !matches!(*w, "final" | "vararg"))
            .collect();
        let ty = words.join("");
        (!ty.is_empty() && !ty.ends_with('.')).then_some(ty)
    }

    #[derive(PartialEq)]
    pub(super) enum Fit {
        Yes,
        No,
        Maybe,
    }

    /// Java and Kotlin spell some types differently (`int`/`Int`, `Object`/`Any`).
    fn canon(t: &str) -> String {
        let (word, dims) = (t.trim_end_matches("[]"), t.matches("[]").count());
        let (word, extra) = match word.strip_suffix("Array").filter(|w| !w.is_empty()) {
            Some(w) => (w, 1),
            None => (word, 0),
        };
        let word = word
            .strip_prefix("Mutable")
            .filter(|w| !w.is_empty())
            .unwrap_or(word);
        let word = match word.to_ascii_lowercase().as_str() {
            "integer" => "int".to_string(),
            "character" => "char".to_string(),
            "any" => "object".to_string(),
            "unit" => "void".to_string(),
            w if [
                "int", "long", "short", "byte", "char", "boolean", "float", "double", "object",
                "void",
            ]
            .contains(&w) =>
            {
                w.to_string()
            }
            _ => word.to_string(),
        };
        format!("{word}{}", "[]".repeat(dims + extra))
    }

    /// Whether two types that match by name are surely the same: package
    /// known and equal, and not a boxed type against its primitive.
    fn same_type(x: &str, y: &str) -> bool {
        let (ux, uy) = (unq(x), unq(y));
        let other_package = x.len() > ux.len() && y.len() > uy.len() && x != y;
        // a capitalised name with no known package may come from a wildcard import
        let unknown = |t: &str| {
            let base = unq(t);
            let base = base.trim_end_matches("[]");
            !t.contains('.')
                && base.starts_with(|c: char| c.is_ascii_uppercase())
                && !IMPLICIT.contains(&canon(base).as_str())
                && !is_primitive(&canon(base))
        };
        !(unknown(x) || unknown(y) || other_package) && (ux == uy || !is_primitive(&canon(&ux)))
    }

    fn is_plain(t: &str) -> bool {
        (unq(t).chars()).all(|c| c.is_alphanumeric() || matches!(c, '_' | '[' | ']'))
    }

    /// Whether two parameter lists are the same overload. A type variable
    /// (`T`) or an unreadable list may stand for anything: `Maybe`.
    pub(super) fn fits(a: &Sig, b: &Sig, overriding: bool, vars: &[String]) -> Fit {
        let (Some(a), Some(b)) = (a, b) else {
            return Fit::Maybe;
        };
        if a.len() != b.len() {
            // An override that does not match by count was probably misread.
            return if overriding { Fit::Maybe } else { Fit::No };
        }
        let var = |t: &str| {
            let word = t.trim_end_matches("[]").rsplit('.').next().unwrap_or(t);
            (word.len() <= 2 && word.starts_with(|c: char| c.is_ascii_uppercase()))
                || vars.iter().any(|v| v == word)
        };
        let mut maybe = false;
        for (x, y) in a.iter().zip(b) {
            if canon(&unq(x)) == canon(&unq(y)) {
                maybe |= !same_type(x, y);
                continue;
            }
            // A definite difference only between plain type names, and never
            // for a method marked as an override (an alias or an unreadable
            // spelling may hide the match).
            if !(var(x) || var(y)) && is_plain(x) && is_plain(y) && !overriding {
                return Fit::No;
            }
            maybe = true;
        }
        if maybe { Fit::Maybe } else { Fit::Yes }
    }

    /// Consumes the rest of a literal opened with `quote` (it ends at a line end).
    fn skip_quoted(it: &mut std::str::CharIndices, quote: char) {
        while let Some((_, c)) = it.next() {
            if c == '\\' {
                it.next();
            } else if c == quote || c == '\n' {
                break;
            }
        }
    }

    /// The declaration head before the parameter list at `open`: modifiers,
    /// annotations, type parameters, return type and name. It starts after
    /// the last `;`, `{` or `}` outside parentheses and literals.
    pub(crate) fn head_of(src: &Source, open: usize) -> &str {
        let text = &src.text[..open];
        let (mut from, mut depth, mut it) = (0, 0u32, text.char_indices());
        while let Some((i, c)) = it.next() {
            match c {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                '"' | '\'' => skip_quoted(&mut it, c),
                ';' | '{' | '}' if depth == 0 => from = i + 1,
                _ => {}
            }
        }
        &text[from..]
    }

    /// Whether the method whose parameter list opens at `open` is marked as an
    /// override: Kotlin `override`, Java `@Override`. Errs towards `true`.
    pub(super) fn overrides(src: &Source, open: usize) -> bool {
        let head = head_of(src, open);
        head.contains("@Override")
            || head
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|w| w == "override")
    }

    /// The names declared by the `<…>` group at the start of `text`
    /// (`<T, K extends X>`, `<out T>`).
    fn group_params(text: &str) -> Vec<String> {
        let (mut out, mut depth, mut from) = (vec![], 0, 1);
        for (i, c) in text.char_indices() {
            let end = c == '>' && depth == 1;
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                _ => {}
            }
            if (c == ',' && depth == 1) || end {
                let part = text.get(from..i).unwrap_or("");
                let skip = ["out", "in", "reified"];
                out.extend(
                    part.split_whitespace()
                        .find(|w| !skip.contains(w))
                        .map(str::to_string),
                );
                from = i + 1;
            }
            if depth == 0 {
                break;
            }
        }
        out
    }

    /// Type parameters of the class `name` declared by `header`.
    pub(super) fn class_vars(header: &str, name: &str) -> Vec<String> {
        for kw in ["class ", "interface ", "record ", "struct ", "object "] {
            let Some(i) = header.find(kw) else { continue };
            let rest = header[i + kw.len()..].trim_start();
            if let Some(after) = rest.strip_prefix(name).filter(|a| a.starts_with('<')) {
                return group_params(after);
            }
        }
        vec![]
    }

    /// Type parameters of the class and of the method at `open`
    /// (`<T> String f(T a)`, `fun <T> f(a: T)`).
    pub(super) fn vars_of(src: &Source, open: usize, class: &[String]) -> Vec<String> {
        const MODS: [&str; 19] = [
            "public",
            "protected",
            "private",
            "static",
            "abstract",
            "default",
            "final",
            "synchronized",
            "fun",
            "override",
            "open",
            "suspend",
            "inline",
            "operator",
            "infix",
            "internal",
            "external",
            "native",
            "strictfp",
        ];
        let mut out = class.to_vec();
        let head = strip_annotations(head_of(src, open)).unwrap_or_default();
        if let Some(lt) = head.find('<')
            && head[..lt].split_whitespace().all(|w| MODS.contains(&w))
        {
            out.extend(group_params(&head[lt..]));
        }
        out
    }

    /// The methods named `name` declared in the body of `class`: `(offset of the
    /// parameter list, parameter types)`. Annotation-less ones are not `Decl`s.
    pub(super) fn declared(src: &Source, class: &Decl, name: &str) -> Vec<(usize, Sig, bool)> {
        let kotlin = src.rel.ends_with(".kt");
        let mut out = vec![];
        for (at, _) in src.text[class.open..class.end.min(src.text.len())].match_indices(name) {
            let start = class.open + at;
            let (before, after) = (&src.text[..start], &src.text[start + name.len()..]);
            let ident = |c: char| c.is_alphanumeric() || c == '_';
            if before.ends_with(ident) || after.starts_with(ident) {
                continue;
            }
            let open = start + name.len() + (after.len() - after.trim_start().len());
            if !src.text[open..].starts_with('(') {
                continue;
            }
            let tail = src
                .pair(open)
                .map_or("", |c| src.text[c + 1..].trim_start());
            let head = before.trim_end();
            let is_decl = if kotlin {
                // `fun name(` or `fun <T> name(`
                head.rfind("fun").is_some_and(|i| {
                    let rest = head[i + 3..].trim();
                    !head[..i].ends_with(ident)
                        && (rest.is_empty() || rest.starts_with('<') && rest.ends_with('>'))
                })
            } else {
                !head.ends_with('.')
                    && (tail.starts_with('{') || tail.starts_with("throws"))
                    && !head.ends_with("new")
            };
            if is_decl {
                out.push((open, sig_of(src, open), overrides(src, open)));
            }
        }
        out
    }
}
pub(crate) use sig::head_of;

/// The declaration head before `name(` in the method starting at `line` (the
/// call model may give the first annotation's line): annotations and modifiers
/// up to the previous `;`, `{` or `}`, on however many lines.
pub(crate) fn head_at(src: &Source, line: u32, name: &str) -> Option<String> {
    let from: usize = (src.text.split_inclusive('\n'))
        .take((line as usize).checked_sub(1)?)
        .map(str::len)
        .sum();
    let at = from + src.text[from..].find(&format!("{name}("))?;
    Some(head_of(src, at + name.len()).to_string())
}
use sig::{Fit, Sig, class_vars, declared, fits, overrides, sig_of, vars_of};

/// The annotated types and methods of a project by simple name.
struct Types(Vec<TypeInfo>);

/// Supertypes that carry no security annotation.
const PLAIN: [&str; 6] = [
    "Object",
    "Any",
    "Serializable",
    "Cloneable",
    "Comparable",
    "AutoCloseable",
];

impl Types {
    fn new(sources: &[Source]) -> Self {
        let (mut all, mut open): (Vec<TypeInfo>, Vec<(usize, usize)>) = (vec![], vec![]);
        for src in sources {
            for d in decls(src, true) {
                while open.last().is_some_and(|o| o.0 < d.open) {
                    open.pop();
                }
                if d.class {
                    open.push((d.end, all.len()));
                    let header = class_header(src, &d);
                    all.push(TypeInfo {
                        vars: class_vars(header, &d.name),
                        name: d.name,
                        attrs: d.attrs,
                        bases: d.bases,
                        methods: vec![],
                    });
                } else if let Some(o) = open.last() {
                    let vars = vars_of(src, d.open, &all[o.1].vars);
                    let sig = sig_of(src, d.open);
                    all[o.1].methods.push((d.name, d.attrs, sig, vars));
                }
            }
        }
        Self(all)
    }

    /// The supertypes of `bases` (transitively), and whether one is not
    /// found exactly once in the scan.
    fn supers(&self, bases: &[String]) -> (Vec<&TypeInfo>, Vec<String>) {
        let (mut out, mut unresolved) = (Vec::new(), Vec::new());
        let mut todo: Vec<&String> = bases.iter().rev().collect();
        let mut seen = BTreeSet::new();
        while let Some(name) = todo.pop() {
            if PLAIN.contains(&name.as_str()) || !seen.insert(name) {
                continue;
            }
            let found: Vec<&TypeInfo> = self.0.iter().filter(|t| &t.name == name).collect();
            match found.as_slice() {
                [t] => {
                    out.push(*t);
                    todo.extend(t.bases.iter().rev());
                }
                _ => unresolved.push(name.clone()),
            }
        }
        (out, unresolved)
    }
}

struct Site<'a> {
    src: &'a Source,
    class: &'a Decl,
}

#[derive(Default)]
struct Project {
    sources: Vec<Source>,
    dependency: bool,
    processor: bool,
    config: Config,
}

type Decision = Result<AccessState, String>;

impl Project {
    fn read_build(&mut self, root: &Path, dir: &str) {
        let build: String = (BUILD_FILES.iter())
            .map(|name| {
                let rel = if dir.is_empty() {
                    (*name).into()
                } else {
                    format!("{dir}/{name}")
                };
                read_text(root, &rel, MAX_BYTES).unwrap_or_default()
            })
            .collect();
        self.dependency = build.contains("micronaut-security");
        self.processor = build.contains("micronaut-security-processor")
            || build.contains("micronaut-security-annotations");
    }

    fn run(&self, app: Option<String>, flow: &mut UiFlow) {
        let rule = self.sources.iter().find_map(|s| {
            let class = decls(s, true)
                .into_iter()
                .find(|d| d.class && d.bases.iter().any(|b| b.starts_with("SecurityRule")))?;
            Some(Rule {
                name: class.name,
                ordered: s.text.contains("getOrder"),
            })
        });
        let env = Env {
            rule,
            types: Types::new(&self.sources),
            app,
            denied: RefCell::default(),
        };
        for src in &self.sources {
            if !src.text.contains("io.micronaut.http.annotation") {
                continue;
            }
            let mut own: HashMap<usize, Vec<Decl>> = HashMap::new();
            each_method(src, true, |class, method| {
                own.entry(class.open).or_default().push(method.clone());
            });
            for class in decls(src, true).iter().filter(|d| d.class) {
                if !class.attrs.iter().any(|a| a.name == "Controller") {
                    continue;
                }
                let methods = own.remove(&class.open).unwrap_or_default();
                let at = Site { src, class };
                for m in &methods {
                    let sig = sig_of(src, m.open);
                    let ov = overrides(src, m.open);
                    self.routes(&at, (&m.name, &m.attrs, &m.attrs, &sig, ov), &env, flow);
                }
                self.inherited_routes(&at, &methods, &env, flow);
            }
        }
    }

    /// Routes a controller inherits from the verb annotations of the methods
    /// it overrides; a supertype that cannot be read may hold more.
    fn inherited_routes(&self, at: &Site, methods: &[Decl], env: &Env, flow: &mut UiFlow) {
        let (supers, unresolved) = env.types.supers(&at.class.bases);
        let line = at.src.line(at.class.attrs.first().map_or(0, |a| a.at));
        for name in unresolved {
            let message = format!(
                "A route may be inherited from supertype {name}, which is not in the scan."
            );
            gap(&mut flow.diagnostics, &at.src.rel, line, message);
        }
        let verbs = |attrs: &[Attr]| attrs.iter().any(|a| VERBS.iter().any(|v| v.0 == a.name));
        let mut seen: BTreeSet<(&str, Sig)> = BTreeSet::new();
        for t in supers {
            for (name, attrs, sig, vars) in t.methods.iter().filter(|m| verbs(&m.1)) {
                if !seen.insert((name, sig.clone())) {
                    continue;
                }
                // Which method of the controller overrides it (by parameters)?
                let found = declared(at.src, at.class, name);
                let fit: Vec<_> = found.iter().map(|f| fits(&f.1, sig, f.2, vars)).collect();
                let chosen = if found.is_empty() || fit.iter().all(|f| *f == Fit::No) {
                    None
                } else if fit.iter().filter(|f| **f == Fit::Yes).count() == 1
                    && !fit.contains(&Fit::Maybe)
                {
                    fit.iter().position(|f| *f == Fit::Yes).map(|i| &found[i])
                } else {
                    let message = format!(
                        "Route {name} of supertype {} is not inherited: the controller declares \
                         {name} and which one overrides it cannot be told from the parameters.",
                        t.name
                    );
                    gap(&mut flow.diagnostics, &at.src.rel, line, message);
                    continue;
                };
                let overriding = chosen.and_then(|c| methods.iter().find(|m| m.open == c.0));
                if overriding.is_some_and(|m| verbs(&m.attrs)) {
                    continue;
                }
                let own: &[Attr] = overriding.map_or(&[], |m| &m.attrs);
                let route_sig = chosen.map_or(sig, |c| &c.1);
                let ov = chosen.is_some_and(|c| c.2);
                self.routes(at, (name, own, attrs, route_sig, ov), env, flow);
            }
        }
    }

    /// The endpoints of one controller method `(name, own annotations,
    /// annotations that carry the verb)`.
    fn routes(
        &self,
        at: &Site,
        (name, own, route_attrs, sig, ov): (&str, &[Attr], &[Attr], &Sig, bool),
        env: &Env,
        flow: &mut UiFlow,
    ) {
        let Site { src, class } = at;
        let Some(ctl) = class.attrs.iter().find(|a| a.name == "Controller") else {
            return;
        };
        let base = args(&ctl.args).first().map(|(_, v)| v.clone());
        let Some(base) = base.map_or(Some(vec!["/".into()]), |v| strings(&v)) else {
            let line = src.line(ctl.at);
            gap(
                &mut flow.diagnostics,
                &src.rel,
                line,
                "@Controller path is not a literal; its routes are not read.",
            );
            return;
        };
        for a in route_attrs {
            let Some((_, verb)) = VERBS.iter().find(|v| v.0 == a.name) else {
                continue;
            };
            let line = if std::ptr::eq(own, route_attrs) {
                src.line(a.at)
            } else {
                src.line(ctl.at)
            };
            let Some(paths) = route_paths(a) else {
                gap(
                    &mut flow.diagnostics,
                    &src.rel,
                    line,
                    "Route path is not a literal; the route is not read.",
                );
                continue;
            };
            for path in base
                .iter()
                .flat_map(|b| paths.iter().map(move |p| join(b, p)))
            {
                let access = match self.decide(class, (name, own, sig, ov), verb, &path, env) {
                    Ok(a) => a,
                    Err(why) => {
                        gap(
                            &mut flow.diagnostics,
                            &src.rel,
                            line,
                            format!("Access of {verb} {path} not read: {why}"),
                        );
                        AccessState::Protected(Condition::Unknown(why))
                    }
                };
                if access == AccessState::Protected(Condition::Never)
                    && env.denied.borrow_mut().insert(src.rel.clone())
                {
                    flow.diagnostics.push(UiFlowDiagnostic {
                        file: src.rel.clone(),
                        line,
                        message: "Micronaut default deny: no annotation or intercept-url-map entry matches, so the request is rejected.".into(),
                        severity: DiagnosticSeverity::Note,
                    });
                }
                flow.endpoints.push(EndpointAccess {
                    method: (*verb).into(),
                    path,
                    access,
                    application: env.app.clone(),
                    file: src.rel.clone(),
                    line,
                    source: SOURCE.into(),
                    handler: Some(format!("{}.{}", class.name, name)),
                    sends: vec![],
                });
            }
        }
    }

    /// `Ok` is the access; `Err` the reason it cannot be read (unknown + gap).
    fn decide(
        &self,
        class: &Decl,
        method: (&str, &[Attr], &Sig, bool),
        verb: &str,
        path: &str,
        env: &Env,
    ) -> Decision {
        if !self.dependency || self.config.enabled == Some(false) {
            return Ok(AccessState::Undeclared);
        }
        if let Some(why) = &self.config.unreadable {
            return Err(why.clone());
        }
        if let Some(r) = env.rule.as_ref().filter(|r| r.ordered) {
            return Err(format!(
                "custom SecurityRule {} overrides getOrder and may run before the annotation rule",
                r.name
            ));
        }
        if let Some(a) = self.inherited(class, method, &env.types) {
            return a;
        }
        let ordered = self
            .config
            .entries
            .iter()
            .filter(|e| e.method.as_deref() == Some(verb))
            .chain(self.config.entries.iter().filter(|e| e.method.is_none()));
        for e in ordered {
            let Some(pattern) = &e.pattern else {
                return Err(format!("an {MAP} entry has no readable pattern"));
            };
            match ant(&segments(pattern), &segments(path)) {
                Tri::No => {}
                Tri::Maybe => {
                    return Err(format!(
                        "pattern {pattern} may match the parameter route {path}"
                    ));
                }
                Tri::Yes => {
                    return e
                        .access
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|p| combine(p));
                }
            }
        }
        if let Some(rule) = &env.rule {
            return Err(format!("custom SecurityRule {} may decide", rule.name));
        }
        Ok(if self.config.conditional {
            AccessState::Undeclared
        } else {
            AccessState::Protected(Condition::Never)
        })
    }

    /// The annotation decision of a route method: its own, then those of the
    /// methods it overrides, then the class's, then its supertypes'. A
    /// supertype outside the scan may carry any of them: unknown.
    fn inherited(
        &self,
        class: &Decl,
        method: (&str, &[Attr], &Sig, bool),
        types: &Types,
    ) -> Option<Decision> {
        if let Some(a) = self.annotation(method.1) {
            return Some(a);
        }
        let (supers, unresolved) = types.supers(&class.bases);
        if !unresolved.is_empty() {
            return Some(Err(
                "a supertype is not in the scan; its annotations are unread".into(),
            ));
        }
        for t in &supers {
            let same: Vec<_> = (t.methods.iter())
                .filter(|m| m.0 == method.0)
                .map(|m| (m, fits(&m.2, method.2, method.3, &m.3)))
                .filter(|m| m.1 != Fit::No)
                .collect();
            if same.len() > 1 || same.iter().any(|m| m.1 == Fit::Maybe) {
                return Some(Err(format!("{} is overloaded in {}", method.0, t.name)));
            }
            if let Some(a) = same.first().and_then(|m| self.annotation(&m.0.1)) {
                return Some(a);
            }
        }
        (self.annotation(&class.attrs).into_iter())
            .chain(supers.iter().filter_map(|t| self.annotation(&t.attrs)))
            .next()
    }

    /// The annotation decision of one element: none, or its access / why unreadable.
    fn annotation(&self, attrs: &[Attr]) -> Option<Decision> {
        let secured: Vec<&Attr> = attrs.iter().filter(|a| a.name == "Secured").collect();
        let jsr: Vec<&Attr> = (attrs.iter())
            .filter(|a| ["PermitAll", "DenyAll", "RolesAllowed"].contains(&a.name.as_str()))
            .collect();
        match (secured.as_slice(), jsr.as_slice()) {
            ([], []) => None,
            ([s], []) => Some(perms(&s.args).and_then(|p| combine(&p))),
            ([], [j]) if !self.processor => Some(Err(format!(
                "@{} is not enforced without micronaut-security-processor",
                j.name
            ))),
            ([], [j]) => Some(match j.name.as_str() {
                "PermitAll" => Ok(AccessState::Open),
                "DenyAll" => Ok(AccessState::Protected(Condition::Never)),
                _ => perms(&j.args).and_then(|p| combine(&p)),
            }),
            _ => Some(Err("several security annotations on one element".into())),
        }
    }
}

/// One access token of an annotation or intercept-url-map entry.
enum Perm {
    Anonymous,
    Authenticated,
    Deny,
    Role(String),
    Unreadable(String),
}

fn token(text: &str) -> Perm {
    match text.trim() {
        "isAnonymous()" | "IS_ANONYMOUS" => Perm::Anonymous,
        "isAuthenticated()" | "IS_AUTHENTICATED" => Perm::Authenticated,
        "denyAll()" | "DENY_ALL" => Perm::Deny,
        t if t.is_empty() || t.starts_with("#{") => Perm::Unreadable(t.into()),
        t => Perm::Role(t.into()),
    }
}

/// The tokens of a `@Secured`/`@RolesAllowed` argument: literals, the
/// `SecurityRule` constants; anything else is unreadable.
fn perms(args_text: &str) -> Result<Vec<Perm>, String> {
    let value = args(args_text)
        .into_iter()
        .find(|(k, _)| k.as_deref().is_none_or(|k| k == "value"));
    let Some((_, value)) = value else {
        return Err("annotation without a value".into());
    };
    Ok(list(&value)
        .iter()
        .map(|p| match literal(p) {
            Some(s) => token(&s),
            None => match p.trim().rsplit('.').next() {
                Some(c @ ("IS_ANONYMOUS" | "IS_AUTHENTICATED" | "DENY_ALL"))
                    if p.trim()
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '.') =>
                {
                    token(c)
                }
                _ => Perm::Unreadable(p.trim().into()),
            },
        })
        .collect())
}

/// Any-of over the tokens. Anything unreadable makes all of it unknown, also
/// next to an open marker.
fn combine(perms: &[Perm]) -> Decision {
    let mut items = Vec::new();
    let mut open = false;
    for p in perms {
        match p {
            Perm::Unreadable(t) => return Err(format!("'{t}' is not a role, constant or marker")),
            Perm::Anonymous => open = true,
            Perm::Authenticated => items.push(Condition::Authenticated),
            Perm::Deny => items.push(Condition::Never),
            Perm::Role(r) => items.push(Condition::Role(r.clone())),
        }
    }
    match (open, items.len()) {
        (true, _) => Ok(AccessState::Open),
        (_, 0) => Err("no access value".into()),
        (_, 1) => Ok(AccessState::Protected(items.remove(0))),
        _ => Ok(AccessState::Protected(Condition::Any(items))),
    }
}

/// Top-level `key = value` / positional parts of an annotation argument text.
fn args(text: &str) -> Vec<(Option<String>, String)> {
    split_top(text)
        .into_iter()
        .map(|part| {
            let named = part.split_once('=').filter(|(k, v)| {
                !v.starts_with('=')
                    && !k.is_empty()
                    && k.trim().chars().all(|c| c.is_alphanumeric() || c == '_')
            });
            match named {
                Some((k, v)) => (Some(k.trim().to_string()), v.trim().to_string()),
                None => (None, part.trim().to_string()),
            }
        })
        .collect()
}

/// The parts of `{a, b}`, `[a, b]`, `arrayOf(a, b)` or the one value.
fn list(value: &str) -> Vec<String> {
    let v = value.trim();
    let inner = [("{", "}"), ("[", "]"), ("arrayOf(", ")")]
        .iter()
        .find_map(|(o, c)| v.strip_prefix(o)?.strip_suffix(c));
    split_top(inner.unwrap_or(v))
        .into_iter()
        .map(|s| s.trim().to_string())
        .collect()
}

fn strings(value: &str) -> Option<Vec<String>> {
    list(value).iter().map(|p| literal(p)).collect()
}

fn split_top(text: &str) -> Vec<&str> {
    let (mut out, mut depth, mut quote, mut start) = (Vec::new(), 0i32, false, 0);
    let mut prev = ' ';
    for (i, c) in text.char_indices() {
        match c {
            '"' if prev != '\\' => quote = !quote,
            '(' | '{' | '[' if !quote => depth += 1,
            ')' | '}' | ']' if !quote => depth -= 1,
            ',' if !quote && depth == 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        prev = c;
    }
    if !text[start..].trim().is_empty() {
        out.push(&text[start..]);
    }
    out
}

/// The literal paths of a verb annotation; none when one is not a literal.
fn route_paths(a: &Attr) -> Option<Vec<String>> {
    let given = args(&a.args)
        .into_iter()
        .find(|(k, _)| matches!(k.as_deref(), None | Some("value" | "uri" | "uris")));
    given.map_or(Some(vec!["/".into()]), |(_, v)| strings(&v))
}

fn join(base: &str, path: &str) -> String {
    let joined = format!("/{base}/{path}");
    let parts: Vec<&str> = joined.split('/').filter(|s| !s.is_empty()).collect();
    format!("/{}", parts.join("/"))
}

fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tri {
    No,
    Maybe,
    Yes,
}

/// Ant pattern against a route path; a `{param}` segment may be anything, so a
/// literal pattern segment there is `Maybe`.
fn ant(pat: &[&str], path: &[&str]) -> Tri {
    match (pat.first(), path.first()) {
        (None, None) => Tri::Yes,
        (None, Some(_)) => Tri::No,
        (Some(&"**"), _) => (0..=path.len())
            .map(|k| ant(&pat[1..], &path[k..]))
            .max()
            .unwrap_or(Tri::No),
        (Some(_), None) => Tri::No,
        (Some(p), Some(s)) => {
            let seg = if s.starts_with('{') {
                if *p == "*" { Tri::Yes } else { Tri::Maybe }
            } else if glob(p.as_bytes(), s.as_bytes()) {
                Tri::Yes
            } else {
                Tri::No
            };
            seg.min(ant(&pat[1..], &path[1..]))
        }
    }
}

fn glob(p: &[u8], s: &[u8]) -> bool {
    match (p.first(), s.first()) {
        (None, None) => true,
        (Some(b'*'), _) => glob(&p[1..], s) || (!s.is_empty() && glob(p, &s[1..])),
        (Some(b'?'), Some(_)) => glob(&p[1..], &s[1..]),
        (Some(a), Some(b)) if a == b => glob(&p[1..], &s[1..]),
        _ => false,
    }
}

#[derive(Default)]
struct Config {
    enabled: Option<bool>,
    entries: Vec<Entry>,
    /// A key of an environment file, or a file not read: later rules unproven.
    conditional: bool,
    /// Why the base configuration cannot be used at all.
    unreadable: Option<String>,
}

struct Entry {
    pattern: Option<String>,
    method: Option<String>,
    access: Result<Vec<Perm>, String>,
}

impl Config {
    fn read(&mut self, rel: &str, text: std::io::Result<String>, out: &mut Vec<UiFlowDiagnostic>) {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let env = !name.starts_with("application.");
        let flat = text
            .map_err(|e| e.to_string())
            .and_then(|t| flatten(name, &t));
        let flat = match flat {
            Ok(flat) => flat,
            Err(e) => {
                gap(
                    out,
                    rel,
                    0,
                    format!("Configuration not read ({e}); the security rules cannot be applied."),
                );
                self.conditional = true;
                self.unreadable = Some(format!("{rel} not readable"));
                return;
            }
        };
        let sec = |k: &&String| k.starts_with("micronaut.security.");
        let relevant = flat.keys().filter(sec).any(|k| {
            k.starts_with(MAP)
                || k == "micronaut.security.enabled"
                || k.starts_with("micronaut.security.ip-patterns")
        });
        if env && relevant {
            gap(
                out,
                rel,
                0,
                "Security keys in an environment file are conditional; they are not applied.",
            );
            self.conditional = true;
        }
        if env {
            return;
        }
        if flat
            .keys()
            .any(|k| k.starts_with("micronaut.security.ip-patterns"))
        {
            gap(
                out,
                rel,
                0,
                "micronaut.security.ip-patterns restrict callers; not evaluated.",
            );
            self.conditional = true;
        }
        match flat.get("micronaut.security.enabled").map(|v| v.trim()) {
            None | Some("true") => {}
            Some("false") => self.enabled = Some(false),
            Some(v) => self.unreadable = Some(format!("micronaut.security.enabled is '{v}'")),
        }
        self.read_entries(rel, &flat, out);
    }

    fn read_entries(
        &mut self,
        rel: &str,
        flat: &BTreeMap<String, String>,
        out: &mut Vec<UiFlowDiagnostic>,
    ) {
        for i in 0.. {
            let base = format!("{MAP}[{i}]");
            if !flat.keys().any(|k| k.starts_with(&base)) {
                break;
            }
            let access = access_of(flat, &format!("{base}.access"));
            if access.is_err() || !flat.contains_key(&format!("{base}.pattern")) {
                gap(
                    out,
                    rel,
                    0,
                    format!("{base} has no readable pattern or access."),
                );
            }
            self.entries.push(Entry {
                pattern: flat.get(&format!("{base}.pattern")).cloned(),
                method: (flat.get(&format!("{base}.http-method"))).map(|m| m.trim().to_uppercase()),
                access,
            });
        }
    }
}

fn access_of(flat: &BTreeMap<String, String>, key: &str) -> Result<Vec<Perm>, String> {
    let mut values: Vec<&str> = (0..)
        .map_while(|j| flat.get(&format!("{key}[{j}]")))
        .map(String::as_str)
        .collect();
    if let Some(v) = flat.get(key) {
        values.extend(v.split(','));
    }
    if values.is_empty() {
        return Err("no access".into());
    }
    Ok(values.into_iter().map(token).collect())
}

/// `a.b[0].c` keys with scalar values, from YAML or `.properties` text.
fn flatten(name: &str, text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    if name.ends_with(".properties") {
        for l in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with(['#', '!']))
        {
            if let Some((k, v)) = l.split_once(['=', ':']) {
                out.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        return Ok(out);
    }
    fn walk(prefix: &str, v: &serde_yaml_ng::Value, out: &mut BTreeMap<String, String>) {
        use serde_yaml_ng::Value as V;
        match v {
            V::Mapping(m) => {
                for (k, v) in m {
                    let k = k
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("{k:?}"));
                    walk(
                        &if prefix.is_empty() {
                            k
                        } else {
                            format!("{prefix}.{k}")
                        },
                        v,
                        out,
                    );
                }
            }
            V::Sequence(s) => s
                .iter()
                .enumerate()
                .for_each(|(i, v)| walk(&format!("{prefix}[{i}]"), v, out)),
            V::String(s) => drop(out.insert(prefix.into(), s.clone())),
            V::Bool(b) => drop(out.insert(prefix.into(), b.to_string())),
            V::Number(n) => drop(out.insert(prefix.into(), n.to_string())),
            _ => drop(out.insert(prefix.into(), String::new())),
        }
    }
    for doc in serde_yaml_ng::Deserializer::from_str(text) {
        let v = serde_yaml_ng::Value::deserialize(doc).map_err(|e| e.to_string())?;
        walk("", &v, &mut out);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{AccessState, Condition, DiagnosticSeverity, EndpointAccess};

    const SEC: &str = "<dependency>micronaut-security</dependency>";
    const PROC: &str = "<dependency>micronaut-security</dependency>micronaut-security-processor";

    fn run(build: &str, files: &[(&str, &str)]) -> UiFlow {
        let dir = tempfile::tempdir().unwrap();
        let build = ("pom.xml", build);
        for (rel, text) in files.iter().copied().chain([build]) {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        extract(dir.path())
    }

    fn ep<'a>(flow: &'a UiFlow, method: &str, path: &str) -> &'a EndpointAccess {
        (flow.endpoints.iter())
            .find(|e| e.method == method && e.path == path)
            .unwrap_or_else(|| panic!("no {method} {path} in {:#?}", flow.endpoints))
    }

    fn gaps(flow: &UiFlow) -> usize {
        (flow.diagnostics.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .count()
    }

    fn role(n: &str) -> Condition {
        Condition::Role(n.into())
    }

    fn prot(c: Condition) -> AccessState {
        AccessState::Protected(c)
    }

    fn is_unknown(flow: &UiFlow, method: &str, path: &str) -> bool {
        matches!(
            ep(flow, method, path).access,
            AccessState::Protected(Condition::Unknown(_))
        )
    }

    fn java(class_ann: &str, body: &str) -> String {
        format!(
            "package a;\nimport io.micronaut.http.annotation.*;\n\
             import io.micronaut.security.annotation.Secured;\n\
             import io.micronaut.security.rules.SecurityRule;\n\
             import jakarta.annotation.security.*;\n\
             {class_ann}\nclass B {{\n{body}\n}}\n"
        )
    }

    const YML: &str = "src/main/resources/application.yml";

    #[test]
    fn n01_secured_class_and_method() {
        let src = java(
            "@Controller(\"/b\") @Secured(SecurityRule.IS_AUTHENTICATED)",
            "@Get(\"/\") String m() {}\n\
             @Get(\"/pub\") @Secured(SecurityRule.IS_ANONYMOUS) String p() {}\n\
             @Get(\"/adm\") @Secured({\"ROLE_ADMIN\",\"ROLE_X\"}) String a() {}",
        );
        let f = run(SEC, &[("src/B.java", &src)]);
        assert_eq!(ep(&f, "GET", "/b").access, prot(Condition::Authenticated));
        assert_eq!(ep(&f, "GET", "/b/pub").access, AccessState::Open);
        assert_eq!(
            ep(&f, "GET", "/b/adm").access,
            prot(Condition::Any(vec![role("ROLE_ADMIN"), role("ROLE_X")]))
        );
        assert_eq!(ep(&f, "GET", "/b/adm").handler.as_deref(), Some("B.a"));
        assert!(f.complete, "{:#?}", f.diagnostics);
    }

    #[test]
    fn n01b_jsr250_and_deny_all() {
        let src = java(
            "@Controller(\"/j\")",
            "@Get(\"/p\") @PermitAll String p() {}\n@Post(\"/d\") @DenyAll String d() {}\n\
             @Put(\"/r\") @RolesAllowed(\"ROLE_A\") String r() {}\n\
             @Get(\"/s\") @Secured(SecurityRule.DENY_ALL) String s() {}",
        );
        let f = run(PROC, &[("src/B.java", &src)]);
        assert_eq!(ep(&f, "GET", "/j/p").access, AccessState::Open);
        assert_eq!(ep(&f, "POST", "/j/d").access, prot(Condition::Never));
        assert_eq!(ep(&f, "PUT", "/j/r").access, prot(role("ROLE_A")));
        assert_eq!(ep(&f, "GET", "/j/s").access, prot(Condition::Never));
    }

    #[test]
    fn n01c_jsr250_without_processor_is_not_trusted() {
        let src = java(
            "@Controller(\"/j\")",
            "@Get(\"/p\") @PermitAll String p() {}",
        );
        let f = run(SEC, &[("src/B.java", &src)]);
        assert!(is_unknown(&f, "GET", "/j/p"));
        assert!(gaps(&f) > 0);
    }

    #[test]
    fn n02_unannotated_is_default_deny_with_note() {
        let src = java("@Controller(\"/c\")", "@Get(\"/x\") String x() {}");
        let f = run(SEC, &[("src/B.java", &src)]);
        assert_eq!(ep(&f, "GET", "/c/x").access, prot(Condition::Never));
        assert!(
            f.diagnostics
                .iter()
                .any(|d| d.message.contains("default deny"))
        );
        assert!(f.complete, "{:#?}", f.diagnostics);
    }

    #[test]
    fn n03_without_the_security_dependency_is_undeclared() {
        let src = java("@Controller(\"/c\")", "@Get(\"/x\") String x() {}");
        let f = run("<project/>", &[("src/B.java", &src)]);
        assert_eq!(ep(&f, "GET", "/c/x").access, AccessState::Undeclared);
    }

    #[test]
    fn n04_annotation_beats_the_url_map() {
        let src = java(
            "@Controller(\"/books\")",
            "@Get(\"/x\") @Secured(SecurityRule.IS_ANONYMOUS) String x() {}\n\
             @Get(\"/y\") String y() {}",
        );
        let yml = "micronaut:\n  security:\n    intercept-url-map:\n      - pattern: /books/**\n        access: [ROLE_ADMIN]\n";
        let f = run(SEC, &[("src/B.java", &src), (YML, yml)]);
        assert_eq!(ep(&f, "GET", "/books/x").access, AccessState::Open);
        assert_eq!(ep(&f, "GET", "/books/y").access, prot(role("ROLE_ADMIN")));
    }

    #[test]
    fn n05_url_map_prefers_the_matching_http_method() {
        let src = java(
            "@Controller(\"/x\")",
            "@Get String g() {}\n@Post String p() {}",
        );
        let yml = "micronaut:\n  security:\n    intercept-url-map:\n      - pattern: /x\n        access:\n          - isAuthenticated()\n      - pattern: /x\n        http-method: POST\n        access:\n          - ROLE_A\n";
        let f = run(SEC, &[("src/B.java", &src), (YML, yml)]);
        assert_eq!(ep(&f, "POST", "/x").access, prot(role("ROLE_A")));
        assert_eq!(ep(&f, "GET", "/x").access, prot(Condition::Authenticated));
    }

    #[test]
    fn n06_first_method_less_entry_wins() {
        let src = java("@Controller(\"/y\")", "@Get String g() {}");
        let yml = "micronaut:\n  security:\n    intercept-url-map:\n      - pattern: /y\n        access: [isAnonymous()]\n      - pattern: /y\n        access: [ROLE_A]\n";
        let f = run(SEC, &[("src/B.java", &src), (YML, yml)]);
        assert_eq!(ep(&f, "GET", "/y").access, AccessState::Open);
    }

    #[test]
    fn n06b_properties_form_and_ambiguous_earlier_entry() {
        let src = java("@Controller(\"/z\")", "@Get(\"/{id}\") String g() {}");
        let props = "micronaut.security.intercept-url-map[0].pattern=/z/1\n\
                     micronaut.security.intercept-url-map[0].access[0]=ROLE_A\n\
                     micronaut.security.intercept-url-map[1].pattern=/z/**\n\
                     micronaut.security.intercept-url-map[1].access[0]=isAnonymous()\n";
        let p = "src/main/resources/application.properties";
        let f = run(SEC, &[("src/B.java", &src), (p, props)]);
        // /z/1 may match /z/{id}: the earlier protected entry must not be skipped to Open.
        assert!(is_unknown(&f, "GET", "/z/{id}"));
        assert!(gaps(&f) > 0);
    }

    #[test]
    fn n07_expression_is_unknown() {
        let src = java(
            "@Controller(\"/e\")",
            "@Get @Secured(\"#{ user?.attributes?.get('email') == 'a@b' }\") String g() {}",
        );
        let f = run(SEC, &[("src/B.java", &src)]);
        assert!(is_unknown(&f, "GET", "/e"));
        assert!(gaps(&f) > 0);
    }

    #[test]
    fn n07b_anonymous_next_to_an_expression_is_not_open() {
        let src = java(
            "@Controller(\"/e\")",
            "@Get @Secured({SecurityRule.IS_ANONYMOUS, \"#{ x }\"}) String g() {}",
        );
        let f = run(SEC, &[("src/B.java", &src)]);
        assert!(is_unknown(&f, "GET", "/e"));
    }

    #[test]
    fn n08_secured_and_roles_allowed_together_is_unknown() {
        let src = java(
            "@Controller(\"/m\")",
            "@Get @Secured(\"ROLE_A\") @RolesAllowed(\"ROLE_B\") String g() {}",
        );
        let f = run(PROC, &[("src/B.java", &src)]);
        assert!(is_unknown(&f, "GET", "/m"));
        assert!(gaps(&f) > 0);
    }

    #[test]
    fn n09_custom_security_rule_makes_undecided_endpoints_unknown() {
        let src = java(
            "@Controller(\"/r\")",
            "@Get(\"/a\") String a() {}\n@Get(\"/b\") @Secured(\"ROLE_A\") String b() {}",
        );
        let rule = "package a;\nimport io.micronaut.security.rules.SecurityRule;\n\
                    class MyRule implements SecurityRule<Object> {}\n";
        let f = run(SEC, &[("src/B.java", &src), ("src/MyRule.java", rule)]);
        assert!(is_unknown(&f, "GET", "/r/a"));
        assert_eq!(ep(&f, "GET", "/r/b").access, prot(role("ROLE_A")));
        assert!(gaps(&f) > 0);
    }

    #[test]
    fn n10_url_map_only_in_an_environment_file_is_not_applied() {
        let src = java("@Controller(\"/p\")", "@Get String g() {}");
        let yml = "micronaut:\n  security:\n    intercept-url-map:\n      - pattern: /p\n        access: [isAnonymous()]\n";
        let prod = "src/main/resources/application-prod.yml";
        let f = run(SEC, &[("src/B.java", &src), (prod, yml)]);
        assert_eq!(ep(&f, "GET", "/p").access, AccessState::Undeclared);
        assert!(gaps(&f) > 0);
    }

    #[test]
    fn security_disabled_or_unreadable_config_is_not_open() {
        let src = java("@Controller(\"/p\")", "@Get String g() {}");
        let off = "micronaut:\n  security:\n    enabled: false\n";
        let f = run(SEC, &[("src/B.java", &src), (YML, off)]);
        assert_eq!(ep(&f, "GET", "/p").access, AccessState::Undeclared);
        let bad = "micronaut: [unclosed\n";
        let f = run(SEC, &[("src/B.java", &src), (YML, bad)]);
        assert_ne!(ep(&f, "GET", "/p").access, AccessState::Open);
        assert!(gaps(&f) > 0);
    }

    #[test]
    fn spring_controller_is_ignored() {
        let src = "package a;\nimport org.springframework.stereotype.Controller;\n\
                   @Controller(\"/s\")\nclass S { @Get(\"/x\") String x() {} }\n";
        let f = run(SEC, &[("src/S.java", src)]);
        assert!(f.endpoints.is_empty());
    }

    const ANY_URL: &str = "micronaut:\n  security:\n    intercept-url-map:\n      - pattern: /**\n        access: [isAnonymous()]\n";

    fn java_type(kind: &str, header: &str, body: &str) -> String {
        format!(
            "package a;\nimport io.micronaut.http.annotation.*;\nimport io.micronaut.security.annotation.Secured;\n\
             import io.micronaut.security.rules.SecurityRule;\n{kind} {header} {{\n{body}\n}}\n"
        )
    }

    #[test]
    fn inherited_interface_method_annotation_is_applied() {
        let api = java_type("interface", "Api", "@Secured(\"ROLE_ADMIN\") String i();");
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "@Get(\"/i\") public String i() { return \"\"; }",
        );
        let f = run(
            SEC,
            &[("src/Api.java", &api), ("src/C.java", &ctl), (YML, ANY_URL)],
        );
        assert_eq!(ep(&f, "GET", "/c/i").access, prot(role("ROLE_ADMIN")));
    }

    #[test]
    fn inherited_class_annotation_beats_the_url_map() {
        let base = java_type("class", "Base", "");
        let base = base.replace("class Base", "@Secured(\"ROLE_X\") class Base");
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C extends Base",
            "@Get(\"/g\") String g() { return \"\"; }",
        );
        let f = run(
            SEC,
            &[
                ("src/Base.java", &base),
                ("src/C.java", &ctl),
                (YML, ANY_URL),
            ],
        );
        assert_eq!(ep(&f, "GET", "/c/g").access, prot(role("ROLE_X")));
    }

    #[test]
    fn base_class_anonymous_marker_is_read() {
        let base = java_type("class", "Base", "").replace(
            "class Base",
            "@Secured(SecurityRule.IS_ANONYMOUS) class Base",
        );
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C extends Base",
            "@Get(\"/g\") String g() { return \"\"; }",
        );
        let f = run(SEC, &[("src/Base.java", &base), ("src/C.java", &ctl)]);
        assert_eq!(ep(&f, "GET", "/c/g").access, AccessState::Open);
    }

    #[test]
    fn unresolved_supertype_is_unknown_not_open_or_never() {
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C extends Lib",
            "@Get(\"/g\") String g() { return \"\"; }",
        );
        let f = run(SEC, &[("src/C.java", &ctl)]);
        assert!(is_unknown(&f, "GET", "/c/g"));
        let f = run(SEC, &[("src/C.java", &ctl), (YML, ANY_URL)]);
        assert!(is_unknown(&f, "GET", "/c/g"));
        assert!(gaps(&f) > 0);
    }

    #[test]
    fn custom_rule_with_get_order_may_precede_the_annotation() {
        let src = java(
            "@Controller(\"/r\")",
            "@Get(\"/b\") @Secured(\"ROLE_A\") String b() {}",
        );
        let rule = "package a;\nimport io.micronaut.security.rules.SecurityRule;\n\
                    class MyRule implements SecurityRule<Object> { public int getOrder() { return -500; } }\n";
        let f = run(SEC, &[("src/B.java", &src), ("src/MyRule.java", rule)]);
        assert!(is_unknown(&f, "GET", "/r/b"));
    }

    #[test]
    fn route_declared_on_an_interface_is_served_by_the_controller() {
        let api = java_type(
            "interface",
            "Pets",
            "@Post(\"/save\") @Secured(\"ROLE_ADMIN\") String save();\n@Get(\"/list\") String list();",
        );
        let ctl = java_type(
            "@Controller(\"/pets\") class",
            "P implements Pets",
            "public String save() { return \"\"; }\n@Get(\"/own\") String own() { return \"\"; }",
        );
        let f = run(SEC, &[("src/Pets.java", &api), ("src/P.java", &ctl)]);
        assert_eq!(
            ep(&f, "POST", "/pets/save").access,
            prot(role("ROLE_ADMIN"))
        );
        assert_eq!(
            ep(&f, "POST", "/pets/save").handler.as_deref(),
            Some("P.save")
        );
        assert_eq!(ep(&f, "GET", "/pets/list").access, prot(Condition::Never));
        assert!(f.endpoints.iter().any(|e| e.path == "/pets/own"));
    }

    #[test]
    fn controller_override_with_its_own_verb_is_not_duplicated() {
        let api = java_type("interface", "Pets", "@Get(\"/a\") String a();");
        let ctl = java_type(
            "@Controller(\"/p\") class",
            "P implements Pets",
            "@Get(\"/b\") String a() { return \"\"; }",
        );
        let f = run(SEC, &[("src/Pets.java", &api), ("src/P.java", &ctl)]);
        assert!(
            f.endpoints.iter().all(|e| e.path != "/p/a"),
            "{:#?}",
            f.endpoints
        );
        assert!(f.endpoints.iter().any(|e| e.path == "/p/b"));
    }

    #[test]
    fn unreadable_supertype_gets_a_route_gap() {
        let ctl = java_type("@Controller(\"/c\") class", "C implements Lib", "");
        let f = run(SEC, &[("src/C.java", &ctl)]);
        assert!(
            f.diagnostics
                .iter()
                .any(|d| d.severity == DiagnosticSeverity::Gap
                    && d.message
                        .contains("route may be inherited from supertype Lib")),
            "{:#?}",
            f.diagnostics
        );
    }

    #[test]
    fn overload_inherits_only_the_matching_supertype_method() {
        let api = java_type(
            "interface",
            "Api",
            "@Secured(\"ROLE_ADMIN\") String i(String a);",
        );
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "@Get(\"/n\") public String i(int a) { return \"\"; }\n\
             @Get(\"/s\") public String i(String a) { return \"\"; }",
        );
        let f = run(
            SEC,
            &[("src/Api.java", &api), ("src/C.java", &ctl), (YML, ANY_URL)],
        );
        assert_eq!(ep(&f, "GET", "/c/s").access, prot(role("ROLE_ADMIN")));
        assert_eq!(ep(&f, "GET", "/c/n").access, AccessState::Open);
    }

    #[test]
    fn overloaded_override_takes_its_own_annotations_by_parameters() {
        let api = java_type("interface", "Api", "@Get(\"/i\") String i(String a);");
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "@Secured(\"ROLE_Y\") public String i(int a) { return \"\"; }\n\
             @Secured(\"ROLE_X\") public String i(String a) { return \"\"; }",
        );
        let f = run(SEC, &[("src/Api.java", &api), ("src/C.java", &ctl)]);
        assert_eq!(ep(&f, "GET", "/c/i").access, prot(role("ROLE_X")));
    }

    #[test]
    fn ambiguous_overload_gets_a_gap_and_no_route() {
        let api = java_type("interface", "Api", "@Get(\"/i\") String i(T a);");
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "public String i(int a) { return \"\"; }\n\
             public String i(String a) { return \"\"; }",
        );
        let f = run(
            SEC,
            &[("src/Api.java", &api), ("src/C.java", &ctl), (YML, ANY_URL)],
        );
        assert!(
            f.endpoints.iter().all(|e| e.path != "/c/i"),
            "{:#?}",
            f.endpoints
        );
        assert!(
            f.diagnostics
                .iter()
                .any(|d| d.severity == DiagnosticSeverity::Gap && d.message.contains("declares i")),
            "{:#?}",
            f.diagnostics
        );
    }

    const KT_HEAD: &str = "package a\nimport io.micronaut.http.annotation.*\nimport io.micronaut.security.annotation.Secured\n";

    #[test]
    fn vararg_override_in_kotlin_matches_the_java_varargs_method() {
        let api = java_type(
            "interface",
            "Api",
            "@Get(\"/l\") @Secured(\"isAnonymous()\") String list(String... ids);",
        );
        let ctl = format!(
            "{KT_HEAD}@Controller(\"/c\") class C : Api {{\n  fun list(): String = \"\"\n  @Secured(\"ROLE_X\") override fun list(vararg ids: String): String = \"\"\n}}\n"
        );
        let f = run(SEC, &[("src/Api.java", &api), ("src/C.kt", &ctl)]);
        assert_eq!(ep(&f, "GET", "/c/l").access, prot(role("ROLE_X")));
    }

    #[test]
    fn unmatched_overload_never_inherits_the_open_route() {
        let api = java_type(
            "interface",
            "Api",
            "@Get(\"/i\") @Secured(\"isAnonymous()\") String i(String a);",
        );
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "@Secured(\"ROLE_X\") public String i(int a) { return \"\"; }\n\
             @Secured(\"ROLE_Y\") public String i(int) { return \"\"; }",
        );
        let f = run(SEC, &[("src/Api.java", &api), ("src/C.java", &ctl)]);
        assert!(
            f.endpoints.iter().all(|e| e.access != AccessState::Open),
            "{:#?}",
            f.endpoints
        );
        assert!(gaps(&f) > 0, "{:#?}", f.diagnostics);
    }

    #[test]
    fn single_supertype_method_with_other_parameters_is_not_inherited() {
        let base = java_type(
            "class",
            "Base",
            "@Get(\"/g\") @Secured(\"ROLE_A\") String get(Long id) { return \"\"; }",
        );
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C extends Base",
            "@Get(\"/h\") public String get(String s) { return \"\"; }",
        );
        let f = run(
            SEC,
            &[
                ("src/Base.java", &base),
                ("src/C.java", &ctl),
                (YML, ANY_URL),
            ],
        );
        assert_eq!(ep(&f, "GET", "/c/h").access, AccessState::Open);
        assert_eq!(ep(&f, "GET", "/c/g").access, prot(role("ROLE_A")));
    }

    #[test]
    fn kotlin_generic_function_counts_as_a_declaration() {
        let api =
            format!("{KT_HEAD}interface Api {{\n  @Get(\"/i\") fun i(a: String): String\n}}\n");
        let ctl = format!(
            "{KT_HEAD}@Controller(\"/c\") class C : Api {{\n  fun i(a: Int): String = \"\"\n  @Secured(\"ROLE_X\") override fun <T> i(a: String): String = \"\"\n}}\n"
        );
        let f = run(SEC, &[("src/Api.kt", &api), ("src/C.kt", &ctl)]);
        assert_eq!(ep(&f, "GET", "/c/i").access, prot(role("ROLE_X")));
    }

    #[test]
    fn annotation_argument_with_parentheses_in_a_string_is_skipped() {
        let api = java_type(
            "interface",
            "Api",
            "@Get(\"/f\") @Secured(\"isAnonymous()\") String find(@Pattern(regexp = \"^(a|b)$\") String code);",
        );
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "@Secured(\"ROLE_X\") public String find(String code) { return \"\"; }",
        );
        let f = run(SEC, &[("src/Api.java", &api), ("src/C.java", &ctl)]);
        assert_eq!(ep(&f, "GET", "/c/f").access, prot(role("ROLE_X")));
    }

    #[test]
    fn kotlin_mutable_collections_match_the_java_interfaces() {
        let api = java_type(
            "interface",
            "Api",
            "@Get(\"/m\") @Secured(\"isAnonymous()\") String m(List<String> a);",
        );
        let ctl = format!(
            "{KT_HEAD}@Controller(\"/c\") class C : Api {{\n  @Secured(\"ROLE_X\") override fun m(a: MutableList<String>): String = \"\"\n}}\n"
        );
        let f = run(SEC, &[("src/Api.java", &api), ("src/C.kt", &ctl)]);
        assert_eq!(ep(&f, "GET", "/c/m").access, prot(role("ROLE_X")));
    }

    #[test]
    fn override_with_an_unmatched_type_is_a_gap_not_the_open_route() {
        let api = format!(
            "{KT_HEAD}interface Api {{\n  @Get(\"/t\") @Secured(\"isAnonymous()\") fun t(a: String): String\n}}\n"
        );
        let alias = format!(
            "{KT_HEAD}typealias UserId = String\n@Controller(\"/c\") class C : Api {{\n  @Secured(\"ROLE_X\") override fun t(a: UserId): String = \"\"\n}}\n"
        );
        let f = run(SEC, &[("src/Api.kt", &api), ("src/C.kt", &alias)]);
        assert!(
            f.endpoints.iter().all(|e| e.access != AccessState::Open),
            "{:#?}",
            f.endpoints
        );
        assert!(gaps(&f) > 0, "{:#?}", f.diagnostics);
        let api = java_type(
            "interface",
            "Api",
            "@Get(\"/u\") @Secured(\"isAnonymous()\") String u(Base a);",
        );
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "@Override @Secured(\"ROLE_X\") public String u(Dto a) { return \"\"; }",
        );
        let f = run(SEC, &[("src/Api.java", &api), ("src/C.java", &ctl)]);
        assert!(
            f.endpoints.iter().all(|e| e.access != AccessState::Open),
            "{:#?}",
            f.endpoints
        );
        assert!(gaps(&f) > 0, "{:#?}", f.diagnostics);
    }

    fn no_open_and_a_gap(f: &UiFlow) {
        assert!(
            f.endpoints.iter().all(|e| e.access != AccessState::Open),
            "{:#?}",
            f.endpoints
        );
        assert!(gaps(f) > 0, "{:#?}", f.diagnostics);
    }

    #[test]
    fn brackets_inside_annotation_strings_do_not_change_the_parameter_count() {
        let api = java_type(
            "interface",
            "Api",
            "@Get(\"/f\") @Secured(\"isAnonymous()\") String f(@Pattern(regexp = \"[^)]*\") String a, String b);",
        );
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "@Override @Secured(\"ROLE_X\") public String f(String a, String b) { return \"\"; }",
        );
        let f = run(SEC, &[("src/Api.java", &api), ("src/C.java", &ctl)]);
        assert_eq!(ep(&f, "GET", "/c/f").access, prot(role("ROLE_X")));
    }

    #[test]
    fn override_marker_survives_braces_in_annotation_arguments() {
        let api = java_type(
            "interface",
            "Api",
            "@Get(\"/u\") @Secured(\"isAnonymous()\") String u(Base a);",
        );
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "@Override @Secured({\"ROLE_X\"}) public String u(Dto a) { return \"\"; }",
        );
        no_open_and_a_gap(&run(SEC, &[("src/Api.java", &api), ("src/C.java", &ctl)]));
    }

    #[test]
    fn declared_type_parameters_of_the_supertype_are_unsure_types() {
        let api = java_type(
            "interface",
            "Api<DTO>",
            "@Get(\"/g\") @Secured(\"isAnonymous()\") String g(DTO a);",
        );
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api<Foo>",
            "@Secured(\"ROLE_X\") public String g(Foo a) { return \"\"; }",
        );
        no_open_and_a_gap(&run(SEC, &[("src/Api.java", &api), ("src/C.java", &ctl)]));
    }

    #[test]
    fn declared_method_type_parameters_are_unsure_types() {
        let api = java_type(
            "interface",
            "Api",
            "@Get(\"/h\") @Secured(\"isAnonymous()\") <ITEM> String h(ITEM a);",
        );
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "@Secured(\"ROLE_X\") public String h(Foo a) { return \"\"; }",
        );
        no_open_and_a_gap(&run(SEC, &[("src/Api.java", &api), ("src/C.java", &ctl)]));
    }
    #[test]
    fn same_simple_type_name_from_other_packages_is_no_definite_match() {
        let api = java_type(
            "interface",
            "Api",
            "@Get(\"/i\") @Secured(\"isAnonymous()\") String i(Dto a);",
        )
        .replace("package a;", "package a;\nimport x.Dto;");
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "@Secured(\"ROLE_X\") public String i(Dto a) { return \"\"; }",
        )
        .replace("package a;", "package a;\nimport y.Dto;");
        no_open_and_a_gap(&run(SEC, &[("src/Api.java", &api), ("src/C.java", &ctl)]));
    }

    #[test]
    fn boxed_and_primitive_are_no_definite_match() {
        let api = java_type(
            "interface",
            "Api",
            "@Get(\"/i\") @Secured(\"isAnonymous()\") String i(long a);",
        );
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "@Secured(\"ROLE_X\") public String i(Long a) { return \"\"; }",
        );
        no_open_and_a_gap(&run(SEC, &[("src/Api.java", &api), ("src/C.java", &ctl)]));
    }

    #[test]
    fn kotlin_override_on_the_previous_line_is_still_an_override_guard() {
        let api = java_type(
            "interface",
            "Api",
            "@Get(\"/u\") @Secured(\"isAnonymous()\") String u(Base a);",
        );
        let ctl = format!(
            "{KT_HEAD}@Controller(\"/c\") class C : Api {{\n  @Secured(\"ROLE_X\")\n  override // see {{x}}; y\n  fun u(a: Dto): String = \"\"\n}}\n"
        );
        no_open_and_a_gap(&run(SEC, &[("src/Api.java", &api), ("src/C.kt", &ctl)]));
    }

    #[test]
    fn wildcard_imported_type_is_not_a_definite_match_for_a_qualified_one() {
        let api = java_type(
            "interface",
            "Api",
            "@Get(\"/i\") @Secured(\"isAnonymous()\") String i(Dto a);",
        )
        .replace("package a;", "package a;\nimport x.*;");
        let ctl = java_type(
            "@Controller(\"/c\") class",
            "C implements Api",
            "@Secured(\"ROLE_X\") public String i(Dto a) { return \"\"; }",
        )
        .replace("package a;", "package a;\nimport y.Dto;");
        no_open_and_a_gap(&run(SEC, &[("src/Api.java", &api), ("src/C.java", &ctl)]));
    }
}
