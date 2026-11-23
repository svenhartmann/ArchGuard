//! Checks below the endpoint (LOGIC.md section 7, items 7-10; `service-level.md`):
//! what the code an endpoint's handler calls declares, as [`ServiceLayer`]s on
//! the flow. Java and Kotlin only; the call graph is the compiler-backed one
//! of `semantic_model` (`semantic_calls::analyze_calls`), the annotations are
//! read with the Spring reader of `ui_flow_backend` (method-security switches,
//! meta-annotations, interfaces) and, for EJB and Quarkus CDI beans, here.
//!
//! A check is `Enforced` (ANDed into `UiFlow::effective_access`) only when the
//! handler calls the checked method directly and unconditionally (not inside
//! an `if`, loop, `try`, lambda, ternary or `&&`/`||`) on another bean: the
//! target is resolved by the compiler (an interface to its project
//! implementations, which must agree). Everything else is `PathOnly` (indirect,
//! conditional, implementations differ) or `Hint` (self-invocation, a `private`
//! or `final` method, a family that is not switched on, `@PostAuthorize`,
//! `@HandleAuthorizationDenied`, `@RolesAllowed` on a plain CDI bean outside
//! Quarkus, EJB annotations while an `ejb-jar.xml` entry is unreadable,
//! imperative checks). Row filters are `DataFilter`. A Spring check under
//! `@EnableMethodSecurity(mode = AdviceMode.ASPECTJ)` stays `Enforced` for
//! self-invocation and `private`/`final` methods when the aspect weaves the
//! annotation (spring-security-aspects: a method-level `@PreAuthorize` or
//! `@Secured` of the implementing class, a class-level `@Secured` on a public
//! method; everything else, e.g. a class-level `@PreAuthorize` or an interface
//! annotation, is `Hint`; the weaving setup itself is not read);
//! an unreadable `mode` keeps the `Hint` and leaves a gap. The `method-permission`
//! and `exclude-list` entries of an `ejb-jar.xml` give an EJB method its state
//! like the annotations do (Jakarta Enterprise Beans 4.0, section 11.3.2.2: the
//! descriptor overrides the annotations, `*` names every method, all matching
//! permissions are united and an `unchecked` one permits every role,
//! `exclude-list` denies; a bean only takes the descriptor of its own module). A reached project
//! class whose source cannot be resolved leaves a gap. Not read: C# services
//! (`IAuthorizationService`; see `service-level.md`), Micronaut. An earlier `return` in the handler makes the call
//! `PathOnly`; an earlier `throw` does not.
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use archguard_core::{
    AccessState, CallAnalysis, CallClass, CallMethod, Condition, DiagnosticSeverity,
    EndpointAccess, MethodCall, ServiceHow, ServiceKind, ServiceLayer, UiFlow, UiFlowDiagnostic,
};

use crate::extract_ctx::{ExtractContext, walk};
use crate::ui_flow_backend::lex::{literals, named, positional, split_top};
use crate::ui_flow_backend::spring::{Conf, declared_access, family};
use crate::ui_flow_backend::{
    Attr, Decl, Index, Out, Project, Source, class_header, decls, quarkus_build, read_projects,
};
use crate::ui_flow_micronaut::head_at;
use archguard_extract::{Lang, strip_root};
use ejb::Decision;

/// Words that make a source file worth a call analysis.
const SIGNS: [&str; 14] = [
    "PreAuthorize",
    "PostAuthorize",
    "Secured",
    "RolesAllowed",
    "PermitAll",
    "DenyAll",
    "Authenticated",
    "PermissionsAllowed",
    "SQLRestriction",
    "@Filter",
    "@Where",
    "SecurityContextHolder",
    "isUserInRole",
    "?#{",
];
/// Imperative checks in a service body.
const IMPERATIVE: [&str; 9] = [
    "SecurityContextHolder",
    "isUserInRole(",
    "AccessDecisionManager",
    "PermissionEvaluator",
    "AuthorizationManager",
    "getUserPrincipal(",
    "SecurityIdentity",
    "hasPermission(",
    "checkPermission(",
];
const SPRING_BEANS: [&str; 8] = [
    "Service",
    "Component",
    "Repository",
    "Controller",
    "RestController",
    "Configuration",
    "ControllerAdvice",
    "RestControllerAdvice",
];
const CDI_SCOPES: [&str; 6] = [
    "ApplicationScoped",
    "RequestScoped",
    "SessionScoped",
    "ConversationScoped",
    "Dependent",
    "Singleton",
];
const PLAIN: [&str; 4] = ["RolesAllowed", "PermitAll", "DenyAll", "Authenticated"];
/// Spring annotations whose effect is not a check before the call.
const DEFERRED: [&str; 4] = [
    "PostFilter",
    "PreFilter",
    "HandleAuthorizationDenied",
    "AuthorizeReturnObject",
];
const DEPTH: usize = 3;
const MAX_LAYERS: usize = 40;

fn handled(e: &EndpointAccess) -> bool {
    e.handler.is_some() && (e.file.ends_with(".java") || e.file.ends_with(".kt"))
}

fn has_signs(projects: &BTreeMap<String, Project>) -> bool {
    (projects.values().flat_map(|p| &p.1))
        .filter(|s| !s.rel.ends_with(".cs"))
        .any(|s| SIGNS.iter().any(|w| s.text.contains(w)))
}

fn not_read(analysis: &CallAnalysis) -> UiFlowDiagnostic {
    let detail = (analysis.diagnostics.first()).map_or("no detail", String::as_str);
    UiFlowDiagnostic {
        file: ".".into(),
        line: 0,
        message: format!(
            "Checks below the endpoints were not read: the Java/Kotlin call analysis is {} ({detail}).",
            analysis.status
        ),
        severity: DiagnosticSeverity::Note,
    }
}

/// Adds the service layers of every Java/Kotlin endpoint of `flow`.
pub(crate) fn apply(ctx: &ExtractContext, flow: &mut UiFlow) {
    if !flow.endpoints.iter().any(handled) {
        return;
    }
    let mut scratch = Out::scratch();
    let projects = read_projects(ctx.root, &mut scratch);
    let descriptor =
        walk(ctx.root).is_ok_and(|w| w.files.iter().any(|f| f.ends_with("ejb-jar.xml")));
    if !has_signs(&projects) && !descriptor {
        return;
    }
    #[allow(unused_mut)] // tests drop classes from it
    let mut analysis = jvm_calls(ctx, &projects);
    #[cfg(test)]
    analysis
        .classes
        .retain(|c| !DROPPED.with(|d| d.borrow().contains(&c.name)));
    if analysis.methods.is_empty() {
        flow.diagnostics.push(not_read(&analysis));
        return;
    }
    let mut world = World::new(ctx, &projects, &analysis, &mut scratch);
    let mut layers = Vec::new();
    for e in flow.endpoints.iter().filter(|e| handled(e)) {
        let site = flow.handler_site(e).map(|s| (s.file.as_str(), s.line));
        let mut found = world.endpoint(e, site);
        found.sort();
        found.dedup();
        found.truncate(MAX_LAYERS);
        layers.extend(found);
    }
    for (file, line, message) in world.gaps {
        flow.complete = false;
        flow.diagnostics.push(UiFlowDiagnostic {
            file,
            line,
            message,
            severity: DiagnosticSeverity::Gap,
        });
    }
    flow.service_layers.extend(layers);
    flow.service_layers.sort();
    flow.service_layers.dedup();
    flow.diagnostics.sort();
    flow.diagnostics.dedup();
}

#[cfg(test)]
thread_local! {
    /// Class names the tests take out of the call model (a library class).
    pub(crate) static DROPPED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The Java call model, plus Kotlin's when the project has Kotlin sources. Not
/// `analyze_calls`: one unavailable provider (e.g. TypeScript) would empty it.
fn jvm_calls(ctx: &ExtractContext, projects: &BTreeMap<String, Project>) -> CallAnalysis {
    let mut analysis = crate::java_calls::analyze_java_calls(ctx.root, ctx.config);
    let kotlin = (projects.values().flat_map(|p| &p.1)).any(|s| s.rel.ends_with(".kt"));
    if kotlin {
        let k = crate::semantic_calls::backend_report(Lang::Kotlin, ctx.root, ctx.config);
        analysis.classes.extend(k.classes);
        analysis.methods.extend(k.methods);
        analysis.calls.extend(k.calls);
    }
    analysis
}

struct World<'a> {
    ctx: &'a ExtractContext<'a>,
    index: Index<'a>,
    sources: HashMap<String, &'a Source>,
    dir_of: HashMap<&'a str, &'a str>,
    confs: HashMap<&'a str, Conf>,
    jars: ejb::Jars,
    /// How method-security advice is applied, by project dir.
    advice: HashMap<&'a str, Advice>,
    analysis: &'a CallAnalysis,
    classes: HashMap<&'a str, &'a CallClass>,
    methods: HashMap<&'a str, &'a CallMethod>,
    by_class: HashMap<&'a str, Vec<&'a CallMethod>>,
    /// Calls by the id of the calling method.
    calls: HashMap<&'a str, Vec<&'a MethodCall>>,
    /// Direct subtypes by class id.
    subtypes: HashMap<&'a str, Vec<&'a str>>,
    gaps: Vec<Gap>,
}

/// How `@EnableMethodSecurity(mode = ...)` applies the advice.
#[derive(Clone)]
enum Advice {
    Proxy,
    Aspectj,
    /// `mode` is not a literal `PROXY` or `ASPECTJ`.
    Unreadable(Gap),
}

fn advice_of(srcs: &[Source]) -> Advice {
    let (mut aspectj, mut proxy) = (false, false);
    for src in srcs.iter().filter(|s| !s.rel.ends_with(".cs")) {
        for a in (decls(src, true).iter())
            .filter(|d| d.class)
            .flat_map(|d| &d.attrs)
            .filter(|a| {
                matches!(
                    a.name.as_str(),
                    "EnableMethodSecurity" | "EnableGlobalMethodSecurity"
                )
            })
        {
            match named(&a.args, "mode").map(|v| v.trim().rsplit('.').next().unwrap_or("")) {
                None | Some("PROXY") => proxy = true,
                Some("ASPECTJ") => aspectj = true,
                Some(_) => {
                    let message = format!(
                        "`@{}` attribute `mode` is not readable, so whether self-invocation is advised is unknown: {}",
                        a.name, a.args
                    );
                    return Advice::Unreadable((src.rel.clone(), src.line(a.at), message));
                }
            }
        }
    }
    // one proxy-mode annotation next to an AspectJ one: assume the weaker
    if aspectj && !proxy {
        Advice::Aspectj
    } else {
        Advice::Proxy
    }
}

/// `(file, line, message)` of something read but not readable.
type Gap = (String, u32, String);

/// What one reached method says.
#[derive(Clone, PartialEq, Eq)]
struct Verdict {
    how: ServiceHow,
    kind: ServiceKind,
    state: AccessState,
    note: String,
    target: String,
    file: String,
    line: u32,
}

/// The element `check` reads: where it is declared and its project.
struct Site<'s> {
    src: &'s Source,
    class: &'s Decl,
    msrc: &'s Source,
    mclass: &'s Decl,
    decl: &'s Decl,
    m: &'s CallMethod,
    dir: &'s str,
}

/// State, notes and the starting `how` of one element.
type Read = (AccessState, Vec<String>, ServiceHow);

/// A security annotation on a class that is no bean: it may not be applied.
fn none(site: &Site) -> Option<Read> {
    let annotated = (site.decl.attrs.iter().chain(&site.class.attrs))
        .any(|a| PLAIN.contains(&a.name.as_str()) || family(&a.name).is_some());
    let why = "the class is not read as a bean (no stereotype or scope annotation), so the annotation may not be applied";
    annotated.then(|| (AccessState::Undeclared, vec![why.into()], ServiceHow::Hint))
}

/// A reached method: its declared class and the implementing classes the call
/// may run, with their own methods (for the next hop).
struct Reached<'a> {
    declared: &'a CallClass,
    method: &'a CallMethod,
    impls: Vec<(&'a CallClass, &'a CallMethod)>,
}

type Hop<'a> = (&'a MethodCall, bool, usize, String);

/// An enforced check stays so only on a direct, unconditional call.
fn settle(v: &mut Verdict, always: bool, depth: usize, via: &str) {
    if v.how != ServiceHow::Enforced || (depth == 0 && always) {
        return;
    }
    v.how = ServiceHow::PathOnly;
    let why = if depth > 0 {
        format!("reached indirectly via {via}")
    } else {
        "the handler calls it conditionally (inside an if, loop, try, lambda or expression)".into()
    };
    v.note = join(&v.note, &why);
}

fn layer_of(e: &EndpointAccess, v: Verdict) -> ServiceLayer {
    ServiceLayer {
        endpoint_file: e.file.clone(),
        endpoint_line: e.line,
        endpoint_method: e.method.clone(),
        endpoint_path: e.path.clone(),
        kind: v.kind,
        target: v.target,
        file: v.file,
        line: v.line,
        state: v.state,
        how: v.how,
        note: v.note,
    }
}

impl<'a> World<'a> {
    fn new(
        ctx: &'a ExtractContext<'a>,
        projects: &'a BTreeMap<String, Project>,
        analysis: &'a CallAnalysis,
        scratch: &mut Out,
    ) -> Self {
        let index = Index::new(
            projects.values().flat_map(|p| &p.1),
            ctx.config.views.access.clone(),
        );
        let mut confs = HashMap::new();
        let mut dir_of = HashMap::new();
        // The call model names files relative to the source roots.
        let roots = &ctx.config.project.source_roots;
        let mut sources: HashMap<String, &Source> = HashMap::new();
        let mut twice = BTreeSet::new();
        let mut advice = HashMap::new();
        for (dir, (_, srcs, quarkus)) in projects {
            advice.insert(dir.as_str(), advice_of(srcs));
            let mut conf = Conf::read(srcs, &index, *quarkus, scratch);
            conf.deferred_off = true;
            confs.insert(dir.as_str(), conf);
            for s in srcs {
                dir_of.insert(s.rel.as_str(), dir.as_str());
                let key = strip_root(&s.rel, roots);
                if sources.insert(key.clone(), s).is_some() {
                    twice.insert(key);
                }
            }
        }
        for key in &twice {
            sources.remove(key);
        }
        let dirs: Vec<&str> = projects.keys().map(String::as_str).collect();
        let jars = ejb::Jars::read(
            ctx.root,
            &walk(ctx.root).map_or_else(|_| vec![], |w| w.files),
            &dirs,
        );
        let mut world = World {
            ctx,
            index,
            sources,
            dir_of,
            confs,
            jars,
            advice,
            classes: (analysis.classes.iter())
                .map(|c| (c.id.as_str(), c))
                .collect(),
            methods: (analysis.methods.iter())
                .map(|m| (m.id.as_str(), m))
                .collect(),
            by_class: HashMap::new(),
            calls: HashMap::new(),
            subtypes: HashMap::new(),
            analysis,
            gaps: vec![],
        };
        for m in &analysis.methods {
            (world.by_class.entry(m.class_id.as_str()).or_default()).push(m);
        }
        for c in &analysis.calls {
            if let Some(from) = c.from_method.as_deref() {
                world.calls.entry(from).or_default().push(c);
            }
        }
        for c in &analysis.classes {
            for sup in &c.supertypes {
                (world.subtypes.entry(sup.as_str()).or_default()).push(c.id.as_str());
            }
        }
        world
    }

    fn endpoint(&mut self, e: &EndpointAccess, site: Option<(&str, u32)>) -> Vec<ServiceLayer> {
        let Some(handler) = self.handler(e, site) else {
            return vec![];
        };
        let Some(src) = self.sources.get(&handler.file).copied() else {
            return vec![];
        };
        let mut found: Vec<Verdict> = Vec::new();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut queue = self.direct(handler, src, &mut seen);
        while let Some((call, always, depth, via)) = queue.pop_front() {
            let Some(reached) = self.reach(call) else {
                // a library class: its checks are never read
                let message = format!(
                    "Check of `{}.{}` not read: the class is missing from the call model (a library class or one the analysis did not read), so a check there is unknown.",
                    call.to_class.as_deref().map_or("?", simple),
                    call.target_label
                );
                self.gaps.push((call.file.clone(), call.line, message));
                continue;
            };
            let same = call.to_class.as_deref() == Some(handler.class_id.as_str());
            let mut verdicts = self.verdicts(&reached, same);
            for v in &mut verdicts {
                settle(v, always, depth, &via);
            }
            found.extend(verdicts);
            if depth + 1 < DEPTH {
                self.hops(&reached, depth, &mut seen, &mut queue);
            }
        }
        found.into_iter().map(|v| layer_of(e, v)).collect()
    }

    /// The calls of `handler` to project methods, each with whether the
    /// handler always runs it.
    fn direct(
        &self,
        handler: &'a CallMethod,
        src: &Source,
        seen: &mut BTreeSet<&'a str>,
    ) -> VecDeque<Hop<'a>> {
        let direct: Vec<&'a MethodCall> =
            (self.calls.get(handler.id.as_str()).into_iter().flatten())
                .copied()
                .filter(|c| Self::project_call(c))
                .collect();
        let me = format!("{}.{}", self.class_name(&handler.class_id), handler.name);
        let mut queue = VecDeque::new();
        for c in &direct {
            let target = c.to_method.as_deref().unwrap_or_default();
            let always = (direct.iter())
                .filter(|o| o.to_method.as_deref() == Some(target))
                .any(|o| unconditional(src, handler, o));
            if seen.insert(target) {
                queue.push_back((*c, always, 0, me.clone()));
            }
        }
        queue
    }

    /// The calls the classes that run `reached` make to other project classes.
    fn hops(
        &self,
        reached: &Reached<'a>,
        depth: usize,
        seen: &mut BTreeSet<&'a str>,
        queue: &mut VecDeque<Hop<'a>>,
    ) {
        let here = format!("{}.{}", simple(&reached.declared.name), reached.method.name);
        for (cls, m) in &reached.impls {
            for next in (self.calls.get(m.id.as_str()).into_iter().flatten()).copied() {
                if Self::project_call(next)
                    && next.to_class.as_deref() != Some(cls.id.as_str())
                    && seen.insert(next.to_method.as_deref().unwrap_or_default())
                {
                    queue.push_back((next, false, depth + 1, here.clone()));
                }
            }
        }
    }

    fn project_call(c: &MethodCall) -> bool {
        c.resolution == "project"
            && c.to_class.is_some()
            && c.to_method.is_some()
            && matches!(c.dispatch.as_str(), "instance" | "interface")
    }

    fn class_name<'b>(&self, id: &'b str) -> &'b str
    where
        'a: 'b,
    {
        simple(self.classes.get(id).map_or(id, |c| c.name.as_str()))
    }

    /// The method that implements endpoint `e`'s handler `Class.method`.
    fn handler(&self, e: &EndpointAccess, site: Option<(&str, u32)>) -> Option<&'a CallMethod> {
        let (class, name) = e.handler.as_deref()?.rsplit_once('.')?;
        let (file, line) = site.unwrap_or((e.file.as_str(), e.line));
        let file = strip_root(file, &self.ctx.config.project.source_roots);
        let analysis: &'a CallAnalysis = self.analysis;
        let mut found: Vec<&'a CallMethod> = (analysis.methods.iter())
            .filter(|m| m.file == file && m.name == name && self.class_name(&m.class_id) == class)
            .collect();
        found.sort_by_key(|m| m.line.abs_diff(line));
        match found.as_slice() {
            [one] => Some(one),
            [a, b, ..] if a.line.abs_diff(line) < b.line.abs_diff(line) => Some(a),
            _ => None,
        }
    }

    /// The target of `call` with the project classes that can run it: for an
    /// interface or abstract class its (transitive) concrete subtypes.
    fn reach(&self, call: &MethodCall) -> Option<Reached<'a>> {
        let declared = *self.classes.get(call.to_class.as_deref()?)?;
        let method = *self.methods.get(call.to_method.as_deref()?)?;
        let mut impls = Vec::new();
        let mut todo: Vec<&str> = vec![declared.id.as_str()];
        let mut seen = BTreeSet::new();
        while let Some(id) = todo.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(class) = self.classes.get(id) else {
                continue;
            };
            if class.kind == "class" && class.is_abstract != Some(true) {
                // the class's own method of that signature, else the declared one
                let own = (self.by_class.get(id).into_iter().flatten())
                    .find(|m| m.signature == method.signature)
                    .copied();
                impls.push((*class, own.unwrap_or(method)));
            }
            todo.extend(self.subtypes.get(id).into_iter().flatten().copied());
        }
        if impls.is_empty() {
            impls.push((declared, method));
        }
        Some(Reached {
            declared,
            method,
            impls,
        })
    }

    fn verdicts(&mut self, r: &Reached<'a>, same_class: bool) -> Vec<Verdict> {
        let checks: Vec<Option<Verdict>> = (r.impls.iter())
            .map(|(cls, m)| self.check(cls, m, same_class))
            .collect();
        let mut out: Vec<Verdict> = Vec::new();
        let first = checks.first().cloned().flatten();
        if checks.iter().all(|c| *c == first) {
            out.extend(first);
        } else if let Some(mut v) = checks.iter().flatten().next().cloned() {
            let names: Vec<String> = (r.impls.iter().zip(&checks))
                .map(|((c, _), v)| {
                    let how = v.as_ref().map_or("no check", |v| how_name(v.how));
                    format!("{} ({how})", simple(&c.name))
                })
                .collect();
            v.how = ServiceHow::PathOnly;
            v.note = format!("the implementations differ: {}", names.join(", "));
            out.push(v);
        }
        for (cls, m) in &r.impls {
            out.extend(self.imperative(cls, m));
            out.extend(self.data(cls));
        }
        out
    }

    fn source_of(&self, c: &CallClass) -> Option<&'a Source> {
        self.sources.get(&c.file).copied()
    }

    /// What the annotations of method `m` of `cls` say.
    fn check(&mut self, cls: &CallClass, m: &CallMethod, same_class: bool) -> Option<Verdict> {
        let Some((src, class, msrc, mclass)) = self.resolve(cls, m) else {
            let message = format!(
                "Check of `{}.{}` not read: its source file or class declaration could not be resolved, so a check there is unknown.",
                simple(&cls.name),
                m.name
            );
            self.gaps.push((cls.file.clone(), m.line, message));
            return None;
        };
        let decl = self.decl_of(msrc, &mclass, m);
        self.gaps.extend(self.unread_annotations(msrc, &decl, m));
        let dir = (self.dir_of.get(src.rel.as_str())).copied();
        let dir = dir.unwrap_or_default();
        let bean = bean_of(src, &class, &self.index);
        let site = Site {
            src,
            class: &class,
            msrc,
            mclass: &mclass,
            decl: &decl,
            m,
            dir,
        };
        let mut gaps = std::mem::take(&mut self.gaps);
        let read = match bean {
            Bean::Spring => self.spring(&site, &mut gaps),
            Bean::Ejb | Bean::Cdi => self.jakarta(&site, bean, &mut gaps),
            Bean::None => none(&site),
        };
        self.gaps = gaps;
        let (state, soft, how) = read?;
        let mut verdict = Verdict {
            how,
            kind: ServiceKind::Service,
            state,
            note: soft.into_iter().fold(String::new(), |n, s| join(&n, &s)),
            target: format!("{}.{}", simple(&cls.name), m.name),
            file: msrc.rel.clone(),
            line: m.line,
        };
        if verdict.how == ServiceHow::Enforced {
            let why = if same_class {
                Some("self-invocation: a call inside the same bean does not pass the proxy")
            } else {
                unadvisable(src, &class, m)
            };
            let advice = (bean == Bean::Spring)
                .then(|| self.advice.get(dir).cloned())
                .flatten();
            match (why, advice) {
                (why, Some(Advice::Aspectj)) => {
                    if let Some(not) = not_woven(&site, msrc, &mut self.gaps) {
                        verdict.how = ServiceHow::Hint;
                        verdict.note = join(&verdict.note, not);
                    } else if let Some(why) = why {
                        let note = format!(
                            "{why}, but `mode = ASPECTJ` weaves the advice into the bytecode (the weaving setup is not read)"
                        );
                        verdict.note = join(&verdict.note, &note);
                    }
                }
                (None, _) => {}
                (Some(why), other) => {
                    if let Some(Advice::Unreadable(gap)) = other {
                        self.gaps.push(gap);
                    }
                    verdict.how = ServiceHow::Hint;
                    verdict.note = join(&verdict.note, why);
                }
            }
        }
        Some(verdict)
    }

    /// The sources and class declarations of `cls` and of the class that declares `m`.
    fn resolve(
        &self,
        cls: &CallClass,
        m: &CallMethod,
    ) -> Option<(&'a Source, Decl, &'a Source, Decl)> {
        let src = self.source_of(cls)?;
        let class = self.index.type_in(src, simple(&cls.name))?.clone();
        if m.class_id == cls.id {
            return Some((src, class.clone(), src, class));
        }
        let dc = self.classes.get(m.class_id.as_str())?;
        let ds = self.source_of(dc)?;
        Some((
            src,
            class,
            ds,
            self.index.type_in(ds, simple(&dc.name))?.clone(),
        ))
    }

    /// A gap when the method has annotations in the source but no declaration was matched.
    fn unread_annotations(&self, msrc: &Source, decl: &Decl, m: &CallMethod) -> Option<Gap> {
        let head = head_at(msrc, m.line, &m.name)?;
        let lost = decl.attrs.is_empty() && head.contains('@');
        let message = format!(
            "Check of `{}` not read: its annotations could not be matched to the call model, so a check there is unknown.",
            m.name
        );
        lost.then(|| (msrc.rel.clone(), m.line, message))
    }

    /// The annotated declaration of `m` in `mclass`, else an empty one.
    fn decl_of(&self, msrc: &Source, mclass: &Decl, m: &CallMethod) -> Decl {
        (self.index.methods_in(msrc, &mclass.name).into_iter())
            .find(|d| {
                d.name == m.name
                    && decl_arity(msrc, d) == arity_of(&m.signature)
                    && near(msrc, d, m)
            })
            .cloned()
            .unwrap_or_else(|| Decl {
                class: false,
                name: m.name.clone(),
                attrs: vec![],
                open: mclass.open,
                end: 0,
                bases: vec![],
                flag: false,
            })
    }

    /// Spring method security through the reader of the controllers.
    fn spring(&self, site: &Site, gaps: &mut Vec<Gap>) -> Option<Read> {
        let conf = self.confs.get(site.dir)?.clone();
        let mut scratch = Out::scratch();
        let arity = arity_of(&site.m.signature);
        let (state, ignored) = declared_access(
            &conf,
            &self.index,
            &mut scratch,
            (site.msrc, site.mclass, site.decl),
            arity,
        );
        let mut soft = Vec::new();
        for d in scratch.diagnostics() {
            if d.message.contains("not readable") {
                gaps.push((d.file.clone(), d.line, d.message.clone()));
            } else if d.message.contains("is not applied") {
                soft.push(d.message.clone());
            }
        }
        let mut how = ServiceHow::Enforced;
        let deferred: Vec<&str> = (site.decl.attrs.iter().chain(&site.class.attrs))
            .map(|a| a.name.as_str())
            .filter(|n| DEFERRED.contains(n))
            .collect();
        if !deferred.is_empty() {
            let names: Vec<String> = deferred.iter().map(|n| format!("`@{n}`")).collect();
            soft.push(format!(
                "{} does not deny the call before it runs (an after-call filter, a replacement value or a deferred check)",
                names.join(", ")
            ));
            if deferred.contains(&"HandleAuthorizationDenied") {
                how = ServiceHow::Hint;
            }
        }
        if state == AccessState::Undeclared {
            if soft.is_empty() {
                return None;
            }
            how = ServiceHow::Hint;
            if ignored {
                soft.insert(0, "ineffective".into());
            }
        }
        Some((state, soft, how))
    }

    /// EJB beans (enforced by the container) and CDI beans (enforced by Quarkus only).
    fn jakarta(&self, site: &Site, bean: Bean, gaps: &mut Vec<Gap>) -> Option<Read> {
        let mut soft = Vec::new();
        let mut how = ServiceHow::Enforced;
        let mut forced = None;
        if bean == Bean::Ejb && self.jars.has(site.dir) {
            let mut names = vec![simple(&site.class.name).to_string()];
            let bean_name = (site.class.attrs.iter())
                .filter(|a| {
                    matches!(
                        a.name.as_str(),
                        "Stateless" | "Stateful" | "Singleton" | "MessageDriven"
                    )
                })
                .find_map(|a| named(&a.args, "name"));
            names.extend(
                bean_name
                    .and_then(literals)
                    .and_then(|v| v.into_iter().next()),
            );
            match self.jars.decide(site.dir, &names, &site.m.name) {
                Decision::State(s) => {
                    let note = "permission read from ejb-jar.xml (it overrides the annotations)";
                    return Some((s, vec![note.into()], how));
                }
                Decision::Unreadable(why) => {
                    let message = format!(
                        "ejb-jar.xml: {why}; the descriptor overrides the annotations and is not read there"
                    );
                    gaps.push((site.msrc.rel.clone(), site.m.line, message.clone()));
                    forced = Some(message);
                }
                Decision::NoEntry => {}
            }
        }
        let state = match plain(gaps, site.src, site.class, site.decl) {
            Some(s) => s,
            None if forced.is_some() => AccessState::Undeclared,
            None => return None,
        };
        if let Some(message) = forced {
            how = ServiceHow::Hint;
            soft.push(message);
        } else if bean == Bean::Cdi && !quarkus_build(self.ctx.root, site.dir) {
            how = ServiceHow::Hint;
            soft.push(
                "`@RolesAllowed` on a plain CDI bean is not specified outside EJB and Quarkus"
                    .into(),
            );
        }
        Some((state, soft, how))
    }

    /// An imperative check in the body of `m`: a hint, never a protection.
    fn imperative(&self, cls: &CallClass, m: &CallMethod) -> Option<Verdict> {
        let src = self.source_of(cls)?;
        let end = m.end_line.unwrap_or(m.line) as usize;
        let body: String = (src.text.lines())
            .skip((m.line as usize).saturating_sub(1))
            .take((end + 1).saturating_sub(m.line as usize))
            .collect::<Vec<_>>()
            .join("\n");
        let hit = IMPERATIVE.iter().find(|w| body.contains(**w))?;
        Some(Verdict {
            how: ServiceHow::Hint,
            kind: ServiceKind::Service,
            state: AccessState::Undeclared,
            note: format!(
                "imperative check `{}` in the body: its outcome is not read, so it is no protection",
                hit.trim_end_matches('(')
            ),
            target: format!("{}.{}", simple(&cls.name), m.name),
            file: src.rel.clone(),
            line: m.line,
        })
    }

    /// Row filters on the entity of a Spring Data repository that `cls` uses (or is).
    fn data(&self, cls: &CallClass) -> Vec<Verdict> {
        let mut repos: Vec<&CallClass> = vec![cls];
        if let Some(d) = &self.analysis.details {
            repos.extend(
                (d.type_uses.iter().filter(|t| t.from_class == cls.id))
                    .filter_map(|t| self.classes.get(t.to_class.as_deref()?).copied()),
            );
        }
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for repo in repos {
            if !seen.insert(repo.id.as_str()) {
                continue;
            }
            let Some(src) = self.source_of(repo) else {
                continue;
            };
            let Some(decl) = self.index.type_in(src, simple(&repo.name)) else {
                continue;
            };
            let header = class_header(src, decl);
            let Some(at) = header.find("Repository<") else {
                continue;
            };
            let entity: String = header[at + "Repository<".len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            for (note, line, file) in self.row_filters(src, simple(&repo.name), &entity) {
                out.push(Verdict {
                    how: ServiceHow::DataFilter,
                    kind: ServiceKind::Data,
                    state: AccessState::Undeclared,
                    note,
                    target: simple(&repo.name).to_string(),
                    file,
                    line,
                });
            }
        }
        out
    }

    /// `(note, line, file)` of the row filters of `entity` and `repo`.
    fn row_filters(
        &self,
        repo_src: &Source,
        repo: &str,
        entity: &str,
    ) -> Vec<(String, u32, String)> {
        let mut out = Vec::new();
        let named: Vec<&CallClass> = (self.analysis.classes.iter())
            .filter(|c| simple(&c.name) == entity)
            .collect();
        if let [one] = named.as_slice()
            && let Some(es) = self.source_of(one)
            && let Some(ed) = self.index.type_in(es, entity)
        {
            for a in &ed.attrs {
                let what = match a.name.as_str() {
                    "SQLRestriction" | "Where" => "always applied to queries of the entity",
                    "Filter" | "FilterDef" => {
                        "applies only while the filter is enabled (`session.enableFilter` or `autoEnabled`), which is not read"
                    }
                    _ => continue,
                };
                let note = format!(
                    "`@{}` on {entity}: {what}; it filters rows, it does not deny",
                    a.name
                );
                out.push((note, es.line(a.at), es.rel.clone()));
            }
        }
        for d in self.index.methods_in(repo_src, repo) {
            for a in (d.attrs.iter()).filter(|a| a.name == "Query" && a.args.contains("?#{")) {
                let note = format!(
                    "`@Query` of {repo}.{} filters by the principal; it filters rows, it does not deny",
                    d.name
                );
                out.push((note, repo_src.line(a.at), repo_src.rel.clone()));
            }
        }
        out
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bean {
    Ejb,
    Spring,
    Cdi,
    None,
}

/// `@RolesAllowed`/`@PermitAll`/`@DenyAll`/`@Authenticated` (method replaces
/// class), `@PermissionsAllowed`/`@AuthorizationPolicy` as unknown.
fn plain(gaps: &mut Vec<Gap>, src: &Source, class: &Decl, decl: &Decl) -> Option<AccessState> {
    let pick = |attrs: &[Attr]| -> Vec<Attr> {
        (attrs.iter())
            .filter(|a| {
                PLAIN.contains(&a.name.as_str())
                    || matches!(
                        a.name.as_str(),
                        "PermissionsAllowed" | "AuthorizationPolicy"
                    )
            })
            .cloned()
            .collect()
    };
    let own = pick(&decl.attrs);
    let used = if own.is_empty() {
        pick(&class.attrs)
    } else {
        own
    };
    if used.is_empty() {
        return None;
    }
    let mut state = AccessState::Undeclared;
    for a in &used {
        let line = src.line(a.at);
        let one = match a.name.as_str() {
            "PermitAll" => AccessState::Open,
            "DenyAll" => AccessState::Protected(Condition::Never),
            "Authenticated" => AccessState::Protected(Condition::Authenticated),
            "RolesAllowed" => {
                let raw = (named(&a.args, "value").or_else(|| positional(&a.args))).unwrap_or("");
                let roles =
                    literals(raw).filter(|r| !r.is_empty() && !r.iter().any(|x| x.contains("${")));
                match roles {
                    Some(roles) => {
                        let mut items: Vec<Condition> =
                            roles.into_iter().map(Condition::Role).collect();
                        AccessState::Protected(if items.len() == 1 {
                            items.remove(0)
                        } else {
                            Condition::Any(items)
                        })
                    }
                    None => unknown(gaps, src, line, &a.args),
                }
            }
            _ => unknown(gaps, src, line, &a.args),
        };
        state = state.and(one);
    }
    Some(state)
}

fn unknown(gaps: &mut Vec<Gap>, src: &Source, line: u32, text: &str) -> AccessState {
    let message = format!("Access condition not readable: {text}");
    gaps.push((src.rel.clone(), line, message));
    AccessState::Protected(Condition::Unknown(text.into()))
}

fn bean_of(src: &Source, class: &Decl, index: &Index) -> Bean {
    let has = |names: &[&str]| class.attrs.iter().any(|a| names.contains(&a.name.as_str()));
    let ejb_singleton =
        has(&["Singleton"]) && (src.text.contains("ejb.Singleton") || src.text.contains("ejb.*"));
    if has(&["Stateless", "Stateful", "MessageDriven"]) || ejb_singleton {
        return Bean::Ejb;
    }
    fn stereotype(attrs: &[Attr], depth: usize, index: &Index) -> bool {
        attrs.iter().any(|a| {
            SPRING_BEANS.contains(&a.name.as_str())
                || (depth < 3
                    && (index.meta(&a.name))
                        .is_some_and(|(_, d)| stereotype(&d.attrs, depth + 1, index)))
        })
    }
    if src.text.contains("org.springframework")
        && (stereotype(&class.attrs, 0, index) || class_header(src, class).contains("Repository<"))
    {
        return Bean::Spring;
    }
    if has(&CDI_SCOPES) {
        return Bean::Cdi;
    }
    Bean::None
}

/// Why `mode = ASPECTJ` does not weave the check on the method of `site`: the
/// aspects of spring-security-aspects match a method-level `@PreAuthorize` or
/// `@Secured` of the implementing class, and a class-level `@Secured` on a
/// public method; interface annotations are never woven.
fn not_woven(site: &Site, msrc: &Source, gaps: &mut Vec<Gap>) -> Option<&'static str> {
    let on =
        |attrs: &[Attr], names: &[&str]| attrs.iter().any(|a| names.contains(&a.name.as_str()));
    // an interface method (also a default one) may not be woven
    if site.mclass.flag {
        let why = "`mode = ASPECTJ` is not known to weave a method declared on an interface; this check may not run";
        gaps.push((msrc.rel.clone(), site.m.line, why.to_string()));
        return Some(why);
    }
    if on(&site.decl.attrs, &["PreAuthorize", "Secured"]) {
        return None;
    }
    let before = &head_at(msrc, site.m.line, &site.m.name)?;
    let public = if msrc.rel.ends_with(".kt") {
        !before
            .split_whitespace()
            .any(|w| matches!(w, "private" | "protected" | "internal"))
    } else {
        before.split_whitespace().any(|w| w == "public")
    };
    if public && on(&site.mclass.attrs, &["Secured"]) {
        return None;
    }
    Some(
        "`mode = ASPECTJ` weaves only a method-level `@PreAuthorize`/`@Secured` of the implementing class or a class-level `@Secured` on a public method; this check is not woven",
    )
}

/// Why a proxy cannot advise method `m` (`private`, `final`).
fn unadvisable(src: &Source, class: &Decl, m: &CallMethod) -> Option<&'static str> {
    if src.rel.ends_with(".kt") {
        return None;
    }
    let before = &head_at(src, m.line, &m.name)?;
    let has = |text: &str, w: &str| text.split_whitespace().any(|x| x == w);
    if has(before, "private") {
        return Some("the method is private, so the proxy cannot advise it");
    }
    if has(before, "final") || has(class_header(src, class), "final") {
        return Some("the method or class is final, so the proxy cannot advise it");
    }
    None
}

/// The simple name of a class id (`proj::pkg.Outer$Inner` is `Inner`).
fn simple(id: &str) -> &str {
    let tail = id.rsplit("::").next().unwrap_or(id);
    tail.rsplit(['.', '$']).next().unwrap_or(tail)
}

fn how_name(h: ServiceHow) -> &'static str {
    match h {
        ServiceHow::Enforced => "enforced",
        ServiceHow::PathOnly => "path only",
        ServiceHow::Hint => "hint",
        ServiceHow::DataFilter => "data filter",
    }
}

fn join(note: &str, more: &str) -> String {
    if note.is_empty() {
        more.to_string()
    } else {
        format!("{note}; {more}")
    }
}

/// Parameter count of a call-model signature `name(a,b)`.
fn arity_of(signature: &str) -> usize {
    let inner = signature
        .split_once('(')
        .map_or("", |(_, r)| r.strip_suffix(')').unwrap_or(r));
    if inner.is_empty() {
        0
    } else {
        inner.split(',').count()
    }
}

fn decl_arity(src: &Source, d: &Decl) -> usize {
    src.pair(d.open)
        .map_or(0, |c| split_top(&src.text[d.open + 1..c]).len())
}

/// Whether declaration `d` is the one the call model puts at `m`.
fn near(src: &Source, d: &Decl, m: &CallMethod) -> bool {
    (m.line..=m.end_line.unwrap_or(m.line)).contains(&src.line(d.open))
}

/// Reading the body of a handler: whether it always runs a call.
mod body {
    use archguard_core::{CallMethod, MethodCall};

    use crate::ui_flow_backend::Source;

    fn line_start(src: &Source, line: u32) -> Option<usize> {
        let mut off = 0;
        for (i, l) in src.text.split_inclusive('\n').enumerate() {
            if i + 1 == line as usize {
                return Some(off);
            }
            off += l.len();
        }
        None
    }

    /// `(start, end, braces)` of the body of `h`: a block, or an expression after `=`.
    fn body_of(src: &Source, h: &CallMethod) -> Option<(usize, usize, bool)> {
        let start = line_start(src, h.line)?;
        let name_at = src.text[start..].find(&format!("{}(", h.name))? + start;
        let close = src.pair(name_at + h.name.len())?;
        let rest = &src.text[close + 1..];
        let delim = rest.find(['{', '=', ';'])?;
        let body_start = close + 1 + delim;
        let braces = rest.as_bytes()[delim] == b'{';
        let end = if braces {
            src.pair(body_start)?
        } else {
            usize::MAX
        };
        Some((body_start, end, braces))
    }

    /// The offset of the expression of `call` in its line.
    fn call_offset(src: &Source, call: &MethodCall) -> Option<usize> {
        let line_at = line_start(src, call.line)?;
        let line = src.text[line_at..].lines().next().unwrap_or("");
        let probe: String = (call.code.lines().next().unwrap_or(""))
            .chars()
            .take(60)
            .collect();
        let probe = probe.trim();
        let col = line.find(probe).filter(|_| !probe.is_empty())?;
        Some(line_at + col)
    }

    /// Whether offset `at` lies inside a block or in the parentheses of a
    /// condition that starts after `from`.
    fn nested(src: &Source, from: usize, at: usize) -> bool {
        let conditional = |p: usize| {
            let word = word_before(&src.text, p);
            matches!(
                word,
                "if" | "while" | "for" | "switch" | "catch" | "when" | "try" | "synchronized"
            )
        };
        (src.bytes().iter().enumerate().take(at).skip(from)).any(|(p, &c)| {
            let open = src.pair(p).is_some_and(|close| close > at);
            (c == b'{' && open) || (c == b'(' && open && conditional(p))
        })
    }

    /// Whether the handler `h` always runs `call`: it is not inside a block of the
    /// body (`if`, loop, `try`, lambda, local class), not in the parentheses of a
    /// condition, and not behind `?`, `&&`, `||`, `->`, `::`, `?.` or `?:`.
    pub(super) fn unconditional(src: &Source, h: &CallMethod, call: &MethodCall) -> bool {
        let (Some((body_start, body_end, braces)), Some(at)) =
            (body_of(src, h), call_offset(src, call))
        else {
            return false;
        };
        if at <= body_start || at >= body_end || nested(src, body_start + usize::from(braces), at) {
            return false;
        }
        let from = (src.bytes()[body_start..at].iter())
            .rposition(|c| matches!(c, b';' | b'{' | b'}'))
            .map_or(body_start, |p| body_start + p + 1);
        let statement = &src.text[from..at];
        // an earlier `return` may leave the handler before the call
        let before = &src.text[body_start..from];
        if before
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w == "return")
        {
            return false;
        }
        let keyword = |w: &str| matches!(w, "if" | "else" | "when" | "try");
        !["?", "&&", "||", "->", "::"]
            .iter()
            .any(|t| statement.contains(t))
            && !(statement.split(|c: char| !c.is_alphanumeric())).any(keyword)
    }

    fn word_before(text: &str, at: usize) -> &str {
        let head = text[..at].trim_end();
        let start = head
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map_or(0, |p| p + 1);
        &head[start..]
    }
}
use body::unconditional;

/// The `method-permission` and `exclude-list` entries of `ejb-jar.xml`.
mod ejb {
    use std::path::Path;

    use archguard_core::{AccessState, Condition};

    enum Rule {
        Roles(Vec<String>),
        Unchecked,
        Excluded,
        /// `method-params`, `method-intf`, or a part that is missing.
        Unreadable(String),
    }

    #[derive(Default)]
    pub(super) struct Jars {
        /// `(module dir, ejb-name, method-name, rule)`.
        rules: Vec<(String, String, String, Rule)>,
        /// `(module dir, ejb-name, simple name of ejb-class)`.
        classes: Vec<(String, String, String)>,
        /// `(module dir, why)`: a descriptor could not be parsed, or `metadata-complete` is not `false`.
        broken: Vec<(String, String)>,
        /// Module dirs that have a descriptor.
        modules: Vec<String>,
    }

    pub(super) enum Decision {
        NoEntry,
        State(AccessState),
        Unreadable(String),
    }

    fn text(n: roxmltree::Node, name: &str) -> Option<String> {
        let child = n.children().find(|c| c.has_tag_name(name))?;
        Some(child.text().unwrap_or("").trim().to_string())
    }

    impl Jars {
        /// A descriptor belongs to the module (project dir) that holds its path.
        pub(super) fn read(root: &Path, files: &[String], dirs: &[&str]) -> Jars {
            let mut jars = Jars::default();
            for f in files.iter().filter(|f| f.ends_with("ejb-jar.xml")) {
                let dir = (dirs.iter())
                    .filter(|d| d.is_empty() || f.starts_with(&format!("{d}/")))
                    .max_by_key(|d| d.len())
                    .map_or("", |d| *d)
                    .to_string();
                jars.modules.push(dir.clone());
                let parsed = std::fs::read_to_string(root.join(f))
                    .map_err(|e| e.to_string())
                    .and_then(|s| {
                        let doc = roxmltree::Document::parse(&s).map_err(|e| e.to_string())?;
                        jars.add(&dir, &doc);
                        Ok(())
                    });
                if let Err(e) = parsed {
                    jars.broken
                        .push((dir, format!("{f} is not readable ({e})")));
                }
            }
            jars
        }

        pub(super) fn has(&self, dir: &str) -> bool {
            self.modules.iter().any(|m| m == dir)
        }

        fn add(&mut self, dir: &str, doc: &roxmltree::Document) {
            let root = doc.root_element();
            if root
                .attribute("metadata-complete")
                .is_some_and(|v| v != "false")
            {
                let why = "`metadata-complete` is set, so the annotations are ignored";
                self.broken.push((dir.to_string(), why.into()));
            }
            for n in doc.descendants().filter(|n| n.is_element()) {
                match n.tag_name().name() {
                    "session" | "message-driven" => {
                        if let (Some(name), Some(class)) =
                            (text(n, "ejb-name"), text(n, "ejb-class"))
                        {
                            let simple = class.rsplit('.').next().unwrap_or("").to_string();
                            self.classes.push((dir.to_string(), name, simple));
                        }
                    }
                    kind @ ("method-permission" | "exclude-list") => {
                        self.permission(dir, n, kind == "exclude-list")
                    }
                    _ => {}
                }
            }
        }

        fn permission(&mut self, dir: &str, n: roxmltree::Node, exclude: bool) {
            let roles: Vec<String> = (n.children().filter(|c| c.has_tag_name("role-name")))
                .map(|c| c.text().unwrap_or("").trim().to_string())
                .collect();
            let unchecked = n.children().any(|c| c.has_tag_name("unchecked"));
            let methods = n.children().filter(|c| c.has_tag_name("method"));
            for m in methods {
                let (Some(ejb), Some(name)) = (text(m, "ejb-name"), text(m, "method-name")) else {
                    let why = "a `method` entry lacks `ejb-name` or `method-name`";
                    self.broken.push((dir.to_string(), why.into()));
                    continue;
                };
                let rule = if m
                    .children()
                    .any(|c| c.has_tag_name("method-params") || c.has_tag_name("method-intf"))
                {
                    Rule::Unreadable(format!(
                        "the entry for `{ejb}.{name}` names method parameters or an interface"
                    ))
                } else if exclude {
                    Rule::Excluded
                } else if unchecked {
                    Rule::Unchecked
                } else if !roles.is_empty() && roles.iter().all(|r| !r.is_empty()) {
                    Rule::Roles(roles.clone())
                } else {
                    Rule::Unreadable(format!(
                        "the `method-permission` for `{ejb}.{name}` has no readable role"
                    ))
                };
                self.rules.push((dir.to_string(), ejb, name, rule));
            }
        }

        /// What the descriptor of module `dir` says about `method` of the bean that may be called `names`.
        pub(super) fn decide(&self, dir: &str, names: &[String], method: &str) -> Decision {
            if let Some((_, why)) = self.broken.iter().find(|(d, _)| d == dir) {
                return Decision::Unreadable(why.clone());
            }
            let mut names = names.to_vec();
            for (d, name, class) in &self.classes {
                if d == dir && names.contains(class) {
                    names.push(name.clone());
                }
            }
            let hit: Vec<&Rule> = (self.rules.iter())
                .filter(|(d, e, m, _)| d == dir && names.contains(e) && (m == method || m == "*"))
                .map(|r| &r.3)
                .collect();
            if let Some(Rule::Unreadable(why)) =
                hit.iter().find(|r| matches!(r, Rule::Unreadable(_)))
            {
                return Decision::Unreadable(why.clone());
            }
            if hit.iter().any(|r| matches!(r, Rule::Excluded)) {
                return Decision::State(AccessState::Protected(Condition::Never));
            }
            if hit.is_empty() {
                return Decision::NoEntry;
            }
            Decision::State(state_of(hit.into_iter()))
        }
    }

    /// The state of the rules that apply: `unchecked` opens, otherwise the roles are unioned.
    fn state_of<'r>(rules: impl Iterator<Item = &'r Rule> + Clone) -> AccessState {
        if rules.clone().any(|r| matches!(r, Rule::Unchecked)) {
            return AccessState::Open;
        }
        let mut roles: Vec<Condition> = Vec::new();
        for x in rules
            .filter_map(|r| {
                if let Rule::Roles(rs) = r {
                    Some(rs)
                } else {
                    None
                }
            })
            .flatten()
        {
            let c = Condition::Role(x.clone());
            if !roles.contains(&c) {
                roles.push(c);
            }
        }
        AccessState::Protected(if roles.len() == 1 {
            roles.remove(0)
        } else {
            Condition::Any(roles)
        })
    }
}
