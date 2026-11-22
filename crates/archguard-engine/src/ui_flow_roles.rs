//! Role and permission catalog of the UI flow (ADR 0021): the roles,
//! permissions and policies a project defines, read by fixed rules and never
//! guessed. Every def records the file and line it was read from; what is not
//! a literal or a readable constant becomes a `gap` diagnostic.
//!
//! - Constants: `const string X = "v"` / `nameof(X)` (C#), `static final String`
//!   (Java), `const val` (Kotlin), keyed by their innermost class. A constant is
//!   a role only where it is used (below); no class name makes a catalog.
//! - Conventions are read per application (nearest build-file project), never
//!   for the whole flow: ASP.NET policy names ignore case, `RoleClaimType`
//!   gives `role_claim` (none read = exact); `RoleHierarchyImpl.withDefaultRolePrefix()`
//!   gives Spring's `role_prefix`. Read outside every project it is a gap.
//! - ASP.NET: `AddPolicy(name, …)` (also on `AddAuthorizationBuilder()`) and
//!   `DefaultPolicy`/`FallbackPolicy = new AuthorizationPolicyBuilder()…` give
//!   policy defs: `RequireAuthenticatedUser` = signed in, `RequireRole(a, b)` =
//!   any role, `RequireClaim(t, v…)` = custom `claim t = v|…`, everything else
//!   unknown. `new IdentityRole(x)` seeds a role; constants or enum members
//!   (`nameof(E.M)`, `E.M.ToString()`) named in `RequireRole` or
//!   `[Authorize(Roles = …)]` are roles at their declaration.
//! - Spring: constants named in `@PreAuthorize`, `@PostAuthorize`, `@Secured`,
//!   `@RolesAllowed`, `.hasAuthority`, `.hasAnyAuthority`, `.hasRole`,
//!   `.hasAnyRole` are roles; `RoleHierarchyImpl.fromHierarchy` / `setHierarchy`
//!   (`ROLE_ADMIN > ROLE_USER`) give `includes`. Names are kept as the runtime
//!   authority (`ROLE_ADMIN`); the prefix is the convention's `role_prefix`.
//! - TypeScript: `enum` or `const … = {…}` named `Authority`, `Role`, `Roles`
//!   (roles) or `Permission`, `Permissions` (permissions); a member is a def only
//!   when `Name.MEMBER` is used in a `.ts` or `.html` file. Names are kept as written.
use crate::extract_ctx::{ExtractContext, ancestors, dir_name, read_text, walk, with_root_config};
use crate::services::support::test_path;
use archguard_core::{
    Condition, ConventionOrigin, DiagnosticSeverity, NameCase, PermissionFormat, PolicyDef,
    RoleConvention, RoleDef, RoleKind, UiFlow, UiFlowDiagnostic,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_FILES: usize = 40_000;
const BUILD_FILES: [&str; 3] = ["pom.xml", "build.gradle", "build.gradle.kts"];
const TS_ROLES: [&str; 3] = ["Authority", "Role", "Roles"];
const TS_PERMISSIONS: [&str; 2] = ["Permission", "Permissions"];
const SPRING_SITES: [&str; 8] = [
    "@PreAuthorize(",
    "@PostAuthorize(",
    "@Secured(",
    "@RolesAllowed(",
    ".hasAuthority(",
    ".hasAnyAuthority(",
    ".hasRole(",
    ".hasAnyRole(",
];

/// Role and policy defs below `root`, with notes for what is not readable.
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut out = Out::default();
    let sources = read_sources(ctx.root, &mut out);
    let consts = Consts::read(&sources);
    let mut used = BTreeSet::new();
    for src in &sources {
        used.extend(ts_uses(src));
    }
    for src in &sources {
        match src.lang {
            Lang::CSharp => {
                let before = out.roles.len() + out.policies.len();
                aspnet(src, &consts, &mut out);
                if let (Some(app), true) = (&src.app, out.roles.len() + out.policies.len() > before)
                {
                    out.aspnet_apps.insert(app.clone());
                }
            }
            Lang::Jvm => spring(src, &consts, &mut out),
            Lang::Ts => frontend(src, &used, &mut out),
            Lang::Html => {}
        }
    }
    out.aspnet_defaults();
    let mut flow = UiFlow {
        roles: out.roles,
        policies: out.policies,
        role_conventions: out.conventions.into_values().collect(),
        complete: out.notes.is_empty(),
        diagnostics: out.notes,
        ..UiFlow::default()
    };
    flow.roles.sort();
    flow.roles.dedup();
    let mut convention = RoleConvention::default();
    if flow.roles.iter().any(|d| d.kind == RoleKind::Permission) {
        // Dotted permission ids cover each other (the former built-in rule).
        convention.permissions = PermissionFormat::Dotted {
            wildcard: "*".into(),
        };
        convention.origins.push(ConventionOrigin {
            field: "permissions".into(),
            file: String::new(),
            line: 0,
            source: "built-in rule for dotted permission ids".into(),
        });
    }
    if convention != RoleConvention::default() {
        flow.role_conventions.push(convention);
    }
    flow.policies.sort();
    flow.policies.dedup();
    flow.diagnostics.sort();
    flow.diagnostics.dedup();
    flow
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    CSharp,
    Jvm,
    Ts,
    Html,
}

struct Source {
    rel: String,
    lang: Lang,
    /// Comments blanked (HTML kept as is).
    text: String,
    lines: Vec<usize>,
    /// The build-file project the file lies in; none outside every project.
    app: Option<String>,
}
impl Source {
    fn new(rel: String, lang: Lang, raw: &str, app: Option<String>) -> Self {
        let text = if lang == Lang::Html {
            raw.to_string()
        } else {
            lex::strip_comments(raw)
        };
        let lines = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self {
            rel,
            lang,
            text,
            lines,
            app,
        }
    }
    fn line(&self, at: usize) -> u32 {
        self.lines.partition_point(|&s| s <= at) as u32
    }
    fn source(&self) -> &'static str {
        match self.lang {
            Lang::CSharp => "aspnet",
            Lang::Jvm => "spring",
            _ => "typescript",
        }
    }
}

fn read_sources(root: &Path, out: &mut Out) -> Vec<Source> {
    let walk = match walk(root) {
        Ok(walk) => walk,
        Err(e) => {
            out.gap(".", 0, format!("Discovery settings not read: {e}"));
            return vec![];
        }
    };
    let lang = |name: &str| match name.rsplit_once('.').map(|(_, e)| e) {
        Some("cs") => Some(Lang::CSharp),
        Some("java" | "kt") => Some(Lang::Jvm),
        Some("ts") if !name.ends_with(".d.ts") => Some(Lang::Ts),
        Some("html") => Some(Lang::Html),
        _ => None,
    };
    let projects: BTreeSet<&str> = (walk.files.iter())
        .map(|f| f.rsplit_once('/').unwrap_or(("", f)))
        .filter(|(_, name)| name.ends_with(".csproj") || BUILD_FILES.contains(name))
        .map(|(dir, _)| dir)
        .collect();
    let app_of =
        |rel: &str| (ancestors(rel).find(|d| projects.contains(d))).map(|d| dir_name(root, d));
    let apps: BTreeMap<String, Option<String>> = (walk.files.iter())
        .map(|f| (f.clone(), app_of(f)))
        .collect();
    let mut files: Vec<(String, Lang)> = (walk.files.into_iter())
        .filter(|rel| !test_path(rel))
        .filter_map(|rel| lang(&rel).map(|l| (rel, l)))
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    if files.len() > MAX_FILES {
        out.gap(
            ".",
            0,
            format!("More than {MAX_FILES} source files; the rest were not read for roles."),
        );
        files.truncate(MAX_FILES);
    }
    let mut sources = Vec::new();
    for (rel, lang) in files {
        match read_text(root, &rel, MAX_BYTES) {
            Ok(text) => {
                let app = apps.get(&rel).cloned().flatten();
                sources.push(Source::new(rel, lang, &text, app));
            }
            Err(e) => out.gap(
                &rel,
                0,
                format!("Source file not read for roles (limit 1 MiB): {e}"),
            ),
        }
    }
    sources
}

#[derive(Default)]
struct Out {
    roles: Vec<RoleDef>,
    policies: Vec<PolicyDef>,
    /// Gaps: recognised, but not readable.
    notes: Vec<UiFlowDiagnostic>,
    /// Conventions read per application; never a whole-flow entry.
    conventions: BTreeMap<String, RoleConvention>,
    /// Applications with ASP.NET authorization declarations.
    aspnet_apps: BTreeSet<String>,
}
impl Out {
    fn gap(&mut self, file: &str, line: u32, message: String) {
        self.notes.push(UiFlowDiagnostic {
            file: file.into(),
            line,
            message,
            severity: DiagnosticSeverity::Gap,
        });
    }
    fn role(&mut self, kind: RoleKind, name: &str, decl: &Decl, includes: Vec<String>) {
        self.roles.push(RoleDef {
            application: None,
            name: name.to_string(),
            kind,
            includes,
            file: decl.file.clone(),
            line: decl.line,
            source: decl.source.into(),
        });
    }
    fn policy(&mut self, src: &Source, line: u32, name: String, requires: Condition) {
        self.policies.push(PolicyDef {
            application: None,
            name,
            requires,
            file: src.rel.clone(),
            line,
            source: src.source().into(),
        });
    }
    fn app_convention(&mut self, app: &str) -> &mut RoleConvention {
        (self.conventions.entry(app.to_string())).or_insert_with(|| RoleConvention {
            application: Some(app.to_string()),
            ..RoleConvention::default()
        })
    }
    /// The convention of `src`'s application for something read at `line`;
    /// outside every project there is none, so the reading is a gap.
    fn convention_of(
        &mut self,
        src: &Source,
        line: u32,
        what: &str,
    ) -> Option<&mut RoleConvention> {
        let Some(app) = src.app.as_deref() else {
            let message = format!("{what} not attributed: outside every build-file project");
            self.gap(&src.rel, line, message);
            return None;
        };
        Some(self.app_convention(app))
    }
    /// ASP.NET defaults of every application with declarations: policy names
    /// ignore case, role names do not; a claim type not read stays exact.
    fn aspnet_defaults(&mut self) {
        for app in std::mem::take(&mut self.aspnet_apps) {
            let c = self.app_convention(&app);
            c.policy_case = NameCase::Insensitive;
            let origin = |field: &str, source: &str| ConventionOrigin {
                field: field.into(),
                file: String::new(),
                line: 0,
                source: source.into(),
            };
            let policy = "framework default aspnetcore authorization (policy names ignore case)";
            c.origins.push(origin("policy_case", policy));
            if c.role_claim.is_none() {
                (c.origins).push(origin("role_claim", "not read (exact match)"));
            }
        }
    }
}

/// Where a value was declared.
#[derive(Debug, Clone)]
struct Decl {
    value: String,
    file: String,
    line: u32,
    source: &'static str,
}

impl Decl {
    /// A def read at this line of `src` (the value is the def's name).
    fn at(src: &Source, line: u32) -> Self {
        Decl {
            value: String::new(),
            file: src.rel.clone(),
            line,
            source: src.source(),
        }
    }
}

/// String constants by (class, name) and enum members by (enum, member).
#[derive(Default)]
struct Consts {
    values: BTreeMap<(String, String), Vec<Decl>>,
    members: BTreeMap<(String, String), Decl>,
}
impl Consts {
    fn read(sources: &[Source]) -> Self {
        let mut c = Consts::default();
        for src in sources
            .iter()
            .filter(|s| matches!(s.lang, Lang::CSharp | Lang::Jvm))
        {
            let b = src.text.as_bytes();
            let types = lex::types(&src.text);
            let owner = |at: usize| {
                (types.iter())
                    .filter(|t| t.body.0 < at && at < t.body.1)
                    .max_by_key(|t| t.body.0)
            };
            for needle in [
                "const string ",
                "static final String ",
                "const val ",
                "static readonly string ",
            ] {
                for at in lex::find_words(&src.text, needle) {
                    let (Some(class), Some((name, value))) =
                        (owner(at), lex::assignment(&src.text, at + needle.len()))
                    else {
                        continue;
                    };
                    let value =
                        lex::literal(value).or_else(|| lex::nameof(value).map(str::to_string));
                    if let Some(value) = value {
                        let decl = Decl {
                            value,
                            file: src.rel.clone(),
                            line: src.line(at),
                            source: src.source(),
                        };
                        c.values
                            .entry((class.name.clone(), name.to_string()))
                            .or_default()
                            .push(decl);
                    }
                }
            }
            for t in types.iter().filter(|t| t.is_enum) {
                for (member, at) in lex::enum_members(b, t.body) {
                    let decl = Decl {
                        value: member.clone(),
                        file: src.rel.clone(),
                        line: src.line(at),
                        source: src.source(),
                    };
                    c.members.insert((t.name.clone(), member), decl);
                }
            }
        }
        c
    }

    /// The constant `A.B` (last two segments of a dotted path), when all its
    /// declarations agree.
    fn constant(&self, path: &str) -> Option<&Decl> {
        let mut segs = path.rsplit('.');
        let (name, class) = (segs.next()?, segs.next()?);
        let decls = self.values.get(&(class.to_string(), name.to_string()))?;
        decls
            .iter()
            .all(|d| d.value == decls[0].value)
            .then(|| &decls[0])
    }

    fn member(&self, path: &str) -> Option<&Decl> {
        let mut segs = path.rsplit('.');
        let (name, owner) = (segs.next()?, segs.next()?);
        self.members.get(&(owner.to_string(), name.to_string()))
    }

    /// The value of a literal, `nameof(…)`, constant or `E.M.ToString()` /
    /// `E.M.name()`, with its declaration when it is a constant or enum member.
    fn resolve(&self, expr: &str) -> Option<(String, Option<Decl>)> {
        let expr = expr.trim();
        if let Some(v) = lex::literal(expr) {
            return Some((v, None));
        }
        if let Some(inner) = lex::call_arg(expr, "nameof") {
            // The declaration of the named identifier, whatever its value.
            let decl = self.member(inner).or_else(|| self.constant(inner)).cloned();
            return Some((lex::nameof(expr)?.to_string(), decl));
        }
        let path = expr
            .strip_suffix(".ToString()")
            .or_else(|| expr.strip_suffix(".name()"));
        if let Some(d) = path
            .filter(|p| lex::is_path(p))
            .and_then(|p| self.member(p))
        {
            return Some((d.value.clone(), Some(d.clone())));
        }
        let d = lex::is_path(expr).then(|| self.constant(expr)).flatten()?;
        Some((d.value.clone(), Some(d.clone())))
    }
}

/// Role names of one role-list expression (`"A,B"`, `Roles.A + "," + Roles.B`,
/// `nameof(E.M)`), each with its declaration; `Err` with the unreadable part.
fn role_names(consts: &Consts, expr: &str) -> Result<Vec<(String, Option<Decl>)>, String> {
    let mut names = Vec::new();
    for part in lex::split_top(expr, b'+') {
        let (value, decl) = consts
            .resolve(part)
            .ok_or_else(|| part.trim().to_string())?;
        for name in value.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            names.push((name.to_string(), decl.clone()));
        }
    }
    Ok(names)
}

fn aspnet(src: &Source, consts: &Consts, out: &mut Out) {
    let spans = add_policies(src, consts, out);
    builder_policies(src, consts, out, &spans);
    let text = src.text.as_str();
    // each RequireRole argument is one role name, not a comma list
    let required = lex::calls(text, "RequireRole")
        .into_iter()
        .flat_map(|(_, a)| lex::split_top(a, b','))
        .filter_map(|a| consts.resolve(a))
        .filter_map(|(name, decl)| Some((name, decl?)));
    let authorize = lex::calls(text, "Authorize")
        .into_iter()
        .filter_map(|(_, args)| {
            lex::split_top(args, b',').into_iter().find_map(|a| {
                let (key, value) = a.split_once('=')?;
                (key.trim() == "Roles").then_some(value)
            })
        });
    let listed = authorize
        .flat_map(|list| role_names(consts, list).unwrap_or_default())
        .filter_map(|(name, decl)| Some((name, decl?)));
    let declared: Vec<(String, Decl)> = required.chain(listed).collect();
    for (name, d) in declared {
        out.role(RoleKind::Role, &name, &d, vec![]);
    }
    seeded_roles(src, consts, out);
    role_claim_type(src, consts, out);
}

/// `RoleClaimType = "roles"` (token validation or identity options): the
/// application's role claim.
fn role_claim_type(src: &Source, consts: &Consts, out: &mut Out) {
    for at in lex::find_words(&src.text, "RoleClaimType") {
        let rest = src.text[at + "RoleClaimType".len()..].trim_start();
        let Some(value) = rest.strip_prefix('=').filter(|v| !v.starts_with('=')) else {
            continue;
        };
        let line = src.line(at);
        let raw = lex::value(value);
        let Some((claim, _)) = consts.resolve(raw) else {
            out.gap(
                &src.rel,
                line,
                format!("Role claim type not readable: {raw}"),
            );
            continue;
        };
        let Some(app) = src.app.clone() else {
            let message = "Role claim type not attributed: outside every build-file project";
            out.gap(&src.rel, line, message.into());
            continue;
        };
        out.aspnet_apps.insert(app.clone());
        let conv = out.app_convention(&app);
        if conv.role_claim.as_ref().is_some_and(|c| *c != claim) {
            let message = format!("Role claim type read twice with different values: {claim}");
            out.gap(&src.rel, line, message);
            continue;
        }
        conv.role_claim = Some(claim);
        conv.origins.push(ConventionOrigin {
            field: "role_claim".into(),
            file: src.rel.clone(),
            line,
            source: "RoleClaimType".into(),
        });
    }
}

/// `AddPolicy(name, …)` defs; returns the spans of their arguments.
fn add_policies(src: &Source, consts: &Consts, out: &mut Out) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for (at, args) in lex::calls(&src.text, "AddPolicy") {
        spans.push((at, at + args.len()));
        let parts: Vec<&str> = (lex::split_top(args, b',').into_iter())
            .map(lex::unname)
            .collect();
        let line = src.line(at);
        let Some((name, _)) = parts.first().and_then(|p| consts.resolve(p)) else {
            let raw = parts.first().copied().unwrap_or("").trim();
            out.gap(&src.rel, line, format!("Policy name not readable: {raw}"));
            continue;
        };
        let rest = parts.get(1..).map(|p| p.join(",")).unwrap_or_default();
        let requires = requirements(src, line, &rest, consts, out);
        out.policy(src, line, name, requires);
    }
    spans
}

/// `DefaultPolicy`/`FallbackPolicy = new AuthorizationPolicyBuilder()…` outside `AddPolicy`.
fn builder_policies(src: &Source, consts: &Consts, out: &mut Out, spans: &[(usize, usize)]) {
    let text = src.text.as_str();
    for at in lex::find_words(text, "new AuthorizationPolicyBuilder") {
        let before = text[..at].trim_end().trim_end_matches('=').trim_end();
        let name = ["DefaultPolicy", "FallbackPolicy"]
            .into_iter()
            .find(|n| before.ends_with(n));
        let inside = spans.iter().any(|(s, e)| *s <= at && at <= *e);
        let Some(name) = name.filter(|_| !inside) else {
            continue;
        };
        let end = text[at..].find(';').map_or(text.len(), |e| at + e);
        let line = src.line(at);
        let requires = requirements(src, line, &text[at..end], consts, out);
        out.policy(src, line, name.into(), requires);
    }
}

/// `new IdentityRole(x)`: a role at the seeding site.
fn seeded_roles(src: &Source, consts: &Consts, out: &mut Out) {
    for (at, args) in lex::calls(&src.text, "new IdentityRole") {
        let arg = lex::split_top(args, b',').into_iter().next().unwrap_or("");
        let here = Decl::at(src, src.line(at));
        match consts.resolve(arg) {
            Some((name, _)) => out.role(RoleKind::Role, &name, &here, vec![]),
            None => out.gap(
                &src.rel,
                here.line,
                format!("Seeded role name not readable: {}", arg.trim()),
            ),
        }
    }
}

/// The policy's requirements in `text`, all of them required.
fn requirements(src: &Source, line: u32, text: &str, consts: &Consts, out: &mut Out) -> Condition {
    let value = |e: &str| (consts.resolve(e)).map_or_else(|| e.trim().to_string(), |(v, _)| v);
    let mut all = Vec::new();
    for (name, args) in lex::require_calls(text) {
        let parts = lex::split_top(args, b',');
        all.push(match name {
            "RequireAuthenticatedUser" => Condition::Authenticated,
            "RequireRole" => required_roles(src, line, &parts, consts, out),
            "RequireClaim" => {
                let mut values = parts.iter().map(|p| value(p));
                let kind = values.next().unwrap_or_default();
                let values: Vec<String> = values.collect();
                match values.is_empty() {
                    true => Condition::Custom(format!("claim {kind}")),
                    false => Condition::Custom(format!("claim {kind} = {}", values.join("|"))),
                }
            }
            other => {
                let message = format!("Policy requirement not readable: {other}");
                out.gap(&src.rel, line, message);
                Condition::Unknown(other.into())
            }
        });
    }
    // `Requirements.Add(new X(1))`: a custom requirement, named as written.
    for (_, arg) in lex::calls(text, "Requirements.Add") {
        let arg = arg.trim();
        all.push(Condition::Custom(
            arg.strip_prefix("new ").unwrap_or(arg).trim().to_string(),
        ));
    }
    if all.is_empty() {
        out.gap(&src.rel, line, "Policy has no readable requirement".into());
        return Condition::Unknown("no requirement read".into());
    }
    one_or(all, Condition::All)
}

/// `RequireRole(a, b)`: any of the roles, each argument ONE role (a comma is
/// part of the name); an unreadable one is unknown.
fn required_roles(
    src: &Source,
    line: u32,
    parts: &[&str],
    consts: &Consts,
    out: &mut Out,
) -> Condition {
    let mut any = Vec::new();
    for part in parts {
        match consts.resolve(part) {
            Some((name, _)) => any.push(Condition::Role(name)),
            None => {
                let raw = part.trim().to_string();
                out.gap(&src.rel, line, format!("Policy role not readable: {raw}"));
                any.push(Condition::Unknown(raw));
            }
        }
    }
    one_or(any, Condition::Any)
}

fn one_or(mut items: Vec<Condition>, many: fn(Vec<Condition>) -> Condition) -> Condition {
    if items.len() == 1 {
        items.remove(0)
    } else {
        many(items)
    }
}

fn spring(src: &Source, consts: &Consts, out: &mut Out) {
    let calls = SPRING_SITES
        .iter()
        .flat_map(|site| lex::calls(&src.text, site.trim_end_matches('(')));
    let paths: Vec<&str> = calls.flat_map(|(_, args)| lex::paths(args)).collect();
    for d in paths.into_iter().filter_map(|p| consts.constant(p)) {
        out.role(RoleKind::Role, &d.value, d, vec![]);
    }
    for call in ["fromHierarchy", "setHierarchy"] {
        for (at, args) in lex::calls(&src.text, call) {
            hierarchy(src, src.line(at), args, consts, out);
        }
    }
    for (name, at, args) in builder_starts(&src.text) {
        hierarchy_builder(src, (name, at, args), consts, out);
    }
}

/// `RoleHierarchyImpl.withDefaultRolePrefix()` / `.withRolePrefix("X")` sites.
fn builder_starts(text: &str) -> Vec<(&'static str, usize, &str)> {
    let mut found = Vec::new();
    for name in ["withDefaultRolePrefix", "withRolePrefix"] {
        let needle = format!("RoleHierarchyImpl.{name}");
        for (at, args) in lex::calls(text, &needle) {
            found.push((name, at, args));
        }
    }
    found
}

/// `….role("ADMIN").implies("STAFF")…`: the builder's roles carry its prefix
/// (the runtime authority, as `fromHierarchy` names them); the prefix is the
/// application's `role_prefix`.
fn hierarchy_builder(
    src: &Source,
    (kind, at, args): (&str, usize, &str),
    consts: &Consts,
    out: &mut Out,
) {
    let line = src.line(at);
    let prefix = match (kind, consts.resolve(args)) {
        ("withDefaultRolePrefix", _) => Some("ROLE_".to_string()),
        (_, Some((p, _))) => Some(p),
        _ => None,
    };
    let Some(prefix) = prefix else {
        let message = format!("Role hierarchy prefix not readable: {}", args.trim());
        out.gap(&src.rel, line, message);
        return;
    };
    if let Some(conv) = out.convention_of(src, line, "Role prefix") {
        if conv.role_prefix.as_ref().is_some_and(|p| *p != prefix) {
            let message = format!("Role prefix read twice with different values: {prefix}");
            out.gap(&src.rel, line, message);
            return;
        }
        conv.role_prefix = Some(prefix.clone());
        conv.origins.push(ConventionOrigin {
            field: "role_prefix".into(),
            file: src.rel.clone(),
            line,
            source: format!("RoleHierarchyImpl.{kind}"),
        });
    }
    let text = &src.text[at..];
    let end = text.find(';').unwrap_or(text.len());
    let chain = &text[..end];
    let mut steps: Vec<(usize, &str, &str)> = ["role", "implies"]
        .into_iter()
        .flat_map(|n| {
            lex::calls(chain, n)
                .into_iter()
                .map(move |(i, a)| (i, n, a))
        })
        .collect();
    steps.sort_by_key(|s| s.0);
    let here = Decl::at(src, line);
    let (mut current, mut includes): (Option<String>, Vec<String>) = (None, vec![]);
    let flush = |role: Option<String>, includes: &mut Vec<String>, out: &mut Out| {
        if let (Some(role), false) = (role, includes.is_empty()) {
            out.role(RoleKind::Role, &role, &here, std::mem::take(includes));
        }
    };
    for (_, name, args) in steps {
        let names: Option<Vec<String>> = (lex::split_top(args, b',').into_iter())
            .map(|a| consts.resolve(a).map(|(v, _)| format!("{prefix}{v}")))
            .collect();
        let Some(names) = names else {
            let message = format!("Role hierarchy not readable: {}", args.trim());
            out.gap(&src.rel, line, message);
            continue;
        };
        if name == "role" {
            flush(current.take(), &mut includes, out);
            current = names.into_iter().next();
        } else {
            includes.extend(names);
        }
    }
    flush(current, &mut includes, out);
}

/// `ROLE_ADMIN > ROLE_STAFF\nROLE_STAFF > ROLE_USER`: each `a > b` makes `a` include `b`.
fn hierarchy(src: &Source, line: u32, args: &str, consts: &Consts, out: &mut Out) {
    let pieces: Option<Vec<String>> = (lex::split_top(args, b'+').into_iter())
        .map(|p| consts.resolve(p).map(|(v, _)| v))
        .collect();
    let Some(pieces) = pieces else {
        let message = format!("Role hierarchy not readable: {}", args.trim());
        out.gap(&src.rel, line, message);
        return;
    };
    let here = Decl::at(src, line);
    for rule in pieces.concat().replace("\\n", "\n").lines() {
        let chain: Vec<&str> = rule.split('>').map(str::trim).collect();
        let pairs = chain
            .windows(2)
            .filter(|p| !p[0].is_empty() && !p[1].is_empty());
        for pair in pairs {
            out.role(RoleKind::Role, pair[0], &here, vec![pair[1].to_string()]);
        }
    }
}

/// `(Name, MEMBER)` for every `Name.MEMBER` of a listed frontend name.
fn ts_uses(src: &Source) -> Vec<(String, String)> {
    if !matches!(src.lang, Lang::Ts | Lang::Html) {
        return vec![];
    }
    let mut uses = Vec::new();
    for name in TS_ROLES.iter().chain(&TS_PERMISSIONS) {
        for at in lex::find_words(&src.text, &format!("{name}.")) {
            let rest = &src.text[at + name.len() + 1..];
            let member: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !member.is_empty() {
                uses.push((name.to_string(), member));
            }
        }
    }
    uses
}

fn frontend(src: &Source, used: &BTreeSet<(String, String)>, out: &mut Out) {
    for name in TS_ROLES.iter().chain(&TS_PERMISSIONS) {
        let kind = match TS_ROLES.contains(name) {
            true => RoleKind::Role,
            false => RoleKind::Permission,
        };
        for (key, value, at) in ts_members(src, name) {
            if !used.contains(&(name.to_string(), key.to_string())) {
                continue;
            }
            let line = src.line(at);
            match lex::literal(value) {
                Some(v) => out.role(kind, &v, &Decl::at(src, line), vec![]),
                None => out.gap(
                    &src.rel,
                    line,
                    format!("{name}.{key} is not a string literal: {}", value.trim()),
                ),
            }
        }
    }
}

/// Members (key, value text, offset) of `enum NAME {…}` and `const NAME = {…}`.
fn ts_members<'a>(src: &'a Source, name: &str) -> Vec<(&'a str, &'a str, usize)> {
    let text = src.text.as_str();
    let mut found = Vec::new();
    for (keyword, sep) in [("enum ", '='), ("const ", ':')] {
        for at in lex::find_words(text, &format!("{keyword}{name}")) {
            let head = at + keyword.len() + name.len();
            let Some(open) = text[head..].find('{').map(|o| head + o) else {
                continue;
            };
            let between = text[head..open].trim().trim_start_matches('=').trim();
            let Some(close) = lex::close(text.as_bytes(), open).filter(|_| between.is_empty())
            else {
                continue;
            };
            found.extend(ts_items(text, open, close, sep));
        }
    }
    found
}

/// `key = value` / `key: value` items between `open` and `close`, with offsets.
fn ts_items(text: &str, open: usize, close: usize, sep: char) -> Vec<(&str, &str, usize)> {
    let items = lex::split_top(&text[open + 1..close], b',');
    (items.into_iter())
        .filter_map(|item| {
            let at = item.as_ptr() as usize - text.as_ptr() as usize
                + (item.len() - item.trim_start().len());
            let (key, value) = item.split_once(sep)?;
            Some((
                key.trim().trim_matches(|c| c == '\'' || c == '"'),
                value,
                at,
            ))
        })
        .collect()
}

/// Comment-, string- and bracket-aware scanning shared by the rules.
mod lex {
    pub(super) struct Type {
        pub name: String,
        pub is_enum: bool,
        /// Byte offsets of `{` and its `}`.
        pub body: (usize, usize),
    }

    fn is_ident(c: u8) -> bool {
        c.is_ascii_alphanumeric() || c == b'_'
    }

    /// End (exclusive) of the string or char literal starting at `i`.
    fn quote_end(b: &[u8], i: usize) -> Option<usize> {
        let q = b[i];
        if !matches!(q, b'"' | b'\'' | b'`') {
            return None;
        }
        if b[i..].starts_with(b"\"\"\"") {
            return Some(find(b, i + 3, b"\"\"\"").map_or(b.len(), |e| e + 3));
        }
        let verbatim = q == b'"' && i > 0 && b[i - 1] == b'@';
        let mut j = i + 1;
        while j < b.len() {
            match step(b, j, q, verbatim) {
                Ok(next) => j = next,
                Err(end) => return Some(end),
            }
        }
        Some(b.len())
    }

    /// One step inside a literal quoted by `q`: the next position, or `Err` with its end.
    fn step(b: &[u8], j: usize, q: u8, verbatim: bool) -> Result<usize, usize> {
        match b[j] {
            b'\\' if !verbatim => Ok(j + 2),
            c if c == q && verbatim && b.get(j + 1) == Some(&q) => Ok(j + 2),
            c if c == q => Err(j + 1),
            // shortcut: an unterminated literal ends at the line end (apostrophes in odd places).
            b'\n' if q != b'`' && !verbatim => Err(j),
            _ => Ok(j + 1),
        }
    }

    fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
        (b.get(from..)?
            .windows(needle.len())
            .position(|w| w == needle))
        .map(|p| p + from)
    }

    pub(super) fn strip_comments(text: &str) -> String {
        let mut b = text.as_bytes().to_vec();
        let mut i = 0;
        while i < b.len() {
            if let Some(end) = quote_end(&b, i) {
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

    /// The bracket closing the one at `open`.
    pub(super) fn close(b: &[u8], open: usize) -> Option<usize> {
        let mut depth = 0;
        let mut j = open;
        while j < b.len() {
            if let Some(end) = quote_end(b, j) {
                j = end;
                continue;
            }
            match b[j] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                _ => {}
            }
            j += 1;
        }
        None
    }

    /// `s` split at top-level `sep`, outside strings and brackets.
    pub(super) fn split_top(s: &str, sep: u8) -> Vec<&str> {
        let b = s.as_bytes();
        let (mut parts, mut start, mut depth, mut j) = (vec![], 0, 0, 0);
        while j < b.len() {
            if let Some(end) = quote_end(b, j) {
                j = end;
                continue;
            }
            match b[j] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                c if c == sep && depth == 0 => {
                    parts.push(&s[start..j]);
                    start = j + 1;
                }
                _ => {}
            }
            j += 1;
        }
        parts.push(&s[start..]);
        parts.retain(|p| !p.trim().is_empty());
        parts
    }

    /// Offsets of `needle` not preceded by an identifier character.
    pub(super) fn find_words(text: &str, needle: &str) -> Vec<usize> {
        let b = text.as_bytes();
        let starts_ident = needle.bytes().next().is_some_and(is_ident);
        (text.match_indices(needle).map(|(i, _)| i))
            .filter(|&i| !starts_ident || i == 0 || !is_ident(b[i - 1]))
            .collect()
    }

    /// `name(args)` calls: offset of the name and the text between the parentheses.
    pub(super) fn calls<'a>(text: &'a str, name: &str) -> Vec<(usize, &'a str)> {
        let b = text.as_bytes();
        let mut found = Vec::new();
        for at in find_words(text, name) {
            let mut open = at + name.len();
            if text[open..].starts_with('<') {
                open += text[open..].find('>').map_or(0, |e| e + 1); // generic arguments
            }
            while open < b.len() && b[open].is_ascii_whitespace() {
                open += 1;
            }
            let next = b.get(at + name.len()).copied().unwrap_or(b' ');
            if b.get(open) != Some(&b'(') || is_ident(next) {
                continue;
            }
            if let Some(close) = close(b, open) {
                found.push((at, &text[open + 1..close]));
            }
        }
        found
    }

    /// `Require…(…)` and `AddRequirements(…)` calls in a policy body.
    pub(super) fn require_calls(text: &str) -> Vec<(&str, &str)> {
        let b = text.as_bytes();
        let mut found = Vec::new();
        for needle in ["Require", "AddRequirements"] {
            for at in find_words(text, needle) {
                let end = at + b[at..].iter().take_while(|c| is_ident(**c)).count();
                let name = &text[at..end];
                if let Some((_, args)) = calls(&text[at..], name).into_iter().find(|(i, _)| *i == 0)
                {
                    found.push((at, name, args));
                }
            }
        }
        found.sort_by_key(|f| f.0);
        found.into_iter().map(|(_, n, a)| (n, a)).collect()
    }

    /// Drops a C# named-argument prefix (`configurePolicy: x` → `x`).
    pub(super) fn unname(arg: &str) -> &str {
        let t = arg.trim();
        match t.split_once(':') {
            Some((name, rest))
                if !name.is_empty() && name.bytes().all(is_ident) && !rest.starts_with(':') =>
            {
                rest
            }
            _ => t,
        }
    }

    /// The content of a plain string literal (`"x"`, `'x'`, `@"x"`).
    pub(super) fn literal(s: &str) -> Option<String> {
        let s = s.trim();
        let s = s.strip_prefix('@').unwrap_or(s);
        let b = s.as_bytes();
        let quoted = b.len() >= 2 && matches!(b[0], b'"' | b'\'') && !s.starts_with("\"\"\"");
        (quoted && quote_end(b, 0) == Some(b.len())).then(|| s[1..s.len() - 1].to_string())
    }

    /// The argument text of `name(arg)` when `s` is exactly that call.
    pub(super) fn call_arg<'a>(s: &'a str, name: &str) -> Option<&'a str> {
        s.trim()
            .strip_prefix(name)?
            .trim_start()
            .strip_prefix('(')?
            .strip_suffix(')')
            .map(str::trim)
    }

    /// `nameof(A.B)` → `B`.
    pub(super) fn nameof(s: &str) -> Option<&str> {
        let inner = call_arg(s, "nameof")?;
        is_path(inner).then(|| inner.rsplit('.').next().unwrap_or(inner))
    }

    /// The value expression at the start of `s`: up to a top-level `,` `;` `}`
    /// or line end, outside strings and brackets.
    pub(super) fn value(s: &str) -> &str {
        let b = s.as_bytes();
        let (mut j, mut depth) = (0, 0);
        while j < b.len() {
            if let Some(end) = quote_end(b, j) {
                j = end;
                continue;
            }
            match b[j] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b',' | b';' | b'}' | b'\n' if depth <= 0 => break,
                _ => {}
            }
            j += 1;
        }
        s[..j].trim()
    }

    /// Dotted identifiers (`A`, `A.B.C`).
    pub(super) fn is_path(s: &str) -> bool {
        !s.is_empty()
            && s.split('.').all(|p| {
                !p.is_empty() && p.bytes().all(is_ident) && !p.as_bytes()[0].is_ascii_digit()
            })
    }

    /// Dotted paths with at least one dot outside strings, in order.
    pub(super) fn paths(s: &str) -> Vec<&str> {
        let b = s.as_bytes();
        let (mut found, mut j) = (vec![], 0);
        while j < b.len() {
            if let Some(end) = quote_end(b, j) {
                j = end;
                continue;
            }
            if is_ident(b[j]) && (j == 0 || b[j - 1] != b'.') {
                let end = j + b[j..]
                    .iter()
                    .take_while(|c| is_ident(**c) || **c == b'.')
                    .count();
                let path = s[j..end].trim_end_matches('.');
                if path.contains('.') && is_path(path) {
                    found.push(path);
                }
                j = end;
                continue;
            }
            j += 1;
        }
        found
    }

    /// `NAME [: Type] = VALUE` after a declaration keyword at `from`, up to `;`
    /// or the line end.
    pub(super) fn assignment(text: &str, from: usize) -> Option<(&str, &str)> {
        let rest = &text[from..];
        let name_len = rest.bytes().take_while(|c| is_ident(*c)).count();
        let name = &rest[..name_len];
        let after = &rest[name_len..];
        let eq = after.find('=')?;
        let typed = after[..eq].trim().trim_start_matches(':').trim();
        if name.is_empty() || !(typed.is_empty() || is_path(typed)) {
            return None;
        }
        let value = &after[eq + 1..];
        let end = value.find([';', '\n']).unwrap_or(value.len());
        Some((name, value[..end].trim()))
    }

    /// Classes, interfaces, objects and enums with their bodies.
    pub(super) fn types(text: &str) -> Vec<Type> {
        let b = text.as_bytes();
        let mut found = Vec::new();
        for keyword in ["class ", "interface ", "object ", "enum "] {
            for at in find_words(text, keyword) {
                let rest = &text[at + keyword.len()..];
                let name: String = rest
                    .trim_start()
                    .bytes()
                    .take_while(|c| is_ident(*c))
                    .map(char::from)
                    .collect();
                let head = at + keyword.len();
                let Some(open) = text[head..].find(['{', ';']).map(|o| head + o) else {
                    continue;
                };
                if name.is_empty() || b[open] != b'{' {
                    continue;
                }
                if let Some(close) = close(b, open) {
                    found.push(Type {
                        name,
                        is_enum: keyword == "enum ",
                        body: (open, close),
                    });
                }
            }
        }
        found
    }

    /// Member names of an enum body with their offsets (Java bodies stop at `;`).
    pub(super) fn enum_members(b: &[u8], body: (usize, usize)) -> Vec<(String, usize)> {
        let text = std::str::from_utf8(&b[body.0 + 1..body.1]).unwrap_or("");
        let text = &text[..text.find(';').unwrap_or(text.len())];
        let mut offset = body.0 + 1;
        let mut found = Vec::new();
        for item in text.split(',') {
            let trimmed = item.trim_start();
            let at = offset + (item.len() - trimmed.len());
            offset += item.len() + 1;
            let name: String = trimmed
                .bytes()
                .take_while(|c| is_ident(*c))
                .map(char::from)
                .collect();
            if !name.is_empty() {
                found.push((name, at));
            }
        }
        found
    }
}
