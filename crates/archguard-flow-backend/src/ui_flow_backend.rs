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
//!   `@PermitAll`, `@DenyAll` (`never`). Names are stored as the runtime
//!   authority: `hasRole('A')` adds the role prefix (unless present), so
//!   `hasRole('A')`, `hasAuthority('ROLE_A')` and `@Secured("ROLE_A")` all
//!   read `role(ROLE_A)`; `@RolesAllowed("A")` always adds it. Any other
//!   authority (`hasAuthority('read')`, `@Secured("x")`, OAuth2 `hasScope`) is
//!   an exact `policy`. The prefix is `GrantedAuthorityDefaults`' literal
//!   (else `ROLE_`; Quarkus Spring compatibility: none); an unreadable one
//!   makes role names `unknown` with a gap. The convention per application
//!   (`role_prefix`, `scope_prefix`, `role_claim` from a
//!   `JwtGrantedAuthoritiesConverter`) is emitted with its origins.
//!   Method-security annotations count only for an enabled family
//!   (`@EnableMethodSecurity`: pre-post; `securedEnabled`, `jsr250Enabled`;
//!   legacy `@EnableGlobalMethodSecurity`: all off by default, the first
//!   family with annotations decides); a disabled family is ignored with a
//!   gap and never lifted to `open` by a chain `permitAll`. Several chains
//!   apply by `@Order` and their literal `securityMatcher` scope.
//!
//! ASP.NET app-wide facts are read per project (`aspnet`): `DefaultPolicy`
//! (what a bare `[Authorize]`/`RequireAuthorization()` means), `FallbackPolicy`
//! (only for endpoints without authorization metadata), global
//! `Filters.Add(new AuthorizeFilter(…))` (MVC actions), and
//! `MapControllers().RequireAuthorization()`; project attributes deriving from
//! `AuthorizeAttribute` or implementing `IAuthorizeData` /
//! `IAuthorizationRequirementData` (also generic `[Authorize<T>]`) are read
//! through their constructors. Actions of base controllers belong to the
//! derived controller; abstract and generic classes are no controllers.
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
//! inside helper extension methods, attributes on a minimal-API lambda, other
//! SpEL (`unknown`), non-constant routes or roles (diagnostic, the endpoint
//! is skipped or the condition is `unknown`), `MapIdentityApi`, Razor Pages.
use archguard_core::{
    ANY_METHOD, AccessMeaning, AccessState, Condition, ConventionOrigin, DiagnosticSeverity,
    EndpointAccess, HandlerSite, RoleConvention, UiFlow, UiFlowDiagnostic,
};
use archguard_flow_clike::decl::{
    Attr, Decl, class_header, decls, directive_attrs, each_method, simple,
};
use archguard_flow_clike::handlers::{self, simple_type};
use archguard_flow_clike::lex;
use archguard_flow_clike::quarkus_build::{BUILD_FILES, MAX_BYTES};
use archguard_flow_clike::source::{Source, statement_end};
use archguard_flow_clike::supertypes::{jdk_type, names_interface};
use archguard_flow_common::extract_ctx::{
    ExtractContext, ancestors, dir_name, read_text, walk, with_root_config,
};
use archguard_flow_common::support::test_path;
use lex::{ident_before, literal_end, split_top};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

const MAX_FILES: usize = 20_000;
const SOURCES: [&str; 3] = [".cs", ".java", ".kt"];

/// Endpoints with their declared access, plus diagnostics. `[views.access]`
/// comes from the root's configuration; an unreadable one is a note.
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, |ctx| extract_with(ctx, None))
}

/// Reads further endpoints of a source file before the central rules run
/// (`ui_flow_webflux`: functional routes).
pub(crate) type RouteHook = fn(&Source, &[Source], &Index, &spring::Conf, &mut Out);

/// [`extract`] with the run's `[views.access]`; `hook` adds endpoints per
/// Java/Kotlin file.
pub(crate) fn extract_with(ctx: &ExtractContext, hook: Option<RouteHook>) -> UiFlow {
    let mut out = Out(
        UiFlow::default(),
        Ignored::default(),
        aspnet::Defaults::default(),
    );
    if let Some(e) = &ctx.config_error {
        let message = format!("Configuration not read; [views.access] not applied: {e}");
        out.note(".", 0, message);
    }
    let (projects, pages, build_dirs) = read_all(ctx.root, &mut out);
    let mut apps = spring::Apps::new(ctx.root, &build_dirs);
    let access = ctx.config.views.access.clone();
    let index = Index::new(projects.values().flat_map(|(_, s, _)| s), access);
    for (dir, (application, sources, quarkus)) in &projects {
        let first = out.0.endpoints.len();
        aspnet::read(sources, &index, target_framework(ctx.root, dir), &mut out);
        let mut call_sites = HashMap::new();
        let routes = conventional::routes(sources, &index, &mut out);
        let conf = spring::Conf::read(sources, &index, *quarkus, &mut out);
        for src in sources {
            if src.rel.ends_with(".cs") {
                controllers::scan(src, &index, &routes, &mut out);
                minimal::scan(src, sources, &index, &mut call_sites, &mut out);
            } else {
                spring::scan(src, &index, &conf, &mut out);
                if let Some(hook) = hook {
                    hook(src, sources, &index, &conf, &mut out);
                }
            }
        }
        razor::scan(
            pages.get(dir).map_or(&[], Vec::as_slice),
            sources,
            &index,
            application.as_deref(),
            &mut out,
        );
        spring::central(dir, &projects, &mut apps, (&index, &conf), &mut out, first);
        for e in &mut out.0.endpoints[first..] {
            e.application = application.clone();
        }
        let spring = out.0.endpoints.len() > first
            && (sources.iter()).any(|s| s.text.contains("org.springframework"));
        if spring && application.is_none() {
            // A whole-flow prefix would leak into every other application.
            let file = sources.first().map_or("", |s| s.rel.as_str());
            let message =
                "Role prefix not attributed: Spring sources lie outside every build-file project"
                    .into();
            out.diag(file, 0, message);
        } else if spring {
            out.0
                .role_conventions
                .push(conf.convention(application.clone()));
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
    pub fn read_projects(root: &Path, out: &mut Out) -> BTreeMap<String, Project> {
        read_all(root, out).0
    }

    /// Projects, Razor `.cshtml` files and build-file directories.
    pub(super) type Read = (
        BTreeMap<String, Project>,
        BTreeMap<String, Vec<Source>>,
        BTreeSet<String>,
    );

    /// [`read_projects`] plus the Razor `.cshtml` files by project directory.
    pub(super) fn read_all(root: &Path, out: &mut Out) -> Read {
        let (projects, files, cshtml) = discover(root, out);
        let mut pages: BTreeMap<String, Vec<Source>> = BTreeMap::new();
        for rel in cshtml {
            if let Ok(text) = read_text(root, &rel, MAX_BYTES) {
                pages
                    .entry(project_of(&rel, &projects))
                    .or_default()
                    .push(Source::new(rel, &text));
            }
        }
        let mut by_project: BTreeMap<String, Project> = BTreeMap::new();
        for rel in files {
            match read_text(root, &rel, MAX_BYTES) {
                Ok(text) => by_project
                    .entry(project_of(&rel, &projects))
                    .or_insert_with_key(|dir| {
                        (
                            application(root, dir, &projects),
                            vec![],
                            quarkus(root, dir),
                        )
                    })
                    .1
                    .push(Source::new(rel, &text)),
                Err(e) => out.diag(&rel, 0, format!("Source file not read (limit 1 MiB): {e}")),
            }
        }
        (by_project, pages, projects)
    }

    /// The owning application of a project directory, its sources, and whether
    /// its build file depends on the Quarkus Spring compatibility extensions.
    pub type Project = (Option<String>, Vec<Source>, bool);

    /// Whether a build file of project directory `dir` names a Quarkus Spring
    /// compatibility extension (`quarkus-spring-web`, `quarkus-spring-security`).
    fn quarkus(root: &Path, dir: &str) -> bool {
        let build = |b: &str| match dir {
            "" => b.to_string(),
            _ => format!("{dir}/{b}"),
        };
        (BUILD_FILES.iter()).any(|b| {
            read_text(root, &build(b), MAX_BYTES).is_ok_and(|t| t.contains("quarkus-spring-"))
        })
    }

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
    fn discover(root: &Path, out: &mut Out) -> (BTreeSet<String>, Vec<String>, Vec<String>) {
        let (mut projects, mut files, mut cshtml) = (BTreeSet::new(), Vec::new(), Vec::new());
        let walk = match walk(root) {
            Ok(walk) => walk,
            Err(e) => {
                out.diag(".", 0, format!("Discovery settings not read: {e}"));
                return (projects, files, cshtml);
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
            } else if name.ends_with(".cshtml") && !test_path(&rel) {
                cshtml.push(rel);
            }
        }
        files.sort();
        if files.len() > MAX_FILES {
            let message = format!("More than {MAX_FILES} source files; the rest were not scanned.");
            out.diag(".", 0, message);
            files.truncate(MAX_FILES);
        }
        cshtml.sort();
        (projects, files, cshtml)
    }
}
use projects::read_all;
pub use projects::{Project, read_projects};

/// The lowest major version of the project's `<TargetFramework(s)>` (`net10.0`
/// is 10, `netcoreapp3.1` 3, `net48` 4); none without a `.csproj` or when a
/// framework is unreadable.
fn target_framework(root: &Path, dir: &str) -> Option<u32> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(dir))
        .ok()?
        .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.ends_with(".csproj"))
        .collect();
    names.sort();
    let rel = match dir {
        "" => names.first()?.clone(),
        _ => format!("{dir}/{}", names.first()?),
    };
    let text = read_text(root, &rel, MAX_BYTES).ok()?;
    let value = ["TargetFrameworks", "TargetFramework"]
        .iter()
        .find_map(|tag| {
            let rest = &text[text.find(&format!("<{tag}>"))? + tag.len() + 2..];
            Some(&rest[..rest.find('<')?])
        })?;
    let major = |t: &str| {
        let rest = ["netcoreapp", "netstandard", "net"]
            .iter()
            .find_map(|p| t.trim().strip_prefix(p))?;
        match rest.contains('.') {
            true => rest.split('.').next()?.parse::<u32>().ok(),
            false => Some(4), // `net48`, `net472`: .NET Framework
        }
    };
    value.split(';').map(major).min().flatten()
}

/// Endpoints whose method-security annotations were not applied (their family is
/// not enabled); a filter chain `permitAll` never turns them `Open`.
#[derive(Default)]
struct Ignored {
    /// Set by the annotation reader while it works on one endpoint.
    hit: bool,
    endpoints: BTreeSet<(String, u32)>,
}

pub struct Out(UiFlow, Ignored, aspnet::Defaults);
impl Out {
    /// An empty run, for readers that only want what the shared annotation
    /// reader reports about one element.
    pub fn scratch() -> Out {
        Out(
            UiFlow::default(),
            Ignored::default(),
            aspnet::Defaults::default(),
        )
    }
    pub fn diagnostics(&self) -> &[UiFlowDiagnostic] {
        &self.0.diagnostics
    }
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
    pub(crate) fn at(&mut self, src: &Source, at: usize, message: String) {
        self.diag(&src.rel, src.line(at), message);
    }
    pub(crate) fn unknown(&mut self, src: &Source, at: usize, text: &str) -> Condition {
        self.at(src, at, format!("Access condition not readable: {text}"));
        Condition::Unknown(text.into())
    }
    pub(crate) fn endpoint(
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

fn combine(mut items: Vec<Condition>, many: fn(Vec<Condition>) -> Condition) -> Option<Condition> {
    items.sort();
    items.dedup();
    match items.len() {
        0 => None,
        1 => items.pop(),
        _ => Some(many(items)),
    }
}

pub(crate) fn path(parts: &[&str]) -> String {
    let segments: Vec<&str> = parts
        .iter()
        .flat_map(|p| p.split('/'))
        .filter(|s| !s.is_empty())
        .collect();
    format!("/{}", segments.join("/"))
}

fn has(attrs: &[Attr], name: &str) -> bool {
    attrs.iter().any(|a| a.name == name)
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

    /// Base classes with the files they are declared in.
    type Bases<'a, 's> = Vec<(&'a Source, &'s Decl)>;

    /// What the supertypes of a class contribute: annotation levels `(source,
    /// attributes)` on a method and on the type, gaps, and ambiguous supertypes
    /// that may carry a check.
    #[derive(Default)]
    pub(super) struct Inherited<'a> {
        pub on_method: Vec<(&'a Source, Vec<Attr>)>,
        pub on_type: Vec<(&'a Source, Vec<Attr>)>,
        pub gaps: Vec<String>,
        pub ambiguous: Vec<String>,
    }

    pub struct Index<'a> {
        access: BTreeMap<String, AccessMeaning>,
        /// `Class.Member` → the initializer texts found for it.
        constants: HashMap<String, BTreeSet<String>>,
        classes: HashMap<String, Vec<(&'a Source, Decl)>>,
        /// Java/Kotlin interfaces and `@interface`s by name.
        interfaces: HashMap<String, Vec<(&'a Source, Decl)>>,
        /// Annotated Java/Kotlin methods by declaring type name.
        methods: HashMap<String, Vec<(&'a Source, Decl)>>,
        /// Attribute names a pipeline behaviour reads from requests.
        enforced: BTreeSet<String>,
    }

    impl<'a> Index<'a> {
        pub fn new(
            sources: impl Iterator<Item = &'a Source>,
            access: BTreeMap<String, AccessMeaning>,
        ) -> Self {
            let mut index = Index {
                access,
                constants: HashMap::new(),
                classes: HashMap::new(),
                interfaces: HashMap::new(),
                methods: HashMap::new(),
                enforced: BTreeSet::new(),
            };
            for src in sources {
                let decls = decls(src, !src.rel.ends_with(".cs"));
                if !src.rel.ends_with(".cs") {
                    each_method(src, true, |class, m| {
                        let list = index.methods.entry(class.name.clone()).or_default();
                        list.push((src, m.clone()));
                    });
                    for d in decls.iter().filter(|d| d.class && d.flag) {
                        let list = index.interfaces.entry(d.name.clone()).or_default();
                        list.push((src, d.clone()));
                    }
                }
                let all = constants(src, &decls).into_iter();
                for (key, init) in all.chain(interface_constants(src, &decls)) {
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
        pub(crate) fn text(&self, raw: &str) -> Option<String> {
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
                (true, 1) => {
                    let init = self.qualify(inits.first()?, &[class.to_string()]);
                    self.text_at(&init, depth + 1)
                }
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
            let (found, ambiguous) = self.chain(class);
            if let Some((name, count)) = ambiguous {
                let message = format!(
                    "Base class {name} is ambiguous ({count} classes of that name); its \
                     attributes are not applied to {}.",
                    class.name
                );
                out.note(&src.rel, src.line(class.open), message);
            }
            found
        }

        /// [`Self::ancestors`] without the note: the bases and the ambiguous
        /// base (name, count) that stopped the walk.
        pub(super) fn chain<'s>(
            &'s self,
            class: &Decl,
        ) -> (Bases<'a, 's>, Option<(String, usize)>) {
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
                    return (found, Some((name.clone(), candidates.len())));
                }
                let (s, d) = &candidates[0];
                found.push((*s, d));
                base = d.bases.first();
            }
            (found, None)
        }

        /// Every class of the scan (interfaces excluded).
        pub(super) fn all_classes(&self) -> impl Iterator<Item = (&'a Source, &Decl)> + '_ {
            self.classes.values().flatten().map(|(s, d)| (*s, d))
        }

        /// The one declaration of annotation type `name`, for meta-annotations.
        pub fn meta(&self, name: &str) -> Option<(&'a Source, &Decl)> {
            match self.interfaces.get(name)?.as_slice() {
                [(s, d)] => Some((*s, d)),
                _ => None,
            }
        }

        /// Annotations of the supertypes of `class`, nearest first, as levels: on
        /// its method `method` (with `arity` parameters) and on the type itself.
        pub(super) fn inherited(&self, class: &Decl, method: &str, arity: usize) -> Inherited<'a> {
            let mut up = Inherited::default();
            let mut seen = BTreeSet::from([class.name.clone()]);
            let mut todo: Vec<String> = class.bases.iter().rev().cloned().collect();
            while let Some(name) = todo.pop() {
                if seen.len() > MAX_DEPTH * 4 || !seen.insert(name.clone()) {
                    continue;
                }
                let found: Vec<_> = [&self.classes, &self.interfaces]
                    .into_iter()
                    .filter_map(|m| m.get(&name))
                    .flatten()
                    .collect();
                let own =
                    |s: &Source, m: &Decl, src: &Source| std::ptr::eq(s, src) && m.name == method;
                let [(src, d)] = found.as_slice() else {
                    // Several types of that name: one of them may carry a check.
                    let secured = |a: &Attr| super::spring::family(&a.name).is_some();
                    let any = found.iter().any(|(s, t)| {
                        t.attrs.iter().any(secured)
                            || (self.methods.get(&name).into_iter().flatten())
                                .any(|(ms, m)| own(ms, m, s) && m.attrs.iter().any(secured))
                    });
                    if any {
                        up.ambiguous
                            .push(format!("{name} is declared {} times", found.len()));
                    }
                    continue;
                };
                let same: Vec<_> = (self.methods.get(&name).into_iter().flatten())
                    .filter(|(s, m)| own(s, m, src))
                    .collect();
                let arity_of = |s: &Source, m: &Decl| {
                    s.pair(m.open)
                        .map_or(0, |c| split_top(&s.text[m.open + 1..c]).len())
                };
                let attrs: Vec<Attr> = (same.iter())
                    .filter(|(s, m)| arity_of(s, m) == arity)
                    .flat_map(|(_, m)| m.attrs.clone())
                    .collect();
                if !same.is_empty() && attrs.is_empty() {
                    up.gaps.push(format!("{name}.{method} has overloads with other parameter counts; their annotations are not applied"));
                }
                if !attrs.is_empty() {
                    up.on_method.push((*src, attrs));
                }
                if !d.attrs.is_empty() {
                    up.on_type.push((*src, d.attrs.clone()));
                }
                todo.extend(d.bases.iter().rev().cloned());
            }
            up
        }

        /// The class or interface `name` declared in `src`.
        pub fn type_in(&self, src: &Source, name: &str) -> Option<&Decl> {
            [&self.classes, &self.interfaces]
                .into_iter()
                .filter_map(|m| m.get(name))
                .flatten()
                .find(|(s, _)| std::ptr::eq(*s, src))
                .map(|(_, d)| d)
        }

        /// The annotated methods of `class` declared in `src`.
        pub fn methods_in(&self, src: &Source, class: &str) -> Vec<&Decl> {
            (self.methods.get(class).into_iter().flatten())
                .filter(|(s, _)| std::ptr::eq(*s, src))
                .map(|(_, d)| d)
                .collect()
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

    /// `(string-typed, `String[]`)` of the tokens between `final`/`const` and
    /// `=`; an array is kept as `Class.X[]`, apart from the scalars.
    fn head_kind(head: &str) -> (bool, bool) {
        let tokens: Vec<&str> = (head.split_whitespace())
            .map(|t| t.trim_end_matches(':'))
            .collect();
        let typed = (tokens.iter()).any(|t| matches!(*t, "string" | "String" | "String[]" | "val"));
        (typed, tokens.contains(&"String[]"))
    }

    /// `(Class.Member, initializer)` of each string constant in `src`.
    fn constants(src: &Source, decls: &[Decl]) -> Vec<(String, String)> {
        const NOT_NAMES: [&str; 8] = [
            "static", "string", "String", "String[]", "val", "final", "readonly", "new",
        ];
        let mut found = Vec::new();
        for kw in ["const", "final"] {
            for p in words(&src.text, kw) {
                let from = p + kw.len();
                let Some(eq) = src.text[from..].find('=').map(|e| from + e) else {
                    continue;
                };
                let head = &src.text[from..eq];
                let (typed, array) = head_kind(head);
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
                    statement_end(src, eq + 1)
                };
                found.push((
                    format!("{}.{name}{}", class.name, ["", "[]"][usize::from(array)]),
                    static_qualified(src, src.text[eq + 1..end].trim()),
                ));
            }
        }
        found
    }

    /// Supertype and constant resolution for the route readers.
    mod resolve {
        use super::*;

        impl<'a> Index<'a> {
            /// [`Self::text`] for `raw` written in type `owner` of `src`: a plain
            /// name is a constant of `owner` or a static import of `src`.
            pub(crate) fn text_in(&self, src: &Source, owner: &str, raw: &str) -> Option<String> {
                let mut owners = vec![owner.to_string()];
                let statics = (src.text.lines())
                    .filter_map(|l| l.trim().strip_prefix("import static "))
                    .map(|i| i.trim().trim_end_matches(';').trim());
                owners.extend(statics.filter_map(|i| {
                    let (class, _) = i.rsplit_once('.')?;
                    Some(class.rsplit('.').next()?.to_string())
                }));
                self.text_at(&self.qualify(raw, &owners), 0)
            }

            /// [`Self::text_in`], or the elements of a `String[]` constant of the
            /// scan (`{ A + "/x", B }`); `None` when any part is not readable.
            pub(crate) fn texts_in(
                &self,
                src: &Source,
                owner: &str,
                raw: &str,
            ) -> Option<Vec<String>> {
                if let Some(one) = self.text_in(src, owner, raw) {
                    return Some(vec![one]);
                }
                let mut parts = raw.trim().rsplit('.');
                let (member, class) = (parts.next()?, parts.next().unwrap_or(owner));
                let inits = self.constants.get(&format!("{class}.{member}[]"))?;
                let init = (inits.len() == 1).then(|| inits.first())??;
                let body = init.trim().strip_prefix('{')?.strip_suffix('}')?;
                (split_top(body).into_iter())
                    .filter(|p| !p.trim().is_empty())
                    .map(|p| self.text_in(src, class, p))
                    .collect()
            }

            /// `raw` with each plain name that is a constant of one of `owners`
            /// written `Owner.NAME` (the first owner that has it).
            pub(super) fn qualify(&self, raw: &str, owners: &[String]) -> String {
                let part = |p: &str| {
                    let known = |o: &String| self.constants.contains_key(&format!("{o}.{p}"));
                    match owners.iter().find(|o| known(o)) {
                        Some(o) if p.bytes().all(lex::is_ident) => format!("{o}.{p}"),
                        _ => p.to_string(),
                    }
                };
                split_by(raw, b'+')
                    .iter()
                    .map(|p| part(p))
                    .collect::<Vec<_>>()
                    .join("+")
            }

            /// The supertypes of `class` (transitively, nearest first) found exactly
            /// once in the scan, and the names that are not (several declarations,
            /// or none: outside the scan). JDK types and `Object`-like names carry
            /// no routes and are left out.
            pub(crate) fn supers(
                &self,
                src: &'a Source,
                class: &Decl,
            ) -> (Bases<'a, '_>, Vec<String>) {
                let (mut found, mut unresolved) = (Vec::new(), Vec::new());
                let mut seen = BTreeSet::from([class.name.clone()]);
                let mut todo: Vec<(&Source, String)> =
                    class.bases.iter().rev().map(|b| (src, b.clone())).collect();
                while let Some((from, name)) = todo.pop() {
                    if seen.len() > MAX_DEPTH * 4 || !seen.insert(name.clone()) {
                        continue;
                    }
                    let all: Vec<_> = [&self.classes, &self.interfaces]
                        .into_iter()
                        .filter_map(|m| m.get(&name))
                        .flatten()
                        .collect();
                    // a JDK type, unless the project declares one of that name
                    if all.is_empty() && jdk_type(from, &name) {
                        continue;
                    }
                    // several of that name: the one the subtype's source can name
                    let named: Vec<_> = (all.iter())
                        .filter(|(s, _)| names_interface(&from.text, &s.text, &name))
                        .collect();
                    let [(s, d)] = if all.len() > 1 {
                        named
                    } else {
                        all.iter().collect()
                    }
                    .as_slice() else {
                        unresolved.push(name);
                        continue;
                    };
                    found.push((*s, d));
                    todo.extend(d.bases.iter().rev().map(|b| (*s, b.clone())));
                }
                (found, unresolved)
            }
        }

        /// `init` with each plain name that `src` imports statically
        /// (`import static p.Class.NAME;`) written `Class.NAME`.
        pub(super) fn static_qualified(src: &Source, init: &str) -> String {
            let imports: Vec<&str> = (src.text.lines())
                .filter_map(|l| l.trim().strip_prefix("import static "))
                .map(|i| i.trim().trim_end_matches(';').trim())
                .collect();
            let part = |p: &str| {
                let hit = imports.iter().find(|i| i.rsplit('.').next() == Some(p));
                let class = hit.and_then(|i| i.rsplit('.').nth(1));
                match class {
                    Some(c) if p.bytes().all(lex::is_ident) => format!("{c}.{p}"),
                    _ => p.to_string(),
                }
            };
            split_by(init, b'+')
                .iter()
                .map(|p| part(p))
                .collect::<Vec<_>>()
                .join("+")
        }

        /// `(Interface.NAME, initializer)` of each `String NAME = …;` of an
        /// interface: its fields are constants without `final`.
        pub(super) fn interface_constants(src: &Source, decls: &[Decl]) -> Vec<(String, String)> {
            let mut found = Vec::new();
            for p in words(&src.text, "String") {
                let from = p + "String".len();
                let Some(eq) = src.text[from..].find('=').map(|e| from + e) else {
                    continue;
                };
                let name = src.text[from..eq].trim();
                let owner = (decls.iter())
                    .filter(|d| d.class && d.open < p && p < d.end)
                    .max_by_key(|d| d.open)
                    .filter(|d| d.flag);
                let Some(owner) =
                    owner.filter(|_| !name.is_empty() && name.bytes().all(lex::is_ident))
                else {
                    continue;
                };
                let end = statement_end(src, eq + 1);
                let init = static_qualified(src, src.text[eq + 1..end].trim());
                found.push((format!("{}.{name}", owner.name), init));
            }
            found
        }
    }
    use resolve::{interface_constants, static_qualified};
}
pub use index::Index;

/// ASP.NET Core controllers: attribute routes, else the project's
/// conventional routes ([`conventional`]). A class without its own `[Route]`
/// takes the one of its nearest base class; `[Authorize]`, `[AllowAnonymous]`,
/// `[Area]` and listed attributes of base classes apply too.
mod controllers {
    use super::conventional::Route;
    use super::lex::{named, positional, words};
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
    pub(super) type Attrs<'a> = Vec<(&'a Source, &'a Attr)>;

    pub(super) fn own<'a>(decl: &'a Decl, src: &'a Source) -> Attrs<'a> {
        decl.attrs.iter().map(|a| (src, a)).collect()
    }

    pub(super) fn scan(src: &Source, index: &Index, routes: &[Route], out: &mut Out) {
        let mut methods: BTreeMap<usize, Vec<Decl>> = BTreeMap::new();
        each_method(src, false, |class, d| {
            methods.entry(class.open).or_default().push(d.clone());
        });
        for class in decls(src, false).iter().filter(|d| d.class) {
            if !controller(src, class, &index.chain(class).0) {
                continue;
            }
            let ancestors = index.ancestors(out, src, class);
            let own = methods.get(&class.open).map_or(&[][..], Vec::as_slice);
            for d in own {
                let at = (src, src, class.name.as_str());
                action(at, index, routes, out, (class, &ancestors), d);
            }
            // Public actions of a base class belong to the derived controller too.
            for (base_src, base) in &ancestors {
                let mut inherited = Vec::new();
                each_method(base_src, false, |c, m| {
                    if c.open == base.open && !own.iter().any(|o| o.name == m.name) {
                        inherited.push(m.clone());
                    }
                });
                for d in &inherited {
                    let at = (src, *base_src, base.name.as_str());
                    action(at, index, routes, out, (class, &ancestors), d);
                }
            }
        }
    }

    /// ASP.NET's own rules: a concrete, non-generic class that is not
    /// `[NonController]` and is named `*Controller`, or marked `[ApiController]` /
    /// `[Controller]`, or derives from `ControllerBase` / `Controller` (which carry
    /// `[Controller]`). Attributes and bases count through the base classes.
    fn controller(src: &Source, class: &Decl, ancestors: &[(&Source, &Decl)]) -> bool {
        let header = class_header(src, class);
        let generic = words(header, &class.name)
            .next()
            .is_some_and(|p| header.as_bytes().get(p + class.name.len()) == Some(&b'<'));
        if generic
            || words(header, "abstract").next().is_some()
            || has(&class.attrs, "NonController")
        {
            return false;
        }
        let chain = || std::iter::once(class).chain(ancestors.iter().map(|(_, d)| *d));
        class.name.ends_with("Controller")
            || chain().any(|c| {
                has(&c.attrs, "ApiController")
                    || has(&c.attrs, "Controller")
                    || c.bases
                        .iter()
                        .any(|b| b == "ControllerBase" || b == "Controller")
            })
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

    /// One action `d`, declared in `declaring` (the class or a base class) and
    /// served by the controller `class`.
    fn action(
        (class_src, src, declaring): (&Source, &Source, &str),
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
        let route_level = std::iter::once((class_src, class))
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
        let mut levels = own(class, class_src);
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
        out.link(first, handlers::of_method(src, declaring, &d.name, d.open));
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

    /// `[AllowAnonymous]`, an `open` listed attribute or `MapControllers()
    /// .AllowAnonymous()` wins; otherwise every `[Authorize]`, listed or project
    /// authorization attribute and `MapControllers().RequireAuthorization()` must
    /// hold, the fallback policy only when none of these exists, and every global
    /// `AuthorizeFilter` in addition.
    pub(super) fn access(index: &Index, out: &mut Out, levels: &Attrs) -> AccessState {
        let open = |a: &Attr| a.name == "AllowAnonymous" || index.is_open(a);
        if levels.iter().any(|(_, a)| open(a))
            || out.2.convention.contains(&minimal::Mod::Anonymous)
        {
            return AccessState::Open;
        }
        let mut conditions = Vec::new();
        for (src, a) in levels {
            if a.name == "Authorize" {
                conditions.push(authorize(index, src, out, a));
            } else if let Some(listed) = index.listed(out, src, a) {
                conditions.extend(listed);
            } else {
                let text = |s: &str| index.text(s);
                let attr = aspnet::Use {
                    src,
                    at: a.at,
                    name: &a.name,
                    args: &a.args,
                    mvc: true,
                };
                conditions.extend(aspnet::custom(out, &attr, &text).flatten());
            }
        }
        for m in &out.2.convention {
            if let minimal::Mod::Auth(c) = m {
                conditions.push(c.clone());
            }
        }
        if conditions.is_empty() {
            conditions.extend(out.2.fallback.clone());
        }
        conditions.extend(out.2.filters.clone());
        combine(conditions, Condition::All).map_or(AccessState::Undeclared, AccessState::Protected)
    }

    pub(super) fn authorize(index: &Index, src: &Source, out: &mut Out, a: &Attr) -> Condition {
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
        combine(parts, Condition::All).unwrap_or_else(|| out.2.default_condition())
    }

    /// The modifiers of a minimal-API delegate argument: the attributes of a
    /// lambda, or those of the method a method group (`Handler`,
    /// `Handlers.Get`) names when exactly one such method is in the sources;
    /// else `Unknown` with a gap (an overload, an instance or a local variable
    /// is not guessed).
    pub(super) fn delegate_mods(
        (src, sources, index): (&Source, &[Source], &Index),
        out: &mut Out,
        at: usize,
        arg: &str,
    ) -> Vec<minimal::Mod> {
        let arg = arg.trim();
        // A lambda, or a piece of one cut at a generic comma (`async Task<A, B> (…)`).
        let lambda = arg.contains("=>")
            || arg.starts_with('[')
            || arg.starts_with("async ")
            || arg.starts_with("static ")
            || arg.matches('<').count() > arg.matches('>').count()
            || arg.matches('(').count() > arg.matches(')').count();
        if lambda {
            return lambda_mods(index, src, out, arg);
        }
        let path = arg
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(lex::is_ident));
        let (class, name) = match arg.rsplit_once('.') {
            Some((c, n)) => (c.rsplit('.').next().map(str::to_string), n),
            // An unqualified name binds in the class around the call.
            None => (handlers::enclosing_class(src, at), arg),
        };
        let mut found = if path {
            group_methods(sources, class.as_deref(), name)
        } else {
            vec![]
        };
        match found.pop() {
            Some((s, Some(attrs))) if found.is_empty() => attr_mods(index, s, out, attrs),
            _ => {
                let text =
                    format!("method group {arg} is not a single readable method of the sources");
                vec![minimal::Mod::Auth(out.unknown(src, at, &text))]
            }
        }
    }

    /// The methods named `name` declared in the class `class` (none: outside
    /// any class) with their attributes; `None` when attributes may have been
    /// lost (a `[` before the method in its statement that no declaration carries).
    fn group_methods<'a>(
        sources: &'a [Source],
        class: Option<&str>,
        name: &str,
    ) -> Vec<(&'a Source, Option<Vec<Attr>>)> {
        let mut found = Vec::new();
        for s in sources.iter().filter(|s| s.rel.ends_with(".cs")) {
            let declared = decls(s, false);
            for open in handlers::definitions(s, name) {
                if handlers::enclosing_class(s, open).as_deref() != class {
                    continue;
                }
                let attrs = (declared.iter().find(|d| !d.class && d.open == open))
                    .map(|d| d.attrs.clone())
                    .unwrap_or_default();
                let start = s.text[..open].rfind([';', '{', '}']).map_or(0, |i| i + 1);
                let lost = attrs.is_empty() && s.text[start..open].contains('[');
                found.push((s, (!lost).then_some(attrs)));
            }
        }
        found
    }

    /// `[Authorize]` / `[AllowAnonymous]` before a lambda handler read like
    /// `.RequireAuthorization()` / `.AllowAnonymous()` (dotnet.md C-04).
    pub(super) fn lambda_mods(
        index: &Index,
        src: &Source,
        out: &mut Out,
        delegate: &str,
    ) -> Vec<minimal::Mod> {
        let base = delegate.as_ptr() as usize - src.text.as_ptr() as usize;
        let attrs = directive_attrs(delegate).into_iter().map(|mut a| {
            a.at += base;
            a
        });
        attr_mods(index, src, out, attrs.collect())
    }

    /// The modifiers the attributes `attrs` (read in `src`) stand for.
    pub(super) fn attr_mods(
        index: &Index,
        src: &Source,
        out: &mut Out,
        attrs: Vec<Attr>,
    ) -> Vec<minimal::Mod> {
        let mut mods = Vec::new();
        for a in attrs {
            if a.name == "AllowAnonymous" || index.is_open(&a) {
                mods.push(minimal::Mod::Anonymous);
            } else if a.name == "Authorize" {
                mods.push(minimal::Mod::Auth(authorize(index, src, out, &a)));
            } else if let Some(c) = index.listed(out, src, &a) {
                mods.extend(c.map(minimal::Mod::Auth));
            } else {
                let text = |s: &str| index.text(s);
                let u = aspnet::Use {
                    src,
                    at: a.at,
                    name: &a.name,
                    args: &a.args,
                    mvc: false,
                };
                mods.extend(
                    aspnet::custom(out, &u, &text)
                        .flatten()
                        .map(minimal::Mod::Auth),
                );
            }
        }
        mods
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
    pub(super) struct Chain {
        prefix: Vec<String>,
        pub(super) mods: Vec<Mod>,
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
        let at = (src, sources, index);
        mods.extend(delegate.map_or_else(Vec::new, |d| controllers::delegate_mods(at, out, p, d)));
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
        let access = access(&mods, &out.2);
        out.endpoint(src, p, method, path(&parts), &access);
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

    /// Without any authorization metadata the fallback policy applies.
    fn access(mods: &[Mod], defaults: &aspnet::Defaults) -> AccessState {
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
        match combine(conditions, Condition::All).or_else(|| defaults.fallback.clone()) {
            Some(c) => AccessState::Protected(c),
            None => AccessState::Undeclared,
        }
    }

    fn modifier(src: &Source, out: &mut Out, at: usize, name: &str, args: &str) -> Vec<Mod> {
        match (name, split_top(args)) {
            ("AllowAnonymous", _) => vec![Mod::Anonymous],
            ("RequireAuthorization", parts) if parts.is_empty() => {
                vec![Mod::Auth(out.2.default_condition())]
            }
            ("RequireAuthorization", parts) => (parts.into_iter())
                .filter_map(|p| aspnet::argument(src, out, at, p))
                .collect(),
            _ => vec![],
        }
    }

    /// Modifiers in up to `max` `.X(…)` calls chained after position `close`.
    pub(super) fn trailing(src: &Source, out: &mut Out, close: usize, max: usize) -> Vec<Mod> {
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
    pub(super) fn group(src: &Source, out: &mut Out, end: usize) -> Result<Chain, String> {
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
        let (lo, hi) = method_scope(src, at);
        let assignments = assignments(src, var, lo, hi);
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
            .unwrap_or(hi);
        chain
            .mods
            .extend(statement_mods(src, out, var, semi, until));
        Ok(chain)
    }

    /// The body of the method around `at`: the outermost block whose header is a
    /// parameter list (not a type, namespace or control statement). A local
    /// variable lives only there. Top-level statements: the whole file.
    fn method_scope(src: &Source, at: usize) -> (usize, usize) {
        let b = src.bytes();
        let is_method = |open: usize| {
            let from = b[..open]
                .iter()
                .rposition(|c| matches!(c, b';' | b'{' | b'}'))
                .map_or(0, |p| p + 1);
            let header = src.text[from..open].trim();
            let first = header.split(|c: char| !c.is_ascii_alphanumeric()).next();
            let keywords = [
                "class",
                "struct",
                "interface",
                "record",
                "namespace",
                "enum",
            ];
            let control = [
                "if", "for", "foreach", "while", "switch", "using", "lock", "catch", "fixed",
            ];
            header.ends_with(')')
                && !keywords.iter().any(|k| words(header, k).next().is_some())
                && !first.is_some_and(|f| control.contains(&f))
        };
        (0..at)
            .filter(|&k| b[k] == b'{')
            .filter_map(|k| src.pair(k).filter(|c| *c > at).map(|c| (k, c)))
            .find(|(open, _)| is_method(*open))
            .unwrap_or((0, src.text.len()))
    }

    /// `(start, end of statement)` of each `var = …;` within `lo..hi`.
    fn assignments(src: &Source, var: &str, lo: usize, hi: usize) -> Vec<(usize, usize)> {
        let b = src.bytes();
        words(&src.text, var)
            .filter(|&p| p >= lo && p < hi)
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

/// ASP.NET authorization facts that no single endpoint states, read once per
/// project before its endpoints (A-06, A-07, A-10, B-02, B-03).
///
/// - `DefaultPolicy`: what a bare `[Authorize]` / `RequireAuthorization()`
///   means (`authenticated` when never assigned).
/// - `FallbackPolicy`: applies to an endpoint without any authorization
///   metadata, never together with it.
/// - `Filters.Add(new AuthorizeFilter(…))`: every MVC action also needs it;
///   minimal endpoints are not affected.
/// - `MapControllers()` / `MapControllerRoute(…)` followed by
///   `.RequireAuthorization(…)` / `.AllowAnonymous()`: metadata of every action.
/// - Attribute classes of the scan that derive from `AuthorizeAttribute` or
///   implement `IAuthorizeData` / `IAuthorizationRequirementData`; their
///   constructor and property assignments are read when literal.
///
/// A setting assigned twice, under a condition, or from an expression that is
/// not a policy-builder chain, a single local variable or `o.DefaultPolicy` is
/// `Unknown` with a gap, never absent.
mod aspnet {
    use super::lex::{literal, named_part, skip_ws, skip_ws_back, split_top, words};
    use super::minimal::{self, Mod, call_close};
    use super::*;
    use archguard_flow_clike::source::statement_end;

    const MAX_DEPTH: usize = 8;
    /// Words that make a statement conditional.
    const BRANCHES: [&str; 7] = ["if", "else", "for", "foreach", "while", "switch", "case"];

    #[derive(Default)]
    pub(super) struct Defaults {
        default: Option<Condition>,
        pub(super) fallback: Option<Condition>,
        pub(super) filters: Vec<Condition>,
        pub(super) convention: Vec<Mod>,
        /// Modifiers chained on `MapRazorPages()`.
        pub(super) pages: Vec<Mod>,
        /// Lowest target framework major version; none when unreadable.
        net: Option<u32>,
        classes: BTreeMap<String, AuthClass>,
        /// The framework's own `AuthorizeAttribute`.
        framework: AuthClass,
    }

    impl Defaults {
        /// What a bare authorization datum means.
        pub(super) fn default_condition(&self) -> Condition {
            self.default.clone().unwrap_or(Condition::Authenticated)
        }
    }

    #[derive(Default, Clone)]
    struct Ctor {
        params: Vec<String>,
        base_args: Vec<String>,
        /// `Policy = …;` / `Roles = …;` in the body.
        assigns: Vec<(String, String)>,
    }

    #[derive(Default)]
    struct AuthClass {
        /// Derives from `AuthorizeAttribute`.
        authorize: bool,
        /// Implements `IAuthorizeData`.
        data: bool,
        /// Implements `IAuthorizationRequirementData`.
        requirement: bool,
        /// Implements a filter interface: it also decides in code.
        filter: bool,
        /// Declared more than once under this name.
        ambiguous: bool,
        /// `Policy`/`Roles` appear in the body outside a constructor assignment.
        opaque: bool,
        bases: Vec<String>,
        primary: Option<Ctor>,
        ctors: Vec<Ctor>,
    }

    #[derive(Default)]
    struct Props {
        policy: Option<String>,
        roles: Option<String>,
    }

    pub(super) fn read(sources: &[Source], index: &Index, net: Option<u32>, out: &mut Out) {
        out.2 = Defaults {
            net,
            classes: classes::build(index),
            framework: AuthClass {
                authorize: true,
                ..AuthClass::default()
            },
            ..Defaults::default()
        };
        let cs: Vec<&Source> = sources.iter().filter(|s| s.rel.ends_with(".cs")).collect();
        out.2.default = policy_setting(&cs, index, out, "DefaultPolicy", "SetDefaultPolicy");
        out.2.fallback = policy_setting(&cs, index, out, "FallbackPolicy", "SetFallbackPolicy");
        filters(&cs, index, out);
        conventions(&cs, out);
    }

    /// The value assigned to `name` (or passed to `setter(…)`): none when never
    /// set, `Unknown` when set twice, conditionally or unreadably.
    fn policy_setting(
        cs: &[&Source],
        index: &Index,
        out: &mut Out,
        name: &str,
        setter: &str,
    ) -> Option<Condition> {
        // (source, position, expression span)
        let mut sites: Vec<(&Source, usize, (usize, usize))> = Vec::new();
        for src in cs {
            let b = src.bytes();
            for p in words(&src.text, name) {
                let eq = skip_ws(b, p + name.len());
                let prev = skip_ws_back(b, p);
                // A declaration (`public X DefaultPolicy = …`) is no assignment.
                let declared =
                    prev > 0 && (lex::is_ident(b[prev - 1]) || matches!(b[prev - 1], b'>' | b']'));
                let assign =
                    b.get(eq) == Some(&b'=') && !matches!(b.get(eq + 1), Some(b'=' | b'>'));
                if assign && !declared {
                    sites.push((src, p, first_part(src, eq + 1, statement_end(src, eq + 1))));
                }
            }
            for p in words(&src.text, setter) {
                if let Some(close) = call_close(src, p, setter) {
                    let open = skip_ws(b, p + setter.len());
                    sites.push((src, p, first_part(src, open + 1, close)));
                }
            }
        }
        match sites.as_slice() {
            [] => None,
            [(src, at, span)] if !conditional(src, *at) => {
                Some(eval(src, index, out, *span, name, 0))
            }
            [(src, at, _), ..] => Some(out.unknown(src, *at, name)),
        }
    }

    /// The span of the first top-level argument/expression in `from..to`.
    fn first_part(src: &Source, from: usize, to: usize) -> (usize, usize) {
        let base = src.text.as_ptr() as usize;
        match split_top(&src.text[from..to]).first() {
            Some(part) => {
                let start = part.as_ptr() as usize - base;
                (start, start + part.len())
            }
            None => (from, from),
        }
    }

    /// Whether the statement at `at` runs only under a condition.
    fn conditional(src: &Source, at: usize) -> bool {
        let b = src.bytes();
        let branches = |from: usize, to: usize| {
            BRANCHES
                .iter()
                .any(|w| words(&src.text[from..to], w).next().is_some())
        };
        let head = |to: usize| {
            b[..to]
                .iter()
                .rposition(|c| matches!(c, b';' | b'{' | b'}'))
                .map_or(0, |p| p + 1)
        };
        branches(head(at), at)
            || (0..at)
                .filter(|&k| b[k] == b'{')
                .filter(|&k| src.pair(k).is_some_and(|c| c > at))
                .any(|k| branches(head(k), k))
    }

    /// A policy expression: a builder chain, `o.DefaultPolicy`, or a local
    /// variable assigned once; anything else is `Unknown(label)`.
    fn eval(
        src: &Source,
        index: &Index,
        out: &mut Out,
        (s, e): (usize, usize),
        label: &str,
        depth: usize,
    ) -> Condition {
        let text = src.text[s..e].trim();
        if text == "DefaultPolicy" || text.ends_with(".DefaultPolicy") {
            return out.2.default_condition();
        }
        if text.contains("new AuthorizationPolicyBuilder") {
            return chain(src, index, out, (s, e), label);
        }
        if !text.is_empty() && text.bytes().all(lex::is_ident) && depth < 3 {
            let b = src.bytes();
            let defs: Vec<usize> = words(&src.text, text)
                .filter(|&p| {
                    let eq = skip_ws(b, p + text.len());
                    let prev = skip_ws_back(b, p);
                    b.get(eq) == Some(&b'=')
                        && !matches!(b.get(eq + 1), Some(b'=' | b'>'))
                        && (prev == 0 || b[prev - 1] != b'.')
                })
                .collect();
            if let [p] = defs.as_slice() {
                let eq = skip_ws(b, p + text.len()) + 1;
                let span = first_part(src, eq, statement_end(src, eq));
                return eval(src, index, out, span, label, depth + 1);
            }
        }
        out.unknown(src, s, label)
    }

    /// The requirements of an `AuthorizationPolicyBuilder` chain, all required.
    fn chain(
        src: &Source,
        index: &Index,
        out: &mut Out,
        (s, e): (usize, usize),
        label: &str,
    ) -> Condition {
        let b = src.bytes();
        let (mut items, mut i) = (Vec::new(), s);
        while i < e {
            if let Some(end) = literal_end(b, i) {
                i = end;
                continue;
            }
            match b[i] {
                b'(' | b'[' | b'{' => i = src.pair(i).map_or(i + 1, |c| c + 1),
                b'.' => {
                    let st = skip_ws(b, i + 1);
                    let en = st + b[st..].iter().take_while(|c| lex::is_ident(**c)).count();
                    let open = skip_ws(b, en);
                    let close = (b.get(open) == Some(&b'('))
                        .then(|| src.pair(open))
                        .flatten();
                    match close {
                        Some(c) => {
                            let args = &src.text[open + 1..c];
                            items.extend(step(src, index, out, st, &src.text[st..en], args));
                            i = c + 1;
                        }
                        None => i += 1,
                    }
                }
                _ => i += 1,
            }
        }
        combine(items, Condition::All).unwrap_or_else(|| out.unknown(src, s, label))
    }

    fn step(
        src: &Source,
        index: &Index,
        out: &mut Out,
        at: usize,
        name: &str,
        args: &str,
    ) -> Option<Condition> {
        let parts = split_top(args);
        let texts: Option<Vec<String>> = parts.iter().map(|p| index.text(p)).collect();
        match (name, texts) {
            ("Build" | "AddAuthenticationSchemes", _) => None,
            ("RequireAuthenticatedUser", _) => Some(Condition::Authenticated),
            ("RequireRole", Some(roles)) if !roles.is_empty() => combine(
                roles.into_iter().map(Condition::Role).collect(),
                Condition::Any,
            ),
            ("RequireClaim", Some(t)) if !t.is_empty() => Some(Condition::Custom(match t.len() {
                1 => format!("claim {}", t[0]),
                _ => format!("claim {} = {}", t[0], t[1..].join("|")),
            })),
            ("AddRequirements", _) if !parts.is_empty() => {
                let custom = parts.iter().map(|p| {
                    let p = p.trim();
                    Condition::Custom(p.strip_prefix("new ").unwrap_or(p).trim().to_string())
                });
                combine(custom.collect(), Condition::All)
            }
            _ => Some(out.unknown(src, at, &format!("{name}({})", args.trim()))),
        }
    }

    const NEW_FILTER: &str = "new AuthorizeFilter";

    /// `Filters.Add(new AuthorizeFilter(…))`; any other `AuthorizeFilter` is a gap.
    fn filters(cs: &[&Source], index: &Index, out: &mut Out) {
        for src in cs {
            let mut read = BTreeSet::new();
            for p in words(&src.text, "Filters") {
                if let Some(q) = added_filter(src, p) {
                    read.insert(q + 4);
                    filter_policy(src, index, out, q);
                }
            }
            for p in words(&src.text, "AuthorizeFilter").filter(|p| !read.contains(p)) {
                let message = "AuthorizeFilter outside Filters.Add(new …) not read";
                out.at(src, p, message.into());
            }
        }
    }

    /// The position of `new AuthorizeFilter` inside the `Filters.Add(…)` call
    /// whose receiver word starts at `p`.
    fn added_filter(src: &Source, p: usize) -> Option<usize> {
        let b = src.bytes();
        let dot = skip_ws(b, p + "Filters".len());
        let add = skip_ws(b, dot + 1);
        let open = skip_ws(b, add + 3);
        if b.get(dot) != Some(&b'.')
            || !src.text[add..].starts_with("Add")
            || b.get(open) != Some(&b'(')
        {
            return None;
        }
        let close = src.pair(open)?;
        src.text[open..close].find(NEW_FILTER).map(|q| open + q)
    }

    /// The policy `new AuthorizeFilter(…)` at `q` enforces.
    fn filter_policy(src: &Source, index: &Index, out: &mut Out, q: usize) {
        let b = src.bytes();
        let ctor = skip_ws(b, q + NEW_FILTER.len());
        let end = (b.get(ctor) == Some(&b'('))
            .then(|| src.pair(ctor))
            .flatten();
        let Some(end) = end else {
            return out.at(src, q, "AuthorizeFilter arguments not readable".into());
        };
        let (s, e) = first_part(src, ctor + 1, end);
        let arg = src.text[s..e].trim();
        let policy = match (arg.is_empty(), literal(arg)) {
            (true, _) => out.2.default_condition(),
            (_, Some(name)) => Condition::Policy(name),
            _ => eval(src, index, out, (s, e), arg, 0),
        };
        out.2.filters.push(policy);
    }

    /// The modifiers of the route builder the call `.name(…)` at `p` (closing
    /// at `close`) extends: those before it in the chain and those after it.
    fn call_mods(src: &Source, out: &mut Out, p: usize, close: usize) -> Vec<Mod> {
        let before = skip_ws_back(src.bytes(), p) - 1;
        let mut mods = minimal::group(src, out, before).map_or_else(|_| vec![], |c| c.mods);
        mods.extend(minimal::trailing(src, out, close, usize::MAX));
        mods
    }

    /// Modifiers chained on `MapControllers()` and the other controller routes.
    fn conventions(cs: &[&Source], out: &mut Out) {
        out.2.convention = convention_of(
            cs,
            out,
            &[
                "MapControllers",
                "MapDefaultControllerRoute",
                "MapControllerRoute",
                "MapAreaControllerRoute",
            ],
        );
        // B-03: the same on `MapRazorPages()`.
        out.2.pages = convention_of(cs, out, &["MapRazorPages"]);
    }

    /// The modifiers chained on the `names` calls; `Unknown` when they differ.
    fn convention_of(cs: &[&Source], out: &mut Out, names: &[&str]) -> Vec<Mod> {
        let mut found: Vec<Vec<Mod>> = Vec::new();
        let mut first = None;
        for src in cs {
            for name in names.iter().copied() {
                for p in words(&src.text, name) {
                    let Some(close) = call_close(src, p, name) else {
                        continue;
                    };
                    let mut mods = call_mods(src, out, p, close);
                    mods.sort();
                    mods.dedup();
                    first.get_or_insert((src, p));
                    found.push(mods);
                }
            }
        }
        let same = found.windows(2).all(|w| w[0] == w[1]);
        match (found.first(), first) {
            (Some(mods), _) if same => mods.clone(),
            (Some(_), Some((src, at))) => {
                let c = out.unknown(src, at, &format!("{} conventions differ", names[0]));
                vec![Mod::Auth(c)]
            }
            _ => vec![],
        }
    }

    /// Reads the attribute classes of the scan: which derive from
    /// `AuthorizeAttribute` / implement the authorization interfaces, and their constructors.
    mod classes {
        use super::*;

        #[derive(Default, Clone, Copy)]
        struct Flags {
            authorize: bool,
            data: bool,
            requirement: bool,
            filter: bool,
        }
        type Memo = BTreeMap<String, Flags>;

        fn class_key(src: &Source, d: &Decl) -> String {
            let header = class_header(src, d);
            let generic = words(header, &d.name)
                .next()
                .is_some_and(|p| header.as_bytes().get(p + d.name.len()) == Some(&b'<'));
            match generic {
                true => format!("{}<>", d.name),
                false => d.name.clone(),
            }
        }

        /// `(authorize, data, requirement)` of the class under `key`, through the
        /// project base classes.
        fn flags(bases: &BTreeMap<String, Vec<String>>, key: &str, memo: &mut Memo) -> Flags {
            if let Some(f) = memo.get(key) {
                return *f;
            }
            // A cycle sees the class as not (yet) deriving.
            memo.insert(key.to_string(), Flags::default());
            let mut f = Flags::default();
            for b in bases.get(key).into_iter().flatten() {
                match b.as_str() {
                    "AuthorizeAttribute" => f.authorize = true,
                    "IAuthorizeData" => f.data = true,
                    "IAuthorizationRequirementData" => f.requirement = true,
                    // The framework's filter interfaces decide in code.
                    n if n.ends_with("Filter") || n == "IFilterFactory" => f.filter = true,
                    _ => {
                        for k in [b.clone(), format!("{b}<>")] {
                            let g = flags(bases, &k, memo);
                            f.authorize |= g.authorize;
                            f.data |= g.data;
                            f.requirement |= g.requirement;
                            f.filter |= g.filter;
                        }
                    }
                }
            }
            memo.insert(key.to_string(), f);
            f
        }

        pub(super) fn build(index: &Index) -> BTreeMap<String, AuthClass> {
            let all: Vec<(&Source, &Decl, String)> = index
                .all_classes()
                .filter(|(s, _)| s.rel.ends_with(".cs"))
                .map(|(s, d)| (s, d, class_key(s, d)))
                .collect();
            let mut bases: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for (_, d, key) in &all {
                bases
                    .entry(key.clone())
                    .or_default()
                    .extend(d.bases.clone());
            }
            let mut found: BTreeMap<String, AuthClass> = BTreeMap::new();
            let mut memo = Memo::new();
            for (src, d, key) in all {
                let f = flags(&bases, &key, &mut memo);
                if !(f.authorize || f.data || f.requirement) {
                    continue;
                }
                if let Some(c) = found.get_mut(&key) {
                    c.ambiguous = true;
                    continue;
                }
                let mut c = parse(src, d);
                (c.authorize, c.data, c.requirement, c.filter) =
                    (f.authorize, f.data, f.requirement, f.filter);
                found.insert(key, c);
            }
            found
        }

        /// Constructors and `Policy`/`Roles` assignments of a class.
        fn parse(src: &Source, d: &Decl) -> AuthClass {
            let b = src.bytes();
            let header = class_header(src, d);
            let open = d.open + header.len();
            let mut c = AuthClass {
                bases: d.bases.clone(),
                primary: primary(header, &d.name),
                ..AuthClass::default()
            };
            let mut assigned = 0;
            let mut j = open + 1;
            while b.get(open) == Some(&b'{') && j < d.end && j < b.len() {
                if let Some(end) = literal_end(b, j) {
                    j = end;
                    continue;
                }
                match b[j] {
                    b'{' | b'(' | b'[' => j = src.pair(j).map_or(j + 1, |c| c + 1),
                    ch if lex::is_ident(ch) && (j == 0 || !lex::is_ident(b[j - 1])) => {
                        let e = j + b[j..].iter().take_while(|c| lex::is_ident(**c)).count();
                        let paren = skip_ws(b, e);
                        let (ps, pe) = ident_before(b, skip_ws_back(b, j));
                        let declared = src.text[j..e] == d.name
                            && b.get(paren) == Some(&b'(')
                            && &src.text[ps..pe] != "new"
                            && !(skip_ws_back(b, j) > 0 && b[skip_ws_back(b, j) - 1] == b'.');
                        j = e;
                        if let (true, Some(close)) = (declared, src.pair(paren)) {
                            let (ctor, next, opaque) = constructor(src, paren, close);
                            assigned += ctor.assigns.len();
                            c.opaque |= opaque;
                            c.ctors.push(ctor);
                            j = next;
                        }
                    }
                    _ => j += 1,
                }
            }
            let body = &src.text[open.min(d.end)..d.end.min(src.text.len())];
            // A bare `Policy { get; set; }` (what `IAuthorizeData` demands) is a
            // declaration holding null, not a mention.
            let mentions = (words(body, "Policy").chain(words(body, "Roles")))
                .filter(|&i| !bare_property(&body[i..]))
                .count();
            c.opaque |= mentions > assigned;
            c
        }

        /// `Policy { get; set; }` without initializer; `rest` starts at the name.
        fn bare_property(rest: &str) -> bool {
            let after = rest.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_');
            let Some((inner, tail)) = after
                .trim_start()
                .strip_prefix('{')
                .and_then(|a| a.split_once('}'))
            else {
                return false;
            };
            let inner: String = inner.split_whitespace().collect();
            matches!(inner.as_str(), "get;" | "get;set;" | "get;init;")
                && !tail.trim_start().starts_with('=')
        }

        /// A constructor with its parameter list `(paren, close)`: the parsed
        /// constructor, where the scan goes on, and whether it chains with `this(…)`.
        fn constructor(src: &Source, paren: usize, close: usize) -> (Ctor, usize, bool) {
            let b = src.bytes();
            let mut ctor = Ctor {
                params: params(&src.text[paren + 1..close]),
                ..Ctor::default()
            };
            let mut j = skip_ws(b, close + 1);
            let mut opaque = false;
            if b.get(j) == Some(&b':') {
                let k = skip_ws(b, j + 1);
                let e = k + b[k..].iter().take_while(|c| lex::is_ident(**c)).count();
                let open = skip_ws(b, e);
                let call = (b.get(open) == Some(&b'('))
                    .then(|| src.pair(open))
                    .flatten();
                match (&src.text[k..e], call) {
                    ("base", Some(c)) => {
                        ctor.base_args = split_top(&src.text[open + 1..c])
                            .iter()
                            .map(|a| a.to_string())
                            .collect();
                        j = skip_ws(b, c + 1);
                    }
                    _ => opaque = true,
                }
            }
            let Some(end) = (b.get(j) == Some(&b'{')).then(|| src.pair(j)).flatten() else {
                return (ctor, close + 1, opaque);
            };
            for prop in ["Policy", "Roles"] {
                for p in words(&src.text[j..end], prop).map(|p| p + j) {
                    let eq = skip_ws(b, p + prop.len());
                    let prev = skip_ws_back(b, p);
                    let this = prev > 1
                        && b[prev - 1] == b'.'
                        && src.text[..prev - 1].trim_end().ends_with("this");
                    let plain = prev == 0 || matches!(b[prev - 1], b'{' | b';' | b'}') || this;
                    if plain && b.get(eq) == Some(&b'=') && b.get(eq + 1) != Some(&b'=') {
                        let expr = &src.text[eq + 1..statement_end(src, eq + 1)];
                        ctor.assigns
                            .push((prop.to_string(), expr.trim().to_string()));
                    }
                }
            }
            (ctor, end + 1, opaque)
        }

        /// Parameter names of a parameter list (`string p, int age = 3`).
        fn params(list: &str) -> Vec<String> {
            split_top(list)
                .into_iter()
                .map(|p| {
                    let p = p.split('=').next().unwrap_or("");
                    (p.rsplit(|c: char| !lex::is_ident(c as u8))
                        .find(|s| !s.is_empty()))
                    .unwrap_or("")
                    .to_string()
                })
                .collect()
        }

        /// The primary constructor `class X(int a) : AuthorizeAttribute(p)`.
        fn primary(header: &str, name: &str) -> Option<Ctor> {
            let at = words(header, name).next()? + name.len();
            let rest = header[at..].trim_start();
            let inner = rest.strip_prefix('(')?;
            let src = Source::new(String::new(), rest);
            let close = src.pair(0)?;
            let after = rest[close + 1..].trim_start();
            let base_args = after
                .strip_prefix(':')
                .and_then(|list| {
                    let list = Source::new(String::new(), list);
                    let item = split_top(&list.text)
                        .into_iter()
                        .find(|i| i.contains('('))?;
                    let (_, args) = item.split_once('(')?;
                    Some(
                        split_top(args.strip_suffix(')')?)
                            .iter()
                            .map(|a| a.to_string())
                            .collect(),
                    )
                })
                .unwrap_or_default();
            Some(Ctor {
                params: params(&inner[..close - 1]),
                base_args,
                assigns: vec![],
            })
        }
    }

    /// Evaluates uses of those classes: constructor arguments, properties, requirement data.
    mod uses {
        use super::*;

        impl AuthClass {
            /// The constructor called with `n` arguments (the implicit one for none).
            fn ctor_for(&self, n: usize) -> Option<Ctor> {
                let fits: Vec<&Ctor> = self.ctors.iter().filter(|k| k.params.len() == n).collect();
                match (&self.primary, fits.as_slice()) {
                    (Some(p), _) if p.params.len() == n => Some(p.clone()),
                    (None, [k]) => Some((*k).clone()),
                    (None, []) if n == 0 && self.ctors.is_empty() => Some(Ctor::default()),
                    _ => None,
                }
            }
        }

        impl Ctor {
            /// The value of expression `e` in this constructor called with `args`.
            fn value(&self, args: &[Option<String>], e: &str) -> Option<String> {
                match self.params.iter().position(|p| p == e.trim()) {
                    Some(i) => args[i].clone(),
                    None => literal(e.trim()),
                }
            }
        }

        impl Props {
            fn set(&mut self, prop: &str, value: String) {
                match prop {
                    "Policy" => self.policy = Some(value),
                    _ => self.roles = Some(value),
                }
            }
        }

        /// The framework's `AuthorizeAttribute()` / `AuthorizeAttribute(policy)`.
        fn framework_props(args: &[Option<String>]) -> Option<Props> {
            match args {
                [] => Some(Props::default()),
                [Some(p)] => Some(Props {
                    policy: Some(p.clone()),
                    roles: None,
                }),
                _ => None,
            }
        }

        /// One use of an attribute-like type.
        pub(in crate::ui_flow_backend) struct Use<'a> {
            pub(in crate::ui_flow_backend) src: &'a Source,
            pub(in crate::ui_flow_backend) at: usize,
            /// As written, without `Attribute` (`MinAge`, `Authorize<T>`).
            pub(in crate::ui_flow_backend) name: &'a str,
            pub(in crate::ui_flow_backend) args: &'a str,
            /// On a controller or action: requirement data counts from .NET 11 on.
            pub(in crate::ui_flow_backend) mvc: bool,
        }

        impl Use<'_> {
            fn shown(&self) -> String {
                match self.args.trim() {
                    "" => self.name.to_string(),
                    a => format!("{}({a})", self.name),
                }
            }
        }

        type Text<'a> = &'a dyn Fn(&str) -> Option<String>;
        /// Positional arguments and named (property, value) pairs.
        type Split = (Vec<Option<String>>, Vec<(String, Option<String>)>);

        /// The condition of a use of a project type deriving from
        /// `AuthorizeAttribute` / implementing `IAuthorizeData` /
        /// `IAuthorizationRequirementData` (or of the framework's own `Authorize`
        /// in a `new` expression): `None` when it is none of these, `Some(None)`
        /// when it adds no condition. `text` resolves argument expressions.
        pub(in crate::ui_flow_backend) fn custom(
            out: &mut Out,
            u: &Use,
            text: Text,
        ) -> Option<Option<Condition>> {
            let defaults = std::mem::take(&mut out.2);
            let result = defaults.custom(out, u, text);
            out.2 = defaults;
            result
        }

        /// One argument of `RequireAuthorization(…)`: a policy name, or `new X(…)`
        /// of a project authorization type (no modifier when it adds none).
        pub(in crate::ui_flow_backend) fn argument(
            src: &Source,
            out: &mut Out,
            at: usize,
            p: &str,
        ) -> Option<Mod> {
            if let Some(policy) = literal(p) {
                return Some(Mod::Auth(Condition::Policy(policy)));
            }
            let custom = new_expression(p).and_then(|(name, args)| {
                let u = Use {
                    src,
                    at,
                    name: &name,
                    args: &args,
                    mvc: false,
                };
                custom(out, &u, &|s| literal(s))
            });
            match custom {
                Some(condition) => condition.map(Mod::Auth),
                None => Some(Mod::Auth(out.unknown(src, at, p))),
            }
        }

        /// `new Type(args) { Name = value }`: the type name as written and all arguments.
        fn new_expression(p: &str) -> Option<(String, String)> {
            let rest = p.trim().strip_prefix("new ")?.trim_start();
            let src = Source::new(String::new(), rest);
            let end = rest.find(['(', '{']).unwrap_or(rest.len());
            let (mut args, mut at) = (Vec::new(), end);
            for open in ['(', '{'] {
                if rest[at..].starts_with(open) {
                    let close = src.pair(at)?;
                    args.push(rest[at + 1..close].trim());
                    at = close + 1;
                }
            }
            Some((simple(rest[..end].trim()), args.join(", ")))
        }

        /// Positional constructor arguments and named properties of `args`; none
        /// when a constructor argument is passed by name.
        fn split_arguments(args: &str, text: Text) -> Option<Split> {
            let (mut positional, mut props) = (Vec::new(), Vec::new());
            for part in split_top(args) {
                match named_part(part) {
                    None => positional.push(text(part)),
                    Some((k, v)) if part[k.len()..].trim_start().starts_with('=') => {
                        props.push((k.to_string(), text(v)));
                    }
                    Some(_) => return None,
                }
            }
            Some((positional, props))
        }

        impl Defaults {
            /// `Policy`/`Roles` of the class under `key` constructed with `args`;
            /// none when unreadable.
            fn class_props(
                &self,
                key: &str,
                args: &[Option<String>],
                depth: usize,
            ) -> Option<Props> {
                let Some(c) = self.classes.get(key) else {
                    return framework_props(args);
                };
                if depth > MAX_DEPTH || c.ambiguous || c.opaque {
                    return None;
                }
                let ctor = c.ctor_for(args.len())?;
                let base_args: Vec<Option<String>> =
                    ctor.base_args.iter().map(|e| ctor.value(args, e)).collect();
                let mut props = match self.base_key(c) {
                    Some(k) => self.class_props(&k, &base_args, depth + 1)?,
                    None => Props::default(),
                };
                for (prop, expr) in &ctor.assigns {
                    props.set(prop, ctor.value(args, expr)?);
                }
                Some(props)
            }

            /// The key of the base class of `c` that carries `Policy`/`Roles`.
            fn base_key(&self, c: &AuthClass) -> Option<String> {
                c.bases.iter().find_map(|b| match b.as_str() {
                    "AuthorizeAttribute" => Some(b.clone()),
                    _ => [b.clone(), format!("{b}<>")]
                        .into_iter()
                        .find(|k| self.classes.contains_key(k)),
                })
            }

            /// The class a use of `name` refers to, with its key.
            fn find(&self, name: &str) -> Option<(String, &AuthClass)> {
                let (head, generic) = match name.split_once('<') {
                    Some((head, _)) => (head, true),
                    None => (name, false),
                };
                let keys = match generic {
                    true => [format!("{head}Attribute<>"), format!("{head}<>")],
                    false => [format!("{head}Attribute"), head.to_string()],
                };
                let found = (keys.iter()).find_map(|k| self.classes.get(k).map(|c| (k.clone(), c)));
                found.or_else(|| {
                    (!generic && head == "Authorize")
                        .then(|| ("AuthorizeAttribute".to_string(), &self.framework))
                })
            }

            fn custom(&self, out: &mut Out, u: &Use, text: Text) -> Option<Option<Condition>> {
                let (key, class) = self.find(u.name)?;
                let mut parts = Vec::new();
                if class.authorize || class.data {
                    match self.authorize_part(&key, u, text) {
                        Some(c) => parts.push(c),
                        None => return Some(Some(out.unknown(u.src, u.at, &u.shown()))),
                    }
                }
                if class.filter {
                    parts.push(out.unknown(u.src, u.at, &u.shown()));
                }
                if class.requirement {
                    parts.extend(self.requirement_part(out, u));
                }
                Some(combine(parts, Condition::All))
            }

            /// What the `AuthorizeAttribute` / `IAuthorizeData` side of the type
            /// demands; none when the use is not readable.
            fn authorize_part(&self, key: &str, u: &Use, text: Text) -> Option<Condition> {
                let (positional, named) = split_arguments(u.args, text)?;
                let mut props = self.class_props(key, &positional, 0)?;
                for (k, v) in named {
                    if matches!(k.as_str(), "Policy" | "Roles") {
                        props.set(&k, v?);
                    }
                }
                let roles: Option<Vec<Condition>> = props.roles.map(|r| {
                    (r.split(',').map(str::trim))
                        .filter(|r| !r.is_empty())
                        .map(|r| Condition::Role(r.into()))
                        .collect()
                });
                let items = (roles.and_then(|r| combine(r, Condition::Any)).into_iter())
                    .chain(props.policy.map(Condition::Policy))
                    .collect();
                // No Policy/Roles: the default policy, for `IAuthorizeData` as for
                // `AuthorizeAttribute` (AuthorizationPolicy.CombineAsync, see
                // https://learn.microsoft.com/en-us/aspnet/core/security/authorization/iauthorizedata).
                Some(combine(items, Condition::All).unwrap_or_else(|| self.default_condition()))
            }

            /// Requirement data counts on MVC from .NET 11 on.
            fn requirement_part(&self, out: &mut Out, u: &Use) -> Option<Condition> {
                if !u.mvc || self.net >= Some(11) {
                    return Some(Condition::Custom(u.shown()));
                }
                let why = match self.net {
                    Some(v) => format!("target framework is net{v}"),
                    None => "target framework not readable".into(),
                };
                let message = format!(
                    "{}: requirement data not enforced on MVC before .NET 11 ({why})",
                    u.shown()
                );
                out.at(u.src, u.at, message);
                None
            }
        }
    }
    pub(super) use uses::{Use, argument, custom};
}

/// Spring MVC controllers and Spring Security / JSR-250 annotations.
pub mod spring {
    use super::lex::{literals, named, positional};
    use super::*;

    const VERBS: [(&str, &str); 5] = [
        ("GetMapping", "GET"),
        ("PostMapping", "POST"),
        ("PutMapping", "PUT"),
        ("DeleteMapping", "DELETE"),
        ("PatchMapping", "PATCH"),
    ];

    pub(super) fn scan(src: &Source, index: &Index, conf: &Conf, out: &mut Out) {
        let controllers = inherit::controllers(src, index, out);
        let mut own: HashMap<usize, Vec<Decl>> = HashMap::new();
        each_method(src, true, |class, d| {
            own.entry(class.open).or_default().push(d.clone());
            if controllers.contains(&class.open) {
                method(src, index, conf, out, class, d);
            }
        });
        for class in decls(src, true)
            .iter()
            .filter(|c| controllers.contains(&c.open))
        {
            let own = own.remove(&class.open).unwrap_or_default();
            inherit::routes((src, index, conf), out, class, &own);
        }
    }

    /// Routes a controller inherits from mapped methods of its supertypes
    /// (Spring finds `@RequestMapping` and its variants through the type
    /// hierarchy) that it does not map itself, and a gap for a supertype that
    /// is not in the scan: it may hold more.
    mod inherit {
        use super::*;

        /// The controller being read.
        struct Cx<'a, 'i> {
            src: &'a Source,
            index: &'i Index<'a>,
            class: &'a Decl,
            own: &'a [Decl],
            /// The class-level paths its routes start with.
            bases: Vec<String>,
            /// Where its routes without a method of their own are placed.
            site: usize,
        }

        fn is_controller(d: &Decl) -> bool {
            has(&d.attrs, "RestController") || has(&d.attrs, "Controller")
        }

        fn is_abstract(src: &Source, class: &Decl) -> bool {
            class_header(src, class)
                .split_whitespace()
                .any(|w| w == "abstract")
        }

        /// The concrete classes (by header offset) that are controllers: they
        /// or a supertype carry `@Controller`/`@RestController` (Spring looks
        /// through the type hierarchy). An abstract controller nothing in the
        /// scan extends is a gap: its routes are not emitted.
        pub(super) fn controllers(src: &Source, index: &Index, out: &mut Out) -> BTreeSet<usize> {
            let mut found = BTreeSet::new();
            for c in decls(src, true).iter().filter(|c| c.class && !c.flag) {
                let (supers, _) = index.supers(src, c);
                if !is_controller(c) && !supers.iter().any(|(_, t)| is_controller(t)) {
                    continue;
                }
                if !is_abstract(src, c) {
                    found.insert(c.open);
                } else if !index.all_classes().any(|(s, d)| {
                    d.bases.contains(&c.name) && names_interface(&s.text, &src.text, &c.name)
                }) {
                    let message = format!(
                        "Routes of abstract controller {} not emitted: no concrete subclass in the scan.",
                        c.name
                    );
                    out.at(src, c.attrs.first().map_or(c.open, |a| a.at), message);
                }
            }
            found
        }

        /// The base paths of `class` from a type-level mapping of a supertype.
        pub(super) fn inherited_bases(
            src: &Source,
            index: &Index,
            out: &mut Out,
            class: &Decl,
        ) -> Option<Vec<String>> {
            let (supers, _) = index.supers(src, class);
            base_paths(src, index, out, class, &supers)
        }

        fn arity(src: &Source, d: &Decl) -> usize {
            (src.pair(d.open)).map_or(0, |c| split_top(&src.text[d.open + 1..c]).len())
        }

        fn mapped(d: &Decl) -> bool {
            d.attrs.iter().any(|a| methods(a).is_some())
        }

        fn type_mapping(attrs: &[Attr]) -> Option<&Attr> {
            attrs.iter().find(|a| a.name == "RequestMapping")
        }

        pub(super) fn routes(
            (src, index, conf): (&Source, &Index, &Conf),
            out: &mut Out,
            class: &Decl,
            own: &[Decl],
        ) {
            let (supers, unresolved) = index.supers(src, class);
            let site = class.attrs.first().map_or(class.open, |a| a.at);
            for name in unresolved {
                let message = format!(
                    "A route may be inherited from supertype {name}, which is not in the scan."
                );
                out.at(src, site, message);
            }
            let Some(bases) = base_paths(src, index, out, class, &supers) else {
                return;
            };
            let cx = Cx {
                src,
                index,
                class,
                own,
                bases,
                site,
            };
            let mut seen: BTreeSet<(String, usize)> = (own.iter().filter(|d| mapped(d)))
                .map(|d| (d.name.clone(), arity(src, d)))
                .collect();
            for (ssrc, t) in supers {
                let methods = index.methods_in(ssrc, &t.name);
                for d in methods.into_iter().filter(|d| mapped(d)) {
                    if seen.insert((d.name.clone(), arity(ssrc, d))) {
                        route(&cx, conf, out, (ssrc, t, d));
                    }
                }
            }
        }

        /// The class's own `@RequestMapping` paths, else the nearest
        /// supertype's; none when a path is unreadable (a gap is reported).
        fn base_paths(
            src: &Source,
            index: &Index,
            out: &mut Out,
            class: &Decl,
            supers: &[(&Source, &Decl)],
        ) -> Option<Vec<String>> {
            if let Some(a) = type_mapping(&class.attrs) {
                return paths(src, index, out, &class.name, a);
            }
            let mut up = supers
                .iter()
                .filter_map(|(s, t)| type_mapping(&t.attrs).map(|a| (*s, *t, a)));
            let Some((s, t, a)) = up.next() else {
                return Some(vec![String::new()]);
            };
            // Spring's search order (interfaces before the superclass) is not
            // modelled: differing type-level mappings are not guessed.
            if up.any(|(_, _, b)| b.args != a.args) {
                let message = format!(
                    "Several supertypes of {} carry a type-level @RequestMapping; its routes are not read.",
                    class.name
                );
                let at = class.attrs.first().map_or(class.open, |x| x.at);
                out.at(src, at, message);
                return None;
            }
            paths(s, index, out, &t.name, a)
        }

        /// The endpoints of supertype method `d` of `t` (in `ssrc`).
        fn route(cx: &Cx, conf: &Conf, out: &mut Out, (ssrc, t, d): (&Source, &Decl, &Decl)) {
            let count = arity(ssrc, d);
            // the class's own method (location, annotations), if it has any
            let mine = (cx.own.iter()).find(|m| m.name == d.name && arity(cx.src, m) == count);
            let own_decl = mine.cloned().unwrap_or_else(|| Decl {
                class: false,
                name: d.name.clone(),
                attrs: vec![],
                open: cx.class.open,
                end: 0,
                bases: vec![],
                flag: true,
            });
            let declared = (cx.src, cx.class, &own_decl);
            let (access, ignored) = declared_access(conf, cx.index, out, declared, count);
            let at = mine.map_or(cx.site, |m| m.open);
            let first = out.0.endpoints.len();
            for a in &d.attrs {
                for (p, m) in mappings(cx, out, (ssrc, t), a) {
                    out.endpoint(cx.src, at, &m, p, &access);
                    if ignored {
                        out.1
                            .endpoints
                            .insert((cx.src.rel.clone(), cx.src.line(at)));
                    }
                }
            }
            let open = mine.map_or(cx.class.open, |m| m.open);
            let link = handlers::of_method(cx.src, &cx.class.name, &d.name, open);
            out.link(first, link);
        }

        /// `(path, HTTP method)` of mapping annotation `a` below the class paths.
        fn mappings(
            cx: &Cx,
            out: &mut Out,
            (ssrc, t): (&Source, &Decl),
            a: &Attr,
        ) -> Vec<(String, String)> {
            let Some(verbs) = methods(a) else {
                return vec![];
            };
            let Some(found) = paths(ssrc, cx.index, out, &t.name, a) else {
                return vec![];
            };
            let full = (cx.bases.iter()).flat_map(|b| found.iter().map(move |p| path(&[b, p])));
            full.flat_map(|p| verbs.iter().map(move |m| (p.clone(), m.clone())))
                .collect()
        }
    }

    pub(super) use chain_apps::{Apps, central};
    pub use conf::Conf;
    use conf::{Atom, Flag};

    /// The conventions and method-security switches of ONE application as its
    /// sources state them (ADR 0021): a default is a framework default and says
    /// so in the convention's origins; code that cannot be read is a gap.
    mod conf {
        use super::super::lex::{named, skip_ws, words};
        use super::super::*;

        #[derive(Clone, Copy, PartialEq, Eq)]
        pub(in super::super) enum Flag {
            On,
            Off,
            Unreadable,
        }
        impl Flag {
            fn join(self, other: Flag) -> Flag {
                match (self, other) {
                    (Flag::On, _) | (_, Flag::On) => Flag::On,
                    (Flag::Unreadable, _) | (_, Flag::Unreadable) => Flag::Unreadable,
                    _ => Flag::Off,
                }
            }
        }

        /// How a name written in a security expression becomes a condition.
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub(in super::super) enum Atom {
            /// `hasRole('A')`: the role prefix is added unless it is there.
            Role,
            /// `@RolesAllowed("A")`: the role prefix is always added.
            Jsr250,
            /// `hasAuthority('A')`, `@Secured("A")`: exact; a role when it carries
            /// the role prefix, otherwise a permission.
            Authority,
            /// OAuth2 `hasScope('s')`.
            Scope,
        }

        #[derive(Clone)]
        struct Methods {
            legacy: bool,
            pre_post: Flag,
            secured: Flag,
            jsr250: Flag,
        }

        /// What of a project's conventions the chain rules are read with.
        pub(in super::super) type ConfKey = (bool, Option<String>, Option<String>, Option<String>);

        #[derive(Clone)]
        pub struct Conf {
            quarkus: bool,
            /// `None`: set by code that is not readable.
            role_prefix: Option<String>,
            scope_prefix: Option<String>,
            role_claim: Option<String>,
            origins: Vec<ConventionOrigin>,
            /// `None`: no enabling annotation read.
            methods: Option<Methods>,
            /// `AnnotationTemplateExpressionDefaults` is in the sources.
            pub(in super::super) templates: bool,
            /// Service layers: `@PostAuthorize` is no check before the call.
            pub deferred_off: bool,
        }

        /// `(source, offset, arguments)` of every call of `name` in the sources.
        fn call_args<'s>(
            sources: &'s [Source],
            name: &'static str,
        ) -> Vec<(&'s Source, usize, &'s str)> {
            (sources.iter())
                .filter(|s| !s.rel.ends_with(".cs"))
                .flat_map(|s| {
                    words(&s.text, name).filter_map(move |p| {
                        let open = skip_ws(s.bytes(), p + name.len());
                        let close = (s.bytes().get(open) == Some(&b'('))
                            .then(|| s.pair(open))
                            .flatten()?;
                        Some((s, p, &s.text[open + 1..close]))
                    })
                })
                .collect()
        }

        /// What `calls` give for one setting.
        enum Setting<'s> {
            Unset,
            One(String, &'s Source, usize),
            /// A call whose argument is not a literal, or calls that disagree.
            Bad(&'s Source, usize, String),
        }

        fn setting<'s>(index: &Index, calls: &[(&'s Source, usize, &str)]) -> Setting<'s> {
            let mut found: Option<(String, &Source, usize)> = None;
            for &(s, at, args) in calls {
                let Some(v) = index.text(args.trim()) else {
                    return Setting::Bad(s, at, format!("`{}` is not a literal", args.trim()));
                };
                match &found {
                    Some((f, ..)) if *f != v => {
                        return Setting::Bad(s, at, format!("it is set to `{f}` and to `{v}`"));
                    }
                    Some(_) => {}
                    None => found = Some((v, s, at)),
                }
            }
            match found {
                Some((v, s, at)) => Setting::One(v, s, at),
                None => Setting::Unset,
            }
        }

        impl Conf {
            pub(in super::super) fn key(&self) -> ConfKey {
                (
                    self.quarkus,
                    self.role_prefix.clone(),
                    self.scope_prefix.clone(),
                    self.role_claim.clone(),
                )
            }

            pub fn read(sources: &[Source], index: &Index, quarkus: bool, out: &mut Out) -> Conf {
                let templates = (sources.iter())
                    .any(|s| s.text.contains("AnnotationTemplateExpressionDefaults"));
                let mut conf = Conf {
                    quarkus,
                    role_prefix: None,
                    scope_prefix: None,
                    role_claim: None,
                    origins: vec![],
                    methods: None,
                    templates,
                    deferred_off: false,
                };
                conf.prefixes(sources, index, out);
                conf.method_security(sources, out);
                conf
            }

            fn origin(&mut self, field: &str, at: Option<(&Source, usize)>, source: &str) {
                self.origins.push(ConventionOrigin {
                    field: field.into(),
                    file: at.map(|(s, _)| s.rel.clone()).unwrap_or_default(),
                    line: at.map_or(0, |(s, p)| s.line(p)),
                    source: source.into(),
                });
            }

            /// `GrantedAuthorityDefaults(..)`, `setAuthorityPrefix(..)` and
            /// `setAuthoritiesClaimName(..)` with literal arguments.
            fn prefixes(&mut self, sources: &[Source], index: &Index, out: &mut Out) {
                if self.quarkus {
                    self.role_prefix = Some(String::new());
                    let note = "quarkus spring-security compatibility: role names are exact (not verified)";
                    return self.origin("role_prefix", None, note);
                }
                let default = "framework default spring-security";
                // `None`: not set; `Some(None)`: set but not readable
                let mut one =
                    |this: &mut Conf, field: &str, call: &'static str, what: &str| match setting(
                        index,
                        &call_args(sources, call),
                    ) {
                        Setting::Unset => None,
                        Setting::One(v, s, at) => {
                            this.origin(field, Some((s, at)), call);
                            Some(Some(v))
                        }
                        Setting::Bad(s, at, why) => {
                            out.at(s, at, format!("`{call}` not read ({why}): {what}."));
                            Some(None)
                        }
                    };
                let role = one(
                    self,
                    "role_prefix",
                    "GrantedAuthorityDefaults",
                    "role names are unknown here",
                );
                let scope = one(
                    self,
                    "scope_prefix",
                    "setAuthorityPrefix",
                    "the scope prefix is unknown",
                );
                let claim = one(
                    self,
                    "role_claim",
                    "setAuthoritiesClaimName",
                    "the role claim is unknown",
                );
                match role {
                    None => {
                        self.role_prefix = Some("ROLE_".into());
                        let source = format!("{default} GrantedAuthorityDefaults");
                        self.origin("role_prefix", None, &source);
                    }
                    Some(v) => self.role_prefix = v,
                }
                match scope {
                    None => {
                        self.scope_prefix = Some("SCOPE_".into());
                        let source = format!("{default} JwtGrantedAuthoritiesConverter");
                        self.origin("scope_prefix", None, &source);
                    }
                    Some(v) => self.scope_prefix = v,
                }
                self.role_claim = claim.flatten();
            }

            /// The switches of `@EnableMethodSecurity` (pre-post on by default) and
            /// the legacy `@EnableGlobalMethodSecurity` (all off by default).
            fn method_security(&mut self, sources: &[Source], out: &mut Out) {
                for src in sources.iter().filter(|s| !s.rel.ends_with(".cs")) {
                    for a in (decls(src, true).iter())
                        .filter(|d| d.class)
                        .flat_map(|d| &d.attrs)
                    {
                        let (legacy, on) = match a.name.as_str() {
                            "EnableMethodSecurity" | "EnableReactiveMethodSecurity" => {
                                (false, Flag::On)
                            }
                            "EnableGlobalMethodSecurity" => (true, Flag::Off),
                            _ => continue,
                        };
                        let mut flag = |key: &str, default: Flag| match named(&a.args, key) {
                            None => default,
                            Some("true") => Flag::On,
                            Some("false") => Flag::Off,
                            Some(_) => {
                                let message = format!(
                                    "`@{}` attribute `{key}` is not a literal boolean; annotations of that family are unknown.",
                                    a.name
                                );
                                out.at(src, a.at, message);
                                Flag::Unreadable
                            }
                        };
                        let one = Methods {
                            legacy,
                            pre_post: flag("prePostEnabled", on),
                            secured: flag("securedEnabled", Flag::Off),
                            jsr250: flag("jsr250Enabled", Flag::Off),
                        };
                        self.methods = Some(match self.methods.take() {
                            None => one,
                            Some(m) => Methods {
                                legacy: m.legacy && one.legacy,
                                pre_post: m.pre_post.join(one.pre_post),
                                secured: m.secured.join(one.secured),
                                jsr250: m.jsr250.join(one.jsr250),
                            },
                        });
                    }
                }
            }

            pub(in super::super) fn convention(
                &self,
                application: Option<String>,
            ) -> RoleConvention {
                RoleConvention {
                    application,
                    role_prefix: self.role_prefix.clone(),
                    scope_prefix: self.scope_prefix.clone(),
                    role_claim: self.role_claim.clone(),
                    origins: self.origins.clone(),
                    ..RoleConvention::default()
                }
            }

            /// The condition of a name written in a security expression.
            pub(in super::super) fn atom(&self, kind: Atom, name: &str) -> Condition {
                if kind == Atom::Scope {
                    return Condition::Policy(name.into());
                }
                let Some(p) = self.role_prefix.as_deref() else {
                    return Condition::Unknown(format!("{name}: the role prefix is not readable"));
                };
                match kind {
                    Atom::Role if name.starts_with(p) => Condition::Role(name.into()),
                    // legacy JSR-250 adds the prefix only when it is missing
                    Atom::Jsr250 if self.legacy() && name.starts_with(p) => {
                        Condition::Role(name.into())
                    }
                    Atom::Role | Atom::Jsr250 => Condition::Role(format!("{p}{name}")),
                    Atom::Authority if name.starts_with(p) => Condition::Role(name.into()),
                    _ => Condition::Policy(name.into()),
                }
            }

            /// Whether the annotations of `family` are enforced.
            pub(in super::super) fn flag(&self, family: &str) -> Flag {
                if self.deferred_off && family == "PostAuthorize" {
                    return Flag::Off;
                }
                let Some(m) = &self.methods else {
                    return if self.quarkus { Flag::On } else { Flag::Off };
                };
                match family {
                    "PreAuthorize" | "PostAuthorize" => m.pre_post,
                    "Secured" => m.secured,
                    "jsr250" => m.jsr250,
                    _ => Flag::On,
                }
            }

            /// Legacy method security applies the first family with annotations.
            pub(in super::super) fn legacy(&self) -> bool {
                self.methods.as_ref().is_some_and(|m| m.legacy)
            }

            /// Why the annotations of `family` are not enforced, for a gap.
            pub(in super::super) fn off(&self, family: &str) -> String {
                if self.deferred_off && family == "PostAuthorize" {
                    return "it is checked after the call, so it is not a check before it".into();
                }
                let switch = match family {
                    "PreAuthorize" | "PostAuthorize" => "prePostEnabled",
                    "Secured" => "securedEnabled",
                    _ => "jsr250Enabled",
                };
                match self.methods {
                    None => "method security not enabled: no `@EnableMethodSecurity` or `@EnableGlobalMethodSecurity` is read in this project (it may be XML or in a library)".into(),
                    Some(_) => format!("method security not enabled: `{switch}` is not read as true"),
                }
            }
        }
    }

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
                    (true, "authorizeRequests" | "authorizeHttpRequests" | "authorizeExchange")
                        if empty =>
                    {
                        return true;
                    }
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
    /// Why the chains of a project are not fully read.
    mod chain_why {
        use super::super::lex::{skip_ws, skip_ws_back};
        use super::super::*;
        use super::chain_span::{
            HTTP_TYPES, enclosing_open, hand_off, mentions_http, scoping_call, word_before,
        };

        /// `Class.method` of the helpers `static void method(HttpSecurity h)` of
        /// the scan whose body only calls `h.addFilter…(…)`: handing the
        /// `HttpSecurity` to them cannot change scope or rules. A name declared
        /// twice counts only when every declaration is such a helper.
        pub(super) fn filter_helpers(sources: &[&Source]) -> BTreeSet<String> {
            let (mut good, mut bad) = (BTreeSet::new(), BTreeSet::new());
            for src in sources.iter().copied().filter(|s| s.rel.ends_with(".java")) {
                let classes: Vec<Decl> = decls(src, true).into_iter().filter(|d| d.class).collect();
                for p in HTTP_TYPES.iter().flat_map(|t| lex::words(&src.text, t)) {
                    match helper_at(src, &classes, p) {
                        Some((key, true)) => good.insert(key),
                        Some((key, false)) => bad.insert(key),
                        None => continue,
                    };
                }
            }
            good.difference(&bad).cloned().collect()
        }

        /// The method declared around the `HttpSecurity` parameter type at `p`,
        /// if its only parameter is that: `("Class.method", only adds filters)`.
        fn helper_at(src: &Source, classes: &[Decl], p: usize) -> Option<(String, bool)> {
            let b = src.bytes();
            let open = enclosing_open(src, p)?;
            let close = src.pair(open)?;
            let params = lex::split_top(&src.text[open + 1..close]);
            let [param] = &params[..] else { return None };
            let from = close + 1 + src.text[close + 1..].find('{')?;
            let (var, to) = (param.split_whitespace().last()?, src.pair(from)?);
            // between `)` and `{` only `throws X` is a method declaration
            let gap = src.text[close + 1..from].trim();
            let class = (classes.iter())
                .filter(|c| c.open < open && open < c.end)
                .max_by_key(|c| c.open)?;
            if !(gap.is_empty() || gap.starts_with("throws")) {
                return None;
            }
            let key = format!("{}.{}", class.name, word_before(src, skip_ws_back(b, open)));
            let only_filters = lex::words(&src.text[from..to], var).all(|q| {
                let at = skip_ws(b, from + q + var.len());
                src.text[at..].starts_with(".addFilter")
            });
            Some((key, only_filters))
        }

        /// What keeps the chains from being read, project wide.
        pub(super) struct Why {
            /// A scoping call in a file that holds no chain (configurers,
            /// extensions, helpers) that is not one of the `read` scopes
            /// `(file, from, to)`: it may scope any chain.
            pub(super) project: Option<String>,
            helpers: BTreeSet<String>,
        }

        fn site(src: &Source, p: usize) -> String {
            format!("{}:{}", src.rel, src.line(p))
        }

        fn scoping_site(src: &Source, read: &[(usize, usize)]) -> Option<String> {
            let lambdas = super::chain::auth_lambdas(src);
            scoping_call(src, &lambdas, read)
                .map(|p| format!("A scoping matcher at {}", site(src, p)))
        }

        pub(super) fn why_not_evaluated(
            sources: &[&Source],
            files: &[&Source],
            read: &[(String, usize, usize)],
        ) -> Why {
            let in_files = |s: &Source| files.iter().any(|f| f.rel == s.rel);
            let project = (sources.iter().copied())
                .filter(|s| !s.rel.ends_with(".cs") && mentions_http(&s.text) && !in_files(s))
                .find_map(|s| scoping_site(s, &ranges_of(read, &s.rel)));
            Why {
                project,
                helpers: filter_helpers(sources),
            }
        }

        fn ranges_of(read: &[(String, usize, usize)], rel: &str) -> Vec<(usize, usize)> {
            (read.iter())
                .filter(|r| r.0 == rel)
                .map(|r| (r.1, r.2))
                .collect()
        }

        impl Why {
            /// Why the chain of `src` in `own` is not fully read, and whether its
            /// scope is lost: a scoping call that is not one of the `read` scopes
            /// (lost), or the `HttpSecurity` handed out (code unknown). The
            /// `others` (the file's other chains) are not looked at.
            pub(super) fn unit(
                &self,
                src: &Source,
                others: &[(usize, usize)],
                read: &[(String, usize, usize)],
            ) -> Option<(String, bool)> {
                let mut skip = ranges_of(read, &src.rel);
                skip.extend_from_slice(others);
                if let Some(why) = scoping_site(src, &skip) {
                    return Some((why, true));
                }
                let at = hand_off(src, others, &self.helpers)?;
                Some((
                    format!("Code at {} hands the HttpSecurity out", site(src, at)),
                    false,
                ))
            }
        }
    }

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

        /// The servlet and the WebFlux security builder.
        pub(super) const HTTP_TYPES: [&str; 2] = ["HttpSecurity", "ServerHttpSecurity"];

        /// Whether `text` mentions a security builder type.
        pub(super) fn mentions_http(text: &str) -> bool {
            HTTP_TYPES
                .iter()
                .any(|t| lex::words(text, t).next().is_some())
        }

        /// Names of the `HttpSecurity` parameters (`HttpSecurity http`, `http: HttpSecurity`).
        pub(super) fn http_vars(src: &Source) -> Vec<String> {
            let mut vars = Vec::new();
            for p in HTTP_TYPES.iter().flat_map(|t| lex::words(&src.text, t)) {
                let Some(open) = enclosing_open(src, p) else {
                    continue;
                };
                let Some(close) = src.pair(open) else {
                    continue;
                };
                for part in lex::split_top(&src.text[open + 1..close]) {
                    if !mentions_http(part) {
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
        pub(super) const SCOPING: [&str; 11] = [
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
            "pathMatchers",
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
            read: &[(usize, usize)],
        ) -> Option<usize> {
            let b = src.bytes();
            (0..b.len()).find(|&s| {
                let e = ident_end(src, s);
                !in_ranges(read, s)
                    && (s == 0 || !lex::is_ident(b[s - 1]))
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
                && HTTP_TYPES
                    .iter()
                    .any(|t| src.text[skip_ws(b, end) + 1..].trim_start().starts_with(t));
            typed || HTTP_TYPES.contains(&word_before(src, skip_ws_back(b, p)))
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
        fn var_hand_off(
            src: &Source,
            inside: &[(usize, usize)],
            vars: &[String],
            helpers: &BTreeSet<String>,
        ) -> Option<usize> {
            let strings = literal_spans(src);
            vars.iter().find_map(|var| {
                lex::words(&src.text, var).find(|&p| {
                    let end = p + var.len();
                    !in_ranges(inside, p)
                        && !in_plain_string(src, &strings, p)
                        && !is_declaration(src, p, end)
                        && !is_receiver(src, end)
                        && scope_block(src, p, end).is_none()
                        && !is_filter_helper_arg(src, p, end, helpers)
                })
            })
        }

        /// `Class.method(http)` where `Class.method` is in `helpers`.
        fn is_filter_helper_arg(
            src: &Source,
            p: usize,
            end: usize,
            helpers: &BTreeSet<String>,
        ) -> bool {
            let b = src.bytes();
            let Some(open) = enclosing_open(src, p) else {
                return false;
            };
            let sole = src
                .pair(open)
                .is_some_and(|c| skip_ws(b, open + 1) == p && skip_ws(b, end) == c);
            let name_end = skip_ws_back(b, open);
            let name = word_before(src, name_end);
            let dot = skip_ws_back(b, name_end - name.len());
            let class = word_before(src, skip_ws_back(b, dot.saturating_sub(1)));
            sole && dot > 0 && b[dot - 1] == b'.' && helpers.contains(&format!("{class}.{name}"))
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
        pub(super) fn hand_off(
            src: &Source,
            inside: &[(usize, usize)],
            helpers: &BTreeSet<String>,
        ) -> Option<usize> {
            let vars = http_vars(src);
            let blocks: Vec<(usize, usize)> = (vars.iter())
                .flat_map(|v| lex::words(&src.text, v).map(move |p| (p, v.len())))
                .filter_map(|(p, len)| scope_block(src, p, p + len))
                .collect();
            var_hand_off(src, inside, &vars, helpers)
                .or_else(|| call_hand_off(src, inside))
                .or_else(|| block_hand_off(src, inside, &blocks))
        }

        /// End of the call chain starting at `from`: the `;`, or (Kotlin has none) a
        /// line end outside brackets whose next code line does not continue the
        /// chain with a leading `.` (comment lines skipped; a trailing `.` continues).
        pub(super) fn chain_end(src: &Source, from: usize) -> usize {
            let b = src.bytes();
            let end = statement_end(src, from);
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

    /// Several security configurations as a sequence: each one's `@Order` and
    /// literal scope, and which of them decides an endpoint.
    mod sequence {
        use super::super::lex::{named, positional, skip_ws, skip_ws_back, split_top, words};
        use super::super::*;
        use super::chain::{
            CHAIN_TYPES, Hit, Rule, authorize_sites, bean, calls, conditional, is_adapter, matches,
        };

        /// One security configuration: a `SecurityFilterChain` bean method, a
        /// legacy adapter class, or, when a file has neither but reads
        /// authorization rules (Kotlin beans), the whole file.
        pub(super) struct Unit<'s> {
            pub(super) src: &'s Source,
            pub(super) name: String,
            /// Text range holding the unit.
            pub(super) span: (usize, usize),
            pub(super) adapter: bool,
            /// `None`: the `@Order` is not readable.
            pub(super) order: Option<i64>,
            /// `None`: the chain takes every request.
            pub(super) scope: Option<Rule>,
            /// Why code outside the readable text may change this chain.
            pub(super) opaque: Option<String>,
            pub(super) rules: Vec<Rule>,
        }

        impl Unit<'_> {
            /// Takes any request, as far as is known: no readable scope, and
            /// code that may have set one.
            fn barrier(&self) -> bool {
                self.opaque.is_some() && self.scope.is_none()
            }
        }

        const ADAPTER_ORDER: i64 = 100;
        const LOWEST: i64 = i32::MAX as i64;

        pub(super) const UNORDERED: &str =
            "Several security configurations without a readable, distinct `@Order`";

        /// Adapters next to chain beans: their order to each other is not read.
        pub(super) fn mixed(units: &[Unit]) -> bool {
            units.iter().any(|u| u.adapter) && units.iter().any(|u| !u.adapter)
        }

        /// Whether several configurations leave Spring's choice open: equal or
        /// unreadable orders. Only requests that two of them may match are
        /// affected (see [`apply`]).
        pub(super) fn ties(units: &[Unit]) -> bool {
            let orders: BTreeSet<Option<i64>> = units.iter().map(|u| u.order).collect();
            units.len() > 1 && (orders.contains(&None) || orders.len() < units.len())
        }

        /// The units of `src`: its adapters, its `SecurityFilterChain` bean methods.
        pub(super) fn units(src: &Source) -> Vec<Unit<'_>> {
            let all = decls(src, true);
            let mut units: Vec<Unit> = all
                .iter()
                .filter(|d| d.class && is_adapter(d))
                .map(|d| Unit {
                    src,
                    name: d.name.clone(),
                    span: (d.open, d.end.max(d.open) + 1),
                    adapter: true,
                    order: order(&d.attrs, ADAPTER_ORDER),
                    scope: None,
                    opaque: None,
                    rules: vec![],
                })
                .collect();
            let beans = CHAIN_TYPES.iter().flat_map(|t| {
                words(&src.text, t)
                    .filter(|p| bean(src, *p, t.len()))
                    .map(move |p| (p, t.len()))
            });
            for (p, len) in beans {
                let b = src.bytes();
                let name_end = super::chain_span::ident_end(src, skip_ws(b, p + len));
                let open = skip_ws(b, name_end);
                let close = src.pair(open).unwrap_or(open);
                let body = src.text[close..].find(['{', ';']).map(|k| k + close);
                let end = match body {
                    Some(k) if b[k] == b'{' => src.pair(k).unwrap_or(k),
                    _ => close,
                };
                let attrs = all.iter().find(|d| !d.class && d.open == open);
                let name = src.text[skip_ws(b, p + len)..name_end].to_string();
                units.push(Unit {
                    src,
                    name,
                    span: (p, end + 1),
                    adapter: false,
                    order: order(attrs.map_or(&[][..], |d| &d.attrs[..]), LOWEST),
                    scope: None,
                    opaque: None,
                    rules: vec![],
                });
            }
            if units.is_empty() && !authorize_sites(src).is_empty() {
                units.push(Unit {
                    src,
                    name: src.rel.clone(),
                    span: (0, src.text.len()),
                    adapter: false,
                    order: None,
                    scope: None,
                    opaque: None,
                    rules: vec![],
                });
            }
            units
        }

        /// `@Order(n)`, `Ordered.HIGHEST_PRECEDENCE` / `LOWEST_PRECEDENCE`; `default`
        /// without the annotation; `None` for anything else.
        fn order(attrs: &[Attr], default: i64) -> Option<i64> {
            let Some(a) = attrs.iter().find(|a| a.name == "Order") else {
                return Some(default);
            };
            match named(&a.args, "value")
                .or_else(|| positional(&a.args))
                .map(str::trim)
            {
                None => Some(LOWEST),
                Some("Ordered.HIGHEST_PRECEDENCE") => Some(i32::MIN as i64),
                Some("Ordered.LOWEST_PRECEDENCE") => Some(LOWEST),
                Some(t) => t.parse().ok(),
            }
        }

        /// Byte offset of `part`, a slice of `src.text`.
        fn offset(src: &Source, part: &str) -> usize {
            part.as_ptr() as usize - src.text.as_ptr() as usize
        }

        /// A scope or rule matcher with literal paths, as a [`Rule`] without grant.
        fn literal_matcher(
            src: &Source,
            index: &Index,
            from: usize,
            to: usize,
            args: &str,
        ) -> Option<Rule> {
            let (mut methods, mut patterns) = (Vec::new(), Vec::new());
            let owner = (decls(src, true).into_iter())
                .filter(|d| d.class && d.open < from && from < d.end)
                .max_by_key(|d| d.open)
                .map_or_else(String::new, |d| d.name);
            for part in split_top(args) {
                match part.trim().strip_prefix("HttpMethod.") {
                    Some(m) => methods.push(m.to_string()),
                    None => patterns.extend(index.texts_in(src, &owner, part)?),
                }
            }
            (!patterns.is_empty()).then(|| Rule {
                line: src.line(from),
                file: src.rel.clone(),
                text: src.text[from..to].split_whitespace().collect(),
                methods,
                patterns: Some(patterns),
                grant: None,
                conditional: false,
            })
        }

        /// The scope of the unit's chain, and the text range of the call that
        /// gives it. Read: `securityMatcher(..)`, `antMatcher(..)`, `mvcMatcher(..)`,
        /// `securityMatchers(m -> m.requestMatchers(..))` and the pre-6
        /// `requestMatchers().antMatchers(..).and()`, all on the `HttpSecurity`
        /// variable, with literal paths and outside `if`/`else`. Several scope
        /// calls or any other form are left to the not-evaluated check.
        pub(super) fn scope(
            src: &Source,
            index: &Index,
            span: (usize, usize),
        ) -> (Option<Rule>, Vec<(usize, usize)>) {
            let mut found = Vec::new();
            for var in super::chain_span::http_vars(src) {
                for p in statement_heads(src, &var, span) {
                    found.extend(chain_scopes(src, index, p));
                }
            }
            match found.pop() {
                Some((rule, call)) if found.is_empty() => (Some(rule), vec![call]),
                _ => (None, vec![]),
            }
        }

        /// Offsets in `span` where `var` starts a statement (`http.x();`,
        /// `return http.x();`); Kotlin has no `;`, there a line break ends one.
        fn statement_heads(src: &Source, var: &str, span: (usize, usize)) -> Vec<usize> {
            let b = src.bytes();
            let head = |p: usize| {
                let before = skip_ws_back(b, p);
                let kotlin_break = src.rel.ends_with(".kt") && src.text[before..p].contains('\n');
                before == 0
                    || kotlin_break
                    || matches!(b[before - 1], b';' | b'{' | b'}')
                    || super::chain_span::word_before(src, before) == "return"
            };
            (words(&src.text[span.0..span.1], var))
                .map(|p| p + span.0)
                .filter(|&p| b.get(skip_ws(b, p + var.len())) == Some(&b'.') && head(p))
                .collect()
        }

        /// The scope calls of the statement chain that starts at `p`, with their text range.
        fn chain_scopes(src: &Source, index: &Index, p: usize) -> Vec<(Rule, (usize, usize))> {
            let chain = calls(src, p, super::chain_span::chain_end(src, p));
            let (mut found, mut i) = (Vec::new(), 0);
            while i < chain.len() {
                let at = chain[i].0;
                match scope_call(src, index, &chain[i..]) {
                    Some((rule, to, used)) if !conditional(src, at) => {
                        found.push((rule, (at, to)));
                        i += used;
                    }
                    _ => {}
                }
                i += 1;
            }
            found
        }

        /// The scope call at the head of `chain`: `(rule, end offset, further calls used)`.
        fn scope_call(
            src: &Source,
            index: &Index,
            chain: &[(usize, &str, &str)],
        ) -> Option<(Rule, usize, usize)> {
            let (at, name, args) = chain[0];
            let close = |a: &str| offset(src, a) + a.len() + 1;
            match name {
                "securityMatcher" | "antMatcher" | "mvcMatcher" => {
                    let inner = reactive_matcher(args).unwrap_or(args);
                    literal_matcher(src, index, at, close(args), inner).map(|r| (r, close(args), 0))
                }
                "securityMatchers" => {
                    lambda_matchers(src, index, at, args).map(|r| (r, close(args), 0))
                }
                "requestMatchers" if args.trim().is_empty() => legacy_matchers(src, index, chain)
                    .map(|(r, last)| (r, close(chain[last].2), last)),
                _ => None,
            }
        }

        /// The literal arguments of a WebFlux scope matcher,
        /// `new PathPatternParserServerWebExchangeMatcher("/a")` or
        /// `ServerWebExchangeMatchers.pathMatchers("/a")`; a slice of `args`.
        fn reactive_matcher(args: &str) -> Option<&str> {
            let t = args.trim();
            let rest = t
                .strip_prefix("new PathPatternParserServerWebExchangeMatcher")
                .or_else(|| t.strip_prefix("ServerWebExchangeMatchers.pathMatchers"))?;
            rest.strip_prefix('(')?.strip_suffix(')')
        }

        /// `securityMatchers(m -> m.requestMatchers("/a", "/b"))`.
        fn lambda_matchers(src: &Source, index: &Index, at: usize, args: &str) -> Option<Rule> {
            let base = offset(src, args);
            let inner = calls(src, base + args.find("->")? + 2, base + args.len());
            let [(_, name, a)] = inner[..] else {
                return None;
            };
            if !matches!(name, "requestMatchers" | "antMatchers" | "mvcMatchers") {
                return None;
            }
            literal_matcher(src, index, at, base + args.len() + 1, a)
        }

        /// `requestMatchers().antMatchers("/api/**")` of pre-6 chains: the matchers
        /// after the empty `requestMatchers()` up to `.and()`; `(rule, index of the
        /// last matcher call)`.
        fn legacy_matchers(
            src: &Source,
            index: &Index,
            chain: &[(usize, &str, &str)],
        ) -> Option<(Rule, usize)> {
            let last = chain[1..]
                .iter()
                .take_while(|c| matches!(c.1, "antMatchers" | "mvcMatchers"))
                .count();
            let [(_, _, a)] = chain[1..=last] else {
                return None;
            };
            let (start, to) = (chain[0].0, offset(src, a) + a.len() + 1);
            literal_matcher(src, index, start, to, a).map(|r| (r, last))
        }

        /// A gap per adapter class without any readable rule.
        pub(super) fn unreadable_classes(units: &[Unit], out: &mut Out) {
            for u in units.iter().filter(|u| u.adapter && u.rules.is_empty()) {
                let message = format!(
                    "Security configuration `{}` has no readable `authorizeRequests()` rules; endpoints it protects stay undeclared.",
                    u.name
                );
                out.at(u.src, u.span.0, message);
            }
        }

        #[derive(Default)]
        pub(super) struct Applied {
            pub(super) changed: usize,
            /// Requests matched only partly, by `(unit, rule)`.
            pub(super) partial: BTreeMap<(usize, usize), usize>,
            /// Endpoints a unit's scope matches only partly, by unit.
            pub(super) scope_partial: BTreeMap<usize, usize>,
        }

        /// A chain that is not fully read is opaque: a lost scope makes it take
        /// any request, otherwise the readable scope and rules stand with an
        /// unknown part added.
        pub(super) fn mark_opaque(
            units: &mut [Unit],
            why: &super::chain_why::Why,
            recognised: &[(String, usize, usize)],
            out: &mut Out,
        ) {
            let spans: Vec<(String, (usize, usize))> =
                units.iter().map(|u| (u.src.rel.clone(), u.span)).collect();
            for u in units {
                let others: Vec<(usize, usize)> = (spans.iter())
                    .filter(|(rel, span)| *rel == u.src.rel && *span != u.span)
                    .map(|(_, span)| *span)
                    .collect();
                let Some((why, lost)) = why.unit(u.src, &others, recognised) else {
                    continue;
                };
                u.opaque = Some(why.clone());
                u.scope = u.scope.take().filter(|_| !lost);
                if u.scope.is_some() {
                    let message = format!(
                        "{why}: the rules read apply to the endpoints in the scope read, with an unknown part added."
                    );
                    out.at(u.src, u.span.0, message);
                } else {
                    let message = format!(
                        "{why}: central rules are not evaluated; endpoints marked undeclared may be protected there."
                    );
                    out.note(&u.src.rel, u.src.line(u.span.0), message);
                }
            }
        }

        /// Notes for rules and scopes that matched only some requests.
        pub(super) fn partial_notes(units: &[Unit], done: &Applied, out: &mut Out) {
            for (&(u, i), &n) in &done.partial {
                let r = &units[u].rules[i];
                let message = format!(
                    "Filter chain rule `{}` matches only some requests of {n} endpoint(s) (a path parameter or ANY method); they keep their declared access.",
                    r.text
                );
                out.note(&r.file, r.line, message);
            }
            for (&u, &n) in &done.scope_partial {
                let Some(s) = &units[u].scope else { continue };
                let message = format!(
                    "Filter chain scope `{}` matches only some requests of {n} endpoint(s) (a path parameter or ANY method); they keep their declared access.",
                    s.text
                );
                out.note(&s.file, s.line, message);
            }
        }

        /// Applies, per endpoint, the first chain (units in order) whose scope can
        /// match it; a scope that matches only some of its requests decides
        /// nothing. In that chain the first matching rule decides.
        pub(super) fn apply(
            units: &[Unit],
            endpoints: &mut [EndpointAccess],
            ignored: &BTreeSet<(String, u32)>,
        ) -> Applied {
            let mut done = Applied::default();
            for e in endpoints {
                let hits: Vec<Hit> = (units.iter())
                    .map(|unit| match &unit.scope {
                        // may take any request: decides nothing, shadows the rest
                        None if unit.barrier() => Hit::Part,
                        // code outside the text may have widened the scope
                        Some(s) if unit.opaque.is_some() => matches(s, e).max(Hit::Part),
                        s => s.as_ref().map_or(Hit::Full, |s| matches(s, e)),
                    })
                    .collect();
                // a chain of unreadable order may come before any other
                if (units.iter().zip(&hits)).any(|(u, h)| u.order.is_none() && *h != Hit::No) {
                    continue;
                }
                let mut from = 0;
                while from < units.len() {
                    let to = (from..units.len())
                        .find(|&k| units[k].order != units[from].order)
                        .unwrap_or(units.len());
                    let live: Vec<usize> = (from..to).filter(|&k| hits[k] != Hit::No).collect();
                    match live[..] {
                        [] => from = to,
                        // alone at its order: it decides
                        [u] if hits[u] == Hit::Full => {
                            decide(u, &units[u], e, ignored, &mut done);
                            break;
                        }
                        [u] => {
                            unknown_or_partial(u, &units[u], e, &mut done);
                            break;
                        }
                        // equal order, both may match: Spring's choice is not read
                        _ => break,
                    }
                }
            }
            done
        }

        /// An endpoint a chain may match only in part, or, if code outside the
        /// readable text may have widened its scope, may match at all.
        fn unknown_or_partial(u: usize, unit: &Unit, e: &mut EndpointAccess, done: &mut Applied) {
            match (&unit.opaque, unit.barrier()) {
                (Some(why), false) => {
                    e.access = require(&e.access, Condition::Unknown(why.clone()))
                }
                (_, false) => *done.scope_partial.entry(u).or_default() += 1,
                _ => {}
            }
        }

        /// The unit's rules for `e`; in a chain that code outside the readable
        /// text may change, what they decide is only part of the requirement.
        fn decide(
            u: usize,
            unit: &Unit,
            e: &mut EndpointAccess,
            ignored: &BTreeSet<(String, u32)>,
            done: &mut Applied,
        ) {
            let before = e.access.clone();
            decide_rules(u, &unit.rules, e, ignored, done);
            if let (Some(why), true) = (&unit.opaque, e.access != before) {
                e.access = require(&e.access, Condition::Unknown(why.clone()));
            }
        }

        fn decide_rules(
            u: usize,
            rules: &[Rule],
            e: &mut EndpointAccess,
            ignored: &BTreeSet<(String, u32)>,
            done: &mut Applied,
        ) {
            let Some((i, hit)) = (rules.iter().enumerate())
                .map(|(i, r)| (i, matches(r, e)))
                .find(|(_, h)| *h != Hit::No)
            else {
                return;
            };
            let rule = &rules[i];
            match (&rule.grant, hit, rule.conditional, &rule.patterns) {
                (Some(grant), Hit::Full, false, Some(_)) => {
                    let before = e.access.clone();
                    e.access = match grant {
                        AccessState::Protected(c) => require(&e.access, c.clone()),
                        // annotations that were not applied may protect it after all
                        _ if ignored.contains(&(e.file.clone(), e.line)) => e.access.clone(),
                        _ if e.access == AccessState::Undeclared => AccessState::Open,
                        _ => e.access.clone(),
                    };
                    done.changed += usize::from(e.access != before);
                }
                (Some(_), Hit::Part, false, Some(_)) => {
                    *done.partial.entry((u, i)).or_default() += 1;
                }
                _ => {}
            }
        }
    }

    /// The central rules of a project in the applications that include it.
    mod chain_apps {
        use super::super::*;
        use super::chain::{Prepared, central_rules, prepare, run};
        use super::conf::{Conf, ConfKey};

        /// The module graph and the chains of each application, read once: by
        /// its set of modules.
        pub(in super::super) struct Apps<'s> {
            modules: crate::ui_flow_apps::Modules,
            cache: HashMap<(BTreeSet<String>, ConfKey), Option<Prepared<'s>>>,
        }

        impl Apps<'_> {
            pub(in super::super) fn new(root: &Path, build_dirs: &BTreeSet<String>) -> Self {
                Apps {
                    modules: crate::ui_flow_apps::Modules::read(root, build_dirs.iter()),
                    cache: HashMap::new(),
                }
            }
        }

        /// The Spring central rules for the endpoints of project `dir` from `first`
        /// on: the project's own chains, or, when applications include it, the chains
        /// of each of them (see [`ui_flow_apps`](crate::ui_flow_apps)). An endpoint
        /// takes the access all applications agree on; otherwise it keeps its
        /// declared access.
        pub(in super::super) fn central<'s>(
            dir: &str,
            projects: &'s BTreeMap<String, Project>,
            known: &mut Apps<'s>,
            (index, conf): (&Index, &Conf),
            out: &mut Out,
            first: usize,
        ) {
            let apps = known.modules.applications(dir);
            if apps.is_empty() || out.0.endpoints.len() == first {
                let own: Vec<&Source> = projects.get(dir).map_or(vec![], |p| p.1.iter().collect());
                return central_rules(&own, index, conf, out, first);
            }
            let before: Vec<EndpointAccess> = out.0.endpoints[first..].to_vec();
            let (mut results, mut notes) = (Vec::new(), Vec::new());
            for app in &apps {
                let prepared = known
                    .cache
                    .entry((app.clone(), conf.key()))
                    .or_insert_with(|| {
                        let all: Vec<&Source> = (app.iter().filter_map(|d| projects.get(d)))
                            .flat_map(|p| p.1.iter())
                            .collect();
                        prepare(&all, index, conf, out)
                    });
                out.0.endpoints[first..].clone_from_slice(&before);
                let mark = out.0.diagnostics.len();
                if let Some(p) = prepared {
                    run(p, out, first);
                }
                notes.push(out.0.diagnostics.split_off(mark));
                let access = out.0.endpoints[first..].iter().map(|e| e.access.clone());
                results.push(access.collect::<Vec<_>>());
            }
            let mut disagree = 0;
            for (i, e) in out.0.endpoints[first..].iter_mut().enumerate() {
                let same = results.iter().all(|r| r[i] == results[0][i]);
                disagree += usize::from(!same);
                e.access = match same {
                    true => results[0][i].clone(),
                    false => before[i].access.clone(),
                };
            }
            // a change that is undone is not reported
            let undone = |m: &str| disagree > 0 && m.contains("endpoint(s) changed");
            (out.0.diagnostics).extend(notes.into_iter().flatten().filter(|d| !undone(&d.message)));
            if let (true, Some(e)) = (disagree > 0, before.first()) {
                let message = format!(
                    "{} applications include this project and apply different central rules to {disagree} endpoint(s); they keep their declared access.",
                    apps.len()
                );
                out.note(&e.file, e.line, message);
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
    /// Several chains/adapters are a sequence by `@Order` (adapters default 100,
    /// beans lowest precedence); the first whose scope can match an endpoint
    /// decides it (a scope that matches only some of its requests decides
    /// nothing). The scope is the literal `securityMatcher(..)`,
    /// `antMatcher(..)`, `mvcMatcher(..)`, `securityMatchers(m -> m.requestMatchers(..))`
    /// or pre-6 `requestMatchers().antMatchers(..)` that starts a statement on
    /// the `HttpSecurity` variable outside `if`/`else`; no scope takes every
    /// request.
    ///
    /// Not evaluated (one note, endpoints keep their annotation access):
    /// - several chains without a readable, distinct `@Order` (or adapters mixed
    ///   with beans);
    /// - a call or method reference of `antMatcher(s)`, `mvcMatcher(s)`,
    ///   `regexMatcher(s)`, `requestMatcher(s)` or `securityMatcher(s)` outside
    ///   the authorization block (and outside `configure(WebSecurity)`) in ANY
    ///   file that mentions `HttpSecurity` (configurers, extensions, helpers)
    ///   that is not one of the scopes above; declarations and `new` are no
    ///   calls;
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
        use super::conf::{Atom, Conf};
        use super::sequence::{
            UNORDERED, Unit, apply, mark_opaque, mixed, partial_notes, scope, ties, units,
            unreadable_classes,
        };
        use super::spel;

        const MATCHERS: [&str; 6] = [
            "requestMatchers",
            "mvcMatchers",
            "antMatchers",
            "anyRequest",
            // WebFlux
            "pathMatchers",
            "anyExchange",
        ];

        pub(super) struct Rule {
            pub(super) line: u32,
            pub(super) file: String,
            pub(super) text: String,
            /// Empty: any method.
            pub(super) methods: Vec<String>,
            /// `None`: not readable, may match anything.
            pub(super) patterns: Option<Vec<String>>,
            /// `None`: not readable.
            pub(super) grant: Option<AccessState>,
            pub(super) conditional: bool,
        }

        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        pub(super) enum Hit {
            No,
            Part,
            Full,
        }

        /// The chains of a set of sources, read once: what does not depend on the
        /// endpoints they are applied to.
        pub(in super::super) struct Prepared<'s> {
            units: Vec<Unit<'s>>,
            home: (&'s Source, usize),
            rules: usize,
        }

        pub(in super::super) fn central_rules(
            sources: &[&Source],
            index: &Index,
            conf: &Conf,
            out: &mut Out,
            first: usize,
        ) {
            if let Some(p) = prepare(sources, index, conf, out) {
                run(&p, out, first);
            }
        }

        /// Reads the chains of `sources`; `None` (with a note) when they cannot be
        /// put in sequence at all, or there are none.
        pub(in super::super) fn prepare<'s>(
            sources: &[&'s Source],
            index: &Index,
            conf: &Conf,
            out: &mut Out,
        ) -> Option<Prepared<'s>> {
            let found: Vec<&Source> = (sources.iter().copied())
                .filter(|s| !s.rel.ends_with(".cs"))
                .filter(|s| {
                    !adapters(s).is_empty()
                        || s.text.contains("SecurityFilterChain")
                        || s.text.contains("SecurityWebFilterChain")
                })
                .collect();
            let mut units: Vec<Unit> = found.iter().flat_map(|s| units(s)).collect();
            let home = units.first().map(|u| (u.src, u.span.0))?;
            let mut recognised = Vec::new();
            for u in &mut units {
                let (scope, spans) = scope(u.src, index, u.span);
                u.scope = scope;
                recognised.extend(spans.into_iter().map(|(f, t)| (u.src.rel.clone(), f, t)));
            }
            let why = super::chain_why::why_not_evaluated(sources, &found, &recognised);
            let abort = why
                .project
                .clone()
                .or_else(|| mixed(&units).then(|| UNORDERED.into()));
            if let Some(why) = abort {
                let message = format!(
                    "{why}: central rules are not evaluated; endpoints marked undeclared may be protected there."
                );
                out.note(&home.0.rel, home.0.line(home.1), message);
                return None;
            }
            mark_opaque(&mut units, &why, &recognised, out);
            if ties(&units) {
                let message = format!(
                    "{UNORDERED}: central rules are not evaluated for requests that two of them may match; endpoints marked undeclared may be protected there."
                );
                out.note(&home.0.rel, home.0.line(home.1), message);
            }
            for u in &mut units {
                u.rules = read(u.src, index, conf, u.span);
            }
            unreadable_classes(&units, out);
            let rules: usize = units.iter().map(|u| u.rules.len()).sum();
            for r in units.iter().flat_map(|u| &u.rules) {
                rule_note(r, out);
            }
            units.sort_by_key(|u| u.order);
            Some(Prepared { units, home, rules })
        }

        /// Applies the prepared chains to the endpoints from `first` on.
        pub(in super::super) fn run(p: &Prepared, out: &mut Out, first: usize) {
            let (units, home) = (&p.units, p.home);
            let done = apply(units, &mut out.0.endpoints[first..], &out.1.endpoints);
            partial_notes(units, &done, out);
            if p.rules == 0 {
                return;
            }
            let source = if units.iter().all(|u| !u.adapter) {
                "Central SecurityFilterChain"
            } else {
                "Security configuration"
            };
            let message = format!(
                "{source} rules evaluated: {} rule(s), access of {} endpoint(s) changed.",
                p.rules, done.changed
            );
            out.note(&home.0.rel, home.0.line(home.1), message);
        }

        pub(super) fn is_adapter(d: &Decl) -> bool {
            (d.bases.iter()).any(|b| {
                matches!(
                    b.as_str(),
                    "WebSecurityConfigurerAdapter" | "ResourceServerConfigurerAdapter"
                )
            })
        }

        /// A note for a rule that is not read completely.
        fn rule_note(r: &Rule, out: &mut Out) {
            let why = match (&r.patterns, &r.grant) {
                (None, _) => "its matcher is not a literal",
                (_, None) => "its grant is not read",
                _ if r.conditional => "it is conditional (inside if/else)",
                _ => return,
            };
            let message = format!(
                "Filter chain rule `{}`: {why}; endpoints it may match keep their declared access.",
                r.text
            );
            out.note(&r.file, r.line, message);
        }

        /// Classes extending the legacy Spring Security configurer adapters, as
        /// `(name, offset)`.
        fn adapters(src: &Source) -> Vec<(String, usize)> {
            (decls(src, true).into_iter())
                .filter(|d| d.class && is_adapter(d))
                .map(|d| (d.name, d.open))
                .collect()
        }

        /// Types of a chain bean: servlet and WebFlux.
        pub(super) const CHAIN_TYPES: [&str; 2] = ["SecurityFilterChain", "SecurityWebFilterChain"];

        /// A `SecurityFilterChain name(` method declaration at `p`; `len` is the
        /// length of the type name there.
        pub(super) fn bean(src: &Source, p: usize, len: usize) -> bool {
            let b = src.bytes();
            let s = skip_ws(b, p + len);
            let e = s + b[s..].iter().take_while(|c| lex::is_ident(**c)).count();
            e > s && b.get(skip_ws(b, e)) == Some(&b'(')
        }

        /// The `(` of every `authorizeHttpRequests`/`authorizeRequests` call, in order.
        pub(super) fn authorize_sites(src: &Source) -> Vec<usize> {
            let b = src.bytes();
            let mut sites: Vec<usize> = [
                "authorizeHttpRequests",
                "authorizeRequests",
                "authorizeExchange",
            ]
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

        /// The rules of every `authorizeHttpRequests`/`authorizeRequests` call of
        /// `src` that opens inside `span`.
        fn read(src: &Source, index: &Index, conf: &Conf, span: (usize, usize)) -> Vec<Rule> {
            let mut rules = Vec::new();
            for open in authorize_sites(src) {
                if span.0 <= open && open < span.1 {
                    site(src, index, conf, open, &mut rules);
                }
            }
            rules
        }

        /// The rules of the `authorize…(` call whose parenthesis opens at `open`.
        fn site(src: &Source, index: &Index, conf: &Conf, open: usize, rules: &mut Vec<Rule>) {
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
                    grant: grant(index, conf, name, args),
                    conditional,
                });
            }
        }

        /// `(position, name, arguments)` of each `.name(args)` call in `from..to`,
        /// skipping nested brackets.
        pub(super) fn calls(src: &Source, from: usize, to: usize) -> Vec<(usize, &str, &str)> {
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
        pub(super) fn conditional(src: &Source, at: usize) -> bool {
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
                "anyRequest" | "anyExchange" => return (at, vec![], Some(vec!["/**".into()])),
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

        fn grant(index: &Index, conf: &Conf, name: &str, args: &str) -> Option<AccessState> {
            let roles = |kind: Atom| -> Option<Condition> {
                let names: Option<Vec<Condition>> = split_top(args)
                    .into_iter()
                    .map(|a| index.text(a).map(|n| conf.atom(kind, &n)))
                    .collect();
                combine(names?, Condition::Any)
            };
            let protected = |c: Condition| Some(AccessState::Protected(c));
            match name {
                "permitAll" => Some(AccessState::Open),
                "denyAll" => protected(Condition::Never),
                "authenticated" | "fullyAuthenticated" => protected(Condition::Authenticated),
                "anonymous" => protected(Condition::Not(Box::new(Condition::Authenticated))),
                "hasRole" | "hasAnyRole" => roles(Atom::Role).map(AccessState::Protected),
                "hasAuthority" | "hasAnyAuthority" => {
                    roles(Atom::Authority).map(AccessState::Protected)
                }
                "access" => {
                    let expr = index.text(args)?;
                    match expr.trim() {
                        "permitAll" | "permitAll()" => Some(AccessState::Open),
                        _ => spel(conf, &expr).map(AccessState::Protected),
                    }
                }
                _ => None,
            }
        }

        pub(super) fn matches(rule: &Rule, e: &EndpointAccess) -> Hit {
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
            // `{id:\d+}`: the regex is not evaluated, so it may or may not match.
            if param(rule) && rule.contains(':') {
                return Hit::Part;
            }
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

    fn method(src: &Source, index: &Index, conf: &Conf, out: &mut Out, class: &Decl, d: &Decl) {
        let bases = match class.attrs.iter().find(|a| a.name == "RequestMapping") {
            Some(a) => match paths(src, index, out, &class.name, a) {
                Some(p) => p,
                None => return,
            },
            None => match inherit::inherited_bases(src, index, out, class) {
                Some(p) => p,
                None => return,
            },
        };
        let first = out.0.endpoints.len();
        for a in &d.attrs {
            let Some(methods) = methods(a) else { continue };
            let Some(paths) = paths(src, index, out, &class.name, a) else {
                continue;
            };
            let arity = src
                .pair(d.open)
                .map_or(0, |c| split_top(&src.text[d.open + 1..c]).len());
            let (access, ignored) = declared_access(conf, index, out, (src, class, d), arity);
            let full: Vec<String> = bases
                .iter()
                .flat_map(|b| paths.iter().map(move |p| path(&[b, p])))
                .collect();
            for (p, m) in full
                .iter()
                .flat_map(|p| methods.iter().map(move |m| (p, m)))
            {
                out.endpoint(src, a.at, m, p.clone(), &access);
                if ignored {
                    out.1.endpoints.insert((src.rel.clone(), src.line(a.at)));
                }
            }
        }
        out.link(
            first,
            handlers::of_method(src, &class.name, &d.name, d.open),
        );
    }

    /// The access the annotations of method `d` of `class` declare (own,
    /// inherited and class level) and whether an annotation was not applied.
    /// Shared with the WebFlux handler methods.
    pub fn declared_access(
        conf: &Conf,
        index: &Index,
        out: &mut Out,
        (src, class, d): (&Source, &Decl, &Decl),
        arity: usize,
    ) -> (AccessState, bool) {
        let up = index.inherited(class, &d.name, arity);
        let (on_method, ambiguous) = without_conflicts(conf, class, d, &up);
        let mut levels: Vec<(&Source, &[Attr])> = vec![(src, &d.attrs)];
        levels.extend(on_method.iter().map(|(s, a)| (*s, a.as_slice())));
        levels.push((src, &class.attrs));
        levels.extend(up.on_type.iter().map(|(s, a)| (*s, a.as_slice())));
        out.1.hit = false;
        let mut access = layer(conf, index, out, &levels).unwrap_or(AccessState::Undeclared);
        let ignored = std::mem::take(&mut out.1.hit);
        for g in &up.gaps {
            out.at(src, d.open, g.clone());
        }
        for text in &ambiguous {
            let unknown = out.unknown(src, d.open, text);
            access = require(&access, unknown);
        }
        (access, ignored)
    }

    /// The supertypes' method annotation levels without the families that
    /// several of them declare while the method itself declares none (Spring
    /// rejects that), and the ambiguous supertypes plus one text per such family.
    fn without_conflicts<'a>(
        conf: &Conf,
        class: &Decl,
        d: &Decl,
        up: &super::index::Inherited<'a>,
    ) -> (Vec<(&'a Source, Vec<Attr>)>, Vec<String>) {
        let own: Vec<&str> = d.attrs.iter().filter_map(|a| family(&a.name)).collect();
        let (mut levels, mut ambiguous) = (up.on_method.clone(), up.ambiguous.clone());
        for f in ["PreAuthorize", "PostAuthorize", "Secured", "jsr250"] {
            let has = |attrs: &Vec<Attr>| attrs.iter().any(|a| family(&a.name) == Some(f));
            let several = levels.iter().filter(|(_, a)| has(a)).count() > 1;
            if own.contains(&f) || conf.flag(f) != Flag::On || !several {
                continue;
            }
            for (_, attrs) in &mut levels {
                attrs.retain(|a| family(&a.name) != Some(f));
            }
            let text = format!(
                "{f} is declared on the method of several supertypes of {}",
                class.name
            );
            ambiguous.push(text);
        }
        (levels, ambiguous)
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

    /// The paths of mapping `a` written in type `owner` of `src`: literals, or
    /// constants of the scan.
    fn paths(
        src: &Source,
        index: &Index,
        out: &mut Out,
        owner: &str,
        a: &Attr,
    ) -> Option<Vec<String>> {
        let raw = named(&a.args, "value")
            .or_else(|| named(&a.args, "path"))
            .or_else(|| positional(&a.args));
        let Some(raw) = raw else {
            return Some(vec![String::new()]);
        };
        let items = |r: &str| {
            let r = r.trim();
            let inner = [("arrayOf(", ')'), ("{", '}'), ("[", ']')]
                .iter()
                .find_map(|(open, close)| r.strip_prefix(open)?.strip_suffix(*close));
            let parts = inner.map_or_else(|| vec![r], split_top);
            (parts.into_iter())
                .map(|p| index.text_in(src, owner, p))
                .collect::<Option<Vec<_>>>()
        };
        match literals(raw).or_else(|| items(raw)) {
            Some(p) if p.is_empty() => Some(vec![String::new()]),
            Some(p) => Some(p),
            None => {
                out.at(src, a.at, format!("Mapping path is not a literal: {raw}"));
                None
            }
        }
    }

    /// The security annotation family of `name`: Spring overrides per family
    /// (a method's annotation replaces the class's of the same family) and
    /// checks every family in turn.
    pub fn family(name: &str) -> Option<&'static str> {
        Some(match name {
            "PreAuthorize" => "PreAuthorize",
            "PostAuthorize" => "PostAuthorize",
            "Secured" => "Secured",
            "RolesAllowed" | "PermitAll" | "DenyAll" => "jsr250",
            _ => return None,
        })
    }

    type Families = BTreeMap<String, Vec<Option<Condition>>>;

    /// Access declared by the annotation `levels`, most specific first (method,
    /// the method in supertypes, class, supertypes): per family the first level
    /// with one wins, the families are ANDed. `None` when there is none.
    fn layer(
        conf: &Conf,
        index: &Index,
        out: &mut Out,
        levels: &[(&Source, &[Attr])],
    ) -> Option<AccessState> {
        let mut families = Families::new();
        for (src, attrs) in levels {
            let mut here = Families::new();
            collect(
                conf,
                index,
                out,
                (src, attrs),
                Nest {
                    depth: 0,
                    site: None,
                },
                &mut here,
            );
            for (f, items) in here {
                families.entry(f).or_insert(items);
            }
        }
        if conf.legacy() {
            // legacy method security: the first source with annotations (pre-post,
            // secured, jsr250) decides alone
            let group = |f: &str| match f {
                "PreAuthorize" | "PostAuthorize" => 0,
                "Secured" => 1,
                "jsr250" => 2,
                _ => 3,
            };
            if let Some(first) = families.keys().map(|f| group(f)).filter(|g| *g < 3).min() {
                families.retain(|f, _| group(f) == first || group(f) == 3);
            }
        }
        if families.is_empty() {
            return None;
        }
        let conditions = families.into_values().flatten().flatten().collect();
        Some(match combine(conditions, Condition::All) {
            Some(c) => AccessState::Protected(c),
            None => AccessState::Open,
        })
    }

    /// Where an annotation is read: its depth below the annotated element and,
    /// inside a meta-annotation, the use site that fills its template.
    #[derive(Clone, Copy)]
    struct Nest<'a> {
        depth: usize,
        site: Option<&'a Attr>,
    }

    /// Items of `level` (a source and its annotations) by family; an unknown
    /// annotation is followed into its declaration (meta-annotation).
    fn collect(
        conf: &Conf,
        index: &Index,
        out: &mut Out,
        (src, attrs): (&Source, &[Attr]),
        nest: Nest,
        into: &mut Families,
    ) {
        for a in attrs {
            let fam = family(&a.name);
            if fam.is_some_and(|f| !enforced(conf, out, src, a, f, into)) {
                continue;
            }
            let Some(a) = filled(conf, out, src, a, nest, into) else {
                continue;
            };
            match item(conf, src, index, out, &a) {
                Some(item) => into
                    .entry(fam.unwrap_or(&a.name).to_string())
                    .or_default()
                    .push(item),
                None if nest.depth < 4 => {
                    if let Some((s, d)) = index.meta(&a.name) {
                        let inner = Nest {
                            depth: nest.depth + 1,
                            site: Some(&*a),
                        };
                        collect(conf, index, out, (s, &d.attrs), inner, into);
                    }
                }
                None => {}
            }
        }
    }

    /// Whether annotation `a` of `family` is enforced; a family that is off adds
    /// nothing (a gap), one whose switch is not readable adds Unknown.
    fn enforced(
        conf: &Conf,
        out: &mut Out,
        src: &Source,
        a: &Attr,
        family: &str,
        into: &mut Families,
    ) -> bool {
        match conf.flag(family) {
            Flag::On => return true,
            Flag::Off => {
                let message = format!("`@{}` is not applied: {}.", a.name, conf.off(family));
                out.at(src, a.at, message);
                out.1.hit = true;
            }
            Flag::Unreadable => {
                let text = format!("@{} (its method-security switch is not readable)", a.name);
                let unknown = out.unknown(src, a.at, &text);
                into.entry(family.to_string())
                    .or_default()
                    .push(Some(unknown));
            }
        }
        false
    }

    /// `a`, with the `{value}` placeholders of an annotation template filled by
    /// the use site; `None` (and an Unknown item) when they cannot be.
    fn filled<'a>(
        conf: &Conf,
        out: &mut Out,
        src: &Source,
        a: &'a Attr,
        nest: Nest,
        into: &mut Families,
    ) -> Option<std::borrow::Cow<'a, Attr>> {
        let fam = family(&a.name);
        if nest.depth == 0 || fam.is_none() || !a.args.contains('{') {
            return Some(std::borrow::Cow::Borrowed(a));
        }
        let Some(args) = fill(conf, a, nest.site) else {
            let unknown = out.unknown(src, a.at, &a.args);
            into.entry(fam.unwrap_or(&a.name).to_string())
                .or_default()
                .push(Some(unknown));
            return None;
        };
        Some(std::borrow::Cow::Owned(Attr { args, ..a.clone() }))
    }

    /// The arguments of the template annotation `a` with each `{name}` replaced by
    /// the literal the use site gives; only with an
    /// `AnnotationTemplateExpressionDefaults` bean, and never for a placeholder
    /// the use site does not fill with a literal.
    fn fill(conf: &Conf, a: &Attr, use_site: Option<&Attr>) -> Option<String> {
        let site = use_site.filter(|_| conf.templates)?;
        let mut text = a.args.clone();
        for _ in 0..8 {
            let Some(open) = text.find('{') else {
                return Some(text);
            };
            let close = open + text[open..].find('}')?;
            let key = &text[open + 1..close];
            if key.is_empty() || !key.bytes().all(lex::is_ident) {
                return None;
            }
            let raw = named(&site.args, key)
                .or_else(|| (key == "value").then(|| positional(&site.args)).flatten())?;
            let value = lex::literal(raw)?;
            text.replace_range(open..=close, &value);
        }
        None
    }

    /// One security annotation: `Some(None)` declares open access.
    fn item(
        conf: &Conf,
        src: &Source,
        index: &Index,
        out: &mut Out,
        a: &Attr,
    ) -> Option<Option<Condition>> {
        let raw = named(&a.args, "value")
            .or_else(|| positional(&a.args))
            .unwrap_or("");
        match a.name.as_str() {
            "PreAuthorize" => Some(pre_authorize(conf, src, index, out, a, raw)),
            "PostAuthorize" => {
                let message = "`@PostAuthorize` is checked after the call: its side effects have already happened.";
                out.note(&src.rel, src.line(a.at), message.into());
                Some(pre_authorize(conf, src, index, out, a, raw))
            }
            "Secured" | "RolesAllowed" => {
                let kind = if a.name == "Secured" {
                    Atom::Authority
                } else {
                    Atom::Jsr250
                };
                let roles = literals(raw).and_then(|r| {
                    combine(
                        r.iter().map(|x| conf.atom(kind, x)).collect(),
                        Condition::Any,
                    )
                });
                Some(Some(roles.unwrap_or_else(|| out.unknown(src, a.at, raw))))
            }
            "PermitAll" => Some(None),
            "DenyAll" => Some(Some(Condition::Never)),
            _ => index.listed(out, src, a),
        }
    }

    /// `None` for `permitAll()`.
    fn pre_authorize(
        conf: &Conf,
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
        Some(spel(conf, &expr).unwrap_or_else(|| out.unknown(src, a.at, &expr)))
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
    fn spel(conf: &Conf, expr: &str) -> Option<Condition> {
        let mut parser = Spel {
            conf,
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

    struct Spel<'c> {
        conf: &'c Conf,
        toks: Vec<Tok>,
        at: usize,
    }
    impl Spel<'_> {
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
            let kind = match name.as_str() {
                "hasRole" | "hasAnyRole" => Atom::Role,
                "hasScope" | "hasAnyScope" => Atom::Scope,
                _ => Atom::Authority,
            };
            let mut args = if self.eat(&Tok::Open) {
                self.args(kind)?
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
        fn args(&mut self, kind: Atom) -> Option<Vec<Condition>> {
            let mut args = Vec::new();
            if self.eat(&Tok::Close) {
                return Some(args);
            }
            loop {
                let Some(Tok::Str(s)) = self.toks.get(self.at) else {
                    return None;
                };
                args.push(self.conf.atom(kind, s));
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

/// Razor Pages: every `.cshtml` with `@page` under a `Pages` root is an endpoint
/// per handler verb of its page model (`@model`), with the access of the model
/// class and its bases, `@attribute [...]` lines and the `Conventions.Authorize…`
/// / `AllowAnonymous…` calls of `AddRazorPages(options => …)`; `MapRazorPages()`
/// modifiers, the fallback policy and global filters apply as to any endpoint.
/// Sources:
/// <https://learn.microsoft.com/en-us/aspnet/core/razor-pages/security/authorization/conventions>
/// <https://learn.microsoft.com/en-us/aspnet/core/razor-pages/security/authorization/simple>
/// Case-insensitive matching of convention paths is not verified in the docs.
/// An unreadable convention argument or a changed pages root is a gap, never
/// open; `[Authorize]` on a handler method has no effect in the framework and is
/// reported as such. Pages are endpoints only: Blazor pages are screens of the
/// Blazor extractor and the screens model is not extended here.
mod razor {
    use super::controllers::{access, own};
    use super::lex::{literal, skip_ws, split_top, words};
    use super::*;

    const VERBS: [&str; 7] = ["Get", "Post", "Put", "Delete", "Patch", "Head", "Options"];
    /// (call, takes an area first, folder, anonymous)
    const CONVENTIONS: [(&str, bool, bool, bool); 8] = [
        ("AuthorizePage", false, false, false),
        ("AuthorizeFolder", false, true, false),
        ("AllowAnonymousToPage", false, false, true),
        ("AllowAnonymousToFolder", false, true, true),
        ("AuthorizeAreaPage", true, false, false),
        ("AuthorizeAreaFolder", true, true, false),
        ("AllowAnonymousToAreaPage", true, false, true),
        ("AllowAnonymousToAreaFolder", true, true, true),
    ];

    struct Convention<'a> {
        src: &'a Source,
        at: usize,
        area: Option<String>,
        path: String,
        folder: bool,
        anonymous: bool,
        /// Raw policy argument.
        policy: Option<String>,
        /// Area and path are literals; else the convention reaches every page.
        readable: bool,
        raw: String,
    }

    pub(super) fn scan(
        pages: &[Source],
        sources: &[Source],
        index: &Index,
        application: Option<&str>,
        out: &mut Out,
    ) {
        if pages.is_empty() {
            return;
        }
        let cs: Vec<&Source> = sources.iter().filter(|s| s.rel.ends_with(".cs")).collect();
        if let Some(src) = cs.iter().find(|s| moved_root(s)) {
            let at = words(&src.text, "RootDirectory").next().unwrap_or(0);
            let message = "Razor Pages root directory changed; pages not read".into();
            return out.at(src, at, message);
        }
        let conventions = conventions(&cs, index);
        let pages_mods = out.2.pages.clone();
        let saved = std::mem::replace(&mut out.2.convention, pages_mods);
        let found: Vec<Page> = (pages.iter())
            .filter_map(|page| one(page, pages, index, &conventions, out))
            .collect();
        out.2.convention = saved;
        screens::emit(pages, &found, application, out);
    }

    /// A page with `@page`: where it is, its route and the access of its endpoints.
    pub(super) struct Page<'a> {
        pub(super) src: &'a Source,
        pub(super) at: usize,
        pub(super) route: String,
        pub(super) place: (Option<String>, String),
        pub(super) state: AccessState,
    }

    fn moved_root(src: &Source) -> bool {
        src.text.contains("WithRazorPagesAtContentRoot")
            || src.text.contains("WithRazorPagesRoot")
            || (src.text.contains("RazorPagesOptions") || src.text.contains("AddRazorPages"))
                && words(&src.text, "RootDirectory").next().is_some()
    }

    fn conventions<'a>(cs: &[&'a Source], index: &Index) -> Vec<Convention<'a>> {
        let mut found = Vec::new();
        for src in cs {
            for kind in CONVENTIONS {
                let calls = words(&src.text, kind.0);
                found.extend(calls.filter_map(|at| convention(src, index, at, kind)));
            }
        }
        found
    }

    /// The call `Name(…)` at `at`, if it is one.
    fn convention<'a>(
        src: &'a Source,
        index: &Index,
        at: usize,
        (name, area, folder, anonymous): (&str, bool, bool, bool),
    ) -> Option<Convention<'a>> {
        let close = minimal::call_close(src, at, name)?;
        let open = skip_ws(src.bytes(), at + name.len());
        let args = split_top(&src.text[open + 1..close]);
        let skip = usize::from(area);
        let read = |i: usize| args.get(i).and_then(|a| index.text(a));
        let (a, path) = (read(0), read(skip));
        Some(Convention {
            src,
            at,
            readable: path.is_some() && (!area || a.is_some()),
            area: a.filter(|_| area),
            path: path.unwrap_or_default(),
            folder,
            anonymous,
            policy: args.get(skip + 1).map(|p| p.trim().to_string()),
            raw: src.text[at..close + 1].to_string(),
        })
    }

    /// The area and the path below the pages root without extension (`/Private/Edit`).
    fn locate(rel: &str) -> Option<(Option<String>, String)> {
        let parts: Vec<&str> = rel.split('/').collect();
        let root = parts.iter().position(|p| *p == "Pages")?;
        let area = (root >= 2 && parts[root - 2] == "Areas").then(|| parts[root - 1].to_string());
        let name = parts[root + 1..].join("/");
        Some((area, format!("/{}", name.strip_suffix(".cshtml")?)))
    }

    fn matches(c: &Convention, area: &Option<String>, page: &str) -> bool {
        let same_area = match (&c.area, area) {
            (None, None) => true,
            (Some(a), Some(b)) => a.to_lowercase() == b.to_lowercase(),
            _ => false,
        };
        let path = format!("/{}", c.path.trim_matches('/')).to_lowercase();
        let page = page.to_lowercase();
        same_area
            && match c.folder {
                true => path == "/" || page.starts_with(&format!("{path}/")),
                false => page == path,
            }
    }

    /// What the `.cshtml` file itself says.
    struct Directives {
        at: usize,
        model: Option<String>,
        inherits: Option<String>,
        /// `@page "template"`; `Err` with the raw text when it is no literal.
        template: Option<Result<String, String>>,
        attrs: Vec<Attr>,
    }

    /// The directives of a file and whether it has `@page`.
    fn directives(page: &Source) -> (Directives, bool) {
        let mut d = Directives {
            at: 0,
            model: None,
            inherits: None,
            template: None,
            attrs: Vec::new(),
        };
        let (mut found, mut offset) = (false, 0);
        // `@* … *@` comments hide directives; blanking keeps the offsets.
        let text = archguard_flow_common::lang_markup::blank(&page.text, "@*", "*@");
        for line in text.split_inclusive('\n') {
            let t = line.trim_start();
            let here = offset + (line.len() - t.len());
            offset += line.len();
            let page_rest = t
                .strip_prefix("@page")
                .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace));
            if let Some(rest) = page_rest {
                (d.at, found) = if found { (d.at, true) } else { (here, true) };
                let rest = rest.trim();
                d.template = (!rest.is_empty()).then(|| literal(rest).ok_or_else(|| rest.into()));
            } else if let Some(rest) = t.strip_prefix("@model ") {
                d.model = Some(rest.trim().rsplit('.').next().unwrap_or("").to_string());
            } else if let Some(rest) = t.strip_prefix("@inherits ") {
                let name = rest.trim().split('<').next().unwrap_or("");
                d.inherits = Some(name.rsplit('.').next().unwrap_or("").to_string());
            } else if let Some(rest) = t.strip_prefix("@attribute ") {
                let base = here + (t.len() - rest.len());
                d.attrs
                    .extend(directive_attrs(rest).into_iter().map(|mut a| {
                        a.at += base;
                        a
                    }));
            }
        }
        (d, found)
    }

    /// The directives of a page, with `@model` / `@inherits` of the
    /// `_ViewImports.cshtml` files above it where it has none (the closest one
    /// wins); none without `@page`. Docs:
    /// <https://learn.microsoft.com/en-us/aspnet/core/mvc/views/layout#importing-shared-directives>
    fn page_directives(page: &Source, pages: &[Source], out: &mut Out) -> Option<Directives> {
        let (mut d, found) = directives(page);
        if !found {
            return None;
        }
        let mut cur = page.rel.as_str();
        while let Some((dir, _)) = cur.rsplit_once('/') {
            cur = dir;
            let name = format!("{dir}/_ViewImports.cshtml");
            if let Some(imp) = pages.iter().find(|p| p.rel == name) {
                let (imp, _) = directives(imp);
                if !imp.attrs.is_empty() {
                    // The docs list the directives `_ViewImports` supports (`@addTagHelper`,
                    // `@removeTagHelper`, `@tagHelperPrefix`, `@using`, `@model`, `@inherits`,
                    // `@inject`, `@namespace`); `@attribute` is not among them, so it is a gap:
                    // https://learn.microsoft.com/en-us/aspnet/core/mvc/views/layout#importing-shared-directives
                    let message = format!("@attribute in _ViewImports not read: {name}");
                    out.at(page, d.at, message);
                }
                d.model = d.model.or(imp.model);
                d.inherits = d.inherits.or(imp.inherits);
            }
        }
        Some(d)
    }

    /// The route: the path below the root (`Index` is its folder), the area, and
    /// the `@page` template (absolute replaces, relative is appended).
    fn route(
        page: &Source,
        (area, path): &(Option<String>, String),
        d: &Directives,
        out: &mut Out,
    ) -> String {
        let mut route = match path.strip_suffix("/Index") {
            Some(p) => p.to_string(),
            None if path == "/Index" => String::new(),
            None => path.clone(),
        };
        if let Some(area) = area {
            route = format!("/{area}{route}");
        }
        match &d.template {
            Some(Ok(t)) if t.starts_with('/') => route = t.clone(),
            Some(Ok(t)) => route = format!("{route}/{}", t.trim_matches('/')),
            Some(Err(raw)) => {
                out.at(
                    page,
                    d.at,
                    format!("@page template is not a literal: {raw}"),
                );
            }
            None => {}
        }
        if route.is_empty() { "/".into() } else { route }
    }

    /// The attributes of the page model class and its bases and the verbs of its
    /// handlers (`GET` for a page without handlers or model).
    fn model_levels<'a>(
        page: &'a Source,
        index: &'a Index,
        d: &'a Directives,
        out: &mut Out,
    ) -> (Vec<(&'a Source, &'a Attr)>, BTreeSet<String>) {
        let mut levels: Vec<(&Source, &Attr)> = d.attrs.iter().map(|a| (page, a)).collect();
        let mut verbs = BTreeSet::new();
        for name in [&d.model, &d.inherits].into_iter().flatten() {
            let classes: Vec<_> = (index.all_classes())
                .filter(|(_, c)| c.class && &c.name == name)
                .collect();
            match classes.as_slice() {
                [(src, class)] => model_class(index, (src, class), &mut levels, &mut verbs, out),
                _ => {
                    let message = format!(
                        "Page model {name} not found or ambiguous; its attributes and handlers are not read"
                    );
                    out.at(page, d.at, message);
                }
            }
        }
        if verbs.is_empty() {
            verbs.insert("GET".to_string());
        }
        (levels, verbs)
    }

    /// The attributes and handler verbs of `class` and its bases.
    fn model_class<'a>(
        index: &'a Index,
        (src, class): (&'a Source, &'a Decl),
        levels: &mut Vec<(&'a Source, &'a Attr)>,
        verbs: &mut BTreeSet<String>,
        out: &mut Out,
    ) {
        let bases = index.ancestors(out, src, class);
        levels.extend(own(class, src));
        let mut chain = vec![(src, class.open)];
        for (s, b) in &bases {
            levels.extend(own(b, s));
            chain.push((*s, b.open));
        }
        for (s, open) in chain {
            each_method(s, false, |c, m| {
                if c.open == open {
                    handler(s, m, verbs, out);
                }
            });
        }
    }

    /// The conventions that reach a page, as attributes; an unreadable one reaches every page.
    fn convention_attrs<'a>(
        conventions: &[Convention<'a>],
        area: &Option<String>,
        path: &str,
    ) -> Vec<(&'a Source, Attr)> {
        let reaches = |c: &&Convention| !c.readable || matches(c, area, path);
        let attr = |c: &Convention| {
            let (name, args) = match (c.readable, c.anonymous) {
                (false, _) => ("Authorize", format!("Policy = {}", c.raw)),
                (true, true) => ("AllowAnonymous", String::new()),
                (true, false) => ("Authorize", c.policy.clone().unwrap_or_default()),
            };
            Attr {
                name: name.into(),
                args,
                at: c.at,
            }
        };
        (conventions.iter().filter(reaches))
            .map(|c| (c.src, attr(c)))
            .collect()
    }

    fn one<'a>(
        page: &'a Source,
        pages: &[Source],
        index: &Index,
        conventions: &[Convention],
        out: &mut Out,
    ) -> Option<Page<'a>> {
        let (Some(d), Some(place)) = (page_directives(page, pages, out), locate(&page.rel)) else {
            return None;
        };
        let route = route(page, &place, &d, out);
        let (mut levels, verbs) = model_levels(page, index, &d, out);
        let synthetic = convention_attrs(conventions, &place.0, &place.1);
        levels.extend(synthetic.iter().map(|(s, a)| (*s, a)));
        let state = access(index, out, &levels);
        for verb in verbs {
            out.endpoint(page, d.at, &verb, route.clone(), &state);
        }
        Some(Page {
            src: page,
            at: d.at,
            route,
            place,
            state,
        })
    }

    /// `OnGet`, `OnPostAsync`, `OnPostDelete`…: the verb of a page handler;
    /// `[Authorize]` on it has no effect in the framework.
    fn handler(src: &Source, m: &Decl, verbs: &mut BTreeSet<String>, out: &mut Out) {
        let Some(rest) = m.name.strip_prefix("On") else {
            return;
        };
        let verb = VERBS.iter().find(|v| {
            (rest.strip_prefix(**v))
                .is_some_and(|r| r.chars().next().is_none_or(char::is_uppercase))
        });
        let Some(verb) = verb else {
            return;
        };
        verbs.insert(verb.to_ascii_uppercase());
        for a in m
            .attrs
            .iter()
            .filter(|a| a.name == "Authorize" || a.name == "AllowAnonymous")
        {
            let message = format!(
                "[{}] on page handler {} is ignored by Razor Pages; put it on the page model",
                a.name, m.name
            );
            out.at(src, a.at, message);
        }
    }
}

/// The screens of Razor Pages (one per page with `@page`, with the access of its
/// endpoint) and the links and forms between them that are literal:
/// `<a asp-page="/Contact">`, `<a href="/Contact">`, `<form method="post"
/// asp-page="/Contact">`, `<form method="post" action="/x">`. Anything else
/// (`@…` in the value, MVC `asp-controller`/`asp-action`, a relative page name,
/// a page that is not in the project, a `<partial>` or layout that holds
/// links) is a gap; a link under an `@if` or other code block is shown with an
/// unreadable condition, never as unconditional.
mod screens {
    use super::razor::Page;
    use super::*;
    use archguard_core::{Action, Call, Origin, Screen, ScreenKind, Target, Transition};

    const SOURCE: &str = "aspnet";

    struct Tag {
        at: usize,
        form: bool,
        attrs: Vec<(String, Option<String>)>,
        /// Plain text up to `</a>`.
        label: Option<String>,
    }

    impl Tag {
        fn get(&self, name: &str) -> Option<&str> {
            (self.attrs.iter())
                .find(|(n, _)| n == name)
                .and_then(|(_, v)| v.as_deref())
        }
        fn has(&self, name: &str) -> bool {
            self.attrs.iter().any(|(n, _)| n == name)
        }
        /// The literal value; `Err` for `@…` or a missing value.
        fn literal(&self, name: &str) -> Result<Option<&str>, String> {
            match self.get(name) {
                Some(v) if v.contains('@') => Err(format!("{name}=\"{v}\" is not literal")),
                None if self.has(name) => Err(format!("{name} without a literal value")),
                v => Ok(v),
            }
        }
    }

    pub(super) fn emit(pages: &[Source], found: &[Page], application: Option<&str>, out: &mut Out) {
        let app = application.unwrap_or("");
        let id = |route: &str| format!("{app}:{route}");
        for p in found {
            let name = p.place.1.trim_start_matches('/');
            let name = match &p.place.0 {
                Some(area) => format!("{area}/{name}"),
                None => name.to_string(),
            };
            out.0.screens.push(Screen {
                weak: false,
                id: id(&p.route),
                name,
                kind: ScreenKind::Page,
                route: Some(p.route.clone()),
                application: application.map(str::to_string),
                file: p.src.rel.clone(),
                line: p.src.line(p.at),
                access: p.state.clone(),
                source: SOURCE.into(),
            });
        }
        for p in found {
            links(p, found, &id, out);
        }
        let others = pages
            .iter()
            .filter(|s| !found.iter().any(|p| p.src.rel == s.rel));
        for other in others {
            let name = other.rel.rsplit('/').next().unwrap_or("");
            if name.starts_with("_ViewImports") || name.starts_with("_ViewStart") {
                continue;
            }
            let text = archguard_flow_common::lang_markup::blank(&other.text, "@*", "*@");
            if tags(&text)
                .iter()
                .any(|t| t.has("href") || t.has("asp-page") || t.has("action"))
            {
                let message = "Links of a layout or partial (a template without @page) are not attributed to screens";
                out.at(other, 0, message.into());
            }
        }
    }

    fn links(p: &Page, all: &[Page], id: &dyn Fn(&str) -> String, out: &mut Out) {
        let text = archguard_flow_common::lang_markup::blank(&p.src.text, "@*", "*@");
        let conditions = conditions(&text);
        let lower = text.to_ascii_lowercase();
        let partials = [
            "<partial",
            "html.partial",
            "renderpartial",
            "<vc:",
            "component.invoke",
            "<component",
        ];
        if partials.iter().any(|k| lower.contains(k)) {
            let message =
                "Partials and components of this page are not followed; their links are unknown";
            out.at(p.src, 0, message.into());
        }
        let from = Origin::Screen(id(&p.route));
        for tag in tags(&text) {
            let line = p.src.line(tag.at);
            let header = (conditions
                .iter()
                .find(|(lo, hi, _)| (*lo..*hi).contains(&tag.at)))
            .map(|(_, _, header)| header.clone());
            let access = match &header {
                Some(h) => {
                    let message = format!(
                        "Link under the Razor block `{h}` is shown with an unreadable condition"
                    );
                    out.at(p.src, tag.at, message);
                    AccessState::Protected(Condition::Unknown(h.clone()))
                }
                None => AccessState::Undeclared,
            };
            match target(&tag, p, all) {
                Err(why) => {
                    let kind = if tag.form { "<form>" } else { "<a>" };
                    out.at(
                        p.src,
                        tag.at,
                        format!("Navigation target of {kind} is not read: {why}"),
                    );
                }
                Ok(None) => {}
                Ok(Some(Dest::Screen(route))) => out.0.transitions.push(Transition {
                    from: from.clone(),
                    target: Target::Screen(id(&route)),
                    trigger: tag.label.clone(),
                    file: p.src.rel.clone(),
                    line,
                    condition: access,
                    weak: false,
                    source: SOURCE.into(),
                }),
                Ok(Some(Dest::Post(path))) => out.0.actions.push(Action {
                    screen: from.clone(),
                    trigger: tag.label.clone(),
                    call: Call::Http {
                        method: "POST".into(),
                        path,
                    },
                    file: p.src.rel.clone(),
                    line,
                    condition: access,
                    weak: false,
                    source: SOURCE.into(),
                }),
            }
        }
    }

    enum Dest {
        Screen(String),
        Post(String),
    }

    /// Where the tag leads: nothing, a page or a POST; `Err` with the reason
    /// when it cannot be read.
    fn target(tag: &Tag, from: &Page, all: &[Page]) -> Result<Option<Dest>, String> {
        let post = tag.form
            && tag
                .literal("method")?
                .is_some_and(|m| m.eq_ignore_ascii_case("post"));
        for name in ["asp-controller", "asp-action", "asp-route"] {
            if tag.has(name) {
                return Err(format!("{name} is not a Razor Pages target"));
            }
        }
        let (page, area) = (tag.literal("asp-page")?, tag.literal("asp-area")?);
        let url = tag.literal(if tag.form { "action" } else { "href" })?;
        match (page, url) {
            (Some(name), _) => page_target(name, area, post, from, all).map(Some),
            (None, Some(url)) => url_target(url, post, all),
            (None, None) if post => Ok(Some(Dest::Post(from.route.clone()))),
            (None, None) => Ok(None),
        }
    }

    /// `asp-page="/Name"` (with `asp-area`): an absolute page of the project.
    fn page_target(
        name: &str,
        area: Option<&str>,
        post: bool,
        from: &Page,
        all: &[Page],
    ) -> Result<Dest, String> {
        let area = match area {
            Some("") => None,
            Some(a) => Some(a.to_lowercase()),
            None if from.place.0.is_some() => {
                return Err("its area is the ambient area of the page".into());
            }
            None => None,
        };
        let hit = all.iter().find(|q| {
            name.starts_with('/')
                && q.place.0.as_ref().map(|a| a.to_lowercase()) == area
                && q.place.1.to_lowercase() == name.to_lowercase()
        });
        match hit {
            Some(q) if post => Ok(Dest::Post(q.route.clone())),
            Some(q) => Ok(Dest::Screen(q.route.clone())),
            None => Err(format!(
                "asp-page=\"{name}\" is not an absolute page of this project"
            )),
        }
    }

    /// `href="/x"` / `action="/x"`: a POST to it, or the page with that route;
    /// external, anchor and relative URLs are no page links.
    fn url_target(url: &str, post: bool, all: &[Page]) -> Result<Option<Dest>, String> {
        let url = url.trim_start_matches('~');
        if !url.starts_with('/') {
            return Ok(None);
        }
        let clean = url.split(['?', '#']).next().unwrap_or("");
        if post {
            return Ok(Some(Dest::Post(clean.to_string())));
        }
        let want = clean.trim_end_matches('/').to_lowercase();
        let hit = all
            .iter()
            .find(|q| q.route.trim_end_matches('/').to_lowercase() == want);
        // Not a literal route of a page: a parameter route, a library page or a file.
        let miss = || format!("href=\"{clean}\" matches no page route literally");
        hit.map(|q| Some(Dest::Screen(q.route.clone())))
            .ok_or_else(miss)
    }

    /// The end of the `{…}` block that opens at `open` (the text's end when unbalanced).
    fn block_end(text: &str, open: usize) -> usize {
        let mut depth = 0i32;
        for (i, c) in text.bytes().enumerate().skip(open) {
            depth += i32::from(c == b'{') - i32::from(c == b'}');
            if depth == 0 {
                return i;
            }
        }
        text.len()
    }

    /// `@if (…) {…}`, `@else {…}` and other code blocks: (start, end, header).
    fn conditions(text: &str) -> Vec<(usize, usize, String)> {
        let mut found = blocks(text);
        // Braces in markup or strings cannot be told from code braces: the block
        // ends are not trusted, every link from the first block on is conditional.
        if let Some(first) = found
            .iter()
            .map(|b| b.0)
            .min()
            .filter(|_| unsure_braces(text))
        {
            found.push((first, text.len(), "braces in markup or strings".to_string()));
        }
        found
    }

    /// Whether a brace may sit in markup or a string: a line with a brace and a
    /// tag, a brace inside quotes, or unequal numbers of `{` and `}`.
    fn unsure_braces(text: &str) -> bool {
        let braces = |l: &str| l.contains(['{', '}']);
        let (open, close) = (text.matches('{').count(), text.matches('}').count());
        open != close
            || text.lines().any(|l| {
                let quoted = l.split('"').skip(1).step_by(2).any(braces);
                braces(l) && (l.contains('<') && l.contains('>') || quoted)
            })
    }

    fn blocks(text: &str) -> Vec<(usize, usize, String)> {
        let mut found = Vec::new();
        let keys = [
            "@if",
            "@switch",
            "@for",
            "@while",
            "@do",
            "@lock",
            "@try",
            "@using (",
            "@using(",
            "@{",
            "@functions",
            "@code",
            "else",
        ];
        for key in keys {
            for (at, _) in text.match_indices(key) {
                let after_block = text[..at].trim_end().ends_with('}');
                let open = text[at..].find('{').map(|o| o + at);
                if let Some(open) = open.filter(|_| key != "else" || after_block) {
                    found.push((
                        open,
                        block_end(text, open),
                        text[at..open].trim().to_string(),
                    ));
                }
            }
        }
        found
    }

    /// The attributes of the tag whose name ends at `i`, and the position of its `>`.
    fn read_attrs(text: &str, mut i: usize) -> (Vec<(String, Option<String>)>, usize) {
        let b = text.as_bytes();
        let mut attrs = Vec::new();
        while i < b.len() && b[i] != b'>' {
            if b[i].is_ascii_whitespace() || b[i] == b'/' {
                i += 1;
                continue;
            }
            let s = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>') {
                i += 1;
            }
            let name = text[s..i].to_ascii_lowercase();
            let value = (b.get(i) == Some(&b'=')).then(|| {
                let (value, next) = read_value(text, i + 1);
                i = next;
                value
            });
            attrs.push((name, value));
        }
        (attrs, i)
    }

    /// A quoted or bare attribute value from `i`, and the position after it.
    fn read_value(text: &str, i: usize) -> (String, usize) {
        let b = text.as_bytes();
        match b.get(i) {
            Some(&q) if q == b'"' || q == b'\'' => {
                let end = text[i + 1..].find(q as char).map_or(b.len(), |e| i + 1 + e);
                (text[i + 1..end].to_string(), end + 1)
            }
            _ => {
                let end = (i..b.len())
                    .find(|&k| b[k].is_ascii_whitespace() || b[k] == b'>')
                    .unwrap_or(b.len());
                (text[i.min(end)..end].to_string(), end)
            }
        }
    }

    /// The `<a …>` and `<form …>` tags of a template.
    fn tags(text: &str) -> Vec<Tag> {
        let mut found = Vec::new();
        for (at, _) in text.match_indices('<') {
            let rest = &text[at + 1..];
            let (form, len) = if rest
                .get(..4)
                .is_some_and(|s| s.eq_ignore_ascii_case("form"))
            {
                (true, 4)
            } else if rest.starts_with(['a', 'A']) {
                (false, 1)
            } else {
                continue;
            };
            if !text
                .as_bytes()
                .get(at + 1 + len)
                .is_some_and(u8::is_ascii_whitespace)
            {
                continue;
            }
            let (attrs, end) = read_attrs(text, at + 1 + len);
            let label = (!form)
                .then(|| text[end.min(text.len())..].split_once("</a>"))
                .flatten()
                .map(|(l, _)| l.trim())
                .filter(|l| !l.is_empty() && !l.contains(['<', '@']))
                .map(str::to_string);
            found.push(Tag {
                at,
                form,
                attrs,
                label,
            });
        }
        found
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

    public static IResult CreateOrderAsync() => null;
    public static IResult GetOrderAsync(int orderId) => null;

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
            ("owners/src/main/java/shop/Enable.java", ENABLE_ALL),
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
            Condition::Any(vec![policy("SCOPE_x"), policy("owner:read")]),
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
                        policy("BLOCKED"),
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
            &signed_in(role("ROLE_AUDITOR"))
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

    #[test]
    fn spring_role_prefix_outside_every_project_is_a_gap_not_a_whole_flow_entry() {
        let dir = project(&[(
            "A.java",
            "import org.springframework.web.bind.annotation.*;\n@RestController class A { @GetMapping(\"/a\") void a() {} }",
        )]);
        let flow = extract(dir.path());
        assert_eq!(flow.endpoints.len(), 1);
        assert!(
            flow.role_conventions.is_empty(),
            "{:?}",
            flow.role_conventions
        );
        assert_eq!(notes(&flow, "Role prefix not attributed"), 1);
        assert!(!flow.complete);
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
        let flow = extract_with(&ExtractContext::new(dir.path(), &config), None);
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
            ("src/main/java/app/Enable.java", ENABLE_ALL),
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
            ("src/main/java/Enable.java", ENABLE_ALL),
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
            &protected(Condition::All(vec![role("ROLE_ADMIN"), policy("server")]))
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/internal"),
            &protected(Condition::All(vec![
                Condition::Authenticated,
                policy("server")
            ]))
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/either"),
            &protected(Condition::All(vec![
                Condition::Any(vec![
                    Condition::Any(vec![policy("a"), policy("b")]),
                    role("ROLE_ROOT")
                ]),
                Condition::Authenticated
            ]))
        );
    }

    fn configured(files: &[(&str, &str)]) -> UiFlow {
        let mut all = vec![
            ("pom.xml", "<project/>"),
            ("src/main/java/Enable.java", ENABLE_ALL),
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
            &AccessState::Undeclared,
            "{:#?}",
            flow.diagnostics
        );
        assert_eq!(notes(flow, "not evaluated"), 1, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn security_matchers_scope_a_filter_chain() {
        let chain = "public class C { @Bean SecurityFilterChain api(HttpSecurity http) { http.securityMatchers(m -> m.requestMatchers(\"/api/**\")).authorizeHttpRequests(a -> a.anyRequest().authenticated()); return http.build(); } }";
        let flow = configured(&[("src/main/java/app/C.java", chain)]);
        // a literal scope is read (F30); the rules apply inside it
        assert_eq!(notes(&flow, "not evaluated"), 0, "{:#?}", flow.diagnostics);
        assert_eq!(
            access(&flow, "GET", "/api/v1/other"),
            &protected(Condition::Authenticated)
        );
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
            // the later call scopes the chain (it is no rule); the rules end before it
            let flow = kotlin(body);
            assert_eq!(notes(&flow, "not evaluated"), 0, "{:#?}", flow.diagnostics);
            for path in ["/api/v1/open/ping", "/api/v1/other"] {
                assert_eq!(
                    access(&flow, "GET", path),
                    &protected(Condition::Authenticated)
                );
            }
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
        for (i, text) in java.iter().enumerate() {
            let flow = configured(&[("src/main/java/app/C.java", text)]);
            if i < 3 {
                // a customizer next to the chain: the scope is read, the rules apply in it
                assert_eq!(notes(&flow, "not evaluated"), 0, "{text}");
                assert_eq!(
                    access(&flow, "GET", "/api/v1/open/ping"),
                    &AccessState::Open
                );
                continue;
            }
            assert_not_evaluated(&flow);
            assert_eq!(
                access(&flow, "GET", "/api/v1/open/ping"),
                &AccessState::Undeclared,
                "{text}"
            );
        }
        let kt = "class C : WebSecurityConfigurerAdapter(), WebSecurityCustomizer {\n    override fun configure(http: HttpSecurity) {\n        http.antMatcher(\"/api/**\")\n        http.authorizeRequests().anyRequest().authenticated()\n    }\n}\n";
        // a literal scope on the same statement chain is read (F32): rules apply inside it
        let flow = configured(&[("src/main/java/app/C.kt", kt)]);
        assert_eq!(notes(&flow, "not evaluated"), 0, "{:#?}", flow.diagnostics);
        assert_eq!(
            access(&flow, "GET", "/api/v1/other"),
            &protected(Condition::Authenticated)
        );
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
        // the lambda form returns the HttpSecurity: `.antMatcher(..)` scopes the chain
        let flow = configured(&[("src/main/java/app/C.java", text)]);
        assert_eq!(notes(&flow, "not evaluated"), 0, "{:#?}", flow.diagnostics);
        assert_eq!(
            access(&flow, "GET", "/api/v1/other"),
            &AccessState::Undeclared
        );
        let kt = "class C : WebSecurityConfigurerAdapter() {\n    override fun configure(http: HttpSecurity) {\n        http.authorizeRequests { a -> a.anyRequest().permitAll() }.antMatcher(\"/public/**\")\n    }\n}\n";
        // Kotlin braces are no readable rule call: a gap, nothing applied
        let flow = configured(&[("src/main/java/app/C.kt", kt)]);
        assert_eq!(
            access(&flow, "GET", "/api/v1/other"),
            &AccessState::Undeclared
        );
        assert!(!flow.complete, "{:#?}", flow.diagnostics);
        // Kotlin statements separated by newlines only: the call scopes the chain (F32)
        for (x, ping) in [
            ("http.antMatcher(\"/admin/**\")", AccessState::Undeclared),
            ("http.antMatcher(\"/api/**\")", AccessState::Open),
        ] {
            let kt = format!(
                "class C : WebSecurityConfigurerAdapter() {{\n    override fun configure(http: HttpSecurity) {{\n        http.authorizeRequests({{ a ->\n            a.antMatchers(\"/api/v1/open/**\").permitAll()\n            a.anyRequest().authenticated()\n            {x}\n        }})\n    }}\n}}\n"
            );
            let flow = configured(&[("src/main/java/app/C.kt", &kt)]);
            assert_eq!(
                notes(&flow, "not evaluated"),
                0,
                "{x}: {:#?}",
                flow.diagnostics
            );
            assert_eq!(access(&flow, "GET", "/api/v1/open/ping"), &ping, "{x}");
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
            "http.regexMatcher(\"/api/.*\").authorizeRequests().anyRequest().authenticated();",
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
            ("src/main/java/app/Enable.java", ENABLE_ALL),
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
    const PERMIT_CHAIN: &str = r#"
@Configuration
public class SecurityConfig {
    @Bean SecurityFilterChain chain(HttpSecurity http) throws Exception {
        http.authorizeHttpRequests(a -> a.anyRequest().permitAll());
        return http.build();
    }
}
"#;

    /// A Spring project without any method-security switch.
    fn spring_bare(files: &[(&str, &str)]) -> UiFlow {
        let mut all = vec![("pom.xml", "<project/>")];
        all.extend(files.iter().map(|(n, t)| (*n, *t)));
        extract(project(&all).path())
    }

    /// A Spring project with all method-security families enabled (a file of
    /// the same name replaces the switches).
    fn spring_flow(files: &[(&str, &str)]) -> UiFlow {
        let mut all = vec![("src/main/java/Enable.java", ENABLE_ALL)];
        all.extend_from_slice(files);
        spring_bare(&all)
    }

    fn controller(body: &str) -> String {
        format!("@RestController\n{body}\n")
    }

    // F01-F05: annotation families override per family, different families AND.
    #[test]
    fn spring_annotation_families_override_per_family() {
        let flow = spring_flow(&[(
            "src/main/java/A.java",
            &controller(
                r#"@RestController @PreAuthorize("hasRole('ADMIN')")
class A {
    @PermitAll @GetMapping("/f01") void a() {}
    @PreAuthorize("permitAll()") @GetMapping("/f02") void b() {}
    @Secured("ROLE_USER") @GetMapping("/f03") void c() {}
    @PreAuthorize("hasRole('USER')") @GetMapping("/f04") void d() {}
}
@RestController @Secured("ROLE_ADMIN")
class B { @PreAuthorize("permitAll()") @GetMapping("/f02b") void b() {} }
@RestController @RolesAllowed("ADMIN")
class C { @PermitAll @GetMapping("/f05") void e() {} }"#,
            ),
        )]);
        let admin = protected(role("ROLE_ADMIN"));
        assert_eq!(access(&flow, "GET", "/f01"), &admin);
        assert_eq!(access(&flow, "GET", "/f02b"), &admin);
        assert_eq!(
            access(&flow, "GET", "/f03"),
            &protected(Condition::All(vec![role("ROLE_ADMIN"), role("ROLE_USER")]))
        );
        assert_eq!(access(&flow, "GET", "/f04"), &protected(role("ROLE_USER")));
        assert_eq!(access(&flow, "GET", "/f05"), &AccessState::Open);
    }

    // F06: `@PostAuthorize` protects, with a note; a chain `permitAll` stays no lift.
    #[test]
    fn spring_post_authorize_protects_with_a_note() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/A.java",
                &controller(
                    r#"class A { @PostAuthorize("hasRole('ADMIN')") @GetMapping("/f06") void a() {} }"#,
                ),
            ),
        ]);
        assert_eq!(access(&flow, "GET", "/f06"), &protected(role("ROLE_ADMIN")));
        assert_eq!(notes(&flow, "after the call"), 1, "{:#?}", flow.diagnostics);
    }

    // F07: meta-annotation; a template placeholder is never guessed.
    #[test]
    fn spring_meta_annotations_are_resolved() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/IsAdmin.java",
                "@PreAuthorize(\"hasRole('ADMIN')\")\n@interface IsAdmin {}\n",
            ),
            (
                "src/main/java/HasRole.java",
                "@PreAuthorize(\"hasRole('{value}')\")\npublic @interface HasRole { String value(); }\n",
            ),
            (
                "src/main/java/A.java",
                &controller(
                    r#"class A {
    @IsAdmin @GetMapping("/f07") void a() {}
    @HasRole("ADMIN") @GetMapping("/tpl") void b() {}
}"#,
                ),
            ),
        ]);
        assert_eq!(access(&flow, "GET", "/f07"), &protected(role("ROLE_ADMIN")));
        assert!(matches!(
            access(&flow, "GET", "/tpl"),
            AccessState::Protected(Condition::Unknown(_))
        ));
        assert!(!flow.complete);
    }

    // F08: annotation on the interface method and on the superclass.
    #[test]
    fn spring_interface_and_superclass_annotations_apply() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Api.java",
                "interface Api {\n    @PreAuthorize(\"hasRole('ADMIN')\")\n    void a();\n}\n",
            ),
            (
                "src/main/java/Base.java",
                "@PreAuthorize(\"hasRole('BASE')\")\nclass Base {}\n",
            ),
            (
                "src/main/java/A.java",
                &controller(
                    r#"class A implements Api { @GetMapping("/f08") public void a() {} }
@RestController class D extends Base { @GetMapping("/sup") void d() {} }"#,
                ),
            ),
        ]);
        assert_eq!(access(&flow, "GET", "/f08"), &protected(role("ROLE_ADMIN")));
        assert_eq!(access(&flow, "GET", "/sup"), &protected(role("ROLE_BASE")));
    }

    // F26: a regex path variable may not match the endpoint: no lift to Open.
    #[test]
    fn spring_regex_path_variable_is_a_partial_match() {
        let chain = PERMIT_CHAIN.replace(
            "a.anyRequest().permitAll()",
            "a.requestMatchers(\"/u/{id:\\\\d+}\").permitAll()",
        );
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", &chain),
            (
                "src/main/java/A.java",
                &controller(r#"class A { @GetMapping("/u/me") void a() {} }"#),
            ),
        ]);
        assert_eq!(access(&flow, "GET", "/u/me"), &AccessState::Undeclared);
        assert_eq!(
            notes(&flow, "only some requests"),
            1,
            "{:#?}",
            flow.diagnostics
        );
    }
    fn minimal_flow(text: &str) -> UiFlow {
        extract(project(&[("Api/Api.csproj", "<Project />"), ("Api/Program.cs", text)]).path())
    }

    // C.6 #1: a variable of the same name in another method is not this one.
    #[test]
    fn minimal_group_variable_is_scoped_to_its_method() {
        let flow = minimal_flow(
            r#"
static class Routes {
    static void A(WebApplication app) {
        var group = app.MapGroup("/secure").RequireAuthorization("Admin");
        group.MapGet("/a", () => 1);
    }
    static void B(RouteGroupBuilder group) {
        group.MapGet("/b", () => 2);
        group.AllowAnonymous();
    }
}
"#,
        );
        assert_eq!(
            access(&flow, "GET", "/secure/a"),
            &protected(policy("Admin"))
        );
        assert_eq!(access(&flow, "GET", "/b"), &AccessState::Open);
    }

    // C.6 #2: two locals of one name in two methods.
    #[test]
    fn minimal_same_local_name_in_two_methods() {
        let flow = minimal_flow(
            r#"
static class Routes {
    static void M1(WebApplication app) {
        var g = app.MapGroup("/x").RequireAuthorization();
        g.MapGet("/1", () => 1);
    }
    static void M2(WebApplication app) {
        var g = app.MapGroup("/y");
        g.MapGet("/2", () => 1);
    }
}
"#,
        );
        assert_eq!(
            access(&flow, "GET", "/x/1"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(access(&flow, "GET", "/y/2"), &AccessState::Undeclared);
    }

    // C.6 #3: a lambda parameter named like a variable of another method.
    #[test]
    fn minimal_lambda_parameter_is_not_an_unrelated_variable() {
        let flow = minimal_flow(
            r#"
static class Routes {
    static void M1(WebApplication app) {
        var group = app.MapGroup("/secure").RequireAuthorization("Admin");
        group.MapGet("/a", () => 1);
    }
    static void M2(WebApplication app) {
        app.MapGroup("/z").RequireAuthorization().Also(group => group.MapGet("/q", () => 1));
    }
}
"#,
        );
        assert_eq!(
            access(&flow, "GET", "/secure/a"),
            &protected(policy("Admin"))
        );
        assert_eq!(access(&flow, "GET", "/q"), &AccessState::Undeclared);
    }

    // C.6 #4 (order independence), kept as a guard for the scoping change.
    #[test]
    fn minimal_group_modifier_after_the_endpoint_still_counts() {
        let flow = minimal_flow(
            r#"
var app = builder.Build();
var g = app.MapGroup("/t");
g.MapGet("/", () => 1);
g.RequireAuthorization("P");
"#,
        );
        assert_eq!(access(&flow, "GET", "/t"), &protected(policy("P")));
    }
    // Review 1: a supertype method with another parameter count is no override.
    #[test]
    fn spring_inherited_overload_with_other_arity_is_not_applied() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Api.java",
                "interface Api {\n    @PermitAll\n    void a(int x);\n}\n",
            ),
            (
                "src/main/java/A.java",
                r#"@RestController @RolesAllowed("ADMIN")
class A implements Api { @GetMapping("/ov") public void a() {} }"#,
            ),
        ]);
        assert_eq!(access(&flow, "GET", "/ov"), &protected(role("ROLE_ADMIN")));
        assert!(!flow.complete);
    }

    // Review 2: an ambiguous supertype that may carry a check is never read as none.
    #[test]
    fn spring_ambiguous_supertype_with_a_check_is_unknown() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/D1.java",
                "@PreAuthorize(\"hasRole('X')\")\nclass Dup {}\n",
            ),
            ("src/main/java/D2.java", "class Dup {}\n"),
            (
                "src/main/java/A.java",
                r#"@RestController
class A extends Dup { @GetMapping("/dup") void a() {} }"#,
            ),
        ]);
        assert!(matches!(
            access(&flow, "GET", "/dup"),
            AccessState::Protected(Condition::Unknown(_))
        ));
        assert!(!flow.complete);
    }

    // W5a: routes declared only on a supertype are the controller's routes.
    #[test]
    fn spring_interface_route_is_emitted_for_the_implementing_controller() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Api.java",
                r#"interface Api {
    String BASE = "/v1";
    @PreAuthorize("hasRole('ADMIN')")
    @GetMapping(BASE + "/items/{id}")
    String get(@PathVariable("id") String id);
    @DeleteMapping("/items/{id}")
    void drop(String id);
}"#,
            ),
            (
                "src/main/java/Impl.java",
                r#"@RestController
class Impl implements Api {
    @Override public String get(String id) { return id; }
    @Override public void drop(String id) {}
}"#,
            ),
        ]);
        assert_eq!(
            access(&flow, "GET", "/v1/items/{id}"),
            &protected(role("ROLE_ADMIN"))
        );
        let e = (flow.endpoints.iter())
            .find(|e| e.method == "DELETE")
            .unwrap();
        assert_eq!(
            (e.file.as_str(), e.source.as_str()),
            ("src/main/java/Impl.java", "spring")
        );
        assert_eq!(e.handler.as_deref(), Some("Impl.drop"));
    }

    #[test]
    fn spring_interface_route_overridden_with_own_mapping_is_not_duplicated() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Api.java",
                "interface Api {\n    @GetMapping(\"/a\") String a();\n    @GetMapping(\"/b\") String b();\n}\n",
            ),
            (
                "src/main/java/Impl.java",
                r#"@RestController
class Impl implements Api {
    @Override @GetMapping("/a2") public String a() { return ""; }
    @Override public String b() { return ""; }
}"#,
            ),
        ]);
        assert_eq!(flow.endpoints.iter().filter(|e| e.path == "/a").count(), 0);
        access(&flow, "GET", "/a2");
        access(&flow, "GET", "/b");
        assert_eq!(flow.endpoints.len(), 2, "{:#?}", flow.endpoints);
    }

    #[test]
    fn spring_interface_type_mapping_is_the_base_path() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Api.java",
                "@RequestMapping(\"/api\")\ninterface Api {\n    @GetMapping(\"/x\") String x();\n}\n",
            ),
            (
                "src/main/java/Impl.java",
                "@RestController\nclass Impl implements Api { public String x() { return \"\"; } }\n",
            ),
        ]);
        access(&flow, "GET", "/api/x");
    }

    #[test]
    fn spring_unreadable_supertype_of_a_controller_is_a_gap() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Impl.java",
                "@RestController\nclass Impl extends Lib { @GetMapping(\"/own\") String own() { return \"\"; } }\n",
            ),
        ]);
        assert_eq!(gaps(&flow, "inherited from supertype Lib"), 1);
        assert!(!flow.complete);
    }

    #[test]
    fn spring_interface_constants_without_final_and_static_imports_resolve() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Consts.java",
                "class Consts { public static final String V1 = \"/rest/v1\"; }\n",
            ),
            (
                "src/main/java/Api.java",
                "import static x.Consts.V1;\ninterface Api {\n    String ITEMS = V1 + \"/items\";\n    @GetMapping(value = ITEMS + \"/{id}\") String x(String id);\n}\n",
            ),
            (
                "src/main/java/Impl.java",
                "@RestController\nclass Impl implements Api { @Override public String x(String id) { return id; } }\n",
            ),
        ]);
        access(&flow, "GET", "/rest/v1/items/{id}");
    }

    #[test]
    fn spring_same_named_supertypes_resolve_by_import() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/a/Api.java",
                "package a;\ninterface Api { @GetMapping(\"/a\") String x(); }\n",
            ),
            (
                "src/main/java/b/Api.java",
                "package b;\ninterface Api { @GetMapping(\"/b\") String x(); }\n",
            ),
            (
                "src/main/java/Impl.java",
                "package c;\nimport b.Api;\n@RestController\nclass Impl implements Api { public String x() { return \"\"; } }\n",
            ),
        ]);
        access(&flow, "GET", "/b");
        assert_eq!(flow.endpoints.len(), 1, "{:#?}", flow.endpoints);
        assert_eq!(gaps(&flow, "inherited from supertype"), 0);
    }

    #[test]
    fn spring_abstract_controller_base_is_emitted_once_by_its_subclass() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Base.java",
                "@RestController\nabstract class Base { @GetMapping(\"/b\") String b() { return \"\"; } }\n",
            ),
            (
                "src/main/java/Impl.java",
                "@RestController\nclass Impl extends Base {}\n",
            ),
        ]);
        let e = flow
            .endpoints
            .iter()
            .filter(|e| e.path == "/b")
            .collect::<Vec<_>>();
        assert_eq!(e.len(), 1, "{e:#?}");
        assert_eq!(e[0].file, "src/main/java/Impl.java");
    }

    #[test]
    fn spring_type_mapping_on_several_supertypes_is_a_gap() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Api.java",
                "@RequestMapping(\"/i\")\ninterface Api { @GetMapping(\"/x\") String x(); }\n",
            ),
            (
                "src/main/java/Base.java",
                "@RequestMapping(\"/s\")\nclass Base {}\n",
            ),
            (
                "src/main/java/Impl.java",
                "@RestController\nclass Impl extends Base implements Api { public String x() { return \"\"; } }\n",
            ),
        ]);
        assert_eq!(gaps(&flow, "type-level @RequestMapping"), 1);
        assert!(
            flow.endpoints.iter().all(|e| e.path != "/s/x"),
            "{:#?}",
            flow.endpoints
        );
    }

    #[test]
    fn spring_project_type_named_like_a_jdk_root_is_a_supertype() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Record.java",
                "interface Record { @GetMapping(\"/r\") String r(); }\n",
            ),
            (
                "src/main/java/Impl.java",
                "@RestController\nclass Impl implements Record { public String r() { return \"\"; } }\n",
            ),
        ]);
        access(&flow, "GET", "/r");
    }

    // Spring finds @Controller through the whole type hierarchy.
    #[test]
    fn spring_class_below_an_abstract_controller_is_a_controller() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Base.java",
                "@RestController\nabstract class Base { @GetMapping(\"/b\") String b() { return \"\"; } }\n",
            ),
            (
                "src/main/java/Impl.java",
                "@Service\nclass Impl extends Base {}\n",
            ),
        ]);
        let e = (flow.endpoints.iter().filter(|e| e.path == "/b")).collect::<Vec<_>>();
        assert_eq!(e.len(), 1, "{e:#?}");
        assert_eq!(e[0].file, "src/main/java/Impl.java");
    }

    #[test]
    fn spring_abstract_controller_without_subclass_is_a_gap() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Base.java",
                "@RestController\nabstract class Base { @GetMapping(\"/b\") String b() { return \"\"; } }\n",
            ),
        ]);
        assert_eq!(gaps(&flow, "no concrete subclass"), 1);
    }

    #[test]
    fn spring_abstract_in_a_type_name_is_not_an_abstract_class() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/C.java",
                "@RestController\nclass C implements x.abstractapi.Api { @GetMapping(\"/c\") String c() { return \"\"; } }\n",
            ),
        ]);
        access(&flow, "GET", "/c");
    }

    #[test]
    fn spring_type_mapping_of_a_supertype_is_the_base_of_own_routes() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Base.java",
                "@RequestMapping(\"/base\")\nclass Base {}\n",
            ),
            (
                "src/main/java/C.java",
                "@RestController\nclass C extends Base { @GetMapping(\"/c\") String c() { return \"\"; } }\n",
            ),
        ]);
        access(&flow, "GET", "/base/c");
    }

    #[test]
    fn spring_abstract_controller_subclass_in_another_package_does_not_hide_the_gap() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/a/Base.java",
                "package a;\n@RestController\nabstract class Base { @GetMapping(\"/b\") String b() { return \"\"; } }\n",
            ),
            ("src/main/java/b/Base.java", "package b;\nclass Base {}\n"),
            (
                "src/main/java/b/X.java",
                "package b;\nclass X extends Base {}\n",
            ),
        ]);
        assert_eq!(gaps(&flow, "no concrete subclass"), 1);
    }

    // Guard: a class that is no controller serves nothing.
    #[test]
    fn spring_interface_route_without_controller_is_no_endpoint() {
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            (
                "src/main/java/Api.java",
                "interface Api { @GetMapping(\"/x\") String x(); }\n",
            ),
            (
                "src/main/java/Impl.java",
                "class Impl implements Api { public String x() { return \"\"; } }\n",
            ),
        ]);
        assert!(flow.endpoints.is_empty(), "{:#?}", flow.endpoints);
    }

    // ---- wave 2a: Spring conventions, method-security switches, chain scoping ----

    const ENABLE_ALL: &str = "import org.springframework.security.config.annotation.method.configuration.EnableMethodSecurity;\n@EnableMethodSecurity(securedEnabled = true, jsr250Enabled = true)\nclass Enable {}\n";
    const AUTH_CHAIN: &str = "@Configuration\npublic class SecurityConfig {\n    @Bean SecurityFilterChain chain(HttpSecurity http) throws Exception {\n        http.authorizeHttpRequests(a -> a.anyRequest().authenticated());\n        return http.build();\n    }\n}\n";

    fn gaps(flow: &UiFlow, part: &str) -> usize {
        (flow.diagnostics.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Gap && d.message.contains(part))
            .count()
    }

    fn convention(flow: &UiFlow) -> &RoleConvention {
        assert_eq!(
            flow.role_conventions.len(),
            1,
            "{:#?}",
            flow.role_conventions
        );
        &flow.role_conventions[0]
    }

    // F09: an annotation template is filled from the use site only with its bean.
    #[test]
    fn f09_annotation_template_needs_its_bean() {
        let template = "@PreAuthorize(\"hasRole('{value}')\")\npublic @interface HasRole { String value(); }\n";
        let user = controller("class A { @HasRole(\"ADMIN\") @GetMapping(\"/f09\") void a() {} }");
        let bean = "class B { @Bean static AnnotationTemplateExpressionDefaults d() { return new AnnotationTemplateExpressionDefaults(); } }\n";
        let with = spring_flow(&[
            ("src/main/java/HasRole.java", template),
            ("src/main/java/B.java", bean),
            ("src/main/java/A.java", &user),
        ]);
        assert_eq!(access(&with, "GET", "/f09"), &protected(role("ROLE_ADMIN")));
        assert!(with.complete, "{:#?}", with.diagnostics);
        let without = spring_flow(&[
            ("src/main/java/HasRole.java", template),
            ("src/main/java/A.java", &user),
        ]);
        assert!(matches!(
            access(&without, "GET", "/f09"),
            AccessState::Protected(Condition::Unknown(_))
        ));
        assert!(!without.complete);
    }

    // F10: two interfaces declaring the same method annotation are ambiguous.
    #[test]
    fn f10_two_interfaces_with_the_annotation_are_unknown() {
        let flow = spring_flow(&[
            (
                "src/main/java/I1.java",
                "interface I1 {\n    @PreAuthorize(\"hasRole('A')\")\n    void a();\n}\n",
            ),
            (
                "src/main/java/I2.java",
                "interface I2 {\n    @PreAuthorize(\"hasRole('B')\")\n    void a();\n}\n",
            ),
            (
                "src/main/java/A.java",
                &controller(
                    "class A implements I1, I2 { @GetMapping(\"/f10\") public void a() {} }",
                ),
            ),
        ]);
        assert!(matches!(
            access(&flow, "GET", "/f10"),
            AccessState::Protected(Condition::Unknown(_))
        ));
        assert!(!flow.complete);
    }

    // F11-F13: an annotation family that is not enabled adds nothing, with a gap.
    #[test]
    fn f11_no_enabling_annotation_ignores_method_security_with_a_gap() {
        let method = controller(
            "class A { @PreAuthorize(\"hasRole('ADMIN')\") @GetMapping(\"/f11\") void a() {} }",
        );
        let flow = spring_bare(&[
            ("src/main/java/Sec.java", AUTH_CHAIN),
            ("src/main/java/A.java", &method),
        ]);
        assert_eq!(
            access(&flow, "GET", "/f11"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(gaps(&flow, "method security"), 1, "{:#?}", flow.diagnostics);
        assert!(!flow.complete);
    }

    #[test]
    fn f12_f13_a_family_that_is_off_is_ignored_with_a_gap() {
        let secured =
            controller("class A { @Secured(\"ROLE_A\") @GetMapping(\"/f12\") void a() {} }");
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", AUTH_CHAIN),
            (
                "src/main/java/Enable.java",
                "@EnableMethodSecurity\nclass Enable {}\n",
            ),
            ("src/main/java/A.java", &secured),
        ]);
        assert_eq!(
            access(&flow, "GET", "/f12"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(gaps(&flow, "securedEnabled"), 1, "{:#?}", flow.diagnostics);
        let pre = controller(
            "class A { @PreAuthorize(\"hasRole('A')\") @GetMapping(\"/f13\") void a() {} }",
        );
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", AUTH_CHAIN),
            (
                "src/main/java/Enable.java",
                "@EnableGlobalMethodSecurity(securedEnabled = true)\nclass Enable {}\n",
            ),
            ("src/main/java/A.java", &pre),
        ]);
        assert_eq!(
            access(&flow, "GET", "/f13"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(gaps(&flow, "prePostEnabled"), 1, "{:#?}", flow.diagnostics);
    }

    // An ignored annotation never lets a chain `permitAll` turn the endpoint Open.
    #[test]
    fn an_ignored_annotation_is_never_lifted_to_open() {
        let method = controller(
            "class A { @PreAuthorize(\"hasRole('ADMIN')\") @GetMapping(\"/x\") void a() {} }",
        );
        let flow = spring_bare(&[
            ("src/main/java/Sec.java", PERMIT_CHAIN),
            ("src/main/java/A.java", &method),
        ]);
        assert_eq!(access(&flow, "GET", "/x"), &AccessState::Undeclared);
        assert!(!flow.complete);
    }

    // A switch that is not a literal boolean is unreadable: Unknown, not off and not on.
    #[test]
    fn an_unreadable_switch_makes_the_family_unknown() {
        let method = controller("class A { @Secured(\"ROLE_A\") @GetMapping(\"/x\") void a() {} }");
        let flow = spring_flow(&[
            (
                "src/main/java/Enable.java",
                "@EnableMethodSecurity(securedEnabled = FLAGS.SECURED)\nclass Enable {}\n",
            ),
            ("src/main/java/A.java", &method),
        ]);
        assert!(matches!(
            access(&flow, "GET", "/x"),
            AccessState::Protected(Condition::Unknown(_))
        ));
        assert!(!flow.complete);
    }

    // F14: the legacy annotation applies the first family with a hit.
    #[test]
    fn f14_legacy_method_security_uses_the_first_family() {
        let method = controller(
            "@PreAuthorize(\"hasRole('A')\")\nclass A { @Secured(\"ROLE_B\") @GetMapping(\"/f14\") void a() {} }",
        );
        let flow = spring_flow(&[
            (
                "src/main/java/Enable.java",
                "@EnableGlobalMethodSecurity(prePostEnabled = true, securedEnabled = true)\nclass Enable {}\n",
            ),
            ("src/main/java/A.java", &method),
        ]);
        assert_eq!(access(&flow, "GET", "/f14"), &protected(role("ROLE_A")));
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    // F15, F18: an authority is an exact name; a role gets the prefix.
    #[test]
    fn f15_f18_authorities_are_not_roles() {
        let flow = spring_flow(&[(
            "src/main/java/A.java",
            &controller(
                r#"class A {
    @PreAuthorize("hasAuthority('ADMIN')") @GetMapping("/auth") void a() {}
    @PreAuthorize("hasRole('ADMIN')") @GetMapping("/role") void b() {}
    @PreAuthorize("hasAuthority('ROLE_ADMIN')") @GetMapping("/stored") void c() {}
    @Secured("ADMIN") @GetMapping("/f18") void d() {}
}"#,
            ),
        )]);
        assert_eq!(access(&flow, "GET", "/auth"), &protected(policy("ADMIN")));
        assert_eq!(
            access(&flow, "GET", "/role"),
            &protected(role("ROLE_ADMIN"))
        );
        assert_eq!(
            access(&flow, "GET", "/stored"),
            &protected(role("ROLE_ADMIN"))
        );
        assert_eq!(access(&flow, "GET", "/f18"), &protected(policy("ADMIN")));
    }

    // F16: the prefix of a `GrantedAuthorityDefaults` bean is read.
    #[test]
    fn f16_granted_authority_defaults_set_the_role_prefix() {
        let beans = "class B { @Bean GrantedAuthorityDefaults d() { return new GrantedAuthorityDefaults(\"\"); } }\n";
        let flow = spring_flow(&[
            ("src/main/java/B.java", beans),
            (
                "src/main/java/A.java",
                &controller(
                    r#"class A {
    @PreAuthorize("hasRole('ADMIN')") @GetMapping("/r") void a() {}
    @PreAuthorize("hasAuthority('ADMIN')") @GetMapping("/a") void b() {}
}"#,
                ),
            ),
        ]);
        assert_eq!(access(&flow, "GET", "/r"), &protected(role("ADMIN")));
        assert_eq!(access(&flow, "GET", "/a"), access(&flow, "GET", "/r"));
        let c = convention(&flow);
        assert_eq!(c.role_prefix.as_deref(), Some(""));
        assert!(
            c.origins
                .iter()
                .any(|o| o.field == "role_prefix" && o.file.ends_with("B.java") && o.line == 1)
        );
    }

    // A prefix that is not a literal is not guessed: role conditions are Unknown.
    #[test]
    fn an_unreadable_role_prefix_makes_role_names_unknown() {
        let beans = "class B { @Bean GrantedAuthorityDefaults d() { return new GrantedAuthorityDefaults(PREFIX); } }\n";
        let flow = spring_flow(&[
            ("src/main/java/B.java", beans),
            (
                "src/main/java/A.java",
                &controller(
                    "class A { @PreAuthorize(\"hasRole('ADMIN')\") @GetMapping(\"/r\") void a() {} }",
                ),
            ),
        ]);
        assert!(matches!(
            access(&flow, "GET", "/r"),
            AccessState::Protected(Condition::Unknown(_))
        ));
        assert!(!flow.complete);
        assert_eq!(convention(&flow).role_prefix, None);
    }

    // F17: JSR-250 adds the prefix, it does not strip it.
    #[test]
    fn f17_roles_allowed_adds_the_prefix() {
        let flow = spring_flow(&[(
            "src/main/java/A.java",
            &controller(
                r#"class A {
    @RolesAllowed("ROLE_ADMIN") @GetMapping("/f17") void a() {}
    @RolesAllowed("ADMIN") @GetMapping("/plain") void b() {}
}"#,
            ),
        )]);
        assert_eq!(
            access(&flow, "GET", "/f17"),
            &protected(role("ROLE_ROLE_ADMIN"))
        );
        assert_eq!(
            access(&flow, "GET", "/plain"),
            &protected(role("ROLE_ADMIN"))
        );
    }

    // F41: scopes keep their name; the scope prefix is the framework default.
    #[test]
    fn f41_scope_authorities_and_the_default_scope_prefix() {
        let flow = spring_flow(&[(
            "src/main/java/A.java",
            &controller(
                "class A { @PreAuthorize(\"hasAuthority('SCOPE_read')\") @GetMapping(\"/f41\") void a() {} }",
            ),
        )]);
        assert_eq!(
            access(&flow, "GET", "/f41"),
            &protected(policy("SCOPE_read"))
        );
        let c = convention(&flow);
        assert_eq!(c.scope_prefix.as_deref(), Some("SCOPE_"));
        assert!(
            c.origins
                .iter()
                .any(|o| o.field == "scope_prefix" && o.source.contains("framework default"))
        );
    }

    // F42: a JwtGrantedAuthoritiesConverter gives the scope prefix and the claim.
    #[test]
    fn f42_jwt_converter_gives_scope_prefix_and_claim() {
        let conv = "class J { JwtGrantedAuthoritiesConverter c() { var c = new JwtGrantedAuthoritiesConverter(); c.setAuthorityPrefix(\"ROLE_\"); c.setAuthoritiesClaimName(\"roles\"); return c; } }\n";
        let flow = spring_flow(&[
            ("src/main/java/J.java", conv),
            (
                "src/main/java/A.java",
                &controller(
                    "class A { @PreAuthorize(\"hasRole('admin')\") @GetMapping(\"/f42\") void a() {} }",
                ),
            ),
        ]);
        assert_eq!(access(&flow, "GET", "/f42"), &protected(role("ROLE_admin")));
        let c = convention(&flow);
        assert_eq!(c.role_claim.as_deref(), Some("roles"));
        assert_eq!(c.scope_prefix.as_deref(), Some("ROLE_"));
    }

    // Quarkus Spring compatibility: role names are exact, nothing is stripped or added.
    #[test]
    fn quarkus_spring_controllers_keep_role_names_exact() {
        let flow = extract(
            project(&[
                (
                    "pom.xml",
                    "<project><dependencies><dependency><artifactId>quarkus-spring-security</artifactId></dependency></dependencies></project>",
                ),
                (
                    "src/main/java/A.java",
                    r#"import org.springframework.web.bind.annotation.GetMapping;
@RestController
class A {
    @Secured("admin") @GetMapping("/s") void a() {}
    @PreAuthorize("hasRole('admin')") @GetMapping("/p") void b() {}
}"#,
                ),
            ])
            .path(),
        );
        assert_eq!(access(&flow, "GET", "/s"), &protected(role("admin")));
        assert_eq!(access(&flow, "GET", "/p"), &protected(role("admin")));
        assert_eq!(convention(&flow).role_prefix.as_deref(), Some(""));
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    const SCOPED_ENDPOINTS: &str = "@RestController class E {\n    @GetMapping(\"/api/x\") void a() {}\n    @GetMapping(\"/web\") void b() {}\n}\n";

    // F30: ordered chains; the first chain whose scope matches decides.
    #[test]
    fn f30_two_chains_apply_by_order_and_scope() {
        let chains = r#"@Configuration
class Sec {
    @Bean @Order(1)
    SecurityFilterChain api(HttpSecurity http) throws Exception {
        http.securityMatcher("/api/**").authorizeHttpRequests(a -> a.anyRequest().hasRole("ADMIN"));
        return http.build();
    }
    @Bean
    SecurityFilterChain web(HttpSecurity http) throws Exception {
        http.authorizeHttpRequests(a -> a.anyRequest().authenticated());
        return http.build();
    }
}
"#;
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", chains),
            ("src/main/java/E.java", SCOPED_ENDPOINTS),
        ]);
        assert_eq!(
            access(&flow, "GET", "/api/x"),
            &protected(role("ROLE_ADMIN"))
        );
        assert_eq!(
            access(&flow, "GET", "/web"),
            &protected(Condition::Authenticated)
        );
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    // Chains whose order is equal or unreadable cannot be put in sequence.
    #[test]
    fn chains_without_a_readable_distinct_order_are_not_evaluated() {
        let chains = |first: &str| {
            format!(
                "@Configuration\nclass Sec {{\n    @Bean {first}\n    SecurityFilterChain api(HttpSecurity http) throws Exception {{\n        http.securityMatcher(\"/api/**\").authorizeHttpRequests(a -> a.anyRequest().hasRole(\"ADMIN\"));\n        return http.build();\n    }}\n    @Bean\n    SecurityFilterChain web(HttpSecurity http) throws Exception {{\n        http.authorizeHttpRequests(a -> a.anyRequest().authenticated());\n        return http.build();\n    }}\n}}\n"
            )
        };
        for first in ["", "@Order(ORDER)"] {
            let flow = spring_flow(&[
                ("src/main/java/Sec.java", &chains(first)),
                ("src/main/java/E.java", SCOPED_ENDPOINTS),
            ]);
            assert_eq!(access(&flow, "GET", "/api/x"), &AccessState::Undeclared);
            assert_eq!(
                notes(&flow, "not evaluated"),
                1,
                "{first}: {:#?}",
                flow.diagnostics
            );
        }
    }

    // F31: one scoped chain leaves what is outside its scope undeclared.
    #[test]
    fn f31_a_scoped_chain_does_not_protect_outside_its_scope() {
        let chain = "@Configuration\nclass Sec {\n    @Bean SecurityFilterChain api(HttpSecurity http) throws Exception {\n        http.securityMatcher(\"/api/**\").authorizeHttpRequests(a -> a.anyRequest().authenticated());\n        return http.build();\n    }\n}\n";
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", chain),
            ("src/main/java/E.java", SCOPED_ENDPOINTS),
        ]);
        assert_eq!(access(&flow, "GET", "/web"), &AccessState::Undeclared);
        assert_eq!(
            access(&flow, "GET", "/api/x"),
            &protected(Condition::Authenticated)
        );
    }

    // Several chains as hawkbit declares them: constant and array matchers,
    // a helper that only adds filters, a chain that is customised by code that
    // is not read, and two chains of equal order.
    #[test]
    fn central_chains_with_constant_matchers_apply_by_order() {
        let api = "public class Api { public static final String V1 = \"/v1\"; public static final String REST = \"/rest\"; }\n";
        let chains = r#"@Configuration
class Sec {
    static final String[] MATCHERS = { Api.V1 + "/**" };
    static final String DL = Api.V1 + "/dl/*";

    @Bean @Order(301)
    SecurityFilterChain device(HttpSecurity http) throws Exception {
        http.securityMatcher(MATCHERS).authorizeHttpRequests(a -> a.anyRequest().hasRole("DEVICE"));
        Mdc.addFilter(http);
        return http.build();
    }
    @Bean @Order(300)
    SecurityFilterChain download(HttpSecurity http) throws Exception {
        http.securityMatcher(DL).authorizeHttpRequests(a -> a.anyRequest().authenticated());
        Mdc.addFilter(http);
        return http.build();
    }
    @Bean @Order(350)
    SecurityFilterChain rest(HttpSecurity http, Customizer<HttpSecurity> custom) throws Exception {
        http.securityMatcher(Api.REST + "/**").authorizeHttpRequests(a -> a.anyRequest().authenticated());
        custom.customize(http);
        return http.build();
    }
    @Bean
    SecurityFilterChain ui(HttpSecurity http) throws Exception {
        http.authorizeHttpRequests(a -> a.requestMatchers("/img/*").permitAll());
        return http.with(Vaadin.vaadin(), c -> {});
    }
    @Bean
    SecurityFilterChain mcp(HttpSecurity http) throws Exception {
        http.securityMatcher("/mcp/**").authorizeHttpRequests(a -> a.anyRequest().permitAll());
        return http.build();
    }
}
"#;
        let mdc = "class Mdc { static void addFilter(final HttpSecurity h) { h.addFilterBefore(new F(), X.class); } }\n";
        let endpoints = "@RestController class E {\n    @GetMapping(\"/v1/dl/x\") void a() {}\n    @GetMapping(\"/v1/other\") void b() {}\n    @GetMapping(\"/rest/y\") void c() {}\n    @GetMapping(\"/mcp/z\") void d() {}\n    @GetMapping(\"/web\") void e() {}\n}\n";
        let flow = spring_flow(&[
            ("src/main/java/Api.java", api),
            ("src/main/java/Sec.java", chains),
            ("src/main/java/Mdc.java", mdc),
            ("src/main/java/E.java", endpoints),
        ]);
        let diag = format!("{:#?}", flow.diagnostics);
        assert_eq!(
            access(&flow, "GET", "/v1/dl/x"),
            &protected(Condition::Authenticated),
            "{diag}"
        );
        assert_eq!(
            access(&flow, "GET", "/v1/other"),
            &protected(role("ROLE_DEVICE")),
            "{diag}"
        );
        // customised by code that is not read: the rule plus an unknown part
        assert!(
            matches!(
                access(&flow, "GET", "/rest/y"),
                AccessState::Protected(Condition::All(v))
                    if v.contains(&Condition::Authenticated)
                        && v.iter().any(|c| matches!(c, Condition::Unknown(_)))
            ),
            "{diag}"
        );
        // reaching the customised chain: its code may have widened its scope, so
        // these are unknown, never Open
        for path in ["/mcp/z", "/web"] {
            assert!(
                matches!(
                    access(&flow, "GET", path),
                    AccessState::Protected(Condition::Unknown(_))
                ),
                "{path} {diag}"
            );
        }
        assert!(!flow.complete, "{diag}");
    }

    // Anything the chain code does not show stays undeclared, never open.
    #[test]
    fn unreadable_chain_code_never_opens_an_endpoint() {
        let endpoints = "@RestController class E {\n    @GetMapping(\"/v1/x\") void a() {}\n}\n";
        let mdc = "class Mdc { static void touch(HttpSecurity h) { h.authorizeHttpRequests(a -> a.anyRequest().permitAll()); } }\n";
        for (scope, rule, extra) in [
            (
                "http.securityMatcher(\"/v1/**\")",
                "a.anyRequest().permitAll()",
                "Mdc.touch(http);",
            ),
            (
                "http.securityMatcher(MATCHERS)",
                "a.anyRequest().permitAll()",
                "",
            ),
            (
                "http.securityMatcher(computed())",
                "a.anyRequest().permitAll()",
                "",
            ),
        ] {
            let chains = format!(
                "@Configuration\nclass Sec {{\n    static final String[] MATCHERS = {{ Api.V1 + \"/**\", Other.X }};\n    @Bean\n    SecurityFilterChain c(HttpSecurity http) throws Exception {{\n        {scope}.authorizeHttpRequests(a -> {rule});\n        {extra}\n        return http.build();\n    }}\n}}\n"
            );
            let flow = spring_flow(&[
                (
                    "src/main/java/Api.java",
                    "public class Api { public static final String V1 = \"/v1\"; }\n",
                ),
                ("src/main/java/Sec.java", &chains),
                ("src/main/java/Mdc.java", mdc),
                ("src/main/java/E.java", endpoints),
            ]);
            let got = access(&flow, "GET", "/v1/x");
            assert_ne!(
                got,
                &AccessState::Open,
                "{scope} {extra}: {:#?}",
                flow.diagnostics
            );
        }
    }

    // A chain in a module that depends on the controllers' module applies to
    // them in the application made of both; another application without it
    // makes the endpoints keep their declared access.
    #[test]
    fn a_chain_in_a_dependent_module_protects_the_controllers_of_its_application() {
        let pom = |name: &str, dep: &str| {
            let deps = match dep {
                "" => String::new(),
                d => format!(
                    "<dependencies><dependency><artifactId>{d}</artifactId></dependency></dependencies>"
                ),
            };
            format!("<project><artifactId>{name}</artifactId>{deps}</project>")
        };
        let chain = "@Configuration\nclass Sec {\n    @Bean SecurityFilterChain api(HttpSecurity http) throws Exception {\n        http.securityMatcher(\"/api/**\").authorizeHttpRequests(a -> a.anyRequest().authenticated());\n        return http.build();\n    }\n}\n";
        let (web, sec, app, other) = (
            pom("web", ""),
            pom("sec", "web"),
            pom("app", "sec"),
            pom("other", "web"),
        );
        let mut files = vec![
            ("pom.xml", "<project/>"),
            ("web/pom.xml", &web[..]),
            ("web/src/main/java/Enable.java", ENABLE_ALL),
            ("web/src/main/java/E.java", SCOPED_ENDPOINTS),
            ("sec/pom.xml", &sec[..]),
            ("sec/src/main/java/Sec.java", chain),
            ("app/pom.xml", &app[..]),
        ];
        let flow = extract(project(&files).path());
        let diag = format!("{:#?}", flow.diagnostics);
        assert_eq!(
            access(&flow, "GET", "/api/x"),
            &protected(Condition::Authenticated),
            "{diag}"
        );
        assert_eq!(
            access(&flow, "GET", "/web"),
            &AccessState::Undeclared,
            "{diag}"
        );
        // a second application includes the controllers without the chain
        files.push(("other/pom.xml", &other[..]));
        let flow = extract(project(&files).path());
        let diag = format!("{:#?}", flow.diagnostics);
        assert_eq!(
            access(&flow, "GET", "/api/x"),
            &AccessState::Undeclared,
            "{diag}"
        );
        assert_eq!(
            notes(&flow, "applications include this project"),
            1,
            "{diag}"
        );
        // the undone change is not reported (guard: written with the cache)
        assert_eq!(notes(&flow, "access of 1 endpoint(s) changed"), 0, "{diag}");
    }

    // Code outside the scan that gets the HttpSecurity may widen the scope it
    // is read with: endpoints outside the readable scope are unknown, so a
    // later chain's permitAll never opens them.
    #[test]
    fn a_hand_off_out_of_the_scan_keeps_a_later_permit_all_from_opening() {
        let chains = r#"@Configuration
class Sec {
    @Bean @Order(1)
    SecurityFilterChain a(HttpSecurity http) throws Exception {
        http.securityMatcher("/a/**").authorizeHttpRequests(x -> x.anyRequest().authenticated());
        External.widen(http);
        return http.build();
    }
    @Bean @Order(2)
    SecurityFilterChain b(HttpSecurity http) throws Exception {
        http.authorizeHttpRequests(x -> x.anyRequest().permitAll());
        return http.build();
    }
}
"#;
        let endpoints = "@RestController class E {\n    @GetMapping(\"/a/x\") void a() {}\n    @GetMapping(\"/b/y\") void b() {}\n}\n";
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", chains),
            ("src/main/java/E.java", endpoints),
        ]);
        let diag = format!("{:#?}", flow.diagnostics);
        let unknown = |m: &str, p: &str| matches!(access(&flow, m, p), AccessState::Protected(c) if format!("{c:?}").contains("Unknown"));
        assert!(unknown("GET", "/a/x"), "{diag}");
        assert!(
            unknown("GET", "/b/y"),
            "{:?} {diag}",
            access(&flow, "GET", "/b/y")
        );
        assert!(!flow.complete, "{diag}");
    }

    // The role prefix is read per project: a project whose prefix is not
    // readable stays unknown whichever project of the application is read first.
    #[test]
    fn a_shared_application_reads_the_chain_with_each_projects_role_prefix() {
        let pom = |name: &str, deps: &[&str]| {
            let d: String = deps
                .iter()
                .map(|d| format!("<dependency><artifactId>{d}</artifactId></dependency>"))
                .collect();
            format!(
                "<project><artifactId>{name}</artifactId><dependencies>{d}</dependencies></project>"
            )
        };
        let chain = "@Configuration\nclass Sec {\n    @Bean SecurityFilterChain c(HttpSecurity http) throws Exception {\n        http.authorizeHttpRequests(a -> a.anyRequest().hasRole(\"ADMIN\"));\n        return http.build();\n    }\n}\n";
        let ctl = "@RestController class E {\n    @GetMapping(\"/x\") void a() {}\n}\n";
        let bad = "class B { @Bean GrantedAuthorityDefaults d() { return new GrantedAuthorityDefaults(prefix()); } }\n";
        for (first, second) in [("web-a", "web-b"), ("web-b", "web-a")] {
            let (app, sec) = (pom("app", &["sec", first, second]), pom("sec", &[]));
            let (pa, pb) = (pom(first, &[]), pom(second, &[]));
            let files = [
                ("pom.xml", "<project/>"),
                ("app/pom.xml", &app[..]),
                ("sec/pom.xml", &sec[..]),
                ("sec/src/main/java/Sec.java", chain),
                (&format!("{first}/pom.xml")[..], &pa[..]),
                (&format!("{first}/src/main/java/E.java")[..], ctl),
                (&format!("{second}/pom.xml")[..], &pb[..]),
                (&format!("{second}/src/main/java/E.java")[..], ctl),
                ("web-b/src/main/java/B.java", bad),
            ];
            let flow = extract(project(&files).path());
            let of = |app: &str| {
                (flow.endpoints.iter())
                    .find(|e| e.application.as_deref() == Some(app))
                    .map(|e| e.access.clone())
            };
            assert_eq!(of("web-a"), Some(protected(role("ROLE_ADMIN"))), "{first}");
            assert!(
                matches!(of("web-b"), Some(AccessState::Protected(c)) if format!("{c:?}").contains("Unknown")),
                "{first}: {:?}",
                of("web-b")
            );
        }
    }

    // F32: the pre-6 `requestMatchers().antMatchers(..)` scope of an adapter.
    #[test]
    fn f32_legacy_request_matchers_scope_an_adapter() {
        let config = adapter(
            "http.requestMatchers().antMatchers(\"/api/**\").and().authorizeRequests().anyRequest().authenticated();",
        );
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", &config),
            ("src/main/java/E.java", SCOPED_ENDPOINTS),
        ]);
        assert_eq!(access(&flow, "GET", "/web"), &AccessState::Undeclared);
        assert_eq!(
            access(&flow, "GET", "/api/x"),
            &protected(Condition::Authenticated)
        );
        let antm = adapter(
            "http.antMatcher(\"/api/**\").authorizeRequests().anyRequest().authenticated();",
        );
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", &antm),
            ("src/main/java/E.java", SCOPED_ENDPOINTS),
        ]);
        assert_eq!(access(&flow, "GET", "/web"), &AccessState::Undeclared);
        assert_eq!(
            access(&flow, "GET", "/api/x"),
            &protected(Condition::Authenticated)
        );
    }

    // A scope that matches only some requests of an endpoint decides nothing.
    #[test]
    fn a_partial_scope_match_keeps_the_declared_access() {
        let chain = "@Configuration\nclass Sec {\n    @Bean SecurityFilterChain api(HttpSecurity http) throws Exception {\n        http.securityMatcher(\"/api/{id:\\\\d+}\").authorizeHttpRequests(a -> a.anyRequest().permitAll());\n        return http.build();\n    }\n}\n";
        let flow = spring_flow(&[
            ("src/main/java/Sec.java", chain),
            (
                "src/main/java/E.java",
                "@RestController class E { @GetMapping(\"/api/me\") void a() {} }\n",
            ),
        ]);
        assert_eq!(access(&flow, "GET", "/api/me"), &AccessState::Undeclared);
    }

    // Legacy `@EnableGlobalMethodSecurity(jsr250Enabled)` adds the role prefix only when missing.
    #[test]
    fn legacy_jsr250_adds_the_role_prefix_only_when_missing() {
        let flow = spring_flow(&[
            (
                "src/main/java/Enable.java",
                "@EnableGlobalMethodSecurity(jsr250Enabled = true)\nclass Enable {}\n",
            ),
            (
                "src/main/java/A.java",
                &controller(
                    r#"class A {
    @RolesAllowed("ROLE_ADMIN") @GetMapping("/a") void a() {}
    @RolesAllowed("ADMIN") @GetMapping("/b") void b() {}
}"#,
                ),
            ),
        ]);
        assert_eq!(access(&flow, "GET", "/a"), &protected(role("ROLE_ADMIN")));
        assert_eq!(access(&flow, "GET", "/b"), &protected(role("ROLE_ADMIN")));
    }

    // ---- Wave 2b: ASP.NET defaults, inheritance, custom attributes ----

    fn tfm(version: &str) -> String {
        format!(
            "<Project><PropertyGroup><TargetFramework>{version}</TargetFramework></PropertyGroup></Project>"
        )
    }

    fn web(csproj: &str, files: &[(&str, &str)]) -> UiFlow {
        let named: Vec<(String, &str)> = std::iter::once(("Web/Web.csproj".to_string(), csproj))
            .chain(files.iter().map(|(p, t)| (format!("Web/{p}"), *t)))
            .collect();
        let refs: Vec<(&str, &str)> = named.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        extract(project(&refs).path())
    }

    fn startup(body: &str) -> String {
        format!("builder.Services.AddAuthorization(o => {{ {body} }});\n")
    }

    const CTRL: &str = r#"
[ApiController]
[Route("api/c")]
public class CController : ControllerBase
{
    [HttpGet("bare")][Authorize] public IActionResult Bare() => Ok();
    [HttpGet("roles")][Authorize(Roles = "X")] public IActionResult Roles() => Ok();
    [HttpGet("pol")][Authorize("P")] public IActionResult Pol() => Ok();
    [HttpGet("anon")][AllowAnonymous] public IActionResult Anon() => Ok();
    [HttpGet("none")] public IActionResult None() => Ok();
}
"#;

    fn with_program(program: &str) -> UiFlow {
        web(&tfm("net10.0"), &[("Program.cs", program), ("C.cs", CTRL)])
    }

    const AUTH_BUILDER: &str =
        "new AuthorizationPolicyBuilder().RequireAuthenticatedUser().Build()";

    // A.6 #1, #8
    #[test]
    fn w2b_no_defaults_bare_authorize_is_authenticated_and_unmarked_is_undeclared() {
        let flow = with_program("");
        assert_eq!(
            access(&flow, "GET", "/api/c/bare"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(
            access(&flow, "GET", "/api/c/none"),
            &AccessState::Undeclared
        );
    }

    // A.6 #2
    #[test]
    fn w2b_replaced_default_policy_applies_to_bare_authorize_only() {
        let flow = with_program(&startup(
            r#"o.DefaultPolicy = new AuthorizationPolicyBuilder().RequireAuthenticatedUser().RequireRole("Staff").Build();"#,
        ));
        let all = Condition::All(vec![Condition::Authenticated, role("Staff")]);
        assert_eq!(access(&flow, "GET", "/api/c/bare"), &protected(all));
        assert_eq!(access(&flow, "GET", "/api/c/roles"), &protected(role("X")));
        assert_eq!(
            access(&flow, "GET", "/api/c/none"),
            &AccessState::Undeclared
        );
    }

    // A.6 #3
    #[test]
    fn w2b_unreadable_default_policy_is_unknown_with_a_gap() {
        let flow = with_program(&startup("o.DefaultPolicy = Build(config);"));
        assert_eq!(
            access(&flow, "GET", "/api/c/bare"),
            &protected(Condition::Unknown("DefaultPolicy".into()))
        );
        assert_eq!(access(&flow, "GET", "/api/c/roles"), &protected(role("X")));
        assert!(gaps(&flow, "DefaultPolicy") >= 1, "{:#?}", flow.diagnostics);
    }

    // A.6 #4
    #[test]
    fn w2b_fallback_policy_covers_unmarked_actions_only() {
        let flow = with_program(&startup(&format!("o.FallbackPolicy = {AUTH_BUILDER};")));
        assert_eq!(
            access(&flow, "GET", "/api/c/none"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(access(&flow, "GET", "/api/c/anon"), &AccessState::Open);
        assert_eq!(access(&flow, "GET", "/api/c/pol"), &protected(policy("P")));
    }

    // A.6 #5
    #[test]
    fn w2b_set_fallback_policy_with_a_local_variable() {
        let flow = with_program(&format!(
            "var requireAuthPolicy = {AUTH_BUILDER};\nbuilder.Services.AddAuthorizationBuilder().SetFallbackPolicy(requireAuthPolicy);\n"
        ));
        assert_eq!(
            access(&flow, "GET", "/api/c/none"),
            &protected(Condition::Authenticated)
        );
    }

    // A.6 #6
    #[test]
    fn w2b_fallback_equal_to_the_default_policy() {
        let flow = with_program(&startup("o.FallbackPolicy = o.DefaultPolicy;"));
        assert_eq!(
            access(&flow, "GET", "/api/c/none"),
            &protected(Condition::Authenticated)
        );
        let flow = with_program(&startup(
            "o.DefaultPolicy = new AuthorizationPolicyBuilder().RequireRole(\"R\").Build(); o.FallbackPolicy = o.DefaultPolicy;",
        ));
        assert_eq!(access(&flow, "GET", "/api/c/none"), &protected(role("R")));
    }

    // A.6 #7
    #[test]
    fn w2b_conditional_fallback_is_unknown_with_a_gap() {
        let flow = with_program(&startup(&format!(
            "if (env.IsProduction()) o.FallbackPolicy = {AUTH_BUILDER};"
        )));
        assert_eq!(
            access(&flow, "GET", "/api/c/none"),
            &protected(Condition::Unknown("FallbackPolicy".into()))
        );
        assert_eq!(access(&flow, "GET", "/api/c/anon"), &AccessState::Open);
        assert!(
            gaps(&flow, "FallbackPolicy") >= 1,
            "{:#?}",
            flow.diagnostics
        );
        // Two assignments are as unreadable as a conditional one.
        let flow = with_program(&startup(&format!(
            "o.FallbackPolicy = {AUTH_BUILDER}; o.FallbackPolicy = {AUTH_BUILDER};"
        )));
        assert_eq!(
            access(&flow, "GET", "/api/c/none"),
            &protected(Condition::Unknown("FallbackPolicy".into()))
        );
    }

    // C.6 #16: the fallback reaches minimal endpoints too.
    #[test]
    fn w2b_fallback_reaches_minimal_endpoints() {
        let flow = with_program(&format!(
            "{}app.MapGet(\"/m\", () => 1);\napp.MapGet(\"/a\", () => 1).AllowAnonymous();\napp.MapGet(\"/p\", () => 1).RequireAuthorization(\"P\");\napp.MapGet(\"/b\", () => 1).RequireAuthorization();\n",
            startup(&format!(
                "o.FallbackPolicy = {AUTH_BUILDER}; o.DefaultPolicy = new AuthorizationPolicyBuilder().RequireRole(\"R\").Build();"
            ))
        ));
        assert_eq!(
            access(&flow, "GET", "/m"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(access(&flow, "GET", "/a"), &AccessState::Open);
        assert_eq!(access(&flow, "GET", "/p"), &protected(policy("P")));
        // A bare RequireAuthorization() is the (replaced) default policy.
        assert_eq!(access(&flow, "GET", "/b"), &protected(role("R")));
    }

    // A.6 #16: names containing Authorize derive nothing.
    #[test]
    fn w2b_unknown_attributes_named_authorize_derive_nothing() {
        let flow = web(
            &tfm("net10.0"),
            &[(
                "C.cs",
                r#"
[Route("n")]
public class NController : ControllerBase
{
    [HttpGet("a")][AuthorizeForScopes("x")] public IActionResult A() => Ok();
    [HttpGet("b")][AuthorizationHeader] public IActionResult B() => Ok();
    [HttpGet("c")][MyAuthorize] public IActionResult C() => Ok();
}
"#,
            )],
        );
        for p in ["/n/a", "/n/b", "/n/c"] {
            assert_eq!(access(&flow, "GET", p), &AccessState::Undeclared, "{p}");
        }
    }

    const MY_AUTH: &str = r#"
public class MyAuthorizeAttribute : AuthorizeAttribute
{
    public MyAuthorizeAttribute(string p) : base(p) {}
}
public class ComputedAttribute : AuthorizeAttribute
{
    public ComputedAttribute(string p) { Policy = Prefix + p; }
}
public class RolesAttribute : AuthorizeAttribute
{
    public RolesAttribute() { Roles = "A,B"; }
}
public class SubAttribute : MyAuthorizeAttribute
{
    public SubAttribute() : base("sub") {}
}
[Route("m")]
public class MController : ControllerBase
{
    [HttpGet("a")][MyAuthorize("x")] public IActionResult A() => Ok();
    [HttpGet("b")][Computed("x")] public IActionResult B() => Ok();
    [HttpGet("c")][Roles] public IActionResult C() => Ok();
    [HttpGet("d")][Sub] public IActionResult D() => Ok();
    [HttpGet("e")][Authorize("P")][MyAuthorize("x")] public IActionResult E() => Ok();
}
"#;

    // A.6 #17
    #[test]
    fn w2b_authorize_subclasses_read_constructor_arguments() {
        let flow = web(&tfm("net10.0"), &[("C.cs", MY_AUTH)]);
        assert_eq!(access(&flow, "GET", "/m/a"), &protected(policy("x")));
        assert_eq!(
            access(&flow, "GET", "/m/b"),
            &protected(Condition::Unknown("Computed(\"x\")".into()))
        );
        assert!(gaps(&flow, "Computed") >= 1, "{:#?}", flow.diagnostics);
        assert_eq!(
            access(&flow, "GET", "/m/c"),
            &protected(Condition::Any(vec![role("A"), role("B")]))
        );
        assert_eq!(access(&flow, "GET", "/m/d"), &protected(policy("sub")));
        assert_eq!(
            access(&flow, "GET", "/m/e"),
            &protected(Condition::All(vec![policy("P"), policy("x")]))
        );
    }

    const PAGES: &[(&str, &str)] = &[
        ("Pages/Index.cshtml", "@page\n@model IndexModel\n"),
        (
            "Pages/Index.cshtml.cs",
            "public class IndexModel : PageModel { public void OnGet() {} }",
        ),
        ("Pages/Contact.cshtml", "@page\n@model ContactModel\n"),
        (
            "Pages/Contact.cshtml.cs",
            "[Authorize] public class ContactModel : PageModel { public void OnGet() {} public void OnPostSend() {} }",
        ),
        ("Pages/Anon.cshtml", "@page\n@model AnonModel\n"),
        (
            "Pages/Anon.cshtml.cs",
            "[AllowAnonymous] public class AnonModel : PageModel { public void OnGet() {} }",
        ),
        ("Pages/Private/Index.cshtml", "@page\n@model PIndexModel\n"),
        (
            "Pages/Private/Index.cshtml.cs",
            "public class PIndexModel : PageModel { public void OnGet() {} }",
        ),
        ("Pages/Private/Public.cshtml", "@page\n@model PublicModel\n"),
        (
            "Pages/Private/Public.cshtml.cs",
            "public class PublicModel : PageModel { public void OnGet() {} }",
        ),
        (
            "Pages/Admin/Panel.cshtml",
            "@page \"{id:int}\"\n@model PanelModel\n",
        ),
        (
            "Pages/Admin/Panel.cshtml.cs",
            "public class PanelModel : PageModel { public void OnGet(int id) {} }",
        ),
        ("Pages/Handler.cshtml", "@page\n@model HandlerModel\n"),
        (
            "Pages/Handler.cshtml.cs",
            "public class HandlerModel : PageModel { [Authorize] public void OnGet() {} }",
        ),
        ("Pages/NotAPage.cshtml", "<p>a view, no @page directive</p>"),
    ];
    const PAGE_CONVENTIONS: &str = r#"
builder.Services.AddRazorPages(options =>
{
    options.Conventions.AuthorizeFolder("/Private").AllowAnonymousToPage("/Private/Public");
    options.Conventions.AuthorizeFolder("/Admin", "AdminOnly");
});
"#;

    // dotnet.md B-04/B-05; docs:
    // https://learn.microsoft.com/en-us/aspnet/core/razor-pages/security/authorization/conventions
    // https://learn.microsoft.com/en-us/aspnet/core/razor-pages/security/authorization/simple
    #[test]
    fn w5e_razor_pages_are_endpoints_with_attributes_and_conventions() {
        let mut files = vec![("Program.cs", PAGE_CONVENTIONS)];
        files.extend_from_slice(PAGES);
        let flow = web(&tfm("net10.0"), &files);
        assert_eq!(access(&flow, "GET", "/"), &AccessState::Undeclared);
        assert_eq!(
            access(&flow, "GET", "/Contact"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(
            access(&flow, "POST", "/Contact"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(access(&flow, "GET", "/Anon"), &AccessState::Open);
        assert_eq!(
            access(&flow, "GET", "/Private"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(access(&flow, "GET", "/Private/Public"), &AccessState::Open);
        assert_eq!(
            access(&flow, "GET", "/Admin/Panel/{id:int}"),
            &protected(policy("AdminOnly"))
        );
        // `[Authorize]` on a handler is ignored by the framework: a gap, never protected.
        assert_eq!(access(&flow, "GET", "/Handler"), &AccessState::Undeclared);
        assert!(gaps(&flow, "ignored") >= 1, "{:#?}", flow.diagnostics);
        assert!(!flow.endpoints.iter().any(|e| e.path.contains("NotAPage")));
    }

    #[test]
    fn w5e_razor_page_convention_with_unreadable_path_is_unknown_with_a_gap() {
        let program = r#"
builder.Services.AddRazorPages(o => { o.Conventions.AuthorizeFolder(Folders.Private); });
"#;
        let mut files = vec![("Program.cs", program)];
        files.extend_from_slice(PAGES);
        let flow = web(&tfm("net10.0"), &files);
        let state = access(&flow, "GET", "/");
        assert!(
            matches!(state, AccessState::Protected(Condition::Unknown(_))),
            "{state:?}"
        );
        assert!(
            gaps(&flow, "Folders.Private") >= 1,
            "{:#?}",
            flow.diagnostics
        );
        // An explicit marker still wins.
        assert_eq!(access(&flow, "GET", "/Anon"), &AccessState::Open);
    }

    // Review: Razor comments `@* … *@` hide directives; they stay hidden.
    #[test]
    fn w5e_razor_comments_hide_directives() {
        let files = [
            ("Program.cs", PAGE_CONVENTIONS),
            (
                "Pages/Admin/Secret.cshtml",
                "@page\n@*\n@attribute [AllowAnonymous]\n*@\n",
            ),
            ("Pages/Admin/Gone.cshtml", "@* @page *@\n"),
        ];
        let flow = web(&tfm("net10.0"), &files);
        assert_eq!(
            access(&flow, "GET", "/Admin/Secret"),
            &protected(policy("AdminOnly"))
        );
        assert!(!flow.endpoints.iter().any(|e| e.path.contains("Gone")));
    }

    // Review: OrdinalIgnoreCase is not ASCII-only.
    #[test]
    fn w5e_razor_folder_conventions_match_non_ascii_case_insensitively() {
        let program = "builder.Services.AddRazorPages(o => { o.Conventions.AuthorizeFolder(\"/übersicht\"); });";
        let flow = web(
            &tfm("net10.0"),
            &[
                ("Program.cs", program),
                ("Pages/Übersicht/Index.cshtml", "@page\n"),
            ],
        );
        assert_eq!(
            access(&flow, "GET", "/Übersicht"),
            &protected(Condition::Authenticated)
        );
    }

    // Review: `_ViewImports.cshtml` supplies `@model`/`@inherits`, the closest wins
    // (https://learn.microsoft.com/en-us/aspnet/core/mvc/views/layout#importing-shared-directives).
    #[test]
    fn w5e_razor_view_imports_supply_model_and_inherits() {
        let cs = "[AllowAnonymous] public class RootModel : PageModel {}\n\
                  [Authorize] public class ImpModel : PageModel {}\n\
                  [Authorize(\"B\")] public class BaseP : PageModel {}\n";
        let files = [
            ("Pages/_ViewImports.cshtml", "@model RootModel\n"),
            ("Pages/Other.cshtml", "@page\n"),
            ("Pages/Imp/_ViewImports.cshtml", "@model ImpModel\n"),
            ("Pages/Imp/Page.cshtml", "@page\n"),
            ("Pages/Inh/_ViewImports.cshtml", "@inherits BaseP\n"),
            ("Pages/Inh/Page.cshtml", "@page\n@model RootModel\n"),
            ("Pages/Miss/_ViewImports.cshtml", "@model Nowhere\n"),
            ("Pages/Miss/Page.cshtml", "@page\n"),
            ("M.cs", cs),
        ];
        let flow = web(&tfm("net10.0"), &files);
        assert_eq!(access(&flow, "GET", "/Other"), &AccessState::Open);
        assert_eq!(
            access(&flow, "GET", "/Imp/Page"),
            &protected(Condition::Authenticated)
        );
        // The page's own @model is RootModel (open); @inherits adds BaseP; open wins.
        assert_eq!(access(&flow, "GET", "/Inh/Page"), &AccessState::Open);
        assert!(gaps(&flow, "Nowhere") >= 1, "{:#?}", flow.diagnostics);
    }

    // Review: `@attribute` in `_ViewImports` is not confirmed by the docs: a gap,
    // and the access is never upgraded by it.
    #[test]
    fn w5e_razor_view_imports_attribute_is_a_gap() {
        let flow = web(
            &tfm("net10.0"),
            &[
                ("Program.cs", "var x = 1;"),
                ("Pages/_ViewImports.cshtml", "@attribute [AllowAnonymous]\n"),
                ("Pages/P.cshtml", "@page\n"),
            ],
        );
        assert_eq!(access(&flow, "GET", "/P"), &AccessState::Undeclared);
        assert!(
            gaps(&flow, "@attribute in _ViewImports") >= 1,
            "{:#?}",
            flow.diagnostics
        );
    }

    // W6c: Razor Pages are screens too, with the access of their endpoint; links
    // and forms only where the target is literal.
    #[test]
    fn w6c_razor_pages_emit_screens_links_and_forms() {
        let index = "@page\n<a asp-page=\"/Contact\">c</a>\n<a href=\"/Anon\">a</a>\n<a href=\"@Url.Foo()\">x</a>\n<form method=\"post\" asp-page=\"/Contact\"></form>\n";
        let mut files = vec![
            ("Program.cs", PAGE_CONVENTIONS),
            ("Pages/Index.cshtml", index),
        ];
        files.extend(
            PAGES
                .iter()
                .filter(|(p, _)| *p != "Pages/Index.cshtml")
                .copied(),
        );
        let flow = web(&tfm("net10.0"), &files);
        let screen = |route: &str| {
            (flow.screens.iter())
                .find(|s| s.route.as_deref() == Some(route))
                .unwrap_or_else(|| panic!("no screen {route}: {:#?}", flow.screens))
        };
        assert_eq!(
            screen("/Contact").access,
            protected(Condition::Authenticated)
        );
        assert_eq!(screen("/Anon").access, AccessState::Open);
        assert_eq!(screen("/").id, "Web:/");
        let to = |t: &str| archguard_core::Target::Screen(t.into());
        let targets: Vec<_> = flow.transitions.iter().map(|t| t.target.clone()).collect();
        assert!(targets.contains(&to("Web:/Contact")), "{targets:?}");
        assert!(targets.contains(&to("Web:/Anon")), "{targets:?}");
        assert!(
            flow.actions.iter().any(|a| matches!(&a.call,
            archguard_core::Call::Http { method, path } if method == "POST" && path == "/Contact"))
        );
        assert!(gaps(&flow, "@Url.Foo()") >= 1, "{:#?}", flow.diagnostics);
        assert!(!flow.screens.iter().any(|s| s.name.contains("NotAPage")));
    }

    // W6c review: a method the scan cannot take apart (attribute lost to a line
    // break, tuple return) and delegates that are no plain name are Unknown, never
    // attribute-free; a same-class overload is ambiguous.
    #[test]
    fn w6c_review_unreadable_method_groups_are_unknown() {
        let program = r#"
app.MapGet("/nl", Api.GetAsync);
app.MapGet("/tuple", Api.Pair);
app.MapGet("/gen", Api.Plain<int>);
app.MapGet("/cast", (Func<IResult>)Api.Plain2);
app.MapGet("/fac", Api.Make());
"#;
        let api = r#"
public static class Api
{
    [Authorize] public static async Task<Results<Ok, NotFound>>
        GetAsync(int id) => null;
    [Authorize] public static (int, string)
        Pair() => (1, "a");
    public static IResult Plain<T>() => null;
    public static IResult Plain2() => null;
    public static Delegate Make() => null;
}
public static class Ov
{
    public static void Map(IEndpointRouteBuilder app) { app.MapGet("/ov", Pick); }
    static IResult Pick() => null;
    [Authorize] static IResult Pick(int x) => null;
}
"#;
        let flow = web(&tfm("net10.0"), &[("Program.cs", program), ("Api.cs", api)]);
        // `/nl` and `/tuple` have `[Authorize]`: never Undeclared or Open, whether the
        // declaration scan reads the attribute (Protected) or not (Unknown).
        for path in ["/nl", "/tuple"] {
            let state = access(&flow, "GET", path);
            assert!(
                matches!(state, AccessState::Protected(_)),
                "{path}: {state:?}"
            );
        }
        for path in ["/gen", "/cast", "/fac", "/ov"] {
            let state = access(&flow, "GET", path);
            assert!(
                matches!(state, AccessState::Protected(Condition::Unknown(_))),
                "{path}: {state:?}"
            );
        }
    }

    // W6c review: braces in markup or strings cannot be ruled out, other code
    // blocks count, and a local href without a page is a gap.
    #[test]
    fn w6c_review_razor_blocks_braces_and_hrefs() {
        let p = "@page\n@if (a) {\n<p>}</p>\n}\n<a asp-page=\"/Q\">q</a>\n";
        let r = "@page\n@do {\n<a asp-page=\"/Q\">q</a>\n} while (b);\n";
        let o = "@page \"{id}\"\n<a href=\"/Orders/5\">five</a>\n";
        let flow = web(
            &tfm("net10.0"),
            &[
                ("Program.cs", "var x = 1;"),
                ("Pages/P.cshtml", p),
                ("Pages/R.cshtml", r),
                ("Pages/Q.cshtml", "@page\n"),
                ("Pages/Orders.cshtml", o),
            ],
        );
        for file in ["Pages/P.cshtml", "Pages/R.cshtml"] {
            let t = flow
                .transitions
                .iter()
                .find(|t| t.file == format!("Web/{file}"));
            let t = t.unwrap_or_else(|| panic!("{file}: {:#?}", flow.transitions));
            assert!(
                matches!(&t.condition, AccessState::Protected(Condition::Unknown(_))),
                "{file}: {:?}",
                t.condition
            );
        }
        assert!(gaps(&flow, "/Orders/5") >= 1, "{:#?}", flow.diagnostics);
    }

    // W6c: a link under a Razor condition is never unconditional; layout links are a gap.
    #[test]
    fn w6c_razor_links_under_conditions_and_in_layouts_are_gaps() {
        let p = "@page\n@if (User.IsInRole(\"a\")) {\n<a asp-page=\"/Q\">q</a>\n}\n";
        let flow = web(
            &tfm("net10.0"),
            &[
                ("Program.cs", "var x = 1;"),
                ("Pages/P.cshtml", p),
                ("Pages/Q.cshtml", "@page\n"),
                ("Pages/Shared/_Layout.cshtml", "<a href=\"/Q\">q</a>"),
            ],
        );
        let t = &flow.transitions;
        assert_eq!(t.len(), 1, "{t:#?}");
        assert!(
            matches!(&t[0].condition, AccessState::Protected(Condition::Unknown(h)) if h.contains("@if")),
            "{:?}",
            t[0].condition
        );
        assert!(gaps(&flow, "Razor block") >= 1, "{:#?}", flow.diagnostics);
        assert!(
            gaps(&flow, "layout or partial") >= 1,
            "{:#?}",
            flow.diagnostics
        );
    }

    // W6c: a method group is resolved in the scanned sources and its attributes apply.
    #[test]
    fn w6c_minimal_method_groups_apply_their_attributes() {
        let program = r#"
app.MapGet("/sec", Handlers.Secure);
app.MapGet("/anon", Handlers.Anon);
app.MapGet("/plain", Handlers.Plain);
app.MapGet("/gone", Handlers.Missing);
app.MapGet("/twice", Twice);
app.MapGet("/typed", async Task<Results<Ok<int>, NotFound>> () => 1);
"#;
        let handlers = r#"
public static class Handlers
{
    [Authorize(Roles = "Admin")] public static IResult Secure() => null;
    [AllowAnonymous] public static IResult Anon() => null;
    public static IResult Plain() => null;
}
public static class Reg
{
    public static void Map(IEndpointRouteBuilder app) { app.MapGet("/own", Own); }
    [Authorize] static IResult Own() => null;
}
public static class Elsewhere { static IResult Own() => null; }
public static class A { public static IResult Twice() => null; }
public static class B { [Authorize] public static IResult Twice() => null; }
"#;
        let flow = web(
            &tfm("net10.0"),
            &[("Program.cs", program), ("Handlers.cs", handlers)],
        );
        assert_eq!(
            access(&flow, "GET", "/sec"),
            &protected(Condition::Role("Admin".into()))
        );
        assert_eq!(access(&flow, "GET", "/anon"), &AccessState::Open);
        assert_eq!(access(&flow, "GET", "/plain"), &AccessState::Undeclared);
        // An unqualified name binds in the class around the call, not in other classes.
        assert_eq!(
            access(&flow, "GET", "/own"),
            &protected(Condition::Authenticated)
        );
        // A lambda cut at a generic comma is no method group.
        assert_eq!(access(&flow, "GET", "/typed"), &AccessState::Undeclared);
        for path in ["/gone", "/twice"] {
            let state = access(&flow, "GET", path);
            assert!(
                matches!(state, AccessState::Protected(Condition::Unknown(_))),
                "{path}: {state:?}"
            );
        }
        assert!(
            gaps(&flow, "Handlers.Missing") >= 1,
            "{:#?}",
            flow.diagnostics
        );
    }

    // Review: blanking a comment with multi-byte text keeps byte offsets.
    #[test]
    fn w5e_razor_comment_with_multibyte_text_keeps_lines() {
        let flow = web(
            &tfm("net10.0"),
            &[
                ("Program.cs", "var x = 1;"),
                ("Pages/P.cshtml", "@* für Admins *@\n@page\n"),
            ],
        );
        let e = flow
            .endpoints
            .iter()
            .find(|e| e.path == "/P")
            .expect("endpoint");
        assert_eq!(e.line, 2);
    }

    // Guard: `MapControllers().AllowAnonymous()` opens controllers, not pages;
    // `MapRazorPages().RequireAuthorization()` protects pages (B-03).
    #[test]
    fn w5e_page_endpoints_use_the_map_razor_pages_conventions_only() {
        let a = "app.MapControllers().AllowAnonymous();\napp.MapRazorPages();\n";
        let b = "app.MapRazorPages().RequireAuthorization();\n";
        for (program, want) in [
            (a, AccessState::Undeclared),
            (b, protected(Condition::Authenticated)),
        ] {
            let mut files = vec![("Program.cs", program)];
            files.extend_from_slice(PAGES);
            let flow = web(&tfm("net10.0"), &files);
            assert_eq!(access(&flow, "GET", "/"), &want, "{program}");
        }
    }

    // dotnet.md C-04; docs:
    // https://learn.microsoft.com/en-us/aspnet/core/fundamentals/minimal-apis/security
    // ("[Authorize]" / "[AllowAnonymous]" on the route handler lambda).
    #[test]
    fn w5e_minimal_api_lambda_attributes_are_read() {
        let program = r#"
app.MapGet("/la", [Authorize] () => 1);
app.MapGet("/lb", [AllowAnonymous] () => 1);
app.MapGet("/lc", [Authorize(Roles = "Admin")] async () => 1);
app.MapGet("/ld", [Authorize("P")] [AllowAnonymous] static () => 1);
app.MapPost("/le", [Authorize("P")] ([FromBody] string s) => 1);
app.MapGet("/lf", () => 1);
"#;
        let flow = with_program(program);
        assert_eq!(
            access(&flow, "GET", "/la"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(access(&flow, "GET", "/lb"), &AccessState::Open);
        assert_eq!(access(&flow, "GET", "/lc"), &protected(role("Admin")));
        assert_eq!(access(&flow, "GET", "/ld"), &AccessState::Open);
        assert_eq!(access(&flow, "POST", "/le"), &protected(policy("P")));
        assert_eq!(access(&flow, "GET", "/lf"), &AccessState::Undeclared);
    }

    // dotnet.md A-07: `IAuthorizeData` with neither Policy nor Roles set means the
    // default policy (AuthorizationPolicy.CombineAsync); docs:
    // https://learn.microsoft.com/en-us/aspnet/core/security/authorization/iauthorizedata
    #[test]
    fn w5e_iauthorizedata_without_policy_or_roles_is_the_default_policy() {
        let src = r#"
public class PlainDataAttribute : Attribute, IAuthorizeData
{
    public string? Policy { get; set; }
    public string? Roles { get; set; }
    public string? AuthenticationSchemes { get; set; }
}
[Route("d")]
public class DController : ControllerBase
{
    [HttpGet("a")][PlainData] public IActionResult A() => Ok();
}
"#;
        let flow = web(&tfm("net10.0"), &[("C.cs", src)]);
        assert_eq!(
            access(&flow, "GET", "/d/a"),
            &protected(Condition::Authenticated)
        );
    }

    const MIN_AGE: &str = r#"
public class MinAgeAttribute(int a) : AuthorizeAttribute, IAuthorizationRequirement, IAuthorizationRequirementData
{
}
[Route("g")]
public class GController : ControllerBase
{
    [HttpGet("a")][MinAge(21)] public IActionResult A() => Ok();
}
"#;

    // A.6 #18
    #[test]
    fn w2b_requirement_data_attribute_on_minimal_and_mvc() {
        let all = Condition::All(vec![
            Condition::Authenticated,
            Condition::Custom("MinAge(21)".into()),
        ]);
        let program =
            "app.MapGet(\"/m\", () => 1).RequireAuthorization(new MinAgeAttribute(21));\n";
        let flow = web(
            &tfm("net10.0"),
            &[("C.cs", MIN_AGE), ("Program.cs", program)],
        );
        assert_eq!(access(&flow, "GET", "/m"), &protected(all.clone()));
        // MVC before .NET 11: only the AuthorizeAttribute part, with a gap.
        assert_eq!(
            access(&flow, "GET", "/g/a"),
            &protected(Condition::Authenticated)
        );
        assert!(
            gaps(&flow, "before .NET 11") >= 1,
            "{:#?}",
            flow.diagnostics
        );
        let flow = web(&tfm("net11.0"), &[("C.cs", MIN_AGE)]);
        assert_eq!(access(&flow, "GET", "/g/a"), &protected(all));
    }

    // A.6 #19
    #[test]
    fn w2b_generic_authorize_attribute_is_a_project_type() {
        let generic = "public class AuthorizeAttribute<TRequirement> : AuthorizeAttribute, IAuthorizationRequirementData {}\n";
        let ctrl = r#"
[Route("t")]
public class TController : ControllerBase
{
    [HttpGet("a")][Authorize<MemberRequirement>] public IActionResult A() => Ok();
}
"#;
        let decl = format!("{generic}{ctrl}");
        let all = Condition::All(vec![
            Condition::Authenticated,
            Condition::Custom("Authorize<MemberRequirement>".into()),
        ]);
        let flow = web(&tfm("net11.0"), &[("C.cs", &decl)]);
        assert_eq!(access(&flow, "GET", "/t/a"), &protected(all.clone()));
        let program = "app.MapGroup(\"/q\").RequireAuthorization(new AuthorizeAttribute<MemberRequirement>()).MapGet(\"/x\", () => 1);\n";
        let flow = web(&tfm("net10.0"), &[("C.cs", &decl), ("Program.cs", program)]);
        assert_eq!(access(&flow, "GET", "/q/x"), &protected(all));
        // Without the generic declaration in the scan nothing is derived.
        let flow = web(&tfm("net11.0"), &[("C.cs", ctrl)]);
        assert_eq!(access(&flow, "GET", "/t/a"), &AccessState::Undeclared);
    }

    // B.6 #1
    #[test]
    fn w2b_authorize_on_an_abstract_base_applies_to_derived_actions() {
        let flow = web(
            &tfm("net10.0"),
            &[(
                "C.cs",
                r#"
[Authorize(Roles = "Admin")]
public abstract class BaseCtl : ControllerBase {}
[ApiController]
[Route("api/[controller]")]
public class OrdersController : BaseCtl
{
    [HttpGet] public IActionResult A() => Ok();
    [HttpGet("c")][AllowAnonymous] public IActionResult C() => Ok();
}
"#,
            )],
        );
        assert_eq!(
            access(&flow, "GET", "/api/Orders"),
            &protected(role("Admin"))
        );
        assert_eq!(access(&flow, "GET", "/api/Orders/c"), &AccessState::Open);
        assert_eq!(flow.endpoints.len(), 2, "{:#?}", flow.endpoints);
    }

    // B.6 #2
    #[test]
    fn w2b_controller_base_makes_a_controller_without_the_suffix() {
        let flow = web(
            &tfm("net10.0"),
            &[(
                "C.cs",
                "[Route(\"plain\")]\npublic class Plain : ControllerBase { [HttpGet] public IActionResult G() => Ok(); }\n",
            )],
        );
        assert_eq!(access(&flow, "GET", "/plain"), &AccessState::Undeclared);
    }

    // B.6 #3
    #[test]
    fn w2b_actions_of_a_generic_abstract_base_belong_to_the_derived_controller() {
        let flow = web(
            &tfm("net10.0"),
            &[(
                "C.cs",
                r#"
[Route("api/base")]
public abstract class CrudController<T> : ControllerBase
{
    [HttpGet] public IActionResult List() => Ok();
}
[Authorize]
[Route("api/users")]
public class UsersController : CrudController<User> {}
"#,
            )],
        );
        assert_eq!(
            access(&flow, "GET", "/api/users"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(flow.endpoints.len(), 1, "{:#?}", flow.endpoints);
    }

    // B.6 #4
    #[test]
    fn w2b_global_authorize_filter_applies_to_mvc_actions_only() {
        let program = format!(
            "builder.Services.AddControllers(c => c.Filters.Add(new AuthorizeFilter({AUTH_BUILDER})));\napp.MapGet(\"/m\", () => 1);\n"
        );
        let flow = with_program(&program);
        assert_eq!(
            access(&flow, "GET", "/api/c/none"),
            &protected(Condition::Authenticated)
        );
        assert_eq!(
            access(&flow, "GET", "/api/c/roles"),
            &protected(Condition::All(vec![Condition::Authenticated, role("X")]))
        );
        assert_eq!(access(&flow, "GET", "/api/c/anon"), &AccessState::Open);
        assert_eq!(access(&flow, "GET", "/m"), &AccessState::Undeclared);
        // A filter whose policy is unreadable protects with Unknown.
        let flow = with_program(
            "builder.Services.AddControllers(c => c.Filters.Add(new AuthorizeFilter(Make())));\n",
        );
        assert_eq!(
            access(&flow, "GET", "/api/c/none"),
            &protected(Condition::Unknown("Make()".into()))
        );
        assert!(gaps(&flow, "Make()") >= 1, "{:#?}", flow.diagnostics);
    }

    // B.6 #5
    #[test]
    fn w2b_map_controllers_convention_reaches_actions() {
        let flow = with_program("app.MapControllers().RequireAuthorization(\"Api\");\n");
        assert_eq!(
            access(&flow, "GET", "/api/c/none"),
            &protected(policy("Api"))
        );
        assert_eq!(
            access(&flow, "GET", "/api/c/roles"),
            &protected(Condition::All(vec![role("X"), policy("Api")]))
        );
        assert_eq!(access(&flow, "GET", "/api/c/anon"), &AccessState::Open);
        // The convention is not a minimal endpoint.
        assert_eq!(flow.endpoints.len(), 5, "{:#?}", flow.endpoints);
        let flow = with_program("app.MapControllers().AllowAnonymous();\n");
        assert_eq!(access(&flow, "GET", "/api/c/roles"), &AccessState::Open);
        // Two MapControllers calls that differ are unreadable, never open.
        let flow = with_program(
            "app.MapControllers().RequireAuthorization(\"A\");\napp.MapControllers();\n",
        );
        assert!(matches!(
            access(&flow, "GET", "/api/c/none"),
            AccessState::Protected(Condition::Unknown(_))
        ));
    }

    // A.6 #17 pitfall: an AuthorizeAttribute that is also a filter decides in code.
    #[test]
    fn w2b_authorize_attribute_that_is_also_a_filter_adds_unknown() {
        let flow = web(
            &tfm("net10.0"),
            &[(
                "C.cs",
                r#"
public class ApiPermissionAttribute : AuthorizeAttribute, IAsyncActionFilter
{
    public ApiPermissionAttribute(string id) { }
}
[Route("p")]
public class PController : ControllerBase
{
    [HttpGet("a")][ApiPermission("x")] public IActionResult A() => Ok();
}
"#,
            )],
        );
        assert_eq!(
            access(&flow, "GET", "/p/a"),
            &protected(Condition::All(vec![
                Condition::Authenticated,
                Condition::Unknown("ApiPermission(\"x\")".into())
            ]))
        );
        assert!(gaps(&flow, "ApiPermission") >= 1, "{:#?}", flow.diagnostics);
    }

    // Classes that are no controller never produce base-class notes.
    #[test]
    fn w2b_non_controllers_emit_no_ambiguity_notes() {
        let flow = web(
            &tfm("net10.0"),
            &[(
                "C.cs",
                "class Twin {}\nnamespace B { class Twin {} }\nclass Leaf : Twin {}\n",
            )],
        );
        assert_eq!(notes(&flow, "ambiguous"), 0, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn annotations_survive_a_modifier_only_header_line() {
        for text in [
            "class C {\n  @PreAuthorize(\"x\") final\n  String run() { return \"\"; }\n}\n",
            "class C {\n  @GetMapping(\"/a\") public\n  String a() { return \"\"; }\n}\n",
            "class C {\n  @GetMapping(\"/a\")\n  public\n  String a() { return \"\"; }\n}\n",
        ] {
            let src = Source::new("C.java".into(), text);
            let m: Vec<_> = decls(&src, true).into_iter().filter(|d| !d.class).collect();
            assert_eq!(m.len(), 1, "{text}");
            assert_eq!(m[0].attrs.len(), 1, "{text}");
        }
    }

    #[test]
    fn annotations_of_a_kotlin_property_do_not_reach_the_next_function() {
        for text in [
            "class C {\n  @get:PermitAll\n  val x: String = \"\"\n  @Get(\"/b\") fun b(): String = \"\"\n}\n",
            "class C {\n  @Inject lateinit var s: S\n  @Get(\"/b\") fun b(): String = \"\"\n}\n",
        ] {
            let src = Source::new("C.kt".into(), text);
            let m: Vec<_> = decls(&src, true)
                .into_iter()
                .filter(|d| d.name == "b")
                .collect();
            assert_eq!(m.len(), 1, "{text}");
            assert_eq!(m[0].attrs.len(), 1, "{text}");
        }
    }

    #[test]
    fn annotations_survive_a_return_type_on_its_own_line() {
        let text = "class Ctl {\n  @GetMapping(\"/a\")\n  @PreAuthorize(\"hasRole('B')\")\n  String\n  a() { return \"\"; }\n  @GetMapping(\"/b\")\n  Map<String, List<X>>\n  b() { return null; }\n}\n";
        let src = Source::new("C.java".into(), text);
        let m: Vec<_> = decls(&src, true).into_iter().filter(|d| !d.class).collect();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].attrs.len(), 2);
        assert_eq!(m[1].attrs.len(), 1);
    }
}

#[cfg(test)]
mod handlers_tests {
    use super::extract;
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
