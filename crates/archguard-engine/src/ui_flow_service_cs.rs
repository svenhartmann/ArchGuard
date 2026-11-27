//! Checks below the endpoint for C# / ASP.NET Core (LOGIC.md section 7; the JVM
//! counterpart is `ui_flow_service`). The call graph is the Roslyn one of
//! `semantic_model` (`csharp_calls::analyze_csharp_calls`): the handler's calls
//! to project methods (an interface to its project implementations), up to
//! three hops. In every reached method two things are read, both only ever a
//! `Hint` (never `Enforced`, so never ANDed into the effective access):
//!
//! - a call the compiler resolved to `IAuthorizationService.AuthorizeAsync`
//!   (imperative, resource-based authorization): the check exists, but whether
//!   it denies depends on code that acts on the result, which is not read, so
//!   a gap is left too;
//! - `[Authorize]` on the reached class or method: ASP.NET Core evaluates it
//!   only on controllers, Razor pages and endpoints, so on a service it
//!   protects nothing.
//!
//! A reached project method whose class is missing from the call model, and a
//! method of a file that mentions `Authorize` whose declaration cannot be
//! matched, leave a gap. Unresolved calls named `AuthorizeAsync` do too.
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use archguard_core::{
    AccessState, CallAnalysis, CallClass, CallMethod, DiagnosticSeverity, EndpointAccess,
    MethodCall, ServiceHow, ServiceKind, ServiceLayer, UiFlow, UiFlowDiagnostic,
};
use archguard_extract::strip_root;
use archguard_flow_clike::decl::{Decl, Dialect, decls};
use archguard_flow_clike::source::Source;
use archguard_flow_common::extract_ctx::{ExtractContext, gap, read_source, walk};

const DEPTH: usize = 3;
const MAX_LAYERS: usize = 40;
/// Roslyn display labels of the imperative check (instance and extension form).
const AUTHORIZE_ASYNC: [&str; 2] = [
    "Microsoft.AspNetCore.Authorization.IAuthorizationService.AuthorizeAsync(",
    "Microsoft.AspNetCore.Authorization.AuthorizationServiceExtensions.AuthorizeAsync(",
];
const NOT_ENFORCED: &str = "ASP.NET Core evaluates [Authorize] only on controllers, Razor pages and endpoints; on a service class or method nothing applies it";

type Gap = (String, u32, String);
/// (file, line, target, note) of a check.
type Found = (String, u32, String, String);

fn handled(e: &EndpointAccess) -> bool {
    e.handler.is_some() && e.file.ends_with(".cs")
}

fn simple(id: &str) -> &str {
    let id = id.split('<').next().unwrap_or(id);
    id.rsplit(['.', ':', '+']).next().unwrap_or(id)
}

/// Adds the service layers of every C# endpoint of `flow`.
pub(crate) fn apply(ctx: &ExtractContext, flow: &mut UiFlow) {
    if !flow.endpoints.iter().any(handled) {
        return;
    }
    let roots = &ctx.config.project.source_roots;
    let (sources, scanned) = scan(ctx, flow);
    if sources.is_empty() {
        return;
    }
    let analysis = crate::csharp_calls::analyze_csharp_calls(ctx.root, ctx.config);
    if analysis.methods.is_empty() {
        let detail = (analysis.diagnostics.first()).map_or("no detail", String::as_str);
        flow.diagnostics.push(UiFlowDiagnostic {
            file: ".".into(),
            line: 0,
            message: format!(
                "Checks below the C# endpoints were not read: the C# call analysis is {} ({detail}).",
                analysis.status
            ),
            severity: DiagnosticSeverity::Note,
        });
        return;
    }
    let mut world = World::new(&analysis, &sources, &scanned);
    let mut layers = Vec::new();
    for e in flow.endpoints.iter().filter(|e| handled(e)) {
        let site = flow.handler_site(e).map(|s| (s.file.as_str(), s.line));
        layers.extend(world.endpoint(e, site, roots));
    }
    for (file, line, message) in world.gaps {
        add_gap(flow, &file, line, message);
    }
    flow.service_layers.extend(layers);
    flow.service_layers.sort();
    flow.service_layers.dedup();
    flow.diagnostics.sort();
    flow.diagnostics.dedup();
}

fn add_gap(flow: &mut UiFlow, file: &str, line: u32, message: String) {
    flow.complete = false;
    gap(&mut flow.diagnostics, file, line, message);
}

/// The `.cs` files that mention `Authorize` (by project-relative path) and the paths of all `.cs`
/// files read; anything that cannot be read is a gap, a file that is not read has no check known.
fn scan(ctx: &ExtractContext, flow: &mut UiFlow) -> (HashMap<String, Source>, HashSet<String>) {
    let roots = &ctx.config.project.source_roots;
    let (mut sources, mut scanned) = (HashMap::new(), HashSet::new());
    let walked = match walk(ctx.root) {
        Ok(walked) => walked,
        Err(e) => {
            add_gap(flow, ".", 0, format!("Discovery settings not read: {e}"));
            return (sources, scanned);
        }
    };
    for e in walked.errors {
        add_gap(flow, ".", 0, format!("Directory entry not read: {e}"));
    }
    for rel in walked.files.iter().filter(|r| r.ends_with(".cs")) {
        match read_source(ctx.root, rel) {
            Ok(text) => {
                scanned.insert(strip_root(rel, roots));
                if text.contains("Authorize") {
                    sources.insert(strip_root(rel, roots), Source::new(rel.clone(), &text));
                }
            }
            Err(e) => add_gap(flow, rel, 0, e),
        }
    }
    (sources, scanned)
}

pub(crate) struct World<'a> {
    analysis: &'a CallAnalysis,
    sources: &'a HashMap<String, Source>,
    scanned: &'a HashSet<String>,
    classes: HashMap<&'a str, &'a CallClass>,
    methods: HashMap<&'a str, &'a CallMethod>,
    by_class: HashMap<&'a str, Vec<&'a CallMethod>>,
    calls: HashMap<&'a str, Vec<&'a MethodCall>>,
    subtypes: HashMap<&'a str, Vec<&'a str>>,
    gaps: Vec<Gap>,
}

impl<'a> World<'a> {
    pub(crate) fn new(
        analysis: &'a CallAnalysis,
        sources: &'a HashMap<String, Source>,
        scanned: &'a HashSet<String>,
    ) -> Self {
        let mut w = World {
            analysis,
            sources,
            scanned,
            classes: (analysis.classes.iter())
                .map(|c| (c.id.as_str(), c))
                .collect(),
            methods: (analysis.methods.iter())
                .map(|m| (m.id.as_str(), m))
                .collect(),
            by_class: HashMap::new(),
            calls: HashMap::new(),
            subtypes: HashMap::new(),
            gaps: vec![],
        };
        for m in &analysis.methods {
            (w.by_class.entry(m.class_id.as_str()).or_default()).push(m);
        }
        for c in &analysis.calls {
            if let Some(from) = c.from_method.as_deref() {
                w.calls.entry(from).or_default().push(c);
            }
        }
        for c in &analysis.classes {
            for sup in &c.supertypes {
                (w.subtypes.entry(sup.as_str()).or_default()).push(c.id.as_str());
            }
        }
        w
    }

    fn endpoint(
        &mut self,
        e: &EndpointAccess,
        site: Option<(&str, u32)>,
        roots: &[String],
    ) -> Vec<ServiceLayer> {
        let Some(handler) = self.handler(e, site, roots) else {
            let message = format!(
                "Checks below `{} {}` not read: its handler `{}` is not matched to one method of the call model.",
                e.method,
                e.path,
                e.handler.as_deref().unwrap_or("?")
            );
            self.gaps.push((e.file.clone(), e.line, message));
            return vec![];
        };
        let mut found: Vec<Found> = vec![];
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut queue: VecDeque<(&MethodCall, usize)> = VecDeque::new();
        self.enqueue(handler, 0, &mut seen, &mut queue);
        while let Some((call, depth)) = queue.pop_front() {
            let (Some(declared), Some(method)) = (
                call.to_class.as_deref().and_then(|c| self.classes.get(c)),
                call.to_method.as_deref().and_then(|m| self.methods.get(m)),
            ) else {
                let message = format!(
                    "Check of `{}.{}` not read: the class is missing from the call model, so a check there is unknown.",
                    call.to_class.as_deref().map_or("?", simple),
                    call.target_label
                );
                self.gaps.push((call.file.clone(), call.line, message));
                continue;
            };
            for (cls, m) in self.impls(declared, method) {
                found.extend(self.checks(cls, m, &handler.class_id));
                if depth + 1 < DEPTH {
                    self.enqueue(m, depth + 1, &mut seen, &mut queue);
                }
            }
        }
        found.sort();
        found.dedup();
        if found.len() > MAX_LAYERS {
            let message = format!(
                "Checks below `{} {}`: {} found, only the first {MAX_LAYERS} are listed.",
                e.method,
                e.path,
                found.len()
            );
            self.gaps.push((e.file.clone(), e.line, message));
            found.truncate(MAX_LAYERS);
        }
        (found.into_iter())
            .map(|(file, line, target, note)| ServiceLayer {
                endpoint_file: e.file.clone(),
                endpoint_line: e.line,
                endpoint_method: e.method.clone(),
                endpoint_path: e.path.clone(),
                kind: ServiceKind::Service,
                target,
                file,
                line,
                state: AccessState::Undeclared,
                how: ServiceHow::Hint,
                note,
            })
            .collect()
    }

    /// The project calls of `from` not seen yet.
    fn enqueue(
        &self,
        from: &CallMethod,
        depth: usize,
        seen: &mut BTreeSet<&'a str>,
        queue: &mut VecDeque<(&'a MethodCall, usize)>,
    ) {
        for c in self.calls.get(from.id.as_str()).into_iter().flatten() {
            let project = c.resolution == "project"
                && c.to_class.is_some()
                && matches!(c.dispatch.as_str(), "instance" | "interface");
            if let Some(target) = c.to_method.as_deref()
                && project
                && c.to_class.as_deref() != Some(from.class_id.as_str())
                && seen.insert(target)
            {
                queue.push_back((c, depth));
            }
        }
    }

    /// The method that implements endpoint `e`'s handler `Class.method`.
    fn handler(
        &self,
        e: &EndpointAccess,
        site: Option<(&str, u32)>,
        roots: &[String],
    ) -> Option<&'a CallMethod> {
        let (class, name) = e.handler.as_deref()?.rsplit_once('.')?;
        let (file, line) = site.unwrap_or((e.file.as_str(), e.line));
        let file = strip_root(file, roots);
        let class_of = |id: &str| (self.classes.get(id)).map_or("", |c| simple(&c.name));
        let mut found: Vec<&CallMethod> = (self.analysis.methods.iter())
            .filter(|m| m.file == file && m.name == name && class_of(&m.class_id) == class)
            .collect();
        found.sort_by_key(|m| m.line.abs_diff(line));
        match found.as_slice() {
            [one] => Some(one),
            [a, b, ..] if a.line.abs_diff(line) < b.line.abs_diff(line) => Some(a),
            _ => None,
        }
    }

    /// The target with the project classes that can run it: for an interface or
    /// abstract class its (transitive) concrete subtypes.
    fn impls(
        &self,
        declared: &'a CallClass,
        method: &'a CallMethod,
    ) -> Vec<(&'a CallClass, &'a CallMethod)> {
        let mut impls = Vec::new();
        let mut todo = vec![declared.id.as_str()];
        let mut seen = BTreeSet::new();
        while let Some(id) = todo.pop() {
            let Some(class) = self.classes.get(id).filter(|_| seen.insert(id)) else {
                continue;
            };
            if class.kind == "class" && class.is_abstract != Some(true) {
                let own = (self.by_class.get(id).into_iter().flatten())
                    .find(|m| m.signature == method.signature);
                impls.push((*class, own.copied().unwrap_or(method)));
            }
            todo.extend(self.subtypes.get(id).into_iter().flatten().copied());
        }
        if impls.is_empty() {
            impls.push((declared, method));
        }
        impls
    }

    /// The checks read in method `m` of `cls`.
    fn checks(&mut self, cls: &CallClass, m: &CallMethod, handler_class: &str) -> Vec<Found> {
        let target = format!("{}.{}", simple(&cls.name), m.name);
        let mut out = self.imperative(m, &target);
        out.extend(self.attributes(cls, m, handler_class, &target));
        out
    }

    /// Calls of `m` to `IAuthorizationService.AuthorizeAsync` (a hint and a gap).
    fn imperative(&mut self, m: &CallMethod, target: &str) -> Vec<Found> {
        let mut out = vec![];
        for c in self.calls.get(m.id.as_str()).into_iter().flatten() {
            let imperative = c.resolution == "external"
                && AUTHORIZE_ASYNC
                    .iter()
                    .any(|l| c.target_label.starts_with(l));
            if imperative {
                let note = "imperative `IAuthorizationService.AuthorizeAsync`: the check exists, but whether it denies depends on code acting on the result, which is not read";
                out.push((c.file.clone(), c.line, target.to_owned(), note.into()));
                let gap = format!(
                    "The outcome of the authorization check in `{target}` is not read: whether a failed check stops the request depends on code."
                );
                self.gaps.push((c.file.clone(), c.line, gap));
            } else if c.resolution == "unresolved" && c.target_label.contains("AuthorizeAsync") {
                let gap = format!(
                    "A call named `AuthorizeAsync` in `{target}` is not resolved by the compiler: a check there is unknown."
                );
                self.gaps.push((c.file.clone(), c.line, gap));
            }
        }
        out
    }

    /// `[Authorize]` on the class or method that declares `m`.
    pub(crate) fn attributes(
        &mut self,
        cls: &CallClass,
        m: &CallMethod,
        handler_class: &str,
        target: &str,
    ) -> Vec<Found> {
        let mut out = vec![];
        let Some(src) = self.sources.get(&m.file) else {
            // a file without `Authorize` has no check; one never read is unknown
            if !self.scanned.contains(&m.file) {
                let message = format!(
                    "Check of `{target}` not read: its file was not read, so a check there is unknown."
                );
                self.gaps.push((m.file.clone(), m.line, message));
            }
            return out;
        };
        let all = decls(src, Dialect::CSharp);
        let authorize = |d: &Decl| {
            (d.attrs.iter()).any(|a| matches!(simple(&a.name), "Authorize" | "AuthorizeAttribute"))
        };
        // an inherited method is declared (and annotated) in its base class's file
        let declaring =
            (self.classes.get(m.class_id.as_str())).map_or(cls.name.as_str(), |c| c.name.as_str());
        let class = all.iter().find(|d| d.class && d.name == simple(declaring));
        let method = class.and_then(|c| {
            (all.iter()).find(|d| {
                !d.class
                    && d.name == m.name
                    && d.open > c.open
                    && d.open < c.end
                    // the span of the method starts at its first attribute: a wrapped
                    // signature puts its parameter list lines below
                    && (m.line..=m.end_line.unwrap_or(m.line)).contains(&src.line(d.open))
            })
        });
        // shortcut: the attribute-position match covers the whole span of the method (a body
        // is searched too), a lambda attribute `[Authorize]` in it counts; upgrade on a false gap
        let span = (m.end_line.unwrap_or(m.line) + 1 - m.line) as usize;
        let header: String = (src.text.split_inclusive('\n'))
            .skip(m.line as usize - 1)
            .take(span)
            .collect();
        match (class, method) {
            (Some(c), Some(d)) => {
                if authorize(d) || (m.class_id != handler_class && authorize(c)) {
                    out.push((src.rel.clone(), m.line, target.to_owned(), NOT_ENFORCED.into()));
                }
            }
            // `decls` skips a non-public method without attributes: one that is not matched
            // is one without, unless its span names `Authorize` as an attribute.
            (Some(_), None) if !names_authorize(&header) => {}
            _ => self.gaps.push((
                src.rel.clone(),
                m.line,
                format!(
                    "Check of `{target}` not read: its declaration could not be matched in a file that mentions `Authorize`, so a check there is unknown."
                ),
            )),
        }
        out
    }
}

/// Whether `text` names `Authorize` in attribute position: `[Authorize`, `, Authorize`,
/// `[Ns.Authorize`, optionally `AuthorizeAttribute`, followed by `(`, `]` or `,`. A name that
/// only contains the word (`MyAuthorize`, `AuthorizeFilter`, `_authorizer`) does not.
fn names_authorize(text: &str) -> bool {
    text.match_indices("Authorize").any(|(i, _)| {
        let head = &text[..i];
        let head = if head.ends_with('.') {
            head.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '.')
        } else {
            head
        };
        let rest = &text[i + "Authorize".len()..];
        let rest = rest.strip_prefix("Attribute").unwrap_or(rest);
        head.trim_end().ends_with(['[', ',']) && rest.trim_start().starts_with(['(', ']', ','])
    })
}
