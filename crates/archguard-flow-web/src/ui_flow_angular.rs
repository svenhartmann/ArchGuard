//! Angular extractor for ui-flow/v1 (ADR 0021, phase 2). A text-level scanner
//! without a TypeScript parser; compiler-backed resolution of what it cannot read
//! is ADR 0021 phase 7.
//!
//! Applications: every project of an `angular.json` `projects` map whose
//! `projectType` is not `library` (directory `root`, named by its key), every Nx
//! `project.json` with `projectType: application` (named by its `name`; its
//! `projectType` also overrides an `angular.json` entry for the same directory),
//! and, where no project is declared at or below it, a directory with an
//! `angular.json` without projects or a `package.json` naming `@angular/router`
//! (named after the directory). A source file belongs to the deepest project
//! directory holding it. Files of library projects and files outside every
//! project are shared: each application sees them, but only routes reached from its
//! own files, and only their components that its routes name, count for it.
//! Skipped: `node_modules`, `dist`, `.angular`, the engine's other excluded
//! directories, test files (`*.spec.ts`), Storybook files (`*.stories.ts`) and `*.d.ts`.
//!
//! Recognized:
//! - Route arrays typed `name: Routes = [`, and literal arrays or variables passed to
//!   `RouterModule.forRoot/forChild` or `provideRouter` (variable declared in the same
//!   file or imported by a relative path).
//! - Per route object: `path` (a literal, or a template literal or name made of string
//!   constants, see below), `component`, `loadComponent`
//!   (`import('…').then(m => m.X)` or a default-exported class), `children` (inline or a
//!   variable), `loadChildren` with a literal relative import (prefixed by the parent
//!   path) or `() => SomeModule` (its class in the file itself, its relative import, or
//!   else the only file of the application declaring it; the routes that file hands to the router, else
//!   those of the routing modules in its `imports` that call `RouterModule.forChild`),
//!   `redirectTo` (no screen), `path: '**'` (route `/**`).
//! - Layouts are no screens: a route with a component and `children` whose own path is
//!   `''` or that has a child with path `''`, and a route with a named `outlet`. Their
//!   component acts on every screen of the application (`any_screen`), with the layout's
//!   route as base for relative links.
//! - Access: `canActivate`, `canMatch` and (with `loadChildren`) `canLoad` guards, a
//!   plain name or a factory call (`permissionGuard('x')`, named by the called function),
//!   or `mapToCanActivate([...])`; a guard list that is a variable or call is `unknown`. A guard listed in the
//!   configuration's `[views.access]` takes its meaning there (`open` adds no
//!   condition; `role:$arg` / `policy:$arg` take the factory's string arguments);
//!   any other guard is `custom(name)`. A route `data` key counts only when a guard of the
//!   route (or a `canActivateChild` guard of an ancestor) reads it in its declaration
//!   (`route.data.key`, `route.data['key']`; `data` used otherwise: every key): a literal
//!   array adds `role(…)` (several: `any`), another value is `unknown`; an enum member
//!   (`Authority.ADMIN`) stands for its string value when the application declares that
//!   enum, else it is `unknown`. The `data` of a componentless parent is merged into its
//!   children; keys no guard reads give a note. Routes of one path get ids `…#2`. Several conditions are `all`; child routes inherit
//!   them (plus `canActivateChild`).
//! - In the component's template (`templateUrl` or inline `template`):
//!   `routerLink="…"` and `[routerLink]="'…'"` / `"['…', expr]"`; in its class:
//!   `.navigate([...])` (relative only with `relativeTo`) and `.navigateByUrl('…')`, where
//!   a name may be a string constant and the commands may come from a one-level helper
//!   function whose body is a single `return` of literals and parameters.
//!   Non-literal array parts after the first become a `:param` segment; targets are
//!   matched to the application's routes, a literal segment also to a parameter
//!   (`/orders/42` -> `orders/:id`); an exact literal route wins, and a tie of
//!   equally good routes stays unresolved. Empty commands (`[]`) stay on the current
//!   screen; from a component shown on every screen they add no transition. The
//!   trigger is the element's `data-testid`, `id` or text.
//! - String constants: `const NAME = '…'` (or a template of constants) declared once
//!   in the file, imported by a relative path, or exported once in the application
//!   under an imported name; a global from the project's `angular.json` build `define`;
//!   in route paths and commands also a string member of an enum the application
//!   declares with one value (`AuthRoute.Login`).
//! - HTTP calls: `http.get/post/put/patch/delete/head/options(url…)`, and calls of a
//!   forwarder method (`send("GET", "/ciphers", …)`): a method of the application
//!   whose first parameter is typed as a union of HTTP method literals, which has a
//!   second parameter and whose body names both and calls `fetch(` or `http.`. The URL
//!   is read from string and template literals (holes become `:param`), `+`
//!   concatenations starting with a readable part, `readonly` fields initialised in
//!   the class or a base class (`this.resourceUrl`), `const` locals of the same method
//!   and the string constants above. Anything else is `Call::Unresolved` with the
//!   source text (a local constant's initializer instead of its name).
//! - Calls in a component, or in a class the component names (directly, or by a class
//!   of the application it extends or implements), count as actions on the
//!   component's screens. Of several classes with one name only the one declared in the
//!   component's file or its relative import counts; without such an import all of
//!   them count, with a note.
//! - A component that is no route target (header, navigation) acts on every screen:
//!   its transitions and actions start at `any_screen` of the application, unless
//!   route or layout components embed its element selector in their templates; then
//!   they start at those screens (or at `any_screen` with a layout's route as base),
//!   one per embedding; a selector several components share counts only in hosts
//!   whose file names the class (standalone `imports`).
//!
//! Not recognized; each case is a diagnostic and makes the flow incomplete:
//! non-literal paths, targets and URLs, route spreads and other helper functions,
//! module-path aliases (`@app/…`) for route arrays, guards that are neither names nor
//! factory calls (`unknown`), relative links whose base route is unknown. Not followed:
//! services injected by services, `httpResource`, URL helper methods
//! (`buildUrl('api/x')`), and a missing configuration (no `[views.access]` entries;
//! an unreadable one is a note).
use archguard_core::{
    AccessMeaning, Action, Call, Condition, DiagnosticSeverity, Origin, Screen, ScreenKind, Target,
    Transition, UiFlow, UiFlowDiagnostic, best_route,
};
use archguard_flow_common::extract_ctx::{
    ExtractContext, dir_name as app_name, finish, gap as diag, read_source as read, walk,
    with_root_config,
};
use archguard_flow_common::support::test_path;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

use archguard_flow_common::lang_ts::*;
use archguard_flow_common::lang_ts_decl::*;
use decls::*;
use http::*;

const SOURCE: &str = "angular";
const MAX_FILES: usize = 5_000;
/// Nesting bound for parsed literals and for route arrays that load each other.
const MAX_DEPTH: usize = 32;
/// Bound for names and fields resolved through each other.
const MAX_RESOLVE: usize = 8;
const HTTP_METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "head", "options"];
const ROUTER_CALLS: [&str; 3] = [
    "RouterModule.forRoot(",
    "RouterModule.forChild(",
    "provideRouter(",
];

/// Screens, transitions and HTTP actions of every Angular application below `root`.
/// Read errors and everything not literal become diagnostics.
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run; guards take their meaning from `[views.access]`.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    if let Some(e) = &ctx.config_error {
        flow.diagnostics.push(UiFlowDiagnostic {
            file: String::new(),
            line: 0,
            message: format!(
                "Configuration not read ({e}); [views.access] is not applied, guards stay custom conditions."
            ),
            severity: DiagnosticSeverity::Note,
        });
    }
    match walk(ctx.root) {
        Ok(walk) => {
            let workspace = Workspace::read(ctx.root, walk.files, &mut flow.diagnostics);
            for app in workspace.apps(&ctx.config.views.access) {
                app.extract(&mut flow);
            }
        }
        Err(e) => diag(
            &mut flow.diagnostics,
            "",
            0,
            format!("Discovery exclusions not read; no Angular application analyzed: {e}"),
        ),
    }
    // Every unresolved target, unknown condition or unread file leaves a gap.
    finish(&mut flow);
    flow
}

fn read_json(root: &Path, rel: &str) -> Option<serde_json::Value> {
    serde_json::from_str(&read(root, rel).ok()?).ok()
}

/// A TypeScript source of the application: no declaration, test or Storybook file.
fn is_source(rel: &str) -> bool {
    let skipped = [".d.ts", ".stories.ts"];
    rel.ends_with(".ts") && !skipped.iter().any(|s| rel.ends_with(s)) && !test_path(rel)
}

/// `dir` itself or a directory below it.
fn within(path: &str, dir: &str) -> bool {
    dir.is_empty() || path == dir || path.starts_with(&format!("{dir}/"))
}

/// `angular.json` projects by root directory; `None` without a non-empty `projects` map.
fn angular_projects(v: &serde_json::Value, dir: &str) -> Option<Vec<(String, Project)>> {
    let declared = v.get("projects")?.as_object().filter(|p| !p.is_empty())?;
    let projects = declared.iter().map(|(key, p)| {
        let defines = (p.pointer("/architect/build/options/define"))
            .and_then(|d| d.as_object())
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect();
        let project = Project {
            name: key.clone(),
            application: p["projectType"].as_str() != Some("library"),
            defines,
        };
        (join(dir, p["root"].as_str().unwrap_or("")), project)
    });
    Some(projects.collect())
}

/// An Nx `project.json` with a `projectType`.
fn nx_project(root: &Path, dir: &str, v: &serde_json::Value) -> Option<Project> {
    let kind = v["projectType"].as_str()?;
    Some(Project {
        name: (v["name"].as_str()).map_or_else(|| app_name(root, dir), str::to_string),
        application: kind == "application",
        defines: BTreeMap::new(),
    })
}

/// An Nx type overrides `angular.json` for the same directory; a detected directory
/// counts where no project is declared at or below it.
fn merge_projects(
    root: &Path,
    mut projects: BTreeMap<String, Project>,
    nx: BTreeMap<String, Project>,
    detected: BTreeSet<String>,
) -> BTreeMap<String, Project> {
    for (dir, project) in nx {
        match projects.get_mut(&dir) {
            Some(known) => known.application = project.application,
            None => {
                projects.insert(dir, project);
            }
        }
    }
    let declared: Vec<String> = projects.keys().cloned().collect();
    for dir in detected {
        if !declared.iter().any(|d| within(d, &dir)) {
            let project = Project {
                name: app_name(root, &dir),
                application: true,
                defines: BTreeMap::new(),
            };
            projects.entry(dir).or_insert(project);
        }
    }
    projects
}

/// The condition of guard `name` with factory `args`: its `[views.access]` meaning
/// (`Ok(None)` for `open`), else `custom(name)`; `None` when a `$arg` meaning finds no
/// string literal arguments.
fn guard_condition(
    meaning: Option<&AccessMeaning>,
    name: &str,
    args: Option<&[Item]>,
) -> Option<Option<Condition>> {
    let has_arguments = args.is_some_and(|a| !a.is_empty());
    let make: fn(String) -> Condition = match meaning.map(|m| m.at_use_site(has_arguments)) {
        None => return Some(Some(Condition::Custom(name.into()))),
        Some(AccessMeaning::Open) => return Some(None),
        Some(AccessMeaning::Authenticated) => return Some(Some(Condition::Authenticated)),
        Some(AccessMeaning::Never) => return Some(Some(Condition::Never)),
        Some(AccessMeaning::Role(n)) => return Some(Some(Condition::Role(n.clone()))),
        Some(AccessMeaning::Policy(n)) => return Some(Some(Condition::Policy(n.clone()))),
        Some(AccessMeaning::RoleFromArguments) => Condition::Role,
        Some(AccessMeaning::PolicyFromArguments) => Condition::Policy,
        // `at_use_site` resolved it; a fallback is never a `$arg` form.
        Some(AccessMeaning::FromArgumentsOr(..)) => return None,
    };
    let mut names: Vec<Condition> = (args.into_iter().flatten())
        .map(|a| match &a.value {
            Value::Str(s) => Some(make(s.clone())),
            _ => None,
        })
        .collect::<Option<_>>()?;
    match names.len() {
        0 => None,
        1 => Some(names.pop()),
        _ => Some(Some(Condition::Any(names))),
    }
}

/// The body of the declaration whose name starts at `j`: a `const` initializer, else the
/// first braces (class or function body).
fn declared_body(text: &str, constant: bool, j: usize, name: &str) -> Option<String> {
    if constant {
        return initializer(text, j + name.len()).map(|at| init_source(text, at));
    }
    let open = j + text[j..].find('{')?;
    Some(text[open..close(text, open)].to_string())
}

impl Reads {
    /// Records the use of `data` at `at` in a guard `body`: `.key`, `['key']`, a
    /// destructuring, or anything else (every key).
    fn add(&mut self, body: &str, at: usize) {
        let b = body.as_bytes();
        let mut k = skip_ws(b, at + "data".len());
        if b.get(k) == Some(&b'?') {
            k = skip_ws(b, k + 1);
        }
        let key = match b.get(k) {
            Some(b'.') => Some(ident_at(body, skip_ws(b, k + 1)).to_string()),
            Some(b'[') => match (Parser {
                s: body,
                i: skip_ws(b, k + 1),
            }
            .value(0))
            .value
            {
                Value::Str(s) => Some(s),
                _ => None,
            },
            _ => return self.add_destructured(body, at),
        };
        match key.filter(|k| !k.is_empty()) {
            Some(key) => self.keys.insert(key),
            None => !std::mem::replace(&mut self.all, true),
        };
    }

    fn add_destructured(&mut self, body: &str, at: usize) {
        match destructured(body, at) {
            Some(keys) => self.keys.extend(keys),
            None => self.all = true,
        }
    }
}

/// The keys of `const { a, b: c } = <chain>.data`; `None` for any other use of `data`.
fn destructured(body: &str, data: usize) -> Option<Vec<String>> {
    let b = body.as_bytes();
    let mut i = data;
    while i > 0 && (is_ident_char(b[i - 1] as char) || matches!(b[i - 1], b'.' | b'?')) {
        i -= 1;
    }
    let back = |mut i: usize| {
        while i > 0 && b[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        i
    };
    let eq = back(i).checked_sub(1).filter(|&e| b[e] == b'=')?;
    let close = back(eq).checked_sub(1).filter(|&c| b[c] == b'}')?;
    let open = body[..close].rfind('{')?;
    let pattern = &body[open + 1..close];
    if pattern.contains(['{', '[']) {
        return None;
    }
    pattern
        .split(',')
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let key = p.split([':', '=']).next()?.trim();
            (is_ident(key)).then(|| key.to_string())
        })
        .collect()
}

/// A declared or detected project directory.
struct Project {
    name: String,
    application: bool,
    /// `angular.json` build `define`: global name to JavaScript source.
    defines: BTreeMap<String, String>,
}

struct Workspace<'r> {
    root: &'r Path,
    /// Projects by directory relative to the root.
    projects: BTreeMap<String, Project>,
    /// TypeScript sources by path: the owning application directory (`None`:
    /// shared) and the text.
    files: BTreeMap<String, (Option<String>, String)>,
}

impl<'r> Workspace<'r> {
    fn read(root: &'r Path, files: Vec<String>, out: &mut Vec<UiFlowDiagnostic>) -> Self {
        let mut projects = BTreeMap::new();
        let mut nx = BTreeMap::new();
        let mut detected = BTreeSet::new();
        let mut sources = Vec::new();
        for rel in files {
            if rel.split('/').any(|p| p == ".angular") {
                continue;
            }
            let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
            match name {
                "angular.json" => {
                    match read_json(root, &rel).and_then(|v| angular_projects(&v, dir)) {
                        Some(declared) => projects.extend(declared),
                        None => {
                            detected.insert(dir.to_string());
                        }
                    }
                }
                "project.json" => {
                    if let Some(project) =
                        read_json(root, &rel).and_then(|v| nx_project(root, dir, &v))
                    {
                        nx.insert(dir.to_string(), project);
                    }
                }
                "package.json"
                    if read(root, &rel).is_ok_and(|t| t.contains("\"@angular/router\"")) =>
                {
                    detected.insert(dir.to_string());
                }
                _ if is_source(&rel) => sources.push(rel),
                _ => {}
            }
        }
        let mut workspace = Workspace {
            root,
            projects: merge_projects(root, projects, nx, detected),
            files: BTreeMap::new(),
        };
        if workspace.projects.values().any(|p| p.application) {
            workspace.read_sources(sources, out);
        }
        workspace
    }

    fn read_sources(&mut self, sources: Vec<String>, out: &mut Vec<UiFlowDiagnostic>) {
        let mut counts: BTreeMap<Option<String>, usize> = BTreeMap::new();
        for rel in sources {
            let owner = (self.projects.iter())
                .filter(|(d, _)| d.is_empty() || rel.starts_with(&format!("{d}/")))
                .max_by_key(|(d, _)| d.len())
                .filter(|(_, p)| p.application)
                .map(|(d, _)| d.clone());
            let count = counts.entry(owner.clone()).or_default();
            *count += 1;
            if *count > MAX_FILES {
                if *count == MAX_FILES + 1 {
                    let scope = match &owner {
                        Some(d) => self.projects[d].name.clone(),
                        None => "the shared libraries".into(),
                    };
                    let message = format!(
                        "Angular scan of {scope} stopped after {MAX_FILES} TypeScript files; later files are unknown."
                    );
                    diag(out, &rel, 0, message);
                }
                continue;
            }
            match read(self.root, &rel) {
                Ok(text) => {
                    self.files.insert(rel, (owner, text));
                }
                Err(message) => diag(out, &rel, 0, message),
            }
        }
    }

    fn apps<'a>(&'a self, access: &'a BTreeMap<String, AccessMeaning>) -> Vec<App<'a>> {
        let mut apps = Vec::new();
        for (dir, project) in self.projects.iter().filter(|(_, p)| p.application) {
            let mut own = BTreeSet::new();
            let mut files = BTreeMap::new();
            for (path, (owner, text)) in &self.files {
                match owner {
                    Some(o) if o == dir => {
                        own.insert(path.clone());
                    }
                    Some(_) => continue,
                    None => {}
                }
                files.insert(path.clone(), text.as_str());
            }
            if own.is_empty() {
                continue;
            }
            let exports = exports_of(&files);
            apps.push(App {
                root: self.root,
                name: project.name.clone(),
                files,
                own,
                defines: &project.defines,
                access,
                exports,
            });
        }
        apps
    }
}

struct App<'a> {
    root: &'a Path,
    name: String,
    /// TypeScript sources the application sees (its own and the shared ones) by path.
    files: BTreeMap<String, &'a str>,
    own: BTreeSet<String>,
    defines: &'a BTreeMap<String, String>,
    access: &'a BTreeMap<String, AccessMeaning>,
    /// `export const NAME =`: initializer offsets by name.
    exports: BTreeMap<String, Vec<(String, usize)>>,
}

/// Where a name is resolved: the file, the offset of the use and its class.
#[derive(Clone, Copy)]
struct Scope<'s> {
    file: &'s str,
    pos: usize,
    class: Option<&'s Class>,
    classes: &'s [Class],
}

impl App<'_> {
    fn extract(&self, flow: &mut UiFlow) {
        let mut candidates = BTreeSet::new();
        for file in &self.own {
            for (f, at, _) in self.route_arrays(file, &mut flow.diagnostics) {
                candidates.insert((f, at));
            }
        }
        // Each candidate is walked; arrays reached from another one (children
        // variable, loadChildren) are not roots and keep only their prefixed copy.
        let mut walks = Vec::new();
        let mut refs = BTreeSet::new();
        for (file, at) in candidates {
            let mut walk = Walk {
                app: self,
                screens: vec![],
                layouts: vec![],
                diagnostics: vec![],
                refs: BTreeSet::new(),
                stack: vec![],
                merged: vec![],
                child_guards: vec![],
                reads: BTreeMap::new(),
            };
            walk.array_at(&file, at, "", &[]);
            refs.append(&mut walk.refs);
            walks.push(((file, at), walk));
        }
        let mut screens = Vec::new();
        let mut layouts = Vec::new();
        for (key, walk) in walks {
            if !refs.contains(&key) {
                flow.diagnostics.extend(walk.diagnostics);
                screens.extend(walk.screens);
                layouts.extend(walk.layouts);
            }
        }
        layouts.sort();
        layouts.dedup();
        // Routes of one path (`canMatch` alternatives) are distinct screens.
        let mut seen: BTreeMap<String, Vec<(String, u32)>> = BTreeMap::new();
        for (screen, _) in &mut screens {
            let origin = (screen.file.clone(), screen.line);
            let list = seen.entry(screen.id.clone()).or_default();
            let n = (list.iter().position(|o| *o == origin)).unwrap_or_else(|| {
                list.push(origin);
                list.len() - 1
            });
            if n > 0 {
                screen.id = format!("{}#{}", screen.id, n + 1);
            }
        }
        self.flows(&screens, &layouts, flow);
        flow.screens.extend(screens.into_iter().map(|(s, _)| s));
    }

    /// Route arrays a file declares or hands to the router: (file, offset of `[`, name).
    fn route_arrays(
        &self,
        file: &str,
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Vec<(String, usize, Option<String>)> {
        let Some(text) = self.files.get(file) else {
            return vec![];
        };
        let b = text.as_bytes();
        let mut arrays = typed_routes(file, text);
        for call in ROUTER_CALLS {
            for (pos, _) in text.match_indices(call) {
                let j = skip_ws(b, pos + call.len());
                if b.get(j) == Some(&b'[') {
                    arrays.push((file.into(), j, None));
                    continue;
                }
                let name = ident_at(text, j);
                if name.is_empty() {
                    continue;
                }
                match self.declaration(file, name) {
                    Some((f, at)) => arrays.push((f, at, Some(name.into()))),
                    None => diag(
                        out,
                        file,
                        line_of(text, pos),
                        format!(
                            "Route array {name} handed to the router is not declared as an array literal in this file or a relative import; its routes are unknown."
                        ),
                    ),
                }
            }
        }
        arrays
    }

    /// The route `data` keys the guard `name` reads, from its class, function or
    /// `const` declaration in the application (one hop; calls it makes are not followed).
    fn guard_reads(&self, name: &str) -> Reads {
        let mut reads = Reads::default();
        for body in self.guard_bodies(name) {
            for at in words(&body, "data") {
                reads.add(&body, at);
            }
        }
        reads
    }

    /// Source of every class, function or `const` declaration called `name`.
    fn guard_bodies(&self, name: &str) -> Vec<String> {
        let mut bodies = vec![];
        for text in self.files.values() {
            for kw in ["class", "function", "const"] {
                let starts = words(text, kw).into_iter();
                let starts = starts.map(|p| skip_ws(text.as_bytes(), p + kw.len()));
                let named = starts.filter(|&j| ident_at(text, j) == name);
                bodies.extend(named.filter_map(|j| declared_body(text, kw == "const", j, name)));
            }
        }
        bodies
    }

    /// The array literal `name` is bound to, in `file` or the file it imports it from.
    fn declaration(&self, file: &str, name: &str) -> Option<(String, usize)> {
        let text = self.files.get(file)?;
        if let Some(at) = local_array(text, name) {
            return Some((file.into(), at));
        }
        let target = self.module(file, &import_of(text, name)?)?;
        local_array(self.files[&target], name).map(|at| (target, at))
    }

    /// A relative import resolved to a scanned file.
    fn module(&self, from: &str, spec: &str) -> Option<String> {
        if !spec.starts_with('.') {
            return None;
        }
        let base = join(dir(from), spec);
        [format!("{base}.ts"), format!("{base}/index.ts"), base]
            .into_iter()
            .find(|p| self.files.contains_key(p))
    }

    fn default_class(&self, file: &str) -> Option<String> {
        let text = self.files.get(file)?;
        let at = text.find("export default class")? + "export default class".len();
        let name = ident_at(text, skip_ws(text.as_bytes(), at));
        (!name.is_empty()).then(|| name.to_string())
    }

    /// The file declaring class `name`: `from` itself, its relative import, else the
    /// only file of the application that declares it.
    fn class_file(&self, from: &str, name: &str) -> Option<String> {
        let text = self.files.get(from)?;
        if declares_class(text, name) {
            return Some(from.into());
        }
        if let Some(target) = (import_of(text, name).and_then(|s| self.module(from, &s)))
            .filter(|t| declares_class(self.files[t.as_str()], name))
        {
            return Some(target);
        }
        let mut hits = self.files.iter().filter(|(_, t)| declares_class(t, name));
        match (hits.next(), hits.next()) {
            (Some((path, _)), None) => Some(path.clone()),
            _ => None,
        }
    }

    /// Routes of the NgModule class `name`: the arrays its file hands to the router,
    /// else those of the routing modules in its `imports` that call `forChild`.
    fn module_routes(
        &self,
        from: &str,
        name: &str,
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Vec<(String, usize, Option<String>)> {
        let Some(file) = self.class_file(from, name) else {
            return vec![];
        };
        let arrays = self.route_arrays(&file, out);
        if !arrays.is_empty() {
            return arrays;
        }
        let text = self.files[&file];
        let mut found = Vec::new();
        for (pos, _) in text.match_indices("imports:") {
            let list = Parser {
                s: text,
                i: pos + "imports:".len(),
            }
            .value(0);
            let Value::Arr(items) = list.value else {
                continue;
            };
            for item in items {
                let Value::Raw(n) = &item.value else { continue };
                if let Some(f) = self.class_file(&file, n).filter(|f| {
                    is_ident(n) && self.files[f.as_str()].contains("RouterModule.forChild(")
                }) {
                    found.extend(self.route_arrays(&f, out));
                }
            }
        }
        found
    }

    /// The string value of a name in `file` (see the module docs), `None` when unread.
    fn constant(&self, name: &str, scope: Scope, depth: usize) -> Option<String> {
        let text = self.files.get(scope.file)?;
        if let Some(class) = scope.class.filter(|c| c.body.contains(&scope.pos)) {
            let start = member_start(text, &class.body, scope.pos);
            if let Some(&at) = const_inits(text, start..scope.pos, name).last() {
                return self.resolve(&init_source(text, at), Scope { pos: at, ..scope }, depth);
            }
        }
        if let Some((file, at)) = self.declaration_of(scope.file, name, scope.classes, depth) {
            let inner = Scope {
                file: &file,
                pos: at,
                class: None,
                classes: scope.classes,
            };
            return self.resolve(
                &init_source(self.files[file.as_str()], at),
                inner,
                depth + 1,
            );
        }
        if import_of(text, name).is_some() {
            return None;
        }
        let define = self.defines.get(name)?;
        Parser { s: define, i: 0 }.value(0).value.literal()
    }

    /// The initializer (file, offset) of the module-level `const name` that `file`
    /// sees: declared once in the file, else imported (the relative import's file,
    /// else the only `export const` of that name in the application).
    fn declaration_of(
        &self,
        file: &str,
        name: &str,
        classes: &[Class],
        depth: usize,
    ) -> Option<(String, usize)> {
        let text = self.files.get(file).filter(|_| depth <= MAX_RESOLVE)?;
        let bodies: Vec<&Range<usize>> = (classes.iter())
            .filter(|c| c.file == file)
            .map(|c| &c.body)
            .collect();
        let decls: Vec<usize> = const_inits(text, 0..text.len(), name)
            .into_iter()
            .filter(|at| !bodies.iter().any(|b| b.contains(at)))
            .collect();
        match decls.as_slice() {
            [at] => return Some((file.into(), *at)),
            [] => {}
            _ => return None,
        }
        let spec = import_of(text, name)?;
        if let Some(found) = (self.module(file, &spec))
            .and_then(|target| self.declaration_of(&target, name, classes, depth + 1))
        {
            return Some(found);
        }
        match self.exports.get(name).map(Vec::as_slice) {
            Some([(file, at)]) => Some((file.clone(), *at)),
            _ => None,
        }
    }

    /// `Name.member` of a const object literal (`Object.freeze({…} as const)` too).
    fn object_member(
        &self,
        file: &str,
        object: &str,
        member: &str,
        depth: usize,
    ) -> Option<String> {
        let (file, at) = self.declaration_of(file, object, &[], depth)?;
        let text = self.files[file.as_str()];
        let mut parser = Parser { s: text, i: at };
        if text[at..].starts_with("Object.freeze(") {
            parser.i += "Object.freeze(".len();
        }
        parser.ws();
        if parser.peek() != b'{' {
            return None;
        }
        let fields = parser.object(0);
        let (_, item) = fields.iter().find(|(k, _)| k == member)?;
        let constant = |name: &str| self.file_constant(&file, name, depth + 1);
        match &item.value {
            Value::Str(s) => Some(s.clone()),
            Value::Tpl(t) => fill_all(t, constant),
            Value::Raw(r) if is_name(r) => constant(r),
            _ => None,
        }
    }

    /// The string an expression evaluates to, holes and unread later parts as
    /// `:param`; `None` when its start is not readable.
    fn resolve(&self, expr: &str, scope: Scope, depth: usize) -> Option<String> {
        if depth > MAX_RESOLVE {
            return None;
        }
        let item = Parser {
            s: expr.trim(),
            i: 0,
        }
        .value(0);
        match &item.value {
            Value::Str(s) => Some(s.clone()),
            Value::Tpl(t) => fill(t, |hole| self.resolve(hole, scope, depth + 1)),
            Value::Raw(raw) => {
                let parts = split_plus(raw);
                if let [first, rest @ ..] = parts.as_slice()
                    && !rest.is_empty()
                {
                    let mut out = self.resolve(first, scope, depth + 1)?;
                    for part in rest {
                        let value = self.resolve(part, scope, depth + 1);
                        out.push_str(value.as_deref().unwrap_or(PARAM));
                    }
                    return Some(out);
                }
                if let Some(field) = raw.strip_prefix("this.").filter(|f| is_ident(f)) {
                    return self.field(scope.class?, field, scope.classes, depth + 1);
                }
                is_ident(raw)
                    .then(|| self.constant(raw, scope, depth + 1))
                    .flatten()
            }
            _ => None,
        }
    }

    /// A `readonly` field of `class` or of one base class it names.
    fn field(&self, class: &Class, name: &str, classes: &[Class], depth: usize) -> Option<String> {
        if depth > MAX_RESOLVE {
            return None;
        }
        let text = self.files[&class.file];
        if let Some(at) = field_init(text, &class.body, name) {
            let scope = Scope {
                file: &class.file,
                pos: at,
                class: Some(class),
                classes,
            };
            return self.resolve(&init_source(text, at), scope, depth);
        }
        let found: BTreeSet<String> = (class.heritage.iter())
            .flat_map(|h| classes.iter().filter(move |c| &c.name == h))
            .filter_map(|base| self.field(base, name, classes, depth + 1))
            .collect();
        let mut found = found.into_iter();
        match (found.next(), found.next()) {
            (Some(one), None) => Some(one),
            _ => None,
        }
    }

    /// A name's string constant in `file`, or `Name.member` of a string enum or a
    /// const object literal, without class scope.
    fn file_constant(&self, file: &str, name: &str, depth: usize) -> Option<String> {
        if depth > MAX_RESOLVE {
            return None;
        }
        if let Some((object, member)) = name.split_once('.') {
            return (is_ident(object) && is_ident(member))
                .then(|| {
                    enum_value(&self.files, object, member)
                        .or_else(|| self.object_member(file, object, member, depth))
                })
                .flatten();
        }
        let scope = Scope {
            file,
            pos: 0,
            class: None,
            classes: &[],
        };
        is_ident(name)
            .then(|| self.constant(name, scope, depth))
            .flatten()
    }

    /// The path of navigation commands in `file`: a literal or link array whose names
    /// may be string constants, or a call of a one-level helper function.
    fn link_path(&self, file: &str, item: &Item) -> Option<String> {
        if let Value::Raw(raw) = &item.value
            && let Some((name, _)) = call_parts(raw)
        {
            return is_ident(name).then(|| helper(&self.files, name)).flatten();
        }
        item.value.link(&|name| self.file_constant(file, name, 0))
    }

    /// Transitions and actions of every component.
    fn flows(&self, screens: &[(Screen, String)], layouts: &[(String, String)], flow: &mut UiFlow) {
        let classes: Vec<Class> = (self.files.iter())
            .flat_map(|(f, t)| Class::scan(f, t))
            .collect();
        let forwarders = forwarders(&self.files);
        let calls: Vec<Vec<HttpCall>> = (classes.iter())
            .map(|c| self.http_calls(c, &classes, &forwarders, &mut flow.diagnostics))
            .collect();
        let mut flows = Flows {
            app: self,
            screens,
            layouts,
            classes: &classes,
            calls: &calls,
            everywhere: Origin::AnyScreen {
                application: self.name.clone(),
            },
            hosts: vec![],
            same_name: count(
                classes
                    .iter()
                    .filter(|c| !c.component)
                    .map(|c| c.name.as_str()),
            ),
            selectors: count(classes.iter().filter_map(|c| c.selector.as_deref())),
        };
        flows.hosts = flows.hosts();
        for (class, own_calls) in classes.iter().zip(&calls) {
            if class.component {
                flows.component(class, own_calls, flow);
            }
        }
    }

    fn http_calls(
        &self,
        class: &Class,
        classes: &[Class],
        forwarders: &BTreeSet<String>,
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Vec<HttpCall> {
        let text = self.files[&class.file];
        let mut found = client_calls(text, class.range.clone());
        found.extend(forwarder_calls(text, class.range.clone(), forwarders));
        found.sort_by_key(|(pos, ..)| *pos);
        let mut calls = Vec::new();
        for (pos, method, url) in found {
            let line = line_of(text, pos);
            let scope = Scope {
                file: &class.file,
                pos,
                class: Some(class),
                classes,
            };
            let source = url.source(text);
            let call = match self.resolve(&source, scope, 0) {
                Some(u) => Call::Http {
                    method,
                    path: url_path(&u),
                },
                None => {
                    let source = self.unresolved_text(&source, scope);
                    let message =
                        format!("HTTP call URL is not a literal; the call is unknown: {source}");
                    diag(out, &class.file, line, message);
                    Call::Unresolved(source)
                }
            };
            calls.push(HttpCall {
                file: class.file.clone(),
                line,
                call,
            });
        }
        calls
    }

    /// The source of an unread URL; a local constant's initializer instead of its name.
    fn unresolved_text(&self, source: &str, scope: Scope) -> String {
        let text = self.files[scope.file];
        scope
            .class
            .filter(|c| is_ident(source) && c.body.contains(&scope.pos))
            .and_then(|c| {
                let start = member_start(text, &c.body, scope.pos);
                const_inits(text, start..scope.pos, source).last().copied()
            })
            .map_or_else(|| source.to_string(), |at| init_source(text, at))
    }
}

/// How often each name occurs.
fn count<'n>(names: impl Iterator<Item = &'n str>) -> BTreeMap<&'n str, usize> {
    let mut counts = BTreeMap::new();
    for name in names {
        *counts.entry(name).or_default() += 1;
    }
    counts
}

/// One application's routes, classes and calls while the transitions and actions
/// of its components are collected.
struct Flows<'f, 'a> {
    app: &'f App<'a>,
    screens: &'f [(Screen, String)],
    layouts: &'f [(String, String)],
    classes: &'f [Class],
    calls: &'f [Vec<HttpCall>],
    everywhere: Origin,
    /// Route and layout components with origin, base route and template text.
    hosts: Vec<(&'f Class, Origin, Option<&'f str>, String)>,
    /// Non-component classes per name, components per selector.
    same_name: BTreeMap<&'f str, usize>,
    selectors: BTreeMap<&'f str, usize>,
}

impl<'f> Flows<'f, '_> {
    /// Where a component is shown with which base route: its screens, and its layouts
    /// (on every screen, the layout's route as base).
    fn routed(&self, name: &str) -> Vec<(Origin, Option<&'f str>)> {
        let screens: &'f [(Screen, String)] = self.screens;
        let layouts: &'f [(String, String)] = self.layouts;
        (screens.iter())
            .filter(|(_, c)| c == name)
            .map(|(s, _)| (Origin::Screen(s.id.clone()), s.route.as_deref()))
            .chain(
                (layouts.iter())
                    .filter(|(n, _)| n == name)
                    .map(|(_, r)| (self.everywhere.clone(), Some(r.as_str()))),
            )
            .collect()
    }

    fn hosts(&self) -> Vec<(&'f Class, Origin, Option<&'f str>, String)> {
        let classes: &'f [Class] = self.classes;
        (classes.iter())
            .filter(|c| c.component)
            .flat_map(|c| {
                let template = self.template(c).ok().flatten().map(|(_, t, _)| t);
                (self.routed(&c.name).into_iter())
                    .filter_map(|(o, r)| Some((c, o, r, template.clone()?)))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Where a component's links and calls start; `None` for a shared component no
    /// route of the application names.
    fn froms(&self, class: &Class) -> Option<Vec<(Origin, Option<&'f str>)>> {
        let mut froms = self.routed(&class.name);
        if froms.is_empty() && !self.app.own.contains(&class.file) {
            return None;
        }
        if froms.is_empty()
            && let Some(selector) = &class.selector
        {
            // A selector used twice: the host's file must name the class (`imports`).
            let shared = self
                .selectors
                .get(selector.as_str())
                .is_some_and(|n| *n > 1);
            froms = (self.hosts.iter())
                .filter(|(host, _, _, t)| {
                    embeds(t, selector)
                        && (!shared || !words(self.app.files[&host.file], &class.name).is_empty())
                })
                .map(|(_, o, r, _)| (o.clone(), *r))
                .collect();
            froms.sort();
            froms.dedup();
        }
        if froms.is_empty() {
            froms.push((self.everywhere.clone(), None));
        }
        Some(froms)
    }

    /// For each class name the component uses that several classes carry: the
    /// declaring file it sees (its own or its relative import); `None` keeps all.
    fn chosen<'i>(
        &self,
        class: &Class,
        ids: &BTreeSet<&'i str>,
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> BTreeMap<&'i str, Option<String>> {
        let text = self.app.files[&class.file];
        let mut chosen = BTreeMap::new();
        for &name in (ids.iter()).filter(|n| self.same_name.get(*n).is_some_and(|k| *k > 1)) {
            let declares = |f: &String| {
                (self.classes.iter()).any(|c| !c.component && c.name == name && &c.file == f)
            };
            let file = Some(class.file.clone()).filter(declares).or_else(|| {
                (import_of(text, name).and_then(|s| self.app.module(&class.file, &s)))
                    .filter(declares)
            });
            if file.is_none() {
                out.push(UiFlowDiagnostic {
                    file: class.file.clone(),
                    line: line_of(text, class.range.start),
                    message: format!(
                        "Several classes are named {name} and the import is no relative path to one of them; the calls of all of them count for {}.",
                        class.name
                    ),
                    severity: DiagnosticSeverity::Note,
                });
            }
            chosen.insert(name, file);
        }
        chosen
    }

    /// Calls of the classes a component names, directly or by a class of the
    /// application they extend or implement.
    fn injected(&self, class: &Class, out: &mut Vec<UiFlowDiagnostic>) -> Vec<&'f HttpCall> {
        let text = self.app.files[&class.file];
        let ids = idents(&text[class.range.clone()]);
        let chosen = self.chosen(class, &ids, out);
        let calls: &'f [Vec<HttpCall>] = self.calls;
        (self.classes.iter())
            .zip(calls)
            .filter(|(c, _)| {
                let named = ids.contains(c.name.as_str())
                    && (chosen.get(c.name.as_str()))
                        .is_none_or(|f| f.as_ref().is_none_or(|f| *f == c.file));
                let by_base = (c.heritage.iter())
                    .any(|h| self.same_name.contains_key(h.as_str()) && ids.contains(h.as_str()));
                !c.component && (named || by_base)
            })
            .flat_map(|(_, calls)| calls)
            .collect()
    }

    /// The transitions and actions of one component.
    fn component(&self, class: &Class, own_calls: &[HttpCall], flow: &mut UiFlow) {
        let Some(froms) = self.froms(class) else {
            return;
        };
        let text = self.app.files[&class.file];
        let mut links = self.template_links(class, &mut flow.diagnostics);
        let path_of = |item: &Item| self.app.link_path(&class.file, item);
        links.extend(Link::in_class(
            &class.file,
            text,
            class.range.clone(),
            &path_of,
        ));
        let injected = self.injected(class, &mut flow.diagnostics);
        for (from, base) in froms {
            for link in &links {
                let target = match (&from, link.stay) {
                    (Origin::Screen(id), true) => Target::Screen(id.clone()),
                    (_, true) => continue, // stays on whichever screen shows it
                    _ => self.target(link, base, &mut flow.diagnostics),
                };
                flow.transitions.push(Transition {
                    weak: false,
                    from: from.clone(),
                    target,
                    trigger: link.trigger.clone(),
                    file: link.file.clone(),
                    line: link.line,
                    condition: archguard_core::AccessState::Undeclared,
                    source: SOURCE.into(),
                });
            }
            for call in own_calls.iter().chain(injected.iter().copied()) {
                flow.actions.push(Action {
                    weak: false,
                    screen: from.clone(),
                    trigger: None,
                    call: call.call.clone(),
                    file: call.file.clone(),
                    line: call.line,
                    condition: archguard_core::AccessState::Undeclared,
                    source: SOURCE.into(),
                });
            }
        }
    }

    /// The component's template: (file, text, first line).
    fn template(&self, class: &Class) -> Result<Option<(String, String, u32)>, String> {
        Ok(match &class.template {
            Some(Template::Inline { text, line }) => {
                Some((class.file.clone(), text.clone(), *line))
            }
            Some(Template::File(path)) => Some((path.clone(), read(self.app.root, path)?, 1)),
            None => None,
        })
    }

    fn template_links(&self, class: &Class, out: &mut Vec<UiFlowDiagnostic>) -> Vec<Link> {
        match self.template(class) {
            Ok(Some((file, text, line))) => Link::in_template(&file, &text, line),
            Ok(None) => vec![],
            Err(message) => {
                if let Some(Template::File(path)) = &class.template {
                    diag(out, path, 0, message);
                }
                vec![]
            }
        }
    }

    fn target(&self, link: &Link, base: Option<&str>, out: &mut Vec<UiFlowDiagnostic>) -> Target {
        let message = match &link.path {
            None => format!("Navigation target is not a literal: {}", link.source),
            Some(_) => match link.route(base) {
                None => format!(
                    "Relative navigation {} in a component that is no route target; its base route is unknown.",
                    link.source
                ),
                Some(path) => {
                    let routes = (self.screens)
                        .iter()
                        .filter_map(|(s, _)| Some((&s.id, s.route.as_deref()?)));
                    match best_route(&path, routes) {
                        Ok(id) => return Target::Screen(id.clone()),
                        Err(tied) if tied.is_empty() => format!(
                            "Navigation target {path} matches no route of application {}.",
                            self.app.name
                        ),
                        Err(tied) => format!(
                            "Navigation target {path} matches several routes equally: {}.",
                            tied.iter()
                                .map(|id| id.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    }
                }
            },
        };
        diag(out, &link.file, link.line, message);
        Target::Unresolved(link.source.clone())
    }
}

/// A route source file: its path and text.
#[derive(Clone, Copy)]
struct Src<'a> {
    file: &'a str,
    text: &'a str,
}

/// Route arrays of one walk, from one candidate down through children and lazy routes.
struct Walk<'w> {
    app: &'w App<'w>,
    /// Each screen with its component class name.
    screens: Vec<(Screen, String)>,
    /// Layout and outlet components with their route.
    layouts: Vec<(String, String)>,
    diagnostics: Vec<UiFlowDiagnostic>,
    refs: BTreeSet<(String, usize)>,
    stack: Vec<(String, usize)>,
    /// `data` objects (file, offset) of componentless ancestors: merged into children.
    merged: Vec<(String, usize)>,
    /// Guards of the `canActivateChild` of ancestors: they see the child's `data`.
    child_guards: Vec<String>,
    /// What each guard reads of the route `data`, by guard name.
    reads: BTreeMap<String, Reads>,
}

/// The route `data` keys a guard's declaration reads.
#[derive(Clone, Default)]
struct Reads {
    keys: BTreeSet<String>,
    /// `data` is used other than by key (`const { a } = route.data`): any key may be read.
    all: bool,
}

impl<'w> Walk<'w> {
    fn diag(&mut self, src: Src, at: usize, message: String) {
        diag(
            &mut self.diagnostics,
            src.file,
            line_of(src.text, at),
            message,
        );
    }

    fn note(&mut self, src: Src, at: usize, message: String) {
        self.diagnostics.push(UiFlowDiagnostic {
            file: src.file.into(),
            line: line_of(src.text, at),
            message,
            severity: DiagnosticSeverity::Note,
        });
    }

    fn array_at(&mut self, file: &str, at: usize, prefix: &str, guards: &[Condition]) {
        let app: &'w App<'w> = self.app;
        let src = Src {
            file,
            text: app.files[file],
        };
        let key = (file.to_string(), at);
        if self.stack.len() >= MAX_DEPTH || self.stack.contains(&key) {
            let message = "Route arrays nest too deep or load each other in a cycle; deeper routes are unknown.";
            return self.diag(src, at, message.into());
        }
        self.stack.push(key);
        let items = Parser { s: src.text, i: at }.array(0);
        for item in &items {
            self.route(src, item, prefix, guards);
        }
        self.stack.pop();
    }

    /// Follows a route array other than the walk's own and marks it as not a root.
    fn follow(&mut self, file: String, at: usize, prefix: &str, guards: &[Condition]) {
        self.array_at(&file, at, prefix, guards);
        self.refs.insert((file, at));
    }

    fn route(&mut self, src: Src, item: &Item, prefix: &str, inherited: &[Condition]) {
        let Value::Obj(fields) = &item.value else {
            let message = format!(
                "Route entry is not an object literal; its routes are unknown: {}",
                item.source(src.text)
            );
            return self.diag(src, item.at, message);
        };
        let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);
        if get("redirectTo").is_some() {
            return;
        }
        let Some(path) = self.path(src, get("path")) else {
            return;
        };
        let full = [prefix, &path]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        let mut guards = inherited.to_vec();
        let mut names = self.child_guards.clone();
        self.route_guards(src, fields, &mut guards, &mut names);
        let componentless = get("component").is_none() && get("loadComponent").is_none();
        self.data_roles(src, get("data"), &names, &mut guards);
        let layout = get("outlet").is_some()
            || get("children").is_some_and(|c| path.is_empty() || self.has_empty_child(src, c));
        if let Some(name) = self.component(src, get("component"), get("loadComponent")) {
            if layout {
                self.layouts.push((name, format!("/{full}")));
            } else {
                let screen = self.screen(src, item, &full, name, &guards);
                self.screens.push(screen);
            }
        }
        let (merged, child_guards) = (self.merged.len(), self.child_guards.len());
        if let Some(v) = get("canActivateChild") {
            let mut child = vec![];
            self.guards(src, v, &mut guards, &mut child);
            self.child_guards.extend(child);
        }
        if let (true, Some(d)) = (componentless, get("data")) {
            self.merged.push((src.file.into(), d.at));
        }
        if let Some(v) = get("children") {
            self.children(src, v, &full, &guards);
        }
        if let Some(v) = get("loadChildren") {
            self.load_children(src, v, &full, &guards);
        }
        self.merged.truncate(merged);
        self.child_guards.truncate(child_guards);
    }

    /// `canActivate`, `canMatch` and `canLoad` of a route object.
    fn route_guards(
        &mut self,
        src: Src,
        fields: &[(String, Item)],
        guards: &mut Vec<Condition>,
        names: &mut Vec<String>,
    ) {
        let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);
        for key in ["canActivate", "canMatch"] {
            if let Some(v) = get(key) {
                self.guards(src, v, guards, names);
            }
        }
        let Some(v) = get("canLoad") else { return };
        if get("loadChildren").is_none() {
            let message = "canLoad on a route without loadChildren guards nothing.";
            return self.note(src, v.at, message.into());
        }
        let message = "canLoad only guards loading the lazy routes once; it does not run on later navigations.";
        self.note(src, v.at, message.into());
        self.guards(src, v, guards, names);
    }

    /// Whether a child route (inline or a declared array) has the empty path.
    fn has_empty_child(&self, src: Src, children: &Item) -> bool {
        let empty = |items: &[Item]| {
            items.iter().any(|child| match &child.value {
                Value::Obj(f) => f.iter().find(|(k, _)| k == "path").is_none_or(
                    |(_, p)| matches!(&p.value, Value::Str(s) if s.trim_matches('/').is_empty()),
                ),
                _ => false,
            })
        };
        match &children.value {
            Value::Arr(items) => empty(items),
            Value::Raw(name) if is_name(name) => {
                self.app.declaration(src.file, name).is_some_and(|(f, at)| {
                    empty(
                        &Parser {
                            s: self.app.files[&f],
                            i: at,
                        }
                        .array(0),
                    )
                })
            }
            _ => false,
        }
    }

    /// The path, `""` without one; `None` (and a diagnostic) when not readable.
    fn path(&mut self, src: Src, v: Option<&Item>) -> Option<String> {
        let v = match v {
            None => return Some(String::new()),
            Some(v) => v,
        };
        let constant = |name: &str| self.app.file_constant(src.file, name, 0);
        let value = match &v.value {
            Value::Str(p) => Some(p.clone()),
            Value::Tpl(t) => fill_all(t, constant),
            Value::Raw(name) => constant(name),
            _ => None,
        };
        if value.is_none() {
            let message = format!(
                "Route path is not a literal; the route and its children are unknown: {}",
                v.source(src.text)
            );
            self.diag(src, v.at, message);
        }
        value.map(|p| p.trim_matches('/').to_string())
    }

    fn screen(
        &self,
        src: Src,
        item: &Item,
        full: &str,
        name: String,
        guards: &[Condition],
    ) -> (Screen, String) {
        let route = format!("/{full}");
        let app = &self.app.name;
        let access = match guards {
            [] => None,
            [one] => Some(one.clone()),
            all => Some(Condition::All(all.to_vec())),
        };
        let screen = Screen {
            weak: false,
            id: format!("{app}:{route}"),
            name: name.clone(),
            kind: ScreenKind::Page,
            route: Some(route),
            application: Some(app.clone()),
            file: src.file.into(),
            line: line_of(src.text, item.at),
            access: archguard_core::AccessState::from_parts(access, false),
            source: SOURCE.into(),
        };
        (screen, name)
    }

    fn component(&mut self, src: Src, eager: Option<&Item>, lazy: Option<&Item>) -> Option<String> {
        match (eager, lazy) {
            (
                Some(Item {
                    value: Value::Raw(name),
                    ..
                }),
                _,
            ) if is_name(name) => Some(name.clone()),
            (Some(v), _) => {
                let message = format!(
                    "Route component is not a class name: {}",
                    v.source(src.text)
                );
                self.diag(src, v.at, message);
                None
            }
            (None, Some(v)) => self.load_component(src, v),
            (None, None) => None,
        }
    }

    fn children(&mut self, src: Src, v: &Item, prefix: &str, guards: &[Condition]) {
        let message = match &v.value {
            Value::Arr(items) => {
                for child in items {
                    self.route(src, child, prefix, guards);
                }
                return;
            }
            Value::Raw(name) if is_name(name) => match self.app.declaration(src.file, name) {
                Some((f, at)) => return self.follow(f, at, prefix, guards),
                None => format!(
                    "Child routes {name} are not declared as an array literal in this file or a relative import; they are unknown."
                ),
            },
            _ => format!(
                "Child routes are not a literal array; they are unknown: {}",
                v.source(src.text)
            ),
        };
        self.diag(src, v.at, message);
    }

    fn guards(&mut self, src: Src, v: &Item, out: &mut Vec<Condition>, names: &mut Vec<String>) {
        let unwrapped;
        let items: Vec<&Item> = match &v.value {
            Value::Arr(items) => items.iter().collect(),
            // `mapToCanActivate([A, B])` lists class guards.
            Value::Raw(call) if call_parts(call).is_some_and(|(n, _)| n == "mapToCanActivate") => {
                unwrapped = Parser {
                    s: src.text,
                    i: v.at + call.find('(').unwrap_or(0),
                }
                .array(0);
                match unwrapped.as_slice() {
                    [
                        Item {
                            value: Value::Arr(items),
                            ..
                        },
                    ] => items.iter().collect(),
                    _ => return self.unknown_guards(src, v, out),
                }
            }
            _ => return self.unknown_guards(src, v, out),
        };
        for guard in items {
            let source = guard.source(src.text);
            let parts = match &guard.value {
                Value::Raw(name) if is_name(name) => Some((name.as_str(), None)),
                Value::Raw(call) => call_parts(call).map(|(n, args)| (n, Some(args))),
                _ => None,
            };
            let Some((name, args)) = parts else {
                let message = format!(
                    "Guard is not a plain function or class name; its condition is unknown: {source}"
                );
                self.diag(src, guard.at, message);
                out.push(Condition::Unknown(source));
                continue;
            };
            names.push(name.into());
            match guard_condition(self.app.access.get(name), name, args.as_deref()) {
                Some(condition) => out.extend(condition),
                None => {
                    let message = format!(
                        "Guard {name} takes its roles or policies from string arguments, but they are not literals; its condition is unknown: {source}"
                    );
                    self.diag(src, guard.at, message);
                    out.push(Condition::Unknown(source));
                }
            }
        }
    }

    /// Guards given as a variable, a call or anything but a list: unknown.
    fn unknown_guards(&mut self, src: Src, v: &Item, out: &mut Vec<Condition>) {
        let source = v.source(src.text);
        let message = format!(
            "Guards are not an array literal (a variable or a call); their conditions are unknown: {source}"
        );
        self.diag(src, v.at, message);
        out.push(Condition::Unknown(source));
    }

    /// Roles from the `data` keys that a guard of the route reads (`names`: its own
    /// guards and the `canActivateChild` guards of its ancestors). `data` of
    /// componentless ancestors is merged in; keys no guard reads give a note.
    fn data_roles(
        &mut self,
        src: Src,
        data: Option<&Item>,
        names: &[String],
        out: &mut Vec<Condition>,
    ) {
        let app: &'w App<'w> = self.app;
        let mut reads = Reads::default();
        for name in names {
            let r = (self.reads.entry(name.clone()))
                .or_insert_with(|| app.guard_reads(name))
                .clone();
            reads.keys.extend(r.keys);
            reads.all |= r.all;
        }
        let reading = reads.all || !reads.keys.is_empty();
        let merged: Vec<(Src, Vec<(String, Item)>)> = (self.merged.iter())
            .filter_map(|(file, at)| {
                let (file, text) = app.files.get_key_value(file.as_str())?;
                let obj = Parser { s: text, i: *at }.value(0);
                let Value::Obj(f) = obj.value else {
                    return None;
                };
                Some((Src { file, text }, f))
            })
            .collect();
        let own = data.map_or_else(Vec::new, |v| self.own_data(src, v, reading, out));
        // Own keys override the merged ones; only the own ones get a note.
        let fields: Vec<(&String, Src, &Item, bool)> = (merged.iter())
            .flat_map(|(s, f)| f.iter().map(|(k, i)| (k, *s, i, false)))
            .chain(own.iter().map(|(k, i)| (k, src, i, true)))
            .collect();
        let mut done = BTreeSet::new();
        for (key, s, item, own) in fields.iter().rev() {
            if done.insert(*key) {
                self.data_key(key, *s, item, (&reads, *own), out);
            }
        }
    }

    /// The keys of a route's own `data` (`{ … } satisfies T` is an object literal
    /// too). A spread or non-literal `data` that a guard reads is unknown.
    fn own_data(
        &mut self,
        src: Src,
        v: &Item,
        reading: bool,
        out: &mut Vec<Condition>,
    ) -> Vec<(String, Item)> {
        let mut unknown = |this: &mut Self, at: usize, source: String, what: &str| {
            this.diag(src, at, format!("{what}: {source}"));
            out.push(Condition::Unknown(source));
        };
        if !src.text[v.at..].starts_with('{') {
            if reading {
                let what =
                    "Route data is not an object literal; the keys a guard reads are unknown";
                unknown(self, v.at, v.source(src.text), what);
            }
            return vec![];
        }
        let fields = Parser {
            s: src.text,
            i: v.at,
        }
        .object(0);
        let spread = |i: &Item| {
            let what = "Route data spreads keys a guard may read; they are unknown";
            (i.at, i.source(src.text), what)
        };
        // A spread (empty key) may bring any key.
        for (k, i) in &fields {
            if k.is_empty() && reading {
                let (at, source, what) = spread(i);
                unknown(self, at, source, what);
            }
        }
        fields.into_iter().filter(|(k, _)| !k.is_empty()).collect()
    }

    /// One merged `data` key: roles when a guard reads it, else a note.
    fn data_key(
        &mut self,
        key: &str,
        s: Src,
        item: &Item,
        (reads, own): (&Reads, bool),
        out: &mut Vec<Condition>,
    ) {
        let read = reads.all || reads.keys.contains(key);
        let Value::Arr(items) = &item.value else {
            // Named keys that are no array are unknown; with `data` used as a
            // whole, non-lists (titles, sort orders) are not taken for roles.
            if reads.keys.contains(key) {
                let source = item.source(s.text);
                let message = format!(
                    "Route data {key} is read by a guard but is not a literal array; the roles are unknown: {source}"
                );
                self.diag(s, item.at, message);
                out.push(Condition::Unknown(source));
            }
            return;
        };
        if !read {
            if own && !items.is_empty() {
                let message = format!(
                    "Route data {key} is read by no guard of this route; it gives no access condition."
                );
                self.note(s, item.at, message);
            }
            return;
        }
        let mut roles: Vec<Condition> = items.iter().map(|r| self.role(s, r)).collect();
        let condition = match roles.len() {
            0 => return,
            1 => roles.remove(0),
            _ => Condition::Any(roles),
        };
        if !out.contains(&condition) {
            out.push(condition);
        }
    }

    fn role(&mut self, src: Src, item: &Item) -> Condition {
        let source = item.source(src.text);
        match &item.value {
            Value::Str(s) => return Condition::Role(s.clone()),
            Value::Raw(r) => {
                if let Some((name, member)) = r
                    .split_once('.')
                    .filter(|(n, m)| is_ident(n) && is_ident(m))
                {
                    if let Some(value) = enum_value(&self.app.files, name, member) {
                        return Condition::Role(value);
                    }
                    let message = format!(
                        "{source} is no string member of an enum in application {}; the role is unknown.",
                        self.app.name
                    );
                    self.diag(src, item.at, message);
                    return Condition::Unknown(source);
                }
            }
            _ => {}
        }
        let message = format!("Route role is not a literal; the role is unknown: {source}");
        self.diag(src, item.at, message);
        Condition::Unknown(source)
    }

    fn load_component(&mut self, src: Src, v: &Item) -> Option<String> {
        let text = v.source(src.text);
        if let Some((spec, member)) = Self::lazy_import(&text) {
            let name = member.or_else(|| {
                let module = self.app.module(src.file, &spec)?;
                self.app.default_class(&module)
            });
            if name.is_some() {
                return name;
            }
        }
        let message = format!(
            "Lazy component is not a literal relative import of a named class; the screen is unknown: {text}"
        );
        self.diag(src, v.at, message);
        None
    }

    fn load_children(&mut self, src: Src, v: &Item, prefix: &str, guards: &[Condition]) {
        let text = v.source(src.text);
        let module_class = text
            .split_once("=>")
            .map(|(_, r)| r.trim())
            .filter(|r| is_ident(r) && !text.contains("import("));
        let chosen = if let Some(class) = module_class {
            let arrays = self
                .app
                .module_routes(src.file, class, &mut self.diagnostics);
            if arrays.is_empty() {
                let message = format!(
                    "Lazy module {class} hands no route array to the router in its file or its routing modules; its routes are unknown."
                );
                return self.diag(src, v.at, message);
            }
            arrays
        } else {
            let Some((target, member)) = Self::lazy_import(&text)
                .and_then(|(spec, member)| Some((self.app.module(src.file, &spec)?, member)))
            else {
                let message = format!(
                    "Lazy child routes are not a literal relative import of a scanned file; they are unknown: {text}"
                );
                return self.diag(src, v.at, message);
            };
            let arrays = self.app.route_arrays(&target, &mut self.diagnostics);
            let named: Vec<_> = arrays
                .iter()
                .filter(|(_, _, n)| n.is_some() && *n == member)
                .cloned()
                .collect();
            let mut chosen = if named.is_empty() { arrays } else { named };
            if chosen.is_empty()
                && let Some(m) = &member
            {
                chosen = self.app.module_routes(&target, m, &mut self.diagnostics);
            }
            if chosen.is_empty() {
                let message = format!("No route array found in {target}; its routes are unknown.");
                self.diag(src, v.at, message);
            }
            chosen
        };
        for (f, at, _) in chosen {
            self.follow(f, at, prefix, guards);
        }
    }

    /// `import('spec')` with the member of a following `.then(m => m.Member)`.
    fn lazy_import(text: &str) -> Option<(String, Option<String>)> {
        let at = text.find("import(")? + "import(".len();
        let Value::Str(spec) = (Parser { s: text, i: at }.value(0)).value else {
            return None;
        };
        let member = text.find(".then(").and_then(|t| {
            let arrow = text[t..].find("=>")? + t + 2;
            let body = text[arrow..].trim_start();
            let path = body
                .split(|c: char| !(is_ident_char(c) || c == '.'))
                .next()?;
            path.rsplit('.')
                .next()
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });
        Some((spec, member))
    }
}

struct HttpCall {
    file: String,
    line: u32,
    call: Call,
}

enum Template {
    Inline { text: String, line: u32 },
    File(String),
}

/// A class; `range` runs from its `@Component` decorator (else the `class` keyword)
/// to the next class, `body` is its braces.
struct Class {
    name: String,
    file: String,
    range: Range<usize>,
    body: Range<usize>,
    /// Names it extends or implements, without type arguments.
    heritage: Vec<String>,
    component: bool,
    template: Option<Template>,
    selector: Option<String>,
}

impl Class {
    fn scan(file: &str, text: &str) -> Vec<Class> {
        let decls = class_decls(text);
        // `@Component(` decorators by the class declaration they precede.
        let mut decorated: BTreeMap<usize, (usize, Vec<(String, Item)>)> = BTreeMap::new();
        for (pos, _) in text.match_indices("@Component(") {
            let meta = Parser {
                s: text,
                i: pos + "@Component(".len(),
            }
            .value(0);
            if let Some(d) = decls.iter().find(|d| d.0 >= meta.end) {
                let fields = match meta.value {
                    Value::Obj(fields) => fields,
                    _ => vec![],
                };
                decorated.entry(d.0).or_insert((pos, fields));
            }
        }
        let starts: Vec<usize> = (decls.iter())
            .map(|d| decorated.get(&d.0).map_or(d.0, |(p, _)| *p))
            .collect();
        decls
            .into_iter()
            .enumerate()
            .map(|(k, (at, name, heritage, body))| {
                let meta = decorated.remove(&at);
                let fields = meta.as_ref().map(|(_, f)| f.as_slice()).unwrap_or_default();
                let selector = fields
                    .iter()
                    .find_map(|(k, v)| match (k.as_str(), &v.value) {
                        ("selector", Value::Str(s)) => Some(s.clone()),
                        _ => None,
                    });
                Class {
                    name,
                    file: file.into(),
                    range: starts[k]..starts.get(k + 1).copied().unwrap_or(text.len()),
                    body,
                    heritage,
                    component: meta.is_some(),
                    template: Self::template(file, text, fields),
                    selector,
                }
            })
            .collect()
    }

    fn template(file: &str, text: &str, fields: &[(String, Item)]) -> Option<Template> {
        fields
            .iter()
            .find_map(|(k, v)| match (k.as_str(), &v.value) {
                ("templateUrl", Value::Str(url)) => Some(Template::File(join(dir(file), url))),
                ("template", Value::Str(t) | Value::Tpl(t)) => Some(Template::Inline {
                    text: t.clone(),
                    line: line_of(text, v.at),
                }),
                _ => None,
            })
    }
}

/// One navigation in a template or class. `path` is `None` when not literal.
struct Link {
    file: String,
    line: u32,
    source: String,
    path: Option<String>,
    relative: bool,
    /// Empty commands (`[]`): the current screen with new query or fragment.
    stay: bool,
    trigger: Option<String>,
}

impl Link {
    /// `routerLink="…"` and `[routerLink]="…"`; `first_line` is the template's first line.
    fn in_template(file: &str, tpl: &str, first_line: u32) -> Vec<Link> {
        let b = tpl.as_bytes();
        let mut links = Vec::new();
        for (pos, _) in tpl.match_indices("routerLink") {
            let mut j = pos + "routerLink".len();
            if b.get(j).is_some_and(|c| is_ident_char(*c as char)) {
                continue; // routerLinkActive and friends
            }
            let bound = pos > 0 && b[pos - 1] == b'[' && b.get(j) == Some(&b']');
            j = skip_ws(b, j + usize::from(bound));
            if b.get(j) != Some(&b'=') {
                continue;
            }
            j = skip_ws(b, j + 1);
            let Some(&quote) = b.get(j).filter(|c| matches!(c, b'"' | b'\'')) else {
                continue;
            };
            let end = tpl[j + 1..]
                .find(quote as char)
                .map_or(tpl.len(), |e| j + 1 + e);
            let value = tpl[j + 1..end].trim();
            let parsed = bound.then(|| Parser { s: value, i: 0 }.value(0).value);
            let path = match &parsed {
                Some(v) => v.link(&|_| None),
                None => (!value.contains("{{")).then(|| value.to_string()),
            };
            links.push(Link {
                file: file.into(),
                line: first_line + line_of(tpl, pos) - 1,
                source: value.into(),
                relative: path.as_ref().is_some_and(|p| !p.starts_with('/')),
                stay: matches!(&parsed, Some(Value::Arr(items)) if items.is_empty()),
                path,
                trigger: Self::trigger(tpl, pos),
            });
        }
        links
    }

    /// `data-testid`, `id` or the text of the element whose start tag holds `pos`.
    fn trigger(tpl: &str, pos: usize) -> Option<String> {
        let start = tpl[..pos].rfind('<')?;
        let b = tpl.as_bytes();
        let mut end = pos;
        while end < b.len() && b[end] != b'>' {
            end = if matches!(b[end], b'"' | b'\'') {
                tpl[end + 1..]
                    .find(b[end] as char)
                    .map_or(b.len(), |e| end + 1 + e + 1)
            } else {
                end + 1
            };
        }
        let tag = tpl.get(start..end)?;
        for attr in ["data-testid=\"", " id=\""] {
            if let Some(i) = tag.find(attr) {
                let value = &tag[i + attr.len()..];
                return Some(value[..value.find('"').unwrap_or(value.len())].to_string());
            }
        }
        let inner = tpl.get(end + 1..)?;
        let inner = &inner[..inner.find("</").unwrap_or(inner.len())];
        let mut text = String::new();
        let mut in_tag = false;
        for c in inner.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                c if !in_tag => text.push(c),
                _ => {}
            }
        }
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then_some(text)
    }

    /// `.navigate([...])` and `.navigateByUrl('…')` in a class body; `path_of` reads
    /// the commands.
    fn in_class(
        file: &str,
        text: &str,
        range: Range<usize>,
        path_of: &dyn Fn(&Item) -> Option<String>,
    ) -> Vec<Link> {
        let mut links = Vec::new();
        for (pat, by_url) in [(".navigate(", false), (".navigateByUrl(", true)] {
            for (i, _) in text[range.clone()].match_indices(pat) {
                let pos = range.start + i;
                let mut parser = Parser {
                    s: text,
                    i: pos + pat.len(),
                };
                let first = parser.value(0);
                parser.ws();
                let relative_to = !by_url && parser.peek() == b',' && {
                    parser.i += 1;
                    parser.value(0).source(text).contains("relativeTo")
                };
                let path = path_of(&first);
                links.push(Link {
                    file: file.into(),
                    line: line_of(text, pos),
                    source: first.source(text),
                    relative: relative_to && path.as_ref().is_some_and(|p| !p.starts_with('/')),
                    stay: matches!(&first.value, Value::Arr(items) if items.is_empty()),
                    path,
                    trigger: None,
                });
            }
        }
        links
    }

    /// The target as an absolute route; `None` when relative without a base route.
    fn route(&self, base: Option<&str>) -> Option<String> {
        let path = self.path.as_deref()?;
        let path = path.split(['?', '#']).next().unwrap_or("");
        let base = if self.relative { base? } else { "" };
        Some(format!("/{}", join(base, path)))
    }
}

/// Declarations found in source text: classes, fields, constants, enums, helpers.
mod decls {
    use super::*;

    /// `class Name … {` declarations: (offset of `class`, name, heritage names, body).
    pub(super) fn class_decls(text: &str) -> Vec<(usize, String, Vec<String>, Range<usize>)> {
        let mut decls = Vec::new();
        for pos in words(text, "class") {
            let j = skip_ws(text.as_bytes(), pos + "class".len());
            let name = ident_at(text, j);
            if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
                continue;
            }
            let k = j + name.len();
            let Some(open) = text[k..].find('{').map(|i| k + i) else {
                continue;
            };
            let Some(heritage) = heritage(text[k..open].trim()) else {
                continue;
            };
            let end = block_end(text, open, b'{', b'}');
            decls.push((pos, name.to_string(), heritage, open..end));
        }
        decls
    }

    /// Names after `extends`/`implements` without type arguments; `None` when the
    /// text between name and `{` is no class head.
    fn heritage(head: &str) -> Option<Vec<String>> {
        let starts = ["<", "extends", "implements"];
        if head.contains([';', '(', ')', '=', '"', '\'', '`'])
            || !(head.is_empty() || starts.iter().any(|s| head.starts_with(s)))
        {
            return None;
        }
        let mut names = Vec::new();
        let mut depth = 0usize;
        for part in head.split_inclusive(|c: char| !is_ident_char(c)) {
            let word = part.trim_end_matches(|c: char| !is_ident_char(c));
            if depth == 0 && !word.is_empty() && !matches!(word, "extends" | "implements") {
                names.push(word.to_string());
            }
            match part.chars().last() {
                Some('<') => depth += 1,
                Some('>') => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        Some(names)
    }

    pub(super) fn declares_class(text: &str, name: &str) -> bool {
        words(text, "class")
            .into_iter()
            .any(|pos| ident_at(text, skip_ws(text.as_bytes(), pos + "class".len())) == name)
    }

    /// Whether a template embeds the element selector `<selector`.
    pub(super) fn embeds(template: &str, selector: &str) -> bool {
        if !selector.starts_with(|c: char| c.is_ascii_alphabetic())
            || !selector
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return false;
        }
        let tag = format!("<{selector}");
        template.match_indices(&tag).any(|(i, _)| {
            template[i + tag.len()..]
                .chars()
                .next()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-'))
        })
    }

    /// Start of the class member body holding `pos`; `pos` itself outside every member.
    pub(super) fn member_start(text: &str, body: &Range<usize>, pos: usize) -> usize {
        let mut start = None;
        scan_brackets(text, body.start, (b'{', b'}'), |i, c, depth| {
            if i >= pos {
                return true;
            }
            match (c, depth) {
                (b'{', 1) => start = Some(i),
                (b'}', 2) => start = None,
                _ => {}
            }
            false
        });
        start.unwrap_or(pos)
    }

    /// Offset of the initializer of the `readonly` field `name` declared in `body`.
    pub(super) fn field_init(text: &str, body: &Range<usize>, name: &str) -> Option<usize> {
        for i in words(&text[body.clone()], name) {
            let at = body.start + i;
            let mut depth = 0;
            scan_brackets(text, body.start, (b'{', b'}'), |j, c, d| {
                if j >= at {
                    return true;
                }
                depth = if c == b'{' {
                    d + 1
                } else {
                    d.saturating_sub(1)
                };
                false
            });
            let start = text[..at].rfind(['\n', ';', '{', '}']).map_or(0, |p| p + 1);
            if depth != 1
                || text[..at].trim_end().ends_with('.')
                || words(&text[start..at], "readonly").is_empty()
            {
                continue;
            }
            if let Some(init) = initializer(text, at + name.len()) {
                return Some(init);
            }
        }
        None
    }

    /// After a declared name: an optional `?`/`!`, an optional type annotation, then
    /// `=`; the offset of the initializer.
    pub(super) fn initializer(text: &str, k: usize) -> Option<usize> {
        let b = text.as_bytes();
        let mut j = skip_ws(b, k);
        if matches!(b.get(j), Some(b'?' | b'!')) {
            j = skip_ws(b, j + 1);
        }
        if b.get(j) == Some(&b':') {
            let mut depth = 0usize;
            j += 1;
            loop {
                match *b.get(j)? {
                    b'<' | b'(' | b'[' | b'{' => depth += 1,
                    b'>' | b')' | b']' | b'}' => depth = depth.checked_sub(1)?,
                    b'=' if depth == 0 => match b.get(j + 1) {
                        Some(b'>') => j += 1,
                        _ => break,
                    },
                    b';' | b'\n' | b',' if depth == 0 => return None,
                    _ => {}
                }
                j += 1;
            }
        }
        (b.get(j) == Some(&b'=') && !matches!(b.get(j + 1), Some(b'=' | b'>')))
            .then(|| skip_ws(b, j + 1))
    }

    /// Initializer offsets of `const name =` within `range`.
    pub(super) fn const_inits(text: &str, range: Range<usize>, name: &str) -> Vec<usize> {
        let Some(part) = text.get(range.clone()) else {
            return vec![];
        };
        words(part, "const")
            .into_iter()
            .filter_map(|i| {
                let j = skip_ws(text.as_bytes(), range.start + i + "const".len());
                (ident_at(text, j) == name)
                    .then(|| initializer(text, j + name.len()))
                    .flatten()
            })
            .collect()
    }

    pub(super) fn init_source(text: &str, at: usize) -> String {
        Parser { s: text, i: at }.value(0).source(text)
    }

    /// `name: Routes = [`.
    pub(super) fn typed_routes(file: &str, text: &str) -> Vec<(String, usize, Option<String>)> {
        let b = text.as_bytes();
        let mut arrays = Vec::new();
        for pos in words(text, "Routes") {
            let Some(before) = text[..pos].trim_end().strip_suffix(':') else {
                continue;
            };
            let j = skip_ws(b, pos + "Routes".len());
            if b.get(j) != Some(&b'=') || matches!(b.get(j + 1), Some(b'=' | b'>')) {
                continue;
            }
            let j = skip_ws(b, j + 1);
            if b.get(j) == Some(&b'[') {
                arrays.push((file.into(), j, trailing_ident(before.trim_end())));
            }
        }
        arrays
    }

    /// Module specifier of the `import … from '…'` that names `name`.
    pub(super) fn import_of(text: &str, name: &str) -> Option<String> {
        for pos in words(text, "import") {
            let rest = &text[pos..];
            let Some(from) = rest.find("from") else {
                continue;
            };
            let clause = &rest[..from];
            if clause.contains(';') || words(clause, name).is_empty() {
                continue;
            }
            let at = pos + from + "from".len();
            if let Value::Str(spec) = (Parser { s: text, i: at }).value(0).value {
                return Some(spec);
            }
        }
        None
    }

    /// `function name(params) { return …; }` or `const name = (params) => …`, declared
    /// once in the application, returning literals and parameters only.
    pub(super) fn helper(files: &BTreeMap<String, &str>, name: &str) -> Option<String> {
        let mut found = Vec::new();
        for text in files.values() {
            for pos in words(text, name) {
                let before = text[..pos].trim_end();
                let arrow = before.ends_with("const") && {
                    let j = skip_ws(text.as_bytes(), pos + name.len());
                    text.as_bytes().get(j) == Some(&b'=')
                };
                if before.ends_with("function") || arrow {
                    found.push((*text, pos));
                }
            }
        }
        let [(text, pos)] = found.as_slice() else {
            return None;
        };
        let (params, body) = single_return(text, *pos)?;
        commands(&params, &body)
    }

    /// Parameter names and the returned expression of the function declared at `pos`
    /// whose body is one `return` (or an arrow expression).
    fn single_return(text: &str, pos: usize) -> Option<(Vec<&str>, String)> {
        let open = pos + text[pos..].find('(')?;
        let close = block_end(text, open, b'(', b')');
        let params: Vec<&str> = text[open + 1..close.saturating_sub(1)]
            .split(',')
            .map(|p| ident_at(p.trim(), 0))
            .collect();
        let rest = &text[close..];
        let body_at = close
            + rest
                .find(['{', '='])
                .filter(|&i| rest[..i].find(';').is_none())?;
        let body = if text.as_bytes()[body_at] == b'{' {
            let end = block_end(text, body_at, b'{', b'}');
            let inner = text[body_at + 1..end.saturating_sub(1)].trim();
            let expr = inner.strip_prefix("return")?.trim();
            expr.strip_suffix(';').unwrap_or(expr).trim().to_string()
        } else {
            let after = text[body_at..].strip_prefix("=>")?;
            Parser { s: after, i: 0 }.value(0).source(after)
        };
        (!body.contains(';')).then_some((params, body))
    }

    /// A returned string or command array of literals, parameters (`:param`) and
    /// objects (dropped) as a path.
    fn commands(params: &[&str], body: &str) -> Option<String> {
        let value = Parser { s: body, i: 0 }.value(0).value;
        let param = |n: &str| params.contains(&n).then(|| PARAM.to_string());
        let part = |item: &Item| match &item.value {
            Value::Str(s) => Some(Some(s.clone())),
            Value::Raw(r) => param(r).map(Some),
            Value::Obj(_) => Some(None),
            _ => None,
        };
        match &value {
            Value::Str(s) => Some(s.clone()),
            Value::Arr(items) => {
                let (first, rest) = items.split_first()?;
                let mut out = vec![match &first.value {
                    Value::Str(s) => s.clone(),
                    _ => return None,
                }];
                for item in rest {
                    out.extend(part(item)?);
                }
                Some(out.join("/"))
            }
            _ => None,
        }
    }

    /// `export const NAME =`: initializer offsets by name.
    pub(super) fn exports_of(
        files: &BTreeMap<String, &str>,
    ) -> BTreeMap<String, Vec<(String, usize)>> {
        let mut exports: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
        for (path, text) in files {
            for pos in words(text, "export") {
                let j = skip_ws(text.as_bytes(), pos + "export".len());
                let k = skip_ws(text.as_bytes(), j + "const".len());
                let name = ident_at(text, k);
                let init = (ident_at(text, j) == "const" && !name.is_empty())
                    .then(|| initializer(text, k + name.len()))
                    .flatten();
                if let Some(at) = init {
                    let entry = exports.entry(name.to_string()).or_default();
                    entry.push((path.clone(), at));
                }
            }
        }
        exports
    }

    /// The string value of `Enum.MEMBER` when the application declares the enum and
    /// every declaration of that name gives the member the same value.
    pub(super) fn enum_value(
        files: &BTreeMap<String, &str>,
        name: &str,
        member: &str,
    ) -> Option<String> {
        let mut values = BTreeSet::new();
        for text in files.values() {
            for pos in words(text, "enum") {
                let j = skip_ws(text.as_bytes(), pos + "enum".len());
                if ident_at(text, j) != name {
                    continue;
                }
                let Some(open) = text[j..].find('{').map(|o| j + o) else {
                    continue;
                };
                let close = text[open..].find('}').map_or(text.len(), |c| open + c);
                let value = text[open + 1..close].split(',').find_map(|entry| {
                    let (key, value) = entry.split_once('=')?;
                    let value = Parser {
                        s: value.trim(),
                        i: 0,
                    }
                    .value(0);
                    match value.value {
                        Value::Str(s) if key.trim() == member => Some(s),
                        _ => None,
                    }
                });
                values.insert(value);
            }
        }
        let mut values = values.into_iter();
        match (values.next(), values.next()) {
            (Some(one), None) => one,
            _ => None,
        }
    }
}

/// HTTP calls through `HttpClient` and forwarder methods.
mod http {
    use super::*;

    /// Forwarder method names: a first parameter typed as a union of HTTP method
    /// literals, a second parameter, and a body naming both that calls `fetch(` or `http.`.
    pub(super) fn forwarders(files: &BTreeMap<String, &str>) -> BTreeSet<String> {
        (files.values())
            .flat_map(|text| {
                text.match_indices(':')
                    .filter_map(|(c, _)| forwarder_at(text, c))
            })
            .collect()
    }

    /// The method whose first parameter's type annotation starts after `colon`.
    fn forwarder_at(text: &str, colon: usize) -> Option<String> {
        let after = method_union_end(text, colon + 1)?;
        let before = text[..colon].trim_end();
        let first = trailing_ident(before)?;
        let open = before.len() - first.len();
        let paren = text[..open].trim_end().strip_suffix('(')?.len();
        let name = trailing_ident(text[..paren].trim_end())?;
        let second = ident_at(text, skip_ws(text.as_bytes(), after + 1));
        let close = block_end(text, paren, b'(', b')');
        let brace = close + text[close..].find('{')?;
        let body = &text[brace..block_end(text, brace, b'{', b'}')];
        let forwards = !second.is_empty()
            && !words(body, &first).is_empty()
            && !words(body, second).is_empty()
            && (body.contains("fetch(") || body.contains("http."));
        forwards.then_some(name)
    }

    /// After `"GET" | "POST" …` of upper-case HTTP methods followed by `,`: the
    /// offset of that comma.
    fn method_union_end(text: &str, at: usize) -> Option<usize> {
        let mut parser = Parser { s: text, i: at };
        loop {
            parser.ws();
            if !matches!(parser.peek(), b'"' | b'\'') || http_method(&parser.string()).is_none() {
                return None;
            }
            parser.ws();
            match parser.peek() {
                b'|' => parser.i += 1,
                b',' => return Some(parser.i),
                _ => return None,
            }
        }
    }

    /// An upper-case HTTP method name.
    pub(super) fn http_method(m: &str) -> Option<&str> {
        (HTTP_METHODS.contains(&m.to_ascii_lowercase().as_str()) && m == m.to_ascii_uppercase())
            .then_some(m)
    }

    /// `http.get(url…)` and friends in `range`: (offset, method, URL argument).
    pub(super) fn client_calls(text: &str, range: Range<usize>) -> Vec<(usize, String, Item)> {
        let b = text.as_bytes();
        let mut found = Vec::new();
        for method in HTTP_METHODS {
            let pat = format!(".{method}");
            for (i, _) in text[range.clone()].match_indices(&pat) {
                let pos = range.start + i;
                let receiver =
                    (text[..pos].rsplit(|c: char| !is_ident_char(c)).next()).unwrap_or("");
                let mut j = skip_ws(b, pos + pat.len());
                if b.get(j) == Some(&b'<') {
                    j = skip_ws(b, skip_generic(b, j));
                }
                if receiver.to_ascii_lowercase().contains("http") && b.get(j) == Some(&b'(') {
                    let url = Parser { s: text, i: j + 1 }.value(0);
                    found.push((pos, method.to_ascii_uppercase(), url));
                }
            }
        }
        found
    }

    /// `.name("GET", url…)` of a forwarder in `range`: (offset, method, URL argument).
    pub(super) fn forwarder_calls(
        text: &str,
        range: Range<usize>,
        forwarders: &BTreeSet<String>,
    ) -> Vec<(usize, String, Item)> {
        let mut found = Vec::new();
        for name in forwarders {
            let pat = format!(".{name}(");
            for (i, _) in text[range.clone()].match_indices(&pat) {
                let pos = range.start + i;
                let mut parser = Parser {
                    s: text,
                    i: pos + pat.len(),
                };
                let Value::Str(method) = parser.value(0).value else {
                    continue;
                };
                parser.ws();
                if http_method(&method).is_some() && parser.peek() == b',' {
                    parser.i += 1;
                    found.push((pos, method, parser.value(0)));
                }
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Call, Condition, Origin, ScreenKind, Target};

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn screens(flow: &UiFlow) -> Vec<(String, String)> {
        flow.screens
            .iter()
            .map(|s| (s.id.clone(), s.name.clone()))
            .collect()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    fn page(id: &str) -> Origin {
        Origin::Screen(id.into())
    }

    fn transitions(flow: &UiFlow) -> Vec<(Origin, Target, Option<String>)> {
        flow.transitions
            .iter()
            .map(|t| (t.from.clone(), t.target.clone(), t.trigger.clone()))
            .collect()
    }

    fn to(id: &str) -> Target {
        Target::Screen(id.into())
    }

    const ROUTES: &str = "import { Routes } from '@angular/router';
export const routes: Routes = [
  { path: '', component: HomeComponent },
  // the order page
  { path: 'orders/:id', component: OrderComponent },
  { path: 'orders/:id/edit', component: OrderEditComponent },
  { path: 'help', component: HelpComponent },
  { path: 'old', redirectTo: 'help' },
  { path: '**', component: NotFoundComponent },
];
";

    #[test]
    fn flat_routes_with_redirect_and_wildcard() {
        let dir = project(&[
            ("web/angular.json", "{}"),
            ("web/src/app/app.routes.ts", ROUTES),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            screens(&flow),
            pairs(&[
                ("web:/", "HomeComponent"),
                ("web:/**", "NotFoundComponent"),
                ("web:/help", "HelpComponent"),
                ("web:/orders/:id", "OrderComponent"),
                ("web:/orders/:id/edit", "OrderEditComponent"),
            ])
        );
        let order = flow
            .screens
            .iter()
            .find(|s| s.name == "OrderComponent")
            .unwrap();
        assert_eq!(order.route.as_deref(), Some("/orders/:id"));
        assert_eq!(order.application.as_deref(), Some("web"));
        assert_eq!(order.kind, ScreenKind::Page);
        assert_eq!(order.file, "web/src/app/app.routes.ts");
        assert_eq!(order.line, 5);
        assert_eq!(order.access, archguard_core::AccessState::Undeclared);
        assert!(flow.screens.iter().all(|s| s.source == "angular"));
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn nested_children_inherit_guards_and_router_module_is_not_counted_twice() {
        let dir = project(&[
            ("web/angular.json", "{}"),
            (
                "web/src/app/app-routing.module.ts",
                "const routes: Routes = [
  { path: 'admin', component: AdminComponent, canActivate: [authGuard], children: [
      { path: 'users', component: UsersComponent, canMatch: [adminGuard] },
      { path: '', component: AdminHomeComponent },
  ] },
];
@NgModule({ imports: [RouterModule.forRoot(routes)], exports: [RouterModule] })
export class AppRoutingModule {}
",
            ),
        ]);
        let flow = extract(dir.path());
        // AdminComponent frames a child with path '': a layout, no screen of its own.
        assert_eq!(
            screens(&flow),
            pairs(&[
                ("web:/admin", "AdminHomeComponent"),
                ("web:/admin/users", "UsersComponent"),
            ])
        );
        let auth = Condition::Custom("authGuard".into());
        let users = flow
            .screens
            .iter()
            .find(|s| s.name == "UsersComponent")
            .unwrap();
        assert_eq!(
            users.access,
            archguard_core::AccessState::Protected(Condition::All(vec![
                auth.clone(),
                Condition::Custom("adminGuard".into())
            ]))
        );
        let home = flow
            .screens
            .iter()
            .find(|s| s.name == "AdminHomeComponent")
            .unwrap();
        assert_eq!(home.access, archguard_core::AccessState::Protected(auth));
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn lazy_components_and_literal_children_imports() {
        let dir = project(&[
            (
                "shop/package.json",
                r#"{"dependencies":{"@angular/core":"^21.0.0","@angular/router":"^21.0.0"}}"#,
            ),
            (
                "shop/src/app/app.config.ts",
                "export const appConfig = { providers: [provideRouter(routes)] };
import { routes } from './app.routes';
",
            ),
            (
                "shop/src/app/app.routes.ts",
                "export const routes = [
  { path: 'about', loadComponent: () => import('./about/about.component').then(m => m.AboutComponent) },
  { path: 'cart', canActivate: [authGuard], loadChildren: () => import('./cart/cart.routes').then(m => m.CART_ROUTES) },
  { path: 'x', loadChildren: () => import(modulePath) },
];
",
            ),
            (
                "shop/src/app/cart/cart.routes.ts",
                "export const CART_ROUTES: Routes = [{ path: 'checkout', component: CheckoutComponent }];",
            ),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            screens(&flow),
            pairs(&[
                ("shop:/about", "AboutComponent"),
                ("shop:/cart/checkout", "CheckoutComponent"),
            ])
        );
        let checkout = &flow.screens[1];
        assert_eq!(
            checkout.access,
            archguard_core::AccessState::Protected(Condition::Custom("authGuard".into()))
        );
        assert_eq!(checkout.file, "shop/src/app/cart/cart.routes.ts");
        assert!(!flow.complete);
        assert_eq!(flow.diagnostics.len(), 1, "{:?}", flow.diagnostics);
        assert_eq!(flow.diagnostics[0].file, "shop/src/app/app.routes.ts");
        assert_eq!(flow.diagnostics[0].line, 4);
    }

    fn links_project() -> tempfile::TempDir {
        project(&[
            ("web/angular.json", "{}"),
            ("web/src/app/app.routes.ts", ROUTES),
            (
                "web/src/app/home.component.ts",
                "@Component({ selector: 'app-home', templateUrl: './home.component.html' })
export class HomeComponent {
  constructor(private router: Router, private orders: OrderService, private http: HttpClient) {}
  open(id: string) { this.router.navigate(['/orders', id]); }
  go() { this.router.navigateByUrl('/orders/' + this.id); }
  load() {
    this.http.get<Order[]>('/api/orders');
    this.http.post(`api/orders/${this.id}/pay?now=1`, {});
    this.http.get(this.url);
  }
}
",
            ),
            (
                "web/src/app/home.component.html",
                "<h1>Home</h1>
<a routerLink=\"/help\" routerLinkActive=\"on\" data-testid=\"help-link\">Help</a>
<a [routerLink]=\"['/orders', order.id]\">Open order</a>
<a [routerLink]=\"dynamicTarget\">Somewhere</a>
<a routerLink=\"/missing\">Missing</a>
<a routerLink=\"/orders/42/edit\">Edit 42</a>
",
            ),
            (
                "web/src/app/order.component.ts",
                "@Component({
  selector: 'app-order',
  template: `<p>Order</p>
    <a routerLink=\"edit\">Edit</a>`,
})
export class OrderComponent {
  edit() { this.router.navigate(['edit'], { relativeTo: this.route }); }
}
",
            ),
            (
                "web/src/app/header.component.ts",
                "@Component({ selector: 'app-header', template: '<nav><a routerLink=\"/help\">Help</a></nav>' })
export class HeaderComponent {}
",
            ),
            (
                "web/src/app/order.service.ts",
                "@Injectable({ providedIn: 'root' })
export class OrderService {
  remove(id: string) { return this.http.delete(`/api/orders/${id}`); }
}
",
            ),
        ])
    }

    #[test]
    fn router_links_navigate_calls_and_header_components() {
        let dir = links_project();
        let flow = extract(dir.path());
        let unresolved = |text: &str| Target::Unresolved(text.into());
        let mut expected = vec![
            // header: not a route target, so it navigates from every screen
            (
                Origin::AnyScreen {
                    application: "web".into(),
                },
                to("web:/help"),
                Some("Help".into()),
            ),
            (page("web:/"), to("web:/help"), Some("help-link".into())),
            (
                page("web:/"),
                to("web:/orders/:id"),
                Some("Open order".into()),
            ),
            (
                page("web:/"),
                unresolved("dynamicTarget"),
                Some("Somewhere".into()),
            ),
            // no route but the wildcard `**` matches, as in Angular
            (page("web:/"), to("web:/**"), Some("Missing".into())),
            (page("web:/"), to("web:/orders/:id"), None),
            (page("web:/"), unresolved("'/orders/' + this.id"), None),
            // a literal target matches the parameter route
            (
                page("web:/"),
                to("web:/orders/:id/edit"),
                Some("Edit 42".into()),
            ),
            // relative link and relativeTo navigation from /orders/:id
            (
                page("web:/orders/:id"),
                to("web:/orders/:id/edit"),
                Some("Edit".into()),
            ),
            (page("web:/orders/:id"), to("web:/orders/:id/edit"), None),
        ];
        expected.sort();
        let mut got = transitions(&flow);
        got.sort();
        assert_eq!(got, expected);
        let edit_link = flow
            .transitions
            .iter()
            .find(|t| t.trigger.as_deref() == Some("Edit"))
            .unwrap();
        assert_eq!(
            (edit_link.file.as_str(), edit_link.line),
            ("web/src/app/order.component.ts", 4)
        );
        let help = flow
            .transitions
            .iter()
            .find(|t| t.trigger.as_deref() == Some("help-link"))
            .unwrap();
        assert_eq!(
            (help.file.as_str(), help.line),
            ("web/src/app/home.component.html", 2)
        );
        assert!(flow.transitions.iter().all(|t| t.source == "angular"));
        assert!(!flow.complete);
        let lines: Vec<(&str, u32)> = flow
            .diagnostics
            .iter()
            .map(|d| (d.file.as_str(), d.line))
            .collect();
        for at in [
            ("web/src/app/home.component.html", 4),
            ("web/src/app/home.component.ts", 5),
            ("web/src/app/home.component.ts", 9),
        ] {
            assert!(lines.contains(&at), "missing {at:?} in {lines:?}");
        }
        // `/missing` lands on the wildcard route: no gap.
        assert!(!lines.contains(&("web/src/app/home.component.html", 5)));
    }

    #[test]
    fn http_calls_become_actions_of_the_calling_screen() {
        let dir = links_project();
        let flow = extract(dir.path());
        let http = |method: &str, path: &str| Call::Http {
            method: method.into(),
            path: path.into(),
        };
        let got: Vec<(Origin, Call, String, u32)> = flow
            .actions
            .iter()
            .map(|a| (a.screen.clone(), a.call.clone(), a.file.clone(), a.line))
            .collect();
        let home = "web/src/app/home.component.ts".to_string();
        assert_eq!(
            got,
            vec![
                (
                    page("web:/"),
                    http("DELETE", "/api/orders/:param"),
                    "web/src/app/order.service.ts".into(),
                    3
                ),
                (page("web:/"), http("GET", "/api/orders"), home.clone(), 7),
                (
                    page("web:/"),
                    http("POST", "/api/orders/:param/pay"),
                    home.clone(),
                    8
                ),
                (page("web:/"), Call::Unresolved("this.url".into()), home, 9),
            ]
        );
        assert!(flow.actions.iter().all(|a| a.source == "angular"));
    }

    fn access_of<'f>(flow: &'f UiFlow, id: &str) -> Option<&'f Condition> {
        let screen = flow.screens.iter().find(|s| s.id == id);
        screen
            .unwrap_or_else(|| panic!("no {id} in {:?}", screens(flow)))
            .access
            .condition()
    }

    fn calls(flow: &UiFlow) -> BTreeSet<(Origin, Call)> {
        flow.actions
            .iter()
            .map(|a| (a.screen.clone(), a.call.clone()))
            .collect()
    }

    fn http(method: &str, path: &str) -> Call {
        Call::Http {
            method: method.into(),
            path: path.into(),
        }
    }

    // Shaped like bitwarden-clients: one angular.json with several projects,
    // Nx project.json files, libraries under libs/ and a workspace package.json.
    #[test]
    fn workspace_projects_are_separate_applications_and_libraries_are_shared() {
        let dir = project(&[
            (
                "angular.json",
                r#"{"projects":{
                  "web":{"projectType":"application","root":"apps/web","sourceRoot":"apps/web/src"},
                  "desktop":{"projectType":"application","root":"apps/desktop"},
                  "components":{"projectType":"application","root":"libs/components"}}}"#,
            ),
            (
                "package.json",
                r#"{"dependencies":{"@angular/router":"^21.0.0"}}"#,
            ),
            (
                "libs/components/project.json",
                r#"{"name":"@acme/components","projectType":"library"}"#,
            ),
            (
                "apps/web/src/app/app.routes.ts",
                "export const routes: Routes = [
  { path: 'lock', component: LockComponent },
  { path: 'vault', component: VaultComponent },
];",
            ),
            (
                "apps/web/src/app/vault.component.ts",
                "@Component({ template: '<a routerLink=\"/lock\">Lock</a>' })
export class VaultComponent {}",
            ),
            (
                "apps/desktop/src/app/app.routes.ts",
                "export const routes: Routes = [{ path: 'vault', component: VaultComponent }];",
            ),
            (
                "apps/desktop/src/app/vault.component.ts",
                "@Component({ template: '<a routerLink=\"/settings\">Settings</a>' })
export class VaultComponent {}",
            ),
            (
                "libs/auth/src/lock.component.ts",
                "@Component({ template: '<a routerLink=\"/vault\">Back</a>' })
export class LockComponent {}",
            ),
            // a library component no application routes to: no screen, no link
            (
                "libs/components/src/menu.component.ts",
                "@Component({ template: '<a routerLink=\"/nowhere\">X</a>' })
export class MenuComponent {}",
            ),
            (
                "libs/components/src/stories.ts",
                "export const storyRoutes: Routes = [{ path: 'story', component: MenuComponent }];",
            ),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            screens(&flow),
            pairs(&[
                ("desktop:/vault", "VaultComponent"),
                ("web:/lock", "LockComponent"),
                ("web:/vault", "VaultComponent"),
            ])
        );
        let mut got = transitions(&flow);
        got.sort();
        let mut want = vec![
            (page("web:/lock"), to("web:/vault"), Some("Back".into())),
            (page("web:/vault"), to("web:/lock"), Some("Lock".into())),
            (
                page("desktop:/vault"),
                Target::Unresolved("/settings".into()),
                Some("Settings".into()),
            ),
        ];
        want.sort();
        assert_eq!(got, want);
    }

    // Shaped like jhipster-sample-app (`navbar` outlet) and Squidex (an area
    // component whose children start with `path: ''`).
    #[test]
    fn layouts_and_named_outlets_are_no_screens() {
        let dir = project(&[
            ("web/angular.json", "{}"),
            (
                "web/src/app/app.routes.ts",
                "const routes: Routes = [
  { path: '', loadComponent: () => import('./home/home') },
  { path: '', loadComponent: () => import('./layouts/navbar/navbar'), outlet: 'navbar' },
  { path: 'app', component: AreaComponent, children: [
      { path: '', component: DashboardComponent },
      { path: 'x', component: XComponent },
  ] },
  { path: 'teams', component: TeamsComponent, children: [{ path: ':id', component: TeamComponent }] },
];
export default routes;",
            ),
            (
                "web/src/app/home/home.ts",
                "@Component({ template: '<p>home</p>' })\nexport default class Home {}",
            ),
            (
                "web/src/app/layouts/navbar/navbar.ts",
                "@Component({ template: '<a routerLink=\"/app/x\">X</a>' })\nexport default class Navbar {}",
            ),
            (
                "web/src/app/area.component.ts",
                "@Component({ template: '<a routerLink=\"/teams\">Teams</a>' })\nexport class AreaComponent {}",
            ),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            screens(&flow),
            pairs(&[
                ("web:/", "Home"),
                ("web:/app", "DashboardComponent"),
                ("web:/app/x", "XComponent"),
                ("web:/teams", "TeamsComponent"),
                ("web:/teams/:id", "TeamComponent"),
            ])
        );
        let any = Origin::AnyScreen {
            application: "web".into(),
        };
        let mut got = transitions(&flow);
        got.sort();
        let mut want = vec![
            (any.clone(), to("web:/app/x"), Some("X".into())),
            (any, to("web:/teams"), Some("Teams".into())),
        ];
        want.sort();
        assert_eq!(got, want);
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    // Shaped like jhipster-sample-app: `data.authorities` with enum members.
    fn web(routes: &str, extra: &[(&str, &str)]) -> UiFlow {
        let mut files = vec![
            ("web/angular.json", "{}"),
            ("web/src/app/app.routes.ts", routes),
        ];
        files.extend_from_slice(extra);
        extract(project(&files).path())
    }

    fn gap_lines(flow: &UiFlow) -> Vec<u32> {
        (flow.diagnostics.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .map(|d| d.line)
            .collect()
    }

    fn custom(n: &str) -> Condition {
        Condition::Custom(n.into())
    }

    fn state_of<'f>(flow: &'f UiFlow, id: &str) -> &'f archguard_core::AccessState {
        &flow.screens.iter().find(|s| s.id == id).unwrap().access
    }

    // A1, A2, A3, A6, A8: guard behaviour that already holds on the base.
    #[test]
    fn plain_guards_chains_and_non_access_keys() {
        let flow = web(
            "const r: Routes = [
  { path: 'a', component: A },
  { path: 'b', component: B, canActivate: [authGuard] },
  { path: 'c', component: C, canActivate: [g1, g2('x')] },
  { path: 'p', canActivateChild: [g], component: P, children: [{ path: 'c', component: C2 }] },
  { path: 'd', component: D, canDeactivate: [dirty] },
];",
            &[],
        );
        let undeclared = &archguard_core::AccessState::Undeclared;
        assert_eq!(state_of(&flow, "web:/a"), undeclared);
        assert_eq!(access_of(&flow, "web:/b"), Some(&custom("authGuard")));
        assert_eq!(
            access_of(&flow, "web:/c"),
            Some(&Condition::All(vec![custom("g1"), custom("g2")]))
        );
        assert_eq!(state_of(&flow, "web:/p"), undeclared);
        assert_eq!(access_of(&flow, "web:/p/c"), Some(&custom("g")));
        assert_eq!(state_of(&flow, "web:/d"), undeclared);
    }

    #[test]
    fn map_to_can_activate_unwraps_its_guards() {
        let flow = web(
            "const r: Routes = [
  { path: 'a', component: A, canActivate: mapToCanActivate([AuthGuard]) },
  { path: 'b', component: B, canActivate: mapToCanActivate([G1, g2('x')]) },
];",
            &[],
        );
        assert_eq!(access_of(&flow, "web:/a"), Some(&custom("AuthGuard")));
        assert_eq!(
            access_of(&flow, "web:/b"),
            Some(&Condition::All(vec![custom("G1"), custom("g2")]))
        );
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn inline_guards_and_guards_given_as_a_variable_are_unknown() {
        let flow = web(
            "const r: Routes = [
  { path: 'a', component: A, canActivate: [() => inject(S).ok()] },
  { path: 'b', component: B, canActivate: guards },
  { path: 'c', component: C, canActivate: [openGuard], canMatch: guards },
  { path: 'd', component: D, canActivate: makeGuards() },
];",
            &[(
                ".archguard/archguard.toml",
                "[views.access]\nopenGuard = \"open\"\n",
            )],
        );
        for id in ["a", "b", "c", "d"] {
            assert!(
                matches!(
                    access_of(&flow, &format!("web:/{id}")),
                    Some(Condition::Unknown(_))
                ),
                "{id}: {:?}",
                flow.screens
            );
        }
        assert_eq!(
            access_of(&flow, "web:/b"),
            Some(&Condition::Unknown("guards".into()))
        );
        assert_eq!(gap_lines(&flow), vec![2, 3, 4, 5]);
        assert!(!flow.complete);
    }

    #[test]
    fn can_load_guards_the_lazy_routes() {
        let flow = web(
            "const r: Routes = [
  { path: 'l', canLoad: [g], loadChildren: () => import('./l.routes') },
  { path: 'x', component: X, canLoad: [h] },
];",
            &[(
                "web/src/app/l.routes.ts",
                "const r: Routes = [{ path: 'k', component: K }];\nexport default r;",
            )],
        );
        assert_eq!(access_of(&flow, "web:/l/k"), Some(&custom("g")));
        // No `loadChildren`: `canLoad` does nothing.
        assert_eq!(
            state_of(&flow, "web:/x"),
            &archguard_core::AccessState::Undeclared
        );
        assert!(
            flow.diagnostics
                .iter()
                .any(|d| d.severity == DiagnosticSeverity::Note && d.message.contains("canLoad")),
            "{:?}",
            flow.diagnostics
        );
    }

    #[test]
    fn routes_with_the_same_path_are_distinct_screens() {
        let flow = web(
            "const r: Routes = [
  { path: 'd', component: AdminDash, canMatch: [isAdmin] },
  { path: 'd', component: UserDash },
];",
            &[],
        );
        assert_eq!(flow.screens.len(), 2);
        assert_ne!(flow.screens[0].id, flow.screens[1].id);
        assert_eq!(
            access_of(&flow, &flow.screens[0].id),
            Some(&custom("isAdmin"))
        );
        assert_eq!(
            state_of(&flow, &flow.screens[1].id),
            &archguard_core::AccessState::Undeclared
        );
    }

    const READING_GUARD: &str = "export const userRouteAccessService: CanActivateFn = (route, state) => {
  const authorities = route.data['authorities'];
  return !authorities?.length || hasAny(authorities);
};
export class Plain implements CanActivate { canActivate(route) { return true; } }
export class Keyed implements CanActivate { canActivate(route) { return route.data?.roles.includes('x'); } }
";

    #[test]
    fn data_keys_count_only_with_a_guard_that_reads_them() {
        let flow = web(
            "const r: Routes = [
  { path: 'a', component: A, data: { authorities: ['ROLE_ADMIN'] } },
  { path: 'b', component: B, canActivate: [userRouteAccessService], data: { authorities: [Authority.ADMIN] } },
  { path: 'c', component: C, canActivate: [userRouteAccessService] },
  { path: 'd', component: D, canActivate: [userRouteAccessService], data: { authorities: [Roles.X] } },
  { path: 'e', component: E, canActivate: [Plain], data: { roles: ['x'] } },
  { path: 'f', component: F, canActivate: [Keyed], data: { roles: ['x', 'y'], authorities: ['no'] } },
  { path: '', canActivate: [Keyed], data: { roles: ['z'] }, children: [{ path: 'g', component: G }] },
];",
            &[
                ("web/src/app/auth.ts", READING_GUARD),
                (
                    "web/src/app/constants.ts",
                    "export enum Authority { ADMIN = 'ROLE_ADMIN' }",
                ),
            ],
        );
        // A10: data without a reading guard.
        assert_eq!(
            state_of(&flow, "web:/a"),
            &archguard_core::AccessState::Undeclared
        );
        assert!(
            flow.diagnostics
                .iter()
                .any(|d| d.severity == DiagnosticSeverity::Note
                    && d.line == 2
                    && d.message.contains("authorities")),
            "{:?}",
            flow.diagnostics
        );
        // A11, A12.
        assert_eq!(
            access_of(&flow, "web:/b"),
            Some(&Condition::All(vec![
                custom("userRouteAccessService"),
                Condition::Role("ROLE_ADMIN".into()),
            ]))
        );
        assert_eq!(
            access_of(&flow, "web:/c"),
            Some(&custom("userRouteAccessService"))
        );
        // A13: enum value not found.
        assert_eq!(
            access_of(&flow, "web:/d"),
            Some(&Condition::All(vec![
                custom("userRouteAccessService"),
                Condition::Unknown("Roles.X".into()),
            ]))
        );
        assert_eq!(gap_lines(&flow), vec![5]);
        // The guard reads `roles`, not `authorities`; `Plain` reads nothing.
        assert_eq!(access_of(&flow, "web:/e"), Some(&custom("Plain")));
        assert_eq!(
            access_of(&flow, "web:/f"),
            Some(&Condition::All(vec![
                custom("Keyed"),
                Condition::Any(vec![
                    Condition::Role("x".into()),
                    Condition::Role("y".into())
                ]),
            ]))
        );
        // A16: a componentless parent's guard and data reach its children.
        assert_eq!(
            access_of(&flow, "web:/g"),
            Some(&Condition::All(vec![
                custom("Keyed"),
                Condition::Role("z".into())
            ]))
        );
    }

    #[test]
    fn data_read_by_destructuring_typed_literals_and_spreads() {
        let flow = web(
            "const r: Routes = [
  { path: 'a', component: A, canActivate: [Jh], data: { authorities: ['ROLE_X'], defaultSort: `id,${ASC}` } },
  { path: 'b', component: B, canActivate: [Jh], data: { authorities: ['ROLE_Y'] } satisfies RouteData },
  { path: 'c', component: C, canActivate: [Jh], data: { ...BASE, authorities: ['ROLE_Z'] } },
  { path: 'd', component: D, canActivate: [Plain], data: { ...BASE } },
];",
            &[(
                "web/src/app/jh.ts",
                "export const Jh: CanActivateFn = (next) => { const { authorities } = next.data; return ok(authorities); };\n\
                 export class Plain implements CanActivate { canActivate() { return true; } }",
            )],
        );
        let role = |r: &str| Condition::All(vec![custom("Jh"), Condition::Role(r.into())]);
        assert_eq!(access_of(&flow, "web:/a"), Some(&role("ROLE_X")));
        assert_eq!(access_of(&flow, "web:/b"), Some(&role("ROLE_Y")));
        // A spread may hold the key the guard reads: unknown, never dropped.
        assert_eq!(
            access_of(&flow, "web:/c"),
            Some(&Condition::All(vec![
                custom("Jh"),
                Condition::Unknown("...BASE".into()),
                Condition::Role("ROLE_Z".into()),
            ]))
        );
        assert_eq!(access_of(&flow, "web:/d"), Some(&custom("Plain")));
        assert_eq!(gap_lines(&flow), vec![4]);
    }

    // Shaped like Squidex: functional guards named in `[views.access]`.
    #[test]
    fn views_access_gives_listed_guards_their_meaning() {
        let routes = "export const APP_ROUTES: Routes = [
  { path: '', component: HomePageComponent, canActivate: [mustBeNotAuthenticatedGuard] },
  { path: 'app', component: AppsComponent, canActivate: [mustBeAuthenticatedGuard, loadAppsGuard] },
  { path: 'admin', component: AdminComponent, canActivate: [hasRole('a', 'b')] },
  { path: 'x', component: XComponent, canActivate: [hasRole(someRole)] },
  { path: 'closed', component: ClosedComponent, canMatch: [closedGuard] },
  { path: 'perm', component: PermComponent, canActivate: [permissionGuard('orders.read')] },
];";
        let config = "[views.access]
mustBeAuthenticatedGuard = \"authenticated\"
mustBeNotAuthenticatedGuard = \"open\"
hasRole = \"role:$arg\"
permissionGuard = \"policy:$arg\"
closedGuard = \"never\"
";
        let dir = project(&[
            ("frontend/angular.json", "{}"),
            ("frontend/src/app/app.routes.ts", routes),
            (".archguard/archguard.toml", config),
        ]);
        let flow = extract(dir.path());
        assert_eq!(access_of(&flow, "frontend:/"), None);
        assert_eq!(
            access_of(&flow, "frontend:/app"),
            Some(&Condition::All(vec![
                Condition::Authenticated,
                Condition::Custom("loadAppsGuard".into()),
            ]))
        );
        assert_eq!(
            access_of(&flow, "frontend:/admin"),
            Some(&Condition::Any(vec![
                Condition::Role("a".into()),
                Condition::Role("b".into()),
            ]))
        );
        assert_eq!(
            access_of(&flow, "frontend:/x"),
            Some(&Condition::Unknown("hasRole(someRole)".into()))
        );
        assert_eq!(
            access_of(&flow, "frontend:/closed"),
            Some(&Condition::Never)
        );
        assert_eq!(
            access_of(&flow, "frontend:/perm"),
            Some(&Condition::Policy("orders.read".into()))
        );
        assert_eq!(flow.diagnostics.len(), 1, "{:?}", flow.diagnostics);
        assert_eq!(flow.diagnostics[0].line, 5);

        // An invalid configuration: guards stay custom, a note says why.
        let bad = project(&[
            ("frontend/angular.json", "{}"),
            ("frontend/src/app/app.routes.ts", routes),
            (
                ".archguard/archguard.toml",
                "[views.access]\nx = \"maybe\"\n",
            ),
        ]);
        let flow = extract(bad.path());
        assert_eq!(
            access_of(&flow, "frontend:/"),
            Some(&Condition::Custom("mustBeNotAuthenticatedGuard".into()))
        );
        assert!(
            flow.diagnostics
                .iter()
                .any(|d| d.severity == DiagnosticSeverity::Note
                    && d.message.contains("views.access")),
            "{:?}",
            flow.diagnostics
        );
    }

    #[test]
    fn a_fallback_meaning_applies_to_guards_without_arguments() {
        let routes = "export const APP_ROUTES: Routes = [
  { path: 'a', component: AComponent, canActivate: [permissionGuard] },
  { path: 'b', component: BComponent, canActivate: [permissionGuard()] },
  { path: 'c', component: CComponent, canActivate: [permissionGuard('orders.read')] },
  { path: 'd', component: DComponent, canActivate: [publicGuard] },
];";
        let config = "[views.access]
permissionGuard = \"policy:$arg|authenticated\"
publicGuard = \"role:$arg|open\"
";
        let dir = project(&[
            ("frontend/angular.json", "{}"),
            ("frontend/src/app/app.routes.ts", routes),
            (".archguard/archguard.toml", config),
        ]);
        let flow = extract(dir.path());
        let signed_in = Some(&Condition::Authenticated);
        assert_eq!(access_of(&flow, "frontend:/a"), signed_in);
        assert_eq!(access_of(&flow, "frontend:/b"), signed_in);
        assert_eq!(
            access_of(&flow, "frontend:/c"),
            Some(&Condition::Policy("orders.read".into()))
        );
        assert_eq!(access_of(&flow, "frontend:/d"), None);
        assert!(flow.diagnostics.is_empty(), "{:?}", flow.diagnostics);
    }

    // Shaped like jhipster-sample-app (`@Service`, a base class holding
    // `resourceUrl`, a build-time `define`) and Squidex (`const url = helper(…)`).
    #[test]
    fn service_urls_resolve_through_fields_constants_and_defines() {
        let dir = project(&[
            (
                "angular.json",
                r#"{"projects":{"sample":{"projectType":"application","root":"","sourceRoot":"src/main/webapp",
                   "architect":{"build":{"options":{"define":{"SERVER_API_URL":"''"}}}}}}}"#,
            ),
            (
                "src/main/webapp/app/config/endpoint.constants.ts",
                "export const serverApiUrl = SERVER_API_URL;\n",
            ),
            (
                "src/main/webapp/app/app.routes.ts",
                "const routes: Routes = [{ path: 'label', component: Label }];\nexport default routes;",
            ),
            (
                "src/main/webapp/app/label/label.ts",
                "@Component({ selector: 'jhi-label', template: '<p></p>' })
export class Label {
  protected readonly labelService = inject(LabelService);
}",
            ),
            (
                "src/main/webapp/app/label/label.service.ts",
                "import { serverApiUrl } from 'app/config';
@Service()
export class LabelsService {
  protected readonly resourceUrl = `${serverApiUrl}api/labels`;
}

@Service()
export class LabelService extends LabelsService {
  protected readonly http = inject(HttpClient);
  create(label: NewLabel): Observable<ILabel> {
    return this.http.post<ILabel>(this.resourceUrl, label);
  }
  find(id: number): Observable<ILabel> {
    return this.http.get<ILabel>(`${this.resourceUrl}/${encodeURIComponent(id)}`);
  }
  users(take: number) {
    const url = this.apiUrl.buildUrl(`api/users?take=${take}`);
    return this.http.get<any>(url);
  }
  byUrl(url: string) {
    return this.http.get(url);
  }
}",
            ),
        ]);
        let flow = extract(dir.path());
        let label = page("sample:/label");
        assert_eq!(
            calls(&flow),
            BTreeSet::from([
                (label.clone(), http("POST", "/api/labels")),
                (label.clone(), http("GET", "/api/labels/:param")),
                (
                    label.clone(),
                    Call::Unresolved("this.apiUrl.buildUrl(`api/users?take=${take}`)".into())
                ),
                (label, Call::Unresolved("url".into())),
            ])
        );
    }

    // Shaped like bitwarden-clients: `ApiService.send(method, path, …)` forwards
    // to `fetch`; undecorated services implement abstract classes that
    // components inject.
    #[test]
    fn forwarders_with_an_http_method_union_count_as_http_calls() {
        let dir = project(&[
            ("web/angular.json", "{}"),
            (
                "web/src/app/app.routes.ts",
                "export const routes: Routes = [{ path: 'org', component: OrgComponent }];",
            ),
            (
                "web/src/app/org.component.ts",
                "@Component({ template: '<p></p>' })
export class OrgComponent implements OnInit {
  constructor(private organizationApiService: OrganizationApiServiceAbstraction, private ipc: IpcService) {}
  ngOnInit() { this.ipc.send(message); }
}",
            ),
            (
                "web/src/app/services/api.service.ts",
                "export class ApiService {
  async send(
    method: \"GET\" | \"POST\" | \"DELETE\",
    path: string,
    body: any,
  ): Promise<any> {
    const requestUrl = this.apiUrl + path;
    return this.fetch(new Request(requestUrl, { method, body }));
  }
}",
            ),
            (
                "web/src/app/services/ipc.service.ts",
                "export class IpcService {
  send(message: OutgoingMessage) { this.port.send(message); }
  load() { return this.http.get('/ipc'); }
}
// implements an interface the component implements too, but is not named by it
export class OtherService implements OnInit {
  ngOnInit() { return this.http.get('/other'); }
}",
            ),
            (
                "web/src/app/services/organization-api.service.abstraction.ts",
                "export abstract class OrganizationApiServiceAbstraction {\n  abstract get(id: string): Promise<any>;\n}",
            ),
            (
                "web/src/app/services/organization-api.service.ts",
                "export class OrganizationApiService implements OrganizationApiServiceAbstraction {
  constructor(private apiService: ApiService) {}
  async get(id: string) {
    return await this.apiService.send(\"GET\", \"/organizations/\" + id, null);
  }
  async keys(id: string) {
    return await this.apiService.send(\"DELETE\", `/organizations/${id}/keys`, null);
  }
  async any(path: string) {
    return await this.apiService.send(\"POST\", path, null);
  }
}",
            ),
        ]);
        let flow = extract(dir.path());
        let org = page("web:/org");
        assert_eq!(
            calls(&flow),
            BTreeSet::from([
                // the IPC service is named by the component: its own HTTP call counts
                (org.clone(), http("GET", "/ipc")),
                (org.clone(), http("GET", "/organizations/:param")),
                (org.clone(), http("DELETE", "/organizations/:param/keys")),
                (org, Call::Unresolved("path".into())),
            ])
        );
    }

    // Shaped like Squidex/Bitwarden: `[routerLink]="[]"` with query parameters.
    #[test]
    fn empty_commands_stay_on_the_current_screen() {
        let dir = project(&[
            ("web/angular.json", "{}"),
            (
                "web/src/app/app.routes.ts",
                "export const routes: Routes = [{ path: 'list', component: ListComponent }];",
            ),
            (
                "web/src/app/list.component.ts",
                "@Component({ template: '<a [routerLink]=\"[]\" [queryParams]=\"{ page: 2 }\">Next</a>' })
export class ListComponent {
  filter() { this.router.navigate([], { queryParams: { q: 1 } }); }
}",
            ),
            // shown on every screen: staying adds no transition
            (
                "web/src/app/header.component.ts",
                "@Component({ template: '<a [routerLink]=\"[]\" fragment=\"top\">Top</a>' })
export class HeaderComponent {}",
            ),
        ]);
        let flow = extract(dir.path());
        let mut got = transitions(&flow);
        got.sort();
        assert_eq!(
            got,
            vec![
                (page("web:/list"), to("web:/list"), None),
                (page("web:/list"), to("web:/list"), Some("Next".into())),
            ]
        );
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    // Shaped like Bitwarden (`redirectGuard({...})`) and Squidex
    // (`schemaMustNotBeSingletonGuard(false)`): unlisted factories are custom.
    #[test]
    fn unlisted_guard_factories_are_custom_conditions() {
        let dir = project(&[
            ("web/angular.json", "{}"),
            (
                "web/src/app/app.routes.ts",
                "export const routes: Routes = [
  { path: 's', component: SComponent, canActivate: [redirectGuard({ to: 'x' }), schemaMustNotBeSingletonGuard(false)] },
];",
            ),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            access_of(&flow, "web:/s"),
            Some(&Condition::All(vec![
                Condition::Custom("redirectGuard".into()),
                Condition::Custom("schemaMustNotBeSingletonGuard".into()),
            ]))
        );
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    // Shaped like Squidex `settings-menu.component.html`: relative links in a
    // component embedded by a layout and by a screen.
    #[test]
    fn relative_links_of_embedded_components_take_the_base_of_each_host() {
        let dir = project(&[
            ("web/angular.json", "{}"),
            (
                "web/src/app/app.routes.ts",
                "export const routes: Routes = [
  { path: 'settings', children: [
      { path: '', component: SettingsAreaComponent, children: [
          { path: 'clients', component: ClientsComponent },
      ] },
  ] },
  { path: 'other', component: OtherComponent },
  { path: 'other/clients', component: OtherClientsComponent },
  { path: 'team', component: TeamAreaComponent },
  { path: 'team/members', component: MembersComponent },
];",
            ),
            (
                "web/src/app/settings-area.component.ts",
                "@Component({ imports: [SettingsMenuComponent], template: '<sqx-settings-menu></sqx-settings-menu><router-outlet />' })
export class SettingsAreaComponent {}",
            ),
            (
                "web/src/app/other.component.ts",
                "@Component({ imports: [SettingsMenuComponent], template: '<div><sqx-settings-menu /></div>' })
export class OtherComponent {}",
            ),
            // the same selector a second time: the host's imports tell them apart
            (
                "web/src/app/team/team-area.component.ts",
                "@Component({ imports: [TeamSettingsMenuComponent], template: '<sqx-settings-menu />' })
export class TeamAreaComponent {}",
            ),
            (
                "web/src/app/team/settings-menu.component.ts",
                "@Component({ selector: 'sqx-settings-menu', template: '<a routerLink=\"members\">Members</a>' })
export class TeamSettingsMenuComponent {}",
            ),
            (
                "web/src/app/settings-menu.component.ts",
                "@Component({ selector: 'sqx-settings-menu', template: '<a routerLink=\"clients\">Clients</a>' })
export class SettingsMenuComponent {}",
            ),
            // embedded nowhere: the gap stays
            (
                "web/src/app/lonely.component.ts",
                "@Component({ selector: 'sqx-lonely', template: '<a routerLink=\"x\">X</a>' })
export class LonelyComponent {}",
            ),
        ]);
        let flow = extract(dir.path());
        let any = Origin::AnyScreen {
            application: "web".into(),
        };
        let clients = Some("Clients".to_string());
        let got: BTreeSet<_> = transitions(&flow).into_iter().collect();
        assert!(
            got.contains(&(any.clone(), to("web:/settings/clients"), clients.clone())),
            "{got:?}"
        );
        assert!(
            got.contains(&(page("web:/other"), to("web:/other/clients"), clients)),
            "{got:?}"
        );
        assert!(
            got.contains(&(any, Target::Unresolved("x".into()), Some("X".into()))),
            "{got:?}"
        );
        assert!(
            got.contains(&(
                page("web:/team"),
                to("web:/team/members"),
                Some("Members".into())
            )),
            "{got:?}"
        );
        assert_eq!(got.len(), 4, "{got:?}");
    }

    // Shaped like Bitwarden: route paths and commands built from string constants,
    // `loadChildren: () => VaultModule`, and one-level command helpers.
    #[test]
    fn constants_module_classes_and_helpers_in_routes_and_links() {
        let dir = project(&[
            ("web/angular.json", "{}"),
            (
                "web/src/app/route-constants.ts",
                "export const MY_ITEMS_ROUTE = \"my-items\";\n",
            ),
            (
                "web/src/app/app.routes.ts",
                "import { MY_ITEMS_ROUTE } from './route-constants'; import { AuthWebRoute } from './auth-web-route';
import { VaultModule } from './vault';
const ORG = 'organizations';
export const routes: Routes = [
  { path: `${ORG}/:id/${MY_ITEMS_ROUTE}`, component: MyItemsComponent },
  { path: ORG, component: OrgsComponent },
  { path: 'vault', loadChildren: () => VaultModule },
  { path: `x/${unknownName}`, component: XComponent },
  { path: AuthRoute.Login, component: LoginComponent },
  { path: AuthWebRoute.Hint, component: HintComponent },
];",
            ),
            (
                "web/src/app/auth-web-route.ts",
                "const SEGMENT = Object.freeze({ Account: \"account\" } as const);
export const AuthWebRoute = Object.freeze({
  Hint: `${SEGMENT.Account}/hint`,
} as const);
",
            ),
            (
                "web/src/app/auth-route.ts",
                "export enum AuthRoute {\n  Login = \"login\",\n  Sso = \"sso\",\n}\n",
            ),
            // a barrel: the module class lives behind it
            (
                "web/src/app/vault/index.ts",
                "export * from './vault.module';\n",
            ),
            (
                "web/src/app/vault/vault.module.ts",
                "import { VaultRoutingModule } from './vault-routing.module';
@NgModule({ imports: [VaultRoutingModule, SharedModule] })
export class VaultModule {}",
            ),
            (
                "web/src/app/vault/vault-routing.module.ts",
                "const routes: Routes = [{ path: ':itemId', component: ItemComponent }];
@NgModule({ imports: [RouterModule.forChild(routes)] })
export class VaultRoutingModule {}",
            ),
            (
                "web/src/app/orgs.component.ts",
                "const MINE = 'organizations';
@Component({ template: '<p></p>' })
export class OrgsComponent {
  open(id: string) { this.router.navigate(['/' + MINE]); this.router.navigate([`/${MINE}`, id, 'my-items']); }
  item(id: string) { this.router.navigate(itemCommands(id)); this.router.navigate([AuthRoute.Login]); }
  back() { this.router.navigate(backCommands(this.route)); }
}",
            ),
            (
                "web/src/app/commands.ts",
                "export function itemCommands(id: string): string[] {
  return ['/vault', id];
}
export function backCommands(route: ActivatedRoute): string[] {
  const id = route.snapshot.params.id;
  return ['/vault', id];
}",
            ),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            screens(&flow),
            pairs(&[
                ("web:/account/hint", "HintComponent"),
                ("web:/login", "LoginComponent"),
                ("web:/organizations", "OrgsComponent"),
                ("web:/organizations/:id/my-items", "MyItemsComponent"),
                ("web:/vault/:itemId", "ItemComponent"),
            ])
        );
        let mut got = transitions(&flow);
        got.sort();
        let orgs = page("web:/organizations");
        let mut want = vec![
            (
                orgs.clone(),
                Target::Unresolved("['/' + MINE]".into()),
                None,
            ),
            (orgs.clone(), to("web:/organizations/:id/my-items"), None),
            (orgs.clone(), to("web:/vault/:itemId"), None),
            // a relative command without `relativeTo` resolves from the root
            (orgs.clone(), to("web:/login"), None),
            (
                orgs,
                Target::Unresolved("backCommands(this.route)".into()),
                None,
            ),
        ];
        want.sort();
        assert_eq!(got, want);
        let gaps: Vec<(&str, u32)> = flow
            .diagnostics
            .iter()
            .map(|d| (d.file.as_str(), d.line))
            .collect();
        assert_eq!(
            gaps,
            vec![
                ("web/src/app/app.routes.ts", 8),
                ("web/src/app/orgs.component.ts", 4),
                ("web/src/app/orgs.component.ts", 6),
            ]
        );
    }

    // Shaped like Squidex: `UsersService` exists twice; the relative import decides.
    #[test]
    fn services_of_the_same_name_are_told_apart_by_the_relative_import() {
        let service = |path: &str| {
            format!(
                "@Service()\nexport class UsersService {{\n  get() {{ return this.http.get('{path}'); }}\n}}\n"
            )
        };
        let admin = service("/api/user-management");
        let shared = service("/api/users");
        let dir = project(&[
            ("web/angular.json", "{}"),
            (
                "web/src/app/app.routes.ts",
                "export const routes: Routes = [{ path: 'a', component: AComponent }, { path: 'b', component: BComponent }];",
            ),
            (
                "web/src/app/a.component.ts",
                "import { UsersService } from './admin/users.service';
@Component({ template: '' })
export class AComponent { users = inject(UsersService); }",
            ),
            (
                "web/src/app/b.component.ts",
                "import { UsersService } from '@app/shared';
@Component({ template: '' })
export class BComponent { users = inject(UsersService); }",
            ),
            ("web/src/app/admin/users.service.ts", &admin),
            ("web/src/app/shared/users.service.ts", &shared),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            calls(&flow),
            BTreeSet::from([
                (page("web:/a"), http("GET", "/api/user-management")),
                // an alias import is not followed: both stay, with a note
                (page("web:/b"), http("GET", "/api/user-management")),
                (page("web:/b"), http("GET", "/api/users")),
            ])
        );
        assert!(
            flow.diagnostics
                .iter()
                .any(|d| d.severity == DiagnosticSeverity::Note
                    && d.file == "web/src/app/b.component.ts"),
            "{:?}",
            flow.diagnostics
        );
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn node_modules_build_output_and_spec_files_are_skipped() {
        let nm = "export const r: Routes = [{ path: 'nm', component: NmComponent }];";
        let dir = project(&[
            ("web/angular.json", "{}"),
            (
                "web/src/app/app.routes.ts",
                "export const routes: Routes = [{ path: 'a', component: AComponent }];",
            ),
            ("web/node_modules/lib/routes.ts", nm),
            ("web/dist/routes.ts", nm),
            ("web/.angular/cache/routes.ts", nm),
            ("web/src/app/app.component.spec.ts", nm),
            // Storybook routes are no application routes
            ("web/src/app/menu.component.stories.ts", nm),
            // not an Angular application: no angular.json, no @angular/router
            ("other/package.json", r#"{"dependencies":{"react":"19"}}"#),
            ("other/src/routes.ts", nm),
        ]);
        let flow = extract(dir.path());
        assert_eq!(screens(&flow), pairs(&[("web:/a", "AComponent")]));
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }
}
