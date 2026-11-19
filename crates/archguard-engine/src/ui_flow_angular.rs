//! Angular extractor for ui-flow/v1 (ADR 0021, phase 2). A text-level scanner
//! without a TypeScript parser; compiler-backed resolution of what it cannot read
//! is ADR 0021 phase 7.
//!
//! Applications are directories with `angular.json` or with a `package.json` that
//! names `@angular/router`; a file belongs to the deepest one holding it. Skipped:
//! `node_modules`, `dist`, `.angular`, the engine's other excluded directories,
//! test files (`*.spec.ts`) and `*.d.ts`.
//!
//! Recognized:
//! - Route arrays typed `name: Routes = [`, and literal arrays or variables passed to
//!   `RouterModule.forRoot/forChild` or `provideRouter` (variable declared in the same
//!   file or imported by a relative path).
//! - Per route object: `path`, `component`, `loadComponent`
//!   (`import('…').then(m => m.X)` or a default-exported class), `children` (inline or a
//!   variable), `loadChildren` with a literal relative import (prefixed by the parent
//!   path), `redirectTo` (no screen), `path: '**'` (route `/**`). `canActivate` and
//!   `canMatch` become `custom(guard)`, several are `all`, and child routes inherit
//!   them (plus `canActivateChild`).
//! - In the component's template (`templateUrl` or inline `template`):
//!   `routerLink="…"` and `[routerLink]="'…'"` / `"['…', expr]"`; in its class:
//!   `.navigate([...])` (relative only with `relativeTo`) and `.navigateByUrl('…')`.
//!   Non-literal array parts after the first become a `:param` segment; targets are
//!   matched to the application's routes, a literal segment also to a parameter
//!   (`/orders/42` -> `orders/:id`); an exact literal route wins, and a tie of
//!   equally good routes stays unresolved. The trigger is the element's
//!   `data-testid`, `id` or text.
//! - `http.get/post/put/patch/delete/head/options` with a literal or template-literal
//!   URL (holes become `:param`) in a component, or in an `@Injectable` class the
//!   component names, as `Call::Http` on the component's screens; a computed URL
//!   becomes `Call::Unresolved`.
//! - A component that is no route target (header, navigation) acts on every screen:
//!   its transitions and actions start at `any_screen` of the application.
//!
//! Not recognized; each case is a diagnostic and makes the flow incomplete:
//! non-literal paths, targets and URLs, route spreads and helper functions,
//! module-path aliases (`@app/…`), guards that are not plain names (`unknown`),
//! relative links in components that are no route target. Not followed: lazy
//! NgModules whose routes live in a routing module the loaded file does not declare,
//! services injected by services, and `angular.json` workspaces with several
//! projects (one application named after the directory).
use crate::project_files::read_repo_file;
use crate::services::support::{Discovery, relative, test_path};
use archguard_core::{
    Action, Call, Condition, DiagnosticSeverity, Origin, Screen, ScreenKind, ServiceTopology,
    Target, Transition, UiFlow, UiFlowDiagnostic, best_route,
};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

const SOURCE: &str = "angular";
const MAX_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 5_000;
/// Nesting bound for parsed literals and for route arrays that load each other.
const MAX_DEPTH: usize = 32;
const HTTP_METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "head", "options"];
const PARAM: &str = ":param";
const ROUTER_CALLS: [&str; 3] = [
    "RouterModule.forRoot(",
    "RouterModule.forChild(",
    "provideRouter(",
];

/// Screens, transitions and HTTP actions of every Angular application below `root`.
/// Read errors and everything not literal become diagnostics.
pub fn extract(root: &Path) -> UiFlow {
    let mut flow = UiFlow::default();
    match Discovery::new(root, &mut ServiceTopology::default()) {
        Ok(discovery) => {
            for app in applications(root, &discovery, &mut flow.diagnostics) {
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
    flow.screens.sort();
    flow.screens.dedup();
    flow.transitions.sort();
    flow.transitions.dedup();
    flow.actions.sort();
    flow.actions.dedup();
    flow.diagnostics.sort();
    flow.diagnostics.dedup();
    // Every unresolved target, unknown condition or unread file leaves a gap.
    flow.complete = !flow
        .diagnostics
        .iter()
        .any(|d| d.severity == DiagnosticSeverity::Gap);
    flow
}

fn diag(out: &mut Vec<UiFlowDiagnostic>, file: &str, line: u32, message: impl Into<String>) {
    out.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message: message.into(),
        severity: DiagnosticSeverity::Gap,
    });
}

fn read(root: &Path, rel: &str) -> Result<String, String> {
    read_repo_file(root, Path::new(rel), MAX_BYTES)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .map_err(|e| {
            format!("File not read ({e}): larger than 1 MiB, not a regular file or not readable; its routes, links and calls are unknown.")
        })
}

/// Application directories and TypeScript sources below `root`.
fn discover(root: &Path, discovery: &Discovery) -> (BTreeSet<String>, Vec<String>) {
    let mut dirs = BTreeSet::new();
    let mut sources = Vec::new();
    for entry in discovery.walk(root).flatten() {
        let rel = relative(root, entry.path());
        if !entry.file_type().is_some_and(|t| t.is_file())
            || rel.split('/').any(|p| p == ".angular")
        {
            continue;
        }
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
        if name == "angular.json"
            || (name == "package.json"
                && read(root, &rel).is_ok_and(|t| t.contains("\"@angular/router\"")))
        {
            dirs.insert(dir.to_string());
        } else if name.ends_with(".ts") && !name.ends_with(".d.ts") && !test_path(&rel) {
            sources.push(rel);
        }
    }
    (dirs, sources)
}

fn app_name(root: &Path, dir: &str) -> String {
    match dir.rsplit('/').next() {
        Some(last) if !last.is_empty() => last.to_string(),
        _ => root
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "app".into()),
    }
}

struct App<'a> {
    root: &'a Path,
    name: String,
    /// TypeScript sources by path relative to `root`.
    files: BTreeMap<String, String>,
}

fn applications<'a>(
    root: &'a Path,
    discovery: &Discovery,
    out: &mut Vec<UiFlowDiagnostic>,
) -> Vec<App<'a>> {
    let (dirs, sources) = discover(root, discovery);
    let mut apps: BTreeMap<&str, App> = dirs
        .iter()
        .map(|d| {
            let files = BTreeMap::new();
            (
                d.as_str(),
                App {
                    root,
                    name: app_name(root, d),
                    files,
                },
            )
        })
        .collect();
    let mut capped = BTreeSet::new();
    for rel in sources {
        let Some(dir) = dirs
            .iter()
            .filter(|d| d.is_empty() || rel.starts_with(&format!("{d}/")))
            .max_by_key(|d| d.len())
        else {
            continue;
        };
        let app = apps
            .get_mut(dir.as_str())
            .expect("every directory is an app");
        if app.files.len() >= MAX_FILES {
            if capped.insert(dir.as_str()) {
                diag(
                    out,
                    &rel,
                    0,
                    format!(
                        "Angular scan of {} stopped after {MAX_FILES} TypeScript files; later files are unknown.",
                        app.name
                    ),
                );
            }
            continue;
        }
        match read(root, &rel) {
            Ok(text) => {
                app.files.insert(rel, text);
            }
            Err(message) => diag(out, &rel, 0, message),
        }
    }
    apps.into_values().collect()
}

impl App<'_> {
    fn extract(&self, flow: &mut UiFlow) {
        let mut candidates = BTreeSet::new();
        for file in self.files.keys() {
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
                diagnostics: vec![],
                refs: BTreeSet::new(),
                stack: vec![],
            };
            walk.array_at(&file, at, "", &[]);
            refs.append(&mut walk.refs);
            walks.push(((file, at), walk));
        }
        let mut screens = Vec::new();
        for (key, walk) in walks {
            if !refs.contains(&key) {
                flow.diagnostics.extend(walk.diagnostics);
                screens.extend(walk.screens);
            }
        }
        self.flows(&screens, flow);
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
        let mut arrays = Self::typed_routes(file, text);
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

    /// `name: Routes = [`.
    fn typed_routes(file: &str, text: &str) -> Vec<(String, usize, Option<String>)> {
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

    /// The array literal `name` is bound to, in `file` or the file it imports it from.
    fn declaration(&self, file: &str, name: &str) -> Option<(String, usize)> {
        let text = self.files.get(file)?;
        if let Some(at) = Self::local_array(text, name) {
            return Some((file.into(), at));
        }
        let target = self.module(file, &Self::import_of(text, name)?)?;
        Self::local_array(&self.files[&target], name).map(|at| (target, at))
    }

    /// Offset of the `[` that `const|let|var name … = [` binds.
    fn local_array(text: &str, name: &str) -> Option<usize> {
        let b = text.as_bytes();
        for kw in ["const", "let", "var"] {
            for pos in words(text, kw) {
                let j = skip_ws(b, pos + kw.len());
                if ident_at(text, j) != name {
                    continue;
                }
                let k = j + name.len();
                let Some(eq) = text[k..].find(['=', ';']).map(|e| k + e) else {
                    continue;
                };
                let after = skip_ws(b, eq + 1);
                if b[eq] == b'=' && b.get(after) == Some(&b'[') {
                    return Some(after);
                }
            }
        }
        None
    }

    /// Module specifier of the `import … from '…'` that names `name`.
    fn import_of(text: &str, name: &str) -> Option<String> {
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

    /// Transitions and actions of every component.
    fn flows(&self, screens: &[(Screen, String)], flow: &mut UiFlow) {
        let classes: Vec<Class> = self
            .files
            .iter()
            .flat_map(|(f, t)| Class::scan(f, t))
            .collect();
        let calls: Vec<Vec<HttpCall>> = classes
            .iter()
            .map(|c| self.http_calls(c, &mut flow.diagnostics))
            .collect();
        let everywhere = Origin::AnyScreen {
            application: self.name.clone(),
        };
        for (class, own_calls) in classes.iter().zip(&calls) {
            if !class.component {
                continue;
            }
            let text = &self.files[&class.file];
            let body = &text[class.range.clone()];
            let mut froms: Vec<(Origin, Option<&str>)> = screens
                .iter()
                .filter(|(_, c)| *c == class.name)
                .map(|(s, _)| (Origin::Screen(s.id.clone()), s.route.as_deref()))
                .collect();
            if froms.is_empty() {
                froms.push((everywhere.clone(), None));
            }
            let mut links = self.template_links(class, &mut flow.diagnostics);
            links.extend(Link::in_class(&class.file, text, class.range.clone()));
            let injected: Vec<&HttpCall> = classes
                .iter()
                .zip(&calls)
                .filter(|(c, _)| !c.component && !words(body, &c.name).is_empty())
                .flat_map(|(_, calls)| calls)
                .collect();
            for (from, base) in froms {
                for link in &links {
                    let target = self.target(link, base, screens, &mut flow.diagnostics);
                    flow.transitions.push(Transition {
                        from: from.clone(),
                        target,
                        trigger: link.trigger.clone(),
                        file: link.file.clone(),
                        line: link.line,
                        condition: None,
                        source: SOURCE.into(),
                    });
                }
                for call in own_calls.iter().chain(injected.iter().copied()) {
                    flow.actions.push(Action {
                        screen: from.clone(),
                        trigger: None,
                        call: call.call.clone(),
                        file: call.file.clone(),
                        line: call.line,
                        condition: None,
                        source: SOURCE.into(),
                    });
                }
            }
        }
    }

    fn template_links(&self, class: &Class, out: &mut Vec<UiFlowDiagnostic>) -> Vec<Link> {
        match &class.template {
            Some(Template::Inline { text, line }) => Link::in_template(&class.file, text, *line),
            Some(Template::File(path)) => match read(self.root, path) {
                Ok(text) => Link::in_template(path, &text, 1),
                Err(message) => {
                    diag(out, path, 0, message);
                    vec![]
                }
            },
            None => vec![],
        }
    }

    fn target(
        &self,
        link: &Link,
        base: Option<&str>,
        screens: &[(Screen, String)],
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Target {
        let message = match &link.path {
            None => format!("Navigation target is not a literal: {}", link.source),
            Some(_) => match link.route(base) {
                None => format!(
                    "Relative navigation {} in a component that is no route target; its base route is unknown.",
                    link.source
                ),
                Some(path) => {
                    let routes = screens
                        .iter()
                        .filter_map(|(s, _)| Some((&s.id, s.route.as_deref()?)));
                    match best_route(&path, routes) {
                        Ok(id) => return Target::Screen(id.clone()),
                        Err(tied) if tied.is_empty() => format!(
                            "Navigation target {path} matches no route of application {}.",
                            self.name
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

    fn http_calls(&self, class: &Class, out: &mut Vec<UiFlowDiagnostic>) -> Vec<HttpCall> {
        let text = &self.files[&class.file];
        let b = text.as_bytes();
        let mut calls = Vec::new();
        for method in HTTP_METHODS {
            let pat = format!(".{method}");
            for (i, _) in text[class.range.clone()].match_indices(&pat) {
                let pos = class.range.start + i;
                let receiver = text[..pos]
                    .rsplit(|c: char| !is_ident_char(c))
                    .next()
                    .unwrap_or("");
                if !receiver.to_ascii_lowercase().contains("http") {
                    continue;
                }
                let mut j = skip_ws(b, pos + pat.len());
                if b.get(j) == Some(&b'<') {
                    j = skip_ws(b, skip_generic(b, j));
                }
                if b.get(j) != Some(&b'(') {
                    continue;
                }
                let url = Parser { s: text, i: j + 1 }.value(0);
                let line = line_of(text, pos);
                let call = match url.value.literal() {
                    Some(u) => Call::Http {
                        method: method.to_ascii_uppercase(),
                        path: url_path(&u),
                    },
                    None => {
                        let source = url.source(text);
                        let message = format!(
                            "HTTP call URL is not a literal; the call is unknown: {source}"
                        );
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
        }
        calls
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
    diagnostics: Vec<UiFlowDiagnostic>,
    refs: BTreeSet<(String, usize)>,
    stack: Vec<(String, usize)>,
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

    fn array_at(&mut self, file: &str, at: usize, prefix: &str, guards: &[Condition]) {
        let app: &'w App<'w> = self.app;
        let src = Src {
            file,
            text: app.files[file].as_str(),
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
        let full = [prefix, path]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        let mut guards = inherited.to_vec();
        for key in ["canActivate", "canMatch"] {
            if let Some(v) = get(key) {
                self.guards(src, v, &mut guards);
            }
        }
        if let Some(name) = self.component(src, get("component"), get("loadComponent")) {
            let screen = self.screen(src, item, &full, name, &guards);
            self.screens.push(screen);
        }
        if let Some(v) = get("canActivateChild") {
            self.guards(src, v, &mut guards);
        }
        if let Some(v) = get("children") {
            self.children(src, v, &full, &guards);
        }
        if let Some(v) = get("loadChildren") {
            self.load_children(src, v, &full, &guards);
        }
    }

    /// The literal path, `""` without one; `None` (and a diagnostic) when not literal.
    fn path<'i>(&mut self, src: Src, v: Option<&'i Item>) -> Option<&'i str> {
        match v {
            None => Some(""),
            Some(Item {
                value: Value::Str(p),
                ..
            }) => Some(p.trim_matches('/')),
            Some(v) => {
                let message = format!(
                    "Route path is not a literal; the route and its children are unknown: {}",
                    v.source(src.text)
                );
                self.diag(src, v.at, message);
                None
            }
        }
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
            id: format!("{app}:{route}"),
            name: name.clone(),
            kind: ScreenKind::Page,
            route: Some(route),
            application: Some(app.clone()),
            file: src.file.into(),
            line: line_of(src.text, item.at),
            access,
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

    fn guards(&mut self, src: Src, v: &Item, out: &mut Vec<Condition>) {
        let items = match &v.value {
            Value::Arr(items) => items.iter().collect(),
            _ => vec![v],
        };
        for guard in items {
            match &guard.value {
                Value::Raw(name) if is_name(name) => out.push(Condition::Custom(name.clone())),
                _ => {
                    let s = guard.source(src.text);
                    let message = format!(
                        "Guard is not a plain function or class name; its condition is unknown: {s}"
                    );
                    self.diag(src, guard.at, message);
                    out.push(Condition::Unknown(s));
                }
            }
        }
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
        let chosen = if named.is_empty() { arrays } else { named };
        if chosen.is_empty() {
            let message = format!("No route array found in {target}; its routes are unknown.");
            self.diag(src, v.at, message);
        }
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

/// A `@Component` or `@Injectable` class; `range` runs from its decorator to the next.
struct Class {
    name: String,
    file: String,
    range: Range<usize>,
    component: bool,
    template: Option<Template>,
}

impl Class {
    fn scan(file: &str, text: &str) -> Vec<Class> {
        let mut marks: Vec<(usize, bool)> = text
            .match_indices("@Component(")
            .map(|(i, _)| (i, true))
            .chain(text.match_indices("@Injectable(").map(|(i, _)| (i, false)))
            .collect();
        marks.sort();
        marks
            .iter()
            .enumerate()
            .filter_map(|(k, &(pos, component))| {
                let end = marks.get(k + 1).map_or(text.len(), |m| m.0);
                let open = pos + text[pos..].find('(')? + 1;
                let meta = Parser { s: text, i: open }.value(0);
                let after =
                    meta.end + *words(text.get(meta.end..end)?, "class").first()? + "class".len();
                let name = ident_at(text, skip_ws(text.as_bytes(), after)).to_string();
                let template = match &meta.value {
                    Value::Obj(fields) if component => Self::template(file, text, fields),
                    _ => None,
                };
                Some(Class {
                    name,
                    file: file.into(),
                    range: pos..end,
                    component,
                    template,
                })
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
            let path = if bound {
                Parser { s: value, i: 0 }.value(0).value.link()
            } else {
                (!value.contains("{{")).then(|| value.to_string())
            };
            links.push(Link {
                file: file.into(),
                line: first_line + line_of(tpl, pos) - 1,
                source: value.into(),
                relative: path.as_ref().is_some_and(|p| !p.starts_with('/')),
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

    /// `.navigate([...])` and `.navigateByUrl('…')` in a class body.
    fn in_class(file: &str, text: &str, range: Range<usize>) -> Vec<Link> {
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
                let path = first.value.link();
                links.push(Link {
                    file: file.into(),
                    line: line_of(text, pos),
                    source: first.source(text),
                    relative: relative_to && path.as_ref().is_some_and(|p| !p.starts_with('/')),
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

/// Path of a URL: scheme and host, query and fragment dropped, leading `/`.
fn url_path(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or("");
    let url = match url.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("", |i| &rest[i..]),
        None => url,
    };
    format!("/{}", url.trim_start_matches('/'))
}

/// `dir` and `rel` joined with `.` and `..` applied; `..` stops at the top.
fn join(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for s in rel.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn dir(file: &str) -> &str {
    file.rsplit_once('/').map_or("", |(d, _)| d)
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// A plain identifier or dotted name, e.g. `authGuard` or `Guards.admin`.
fn is_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.chars().all(|c| is_ident_char(c) || c == '.')
}

fn ident_at(text: &str, at: usize) -> &str {
    let Some(rest) = text.get(at..) else {
        return "";
    };
    &rest[..rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len())]
}

fn trailing_ident(text: &str) -> Option<String> {
    let start = text.rfind(|c: char| !is_ident_char(c)).map_or(0, |i| i + 1);
    let name = &text[start..];
    (!name.is_empty()).then(|| name.to_string())
}

/// Offsets of `word` with no identifier character on either side.
fn words(text: &str, word: &str) -> Vec<usize> {
    let b = text.as_bytes();
    let edge = |i: Option<&u8>| i.is_none_or(|c| !is_ident_char(*c as char));
    text.match_indices(word)
        .map(|(i, _)| i)
        .filter(|&i| (i == 0 || edge(b.get(i - 1))) && edge(b.get(i + word.len())))
        .collect()
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while b.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    i
}

/// Past a `<…>` type argument list; stops early at `(`, `;` or after 200 bytes.
fn skip_generic(b: &[u8], start: usize) -> usize {
    let mut depth = 0;
    for (i, c) in b.iter().enumerate().skip(start).take(200) {
        match c {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b'(' | b';' => return i,
            _ => {}
        }
    }
    start
}

fn line_of(text: &str, at: usize) -> u32 {
    let at = at.min(text.len());
    text.as_bytes()[..at]
        .iter()
        .filter(|&&c| c == b'\n')
        .count() as u32
        + 1
}

/// A JavaScript literal as far as this scanner reads it; `at..end` is its source.
struct Item {
    at: usize,
    end: usize,
    value: Value,
}

impl Item {
    fn source(&self, text: &str) -> String {
        text.get(self.at..self.end).unwrap_or("").trim().to_string()
    }
}

enum Value {
    Str(String),
    /// Template literal with `${…}` holes, raw.
    Tpl(String),
    Arr(Vec<Item>),
    Obj(Vec<(String, Item)>),
    /// Any other expression, as source text.
    Raw(String),
}

impl Value {
    /// A string literal, or a template literal that starts literal, holes as `:param`.
    fn literal(&self) -> Option<String> {
        match self {
            Value::Str(s) => Some(s.clone()),
            Value::Tpl(t) if !t.starts_with("${") => Some(Self::holes(t)),
            _ => None,
        }
    }

    /// A literal, or a link array joined: the first part must be literal, later
    /// non-literal parts become `:param`, objects (matrix parameters) are dropped.
    fn link(&self) -> Option<String> {
        let Value::Arr(items) = self else {
            return self.literal();
        };
        let (first, rest) = items.split_first()?;
        let mut out = vec![first.value.literal()?];
        for item in rest {
            match (&item.value, item.value.literal()) {
                (_, Some(s)) => out.push(s),
                (Value::Obj(_), None) => {}
                _ => out.push(PARAM.into()),
            }
        }
        Some(out.join("/"))
    }

    fn holes(t: &str) -> String {
        let mut out = String::new();
        let mut rest = t;
        while let Some(i) = rest.find("${") {
            out.push_str(&rest[..i]);
            out.push_str(PARAM);
            let mut depth = 0;
            let mut end = rest.len();
            for (j, c) in rest[i + 1..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i + 1 + j + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            rest = &rest[end..];
        }
        out.push_str(rest);
        out
    }
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> u8 {
        self.s.as_bytes().get(self.i).copied().unwrap_or(0)
    }

    /// Skips whitespace and comments.
    fn ws(&mut self) {
        let b = self.s.as_bytes();
        loop {
            self.i = skip_ws(b, self.i);
            let rest = b.get(self.i..).unwrap_or_default();
            self.i = if rest.starts_with(b"//") {
                self.s[self.i..].find('\n').map_or(b.len(), |e| self.i + e)
            } else if rest.starts_with(b"/*") {
                self.s[self.i + 2..]
                    .find("*/")
                    .map_or(b.len(), |e| self.i + 2 + e + 2)
            } else {
                return;
            };
        }
    }

    /// Offset after the string literal at the cursor; the end for an open one.
    fn string_end(&self) -> usize {
        let b = self.s.as_bytes();
        let quote = b[self.i];
        let mut i = self.i + 1;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 2,
                c if c == quote => return i + 1,
                _ => i += 1,
            }
        }
        b.len()
    }

    /// Content of the string literal at the cursor; moves past it.
    fn string(&mut self) -> String {
        let at = self.i;
        self.i = self.string_end();
        self.s.get(at + 1..self.i - 1).unwrap_or("").to_string()
    }

    /// One value; anything followed by more than a separator is `Raw`.
    fn value(&mut self, depth: usize) -> Item {
        self.ws();
        let at = self.i;
        let value = match self.peek() {
            b'[' if depth < MAX_DEPTH => Value::Arr(self.array(depth + 1)),
            b'{' if depth < MAX_DEPTH => Value::Obj(self.object(depth + 1)),
            q @ (b'\'' | b'"' | b'`') => {
                let inner = self.string();
                if q == b'`' && inner.contains("${") {
                    Value::Tpl(inner)
                } else {
                    Value::Str(inner)
                }
            }
            _ => return self.raw(at),
        };
        let end = self.i;
        self.ws();
        if matches!(self.peek(), b',' | b']' | b'}' | b')' | b';' | 0) {
            Item { at, end, value }
        } else {
            self.raw(at)
        }
    }

    /// Source text up to the next separator outside brackets and strings.
    fn raw(&mut self, at: usize) -> Item {
        let b = self.s.as_bytes();
        self.i = at;
        let mut depth = 0usize;
        while self.i < b.len() {
            match b[self.i] {
                b'\'' | b'"' | b'`' => {
                    self.i = self.string_end();
                    continue;
                }
                b'/' if matches!(b.get(self.i + 1), Some(b'/' | b'*')) => {
                    self.ws();
                    continue;
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' if depth == 0 => break,
                b')' | b']' | b'}' => depth -= 1,
                b',' | b';' if depth == 0 => break,
                _ => {}
            }
            self.i += 1;
        }
        let text = self.s[at..self.i].trim_end();
        Item {
            at,
            end: at + text.len(),
            value: Value::Raw(text.into()),
        }
    }

    /// Items of the array whose `[` is at the cursor.
    fn array(&mut self, depth: usize) -> Vec<Item> {
        self.i += 1;
        let mut items = Vec::new();
        loop {
            self.ws();
            match self.peek() {
                b']' => {
                    self.i += 1;
                    break;
                }
                0 => break,
                _ => {}
            }
            let item = self.value(depth);
            if item.end > item.at {
                items.push(item);
            }
            self.ws();
            match self.peek() {
                b',' => self.i += 1,
                b']' => {}
                _ => break,
            }
        }
        items
    }

    /// Fields of the object whose `{` is at the cursor; shorthand and methods are `Raw`.
    fn object(&mut self, depth: usize) -> Vec<(String, Item)> {
        self.i += 1;
        let mut fields = Vec::new();
        loop {
            self.ws();
            match self.peek() {
                b'}' => {
                    self.i += 1;
                    break;
                }
                0 => break,
                _ => {}
            }
            let at = self.i;
            let key = if matches!(self.peek(), b'\'' | b'"') {
                self.string()
            } else {
                let key = ident_at(self.s, at).to_string();
                self.i += key.len();
                key
            };
            self.ws();
            let item = match self.peek() {
                b':' if !key.is_empty() => {
                    self.i += 1;
                    self.value(depth)
                }
                b',' | b'}' if !key.is_empty() => Item {
                    at,
                    end: at + key.len(),
                    value: Value::Raw(key.clone()),
                },
                _ => self.raw(at),
            };
            fields.push((key, item));
            self.ws();
            match self.peek() {
                b',' => self.i += 1,
                b'}' => {}
                _ => break,
            }
        }
        fields
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
        assert_eq!(order.access, None);
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
        assert_eq!(
            screens(&flow),
            pairs(&[
                ("web:/admin", "AdminComponent"),
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
            Some(Condition::All(vec![
                auth.clone(),
                Condition::Custom("adminGuard".into())
            ]))
        );
        let home = flow
            .screens
            .iter()
            .find(|s| s.name == "AdminHomeComponent")
            .unwrap();
        assert_eq!(home.access, Some(auth));
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
        assert_eq!(checkout.access, Some(Condition::Custom("authGuard".into())));
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
            (
                page("web:/"),
                unresolved("/missing"),
                Some("Missing".into()),
            ),
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
            ("web/src/app/home.component.html", 5),
            ("web/src/app/home.component.ts", 5),
            ("web/src/app/home.component.ts", 9),
        ] {
            assert!(lines.contains(&at), "missing {at:?} in {lines:?}");
        }
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
            // not an Angular application: no angular.json, no @angular/router
            ("other/package.json", r#"{"dependencies":{"react":"19"}}"#),
            ("other/src/routes.ts", nm),
        ]);
        let flow = extract(dir.path());
        assert_eq!(screens(&flow), pairs(&[("web:/a", "AComponent")]));
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }
}
