//! SvelteKit extractor for ui-flow/v1 (ADR 0021). A text-level scanner without a
//! Svelte or TypeScript parser, built like the Vue extractor; what it cannot read is a
//! diagnostic, and compiler-backed resolution is ADR 0021 phase 7.
//!
//! Applications are directories whose `package.json` names `@sveltejs/kit`; a file
//! belongs to the deepest one holding it. Scanned: `.svelte`, `.ts`, `.js`. Skipped:
//! `.svelte-kit`, `e2e`, `coverage`, `.storybook`, the engine's other excluded
//! directories (`node_modules`, `build`, `dist`, …), test files and `*.d.ts`. The routes
//! directory is `src/routes`, or `kit.files.routes` of `svelte.config.js` when that is a
//! literal; `kit.paths.base`, when literal, is the base path.
//!
//! Screens: every `+page.svelte` (also `+page@x.svelte`) below the routes directory.
//! Segments: `[id]` and `[id=matcher]` become `:id`, `[...rest]` becomes `*`, a segment
//! mixing text and parameters becomes `:param`, groups `(x)` are dropped. An optional
//! `[[x]]` segment is left out of the screen's route (its canonical URL), but every
//! variant with and without it resolves to the screen. The name is the text of the
//! first `<h1>` when it is plain text, else the route. `+layout.svelte` is no screen;
//! its links and calls, like those of every file that is no `+page`, act on every
//! screen of the application (`any_screen`; the layout's subtree is not modelled).
//!
//! Access, as screen conditions (`all` of several, outermost layout first):
//! - A `redirect(…)` (imported from `@sveltejs/kit`, also `throw redirect(…)`) inside the
//!   `load` function of `+layout.ts`/`+layout.server.ts` (the directory and below) or
//!   `+page.ts`/`+page.server.ts` (`.js` too). Guarded by `if (c)`, the screen requires
//!   `not c`; nested `if`s up to the enclosing function are `all`; `try` blocks are
//!   transparent; a redirect in an `else`, loop or `catch` block or in a callback
//!   argument (`.catch(() => redirect(…))`) is `unknown`. Without a guard it is a
//!   forwarding page: a transition, no access. Equal conditions are kept once.
//! - A check `c` is a name (`locals.user`, `a?.b`), optionally negated with `!`: it is
//!   `custom(name)` unless `[views.access]` lists that name; a call is read only when
//!   `[views.access]` lists its function. Anything else is `unknown` and a gap. The
//!   meaning of a check is never guessed.
//! - `error(…)` from `@sveltejs/kit` guards a load like `redirect(…)`.
//! - A call inside `load` to a function listed in `[views.access]` (`authenticate =
//!   "authenticated"`) is that meaning; `$arg` forms take its string literal arguments. A
//!   fixed meaning called with an options object (`authenticate(url, { admin: true })`)
//!   is `unknown` and a gap: one meaning cannot hold for every call site. A listed
//!   `open` guard makes the screen `open`; `open` never hides an unknown next to it.
//! - A helper called as a statement (`await authenticate(url)`) and not listed is read
//!   one hop deep (its own file or the `$lib`/relative module it is imported from) with
//!   the same rules; not found or not a function is `unknown` and a gap. Helpers of
//!   helpers and package functions are not followed.
//! - Layout loads inherit down the layout chain; `+page@x.svelte` and `+layout@x.svelte`
//!   cut the chain at the ancestor directory `x` (empty: the root); an unknown `x` is a
//!   gap and keeps the chain. A screen with a condition from a layout is `weak`.
//! - `handle` of `src/hooks.server.{ts,js}` (or `hooks.server/index`), directly, as
//!   `sequence(a, b)` (local, inline or imported functions, one hop) or re-exported
//!   (`export { handle } from './auth'`; Auth.js `SvelteKitAuth` protects nothing). In its
//!   body an `if (…)` whose block calls `redirect(`/`error(` or returns a `Response`/
//!   `json(` with status 40x, nested `if`s `and`ed; conjuncts `pathname.startsWith`/`===`
//!   and the same on `route.id` (with groups), `[…].includes(…)` of a literal list (or a
//!   constant of the file), and their negations select routes, the others are checks as
//!   above. Matching screens, endpoints and actions require that the block does not run.
//!   `if (selectors) return resolve(event)` exempts the routes from later rules; a route
//!   a read allowlist exempts from a blocking rule is `open` when nothing else applies.
//!   A blocking call outside such an `if`, or a handle that cannot be read, is `unknown`
//!   everywhere and a gap. Prerendered pages (`export const prerender = true`) are not
//!   covered by `handle` (note).
//!
//! Transitions: an `href` attribute on any element (`<a>` and components that forward
//! it), `goto(…)` imported from `$app/navigation`, and `redirect(…)` (in a load, under
//! the guard's condition). The target is a literal, a template literal or `"/a/{b}"`
//! attribute text (holes become `:param`, the `$app/paths` `base` its literal value),
//! or a route helper called one hop away: `Name.member(…)` or `name(…)` declared as
//! `const` in the file or a module it imports by name (relative, `$lib`, or the
//! `package.json` `imports` map), whose arrow or final `return` gives a literal or
//! template literal, plus `+` parts that only add a query or fragment (a literal
//! starting with `?` or `#`, `''`, a `c ? x : y` of those, or a call of a function of
//! that module whose final `return` is one). The base path is stripped; paths resolve
//! with `best_route` within the application; external URLs and anchors are no
//! transition; relative paths depend on the current URL and stay unresolved.
//!
//! Actions: `fetch(url, { method })` and axios through `lang_ts_http`; calls of
//! functions imported from a package whose `package.json` is in the scan (a generated
//! API client such as `@immich/sdk`): the package's exported function is read one hop,
//! and its first `fetch`/`fetchJson`/`fetchText`/`fetchBlob` call gives the path (a
//! final `${…}` not after `/` is the query and dropped) and `method: "…"` (else `GET`),
//! prefixed with the literal `baseUrl` of an exported `defaults` object (oazapfts). An
//! exported function without such a call is no action; a non-literal one is
//! `Call::Unresolved(name)`. Packages outside the scan are not read. `<form
//! method="post">` posts to its `action` (`?/x` is the screen's own route plus `?/x`,
//! none is the default action).
//!
//! Endpoints: `+server.ts`/`.js` exporting `GET`, `POST`, … (`fallback` is `ANY`) and
//! the `actions` object of `+page.server.ts` (`default` is `POST <route>`, `x` is `POST
//! <route>?/x`). Access is `undeclared` unless a `hooks.server` rule applies.
//!
//! Triggers: an element with `onclick`/`on:click`/`onsubmit`/`on:submit` gets its
//! `aria-label`, `data-testid` or plain text; it covers the attribute's code and the
//! body of the handler it names or calls (in the same file). A link's trigger is its own.
//!
//! Not recognized; each case is a diagnostic: non-literal targets, URLs and checks.
//! Not followed: helpers of helpers, object methods written as `name() {…}`, redirects of `hooks.server`, `+page.ts`
//! files without a `+page.svelte`, `resolve()` of `$app/paths`.
use crate::extract_ctx::{
    ExtractContext, access_condition, dir_name as app_name, finish, gap as diag,
    read_source as read, walk, with_root_config,
};
use crate::lang_markup::blank;
use crate::lang_text::{find_top, matching_back, split_top, unparen};
use crate::lang_ts::*;
use crate::lang_ts_decl::{declared_value, has_dep, imports};
use crate::lang_ts_http::*;
use crate::services::support::test_path;
use archguard_core::{
    ANY_METHOD, AccessMeaning, AccessState, Action, Call, Condition, DiagnosticSeverity,
    EndpointAccess, Origin, Screen, ScreenKind, Target, Transition, UiFlow, UiFlowDiagnostic,
    best_route,
};
use std::collections::BTreeMap;
use std::path::Path;

const SOURCE: &str = "svelte";
const MAX_FILES: usize = 5_000;
const SKIPPED_DIRS: [&str; 4] = [".svelte-kit", "e2e", "coverage", ".storybook"];
const EXTENSIONS: [&str; 3] = [".svelte", ".ts", ".js"];
const LOADS: [&str; 4] = [".ts", ".js", ".server.ts", ".server.js"];
const TRIGGERS: [&str; 4] = ["onclick", "on:click", "onsubmit", "on:submit"];
/// Optional segments expanded per route; more stay out of the variants.
const MAX_OPTIONAL: usize = 6;

type Access = BTreeMap<String, AccessMeaning>;

/// Screens, transitions, HTTP actions and server endpoints of every SvelteKit
/// application below `root`. Read errors and everything not literal become diagnostics.
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run; checks take their meaning from `[views.access]`.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    match walk(ctx.root) {
        Ok(walk) => {
            let scan = Scan::read(ctx.root, walk.files, &mut flow.diagnostics);
            if !scan.apps.is_empty()
                && let Some(e) = &ctx.config_error
            {
                let message = format!(
                    "Configuration not read ({e}); [views.access] is not applied, checks stay custom conditions."
                );
                note(&mut flow.diagnostics, "", 0, &message);
            }
            for app in &scan.apps {
                app.extract(&scan, &ctx.config.views.access, &mut flow);
            }
        }
        Err(e) => diag(
            &mut flow.diagnostics,
            "",
            0,
            format!("Discovery exclusions not read; no SvelteKit application analyzed: {e}"),
        ),
    }
    finish(&mut flow);
    flow
}

fn note(out: &mut Vec<UiFlowDiagnostic>, file: &str, line: u32, message: &str) {
    out.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message: message.into(),
        severity: DiagnosticSeverity::Note,
    });
}

fn is_source(rel: &str) -> bool {
    EXTENSIONS.iter().any(|e| rel.ends_with(e)) && !rel.ends_with(".d.ts") && !test_path(rel)
}

/// `None`, the one condition, or `all` of several; repeats (a layout and its page
/// calling the same guard) are kept once, in first order.
fn all(conditions: Vec<Condition>) -> Option<Condition> {
    let mut conditions = conditions.into_iter().fold(Vec::new(), |mut kept, c| {
        if !kept.contains(&c) {
            kept.push(c);
        }
        kept
    });
    match conditions.len() {
        0 => None,
        1 => conditions.pop(),
        _ => Some(Condition::All(conditions)),
    }
}

/// All layers must hold: protected conditions are collected (repeats once, in
/// first order), `Open` only counts when nothing is protected.
fn fold(layers: impl IntoIterator<Item = AccessState>) -> AccessState {
    let (mut conditions, mut open) = (Vec::new(), false);
    for layer in layers {
        match layer {
            AccessState::Protected(Condition::All(items)) => conditions.extend(items),
            AccessState::Protected(c) => conditions.push(c),
            AccessState::Open => open = true,
            AccessState::Undeclared => {}
        }
    }
    AccessState::from_parts(all(conditions), open)
}

/// The negation; `None` stands for "always true".
fn negate(c: Option<Condition>) -> Option<Condition> {
    match c {
        None => Some(Condition::Never),
        Some(Condition::Never) => None,
        Some(Condition::Not(inner)) => Some(*inner),
        Some(c) => Some(Condition::Not(Box::new(c))),
    }
}

/// What the analysis root holds: the applications and the packages by name.
struct Scan {
    apps: Vec<App>,
    /// Exported HTTP functions of scanned packages, by package name.
    sdks: BTreeMap<String, BTreeMap<String, Result<Call, String>>>,
}

struct App {
    name: String,
    /// The application directory with a trailing `/`, `""` at the root.
    prefix: String,
    /// The routes directory with a trailing `/`.
    routes: String,
    base: String,
    /// `package.json` `imports`: specifier (a trailing `*` is a wildcard) to path.
    aliases: Vec<(String, String)>,
    files: BTreeMap<String, String>,
}

/// The package name of a bare import specifier.
fn package_of(spec: &str) -> Option<&str> {
    if spec.starts_with(['.', '/', '$', '#']) {
        return None;
    }
    let cut = if spec.starts_with('@') { 2 } else { 1 };
    let end = spec
        .match_indices('/')
        .nth(cut - 1)
        .map_or(spec.len(), |(i, _)| i);
    Some(&spec[..end])
}

impl Scan {
    fn read(root: &Path, files: Vec<String>, out: &mut Vec<UiFlowDiagnostic>) -> Scan {
        let files: Vec<String> = (files.into_iter())
            .filter(|f| !f.split('/').any(|p| SKIPPED_DIRS.contains(&p)))
            .collect();
        let (mut apps, packages) = Self::manifests(root, &files, out);
        Self::sources(root, &files, &mut apps, out);
        let sdks = Self::sdks(root, &files, &apps, &packages);
        Scan {
            apps: apps.into_values().collect(),
            sdks,
        }
    }

    /// Applications by directory and package directories by package name.
    fn manifests(
        root: &Path,
        files: &[String],
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> (BTreeMap<String, App>, BTreeMap<String, String>) {
        let (mut apps, mut packages) = (BTreeMap::new(), BTreeMap::new());
        for rel in files {
            let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
            let Some(text) = (name == "package.json")
                .then(|| read(root, rel).ok())
                .flatten()
            else {
                continue;
            };
            let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            if let Some(n) = json.get("name").and_then(|n| n.as_str()) {
                packages.insert(n.to_string(), dir.to_string());
            }
            if has_dep(&text, "@sveltejs/kit") {
                apps.insert(dir.to_string(), App::new(root, dir, &json, files, out));
            }
        }
        (apps, packages)
    }

    /// Reads each source into the deepest application holding it.
    fn sources(
        root: &Path,
        files: &[String],
        apps: &mut BTreeMap<String, App>,
        out: &mut Vec<UiFlowDiagnostic>,
    ) {
        let mut capped = false;
        for rel in files.iter().filter(|f| is_source(f)) {
            let Some(app) = (apps.iter_mut())
                .filter(|(d, _)| d.is_empty() || rel.starts_with(&format!("{d}/")))
                .max_by_key(|(d, _)| d.len())
                .map(|(_, app)| app)
            else {
                continue;
            };
            if app.files.len() >= MAX_FILES {
                if !std::mem::replace(&mut capped, true) {
                    let message = format!(
                        "SvelteKit scan of {} stopped after {MAX_FILES} source files; later files are unknown.",
                        app.name
                    );
                    diag(out, rel, 0, message);
                }
                continue;
            }
            match read(root, rel) {
                Ok(text) => {
                    app.files.insert(rel.clone(), blank(&text, "<!--", "-->"));
                }
                Err(message) => diag(out, rel, 0, message),
            }
        }
    }

    /// The scanned packages the applications import, other than the applications.
    fn sdks(
        root: &Path,
        files: &[String],
        apps: &BTreeMap<String, App>,
        packages: &BTreeMap<String, String>,
    ) -> BTreeMap<String, BTreeMap<String, Result<Call, String>>> {
        let imported: std::collections::BTreeSet<String> = (apps.values())
            .flat_map(|app| app.files.values())
            .flat_map(|text| imports(text))
            .filter_map(|i| package_of(&i.module).map(str::to_string))
            .collect();
        let mut sdks = BTreeMap::new();
        for pkg in imported {
            let Some(dir) = packages.get(&pkg).filter(|d| !apps.contains_key(*d)) else {
                continue;
            };
            let prefix = if dir.is_empty() {
                String::new()
            } else {
                format!("{dir}/")
            };
            let sources: Vec<(String, String)> = (files.iter())
                .filter(|f| f.starts_with(&prefix) && is_source(f) && !f.ends_with(".svelte"))
                .take(MAX_FILES)
                .filter_map(|f| read(root, f).ok().map(|t| (f.clone(), t)))
                .collect();
            sdks.insert(pkg, generated_calls(&sources));
        }
        sdks
    }
}

impl App {
    fn new(
        root: &Path,
        dir: &str,
        json: &serde_json::Value,
        files: &[String],
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> App {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let (mut routes, mut base) = ("src/routes".to_string(), String::new());
        let config = ["svelte.config.js", "svelte.config.ts"]
            .map(|n| format!("{prefix}{n}"))
            .into_iter()
            .find(|c| files.contains(c));
        if let Some(config) = config
            && let Ok(text) = read(root, &config)
        {
            for (object, key, slot) in [
                ("files", "routes", &mut routes),
                ("paths", "base", &mut base),
            ] {
                match config_value(&text, object, key) {
                    Some(Ok(v)) => *slot = v.trim_matches('/').to_string(),
                    Some(Err(source)) => {
                        let message = format!(
                            "kit.{object}.{key} is not a literal; the default is used: {source}"
                        );
                        diag(out, &config, 1, message);
                    }
                    None => {}
                }
            }
        }
        let base = if base.is_empty() {
            base
        } else {
            format!("/{base}")
        };
        let aliases = (json.get("imports").and_then(|m| m.as_object()).into_iter())
            .flatten()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect();
        App {
            name: app_name(root, dir),
            routes: format!("{prefix}{}/", routes),
            prefix,
            base,
            aliases,
            files: BTreeMap::new(),
        }
    }

    /// A relative, `$lib` or `imports`-mapped specifier resolved to a scanned file.
    fn module(&self, from: &str, spec: &str) -> Option<(&str, &str)> {
        let app_dir = self.prefix.trim_end_matches('/');
        let base = if let Some(rest) = spec.strip_prefix("$lib") {
            join(app_dir, &format!("src/lib{rest}"))
        } else if spec.starts_with('.') {
            join(dir(from), spec)
        } else {
            let target = self
                .aliases
                .iter()
                .find_map(|(k, v)| match k.strip_suffix('*') {
                    Some(stem) => spec.strip_prefix(stem).map(|rest| v.replace('*', rest)),
                    None => (k == spec).then(|| v.clone()),
                })?;
            join(app_dir, &target)
        };
        let mut candidates = vec![base.clone()];
        if let Some(stem) = base.strip_suffix(".js") {
            candidates.push(format!("{stem}.ts"));
        }
        for ext in EXTENSIONS {
            candidates.push(format!("{base}{ext}"));
            candidates.push(format!("{base}/index{ext}"));
        }
        candidates
            .into_iter()
            .find_map(|p| (self.files.get_key_value(&p)).map(|(k, v)| (k.as_str(), v.as_str())))
    }

    /// The route directory (below the routes directory) and file name of `file`.
    fn route_file<'f>(&self, file: &'f str) -> Option<(&'f str, &'f str)> {
        let inner = file.strip_prefix(self.routes.as_str())?;
        Some(inner.rsplit_once('/').unwrap_or(("", inner)))
    }

    fn extract(&self, scan: &Scan, access: &Access, flow: &mut UiFlow) {
        let checks = Checks { access };
        let hooks = hooks::rules(self, access, &mut flow.diagnostics);
        let loads = self.loads(&checks, &mut flow.diagnostics);
        let mut pages = self.pages();
        for page in &mut pages {
            let (access, weak) = self.page_access(page, &loads, &hooks, &mut flow.diagnostics);
            page.screen.access = access;
            page.screen.weak = weak;
        }
        endpoints::extract(self, &hooks, &mut flow.endpoints);
        let clients = Clients::new(&self.files);
        for (file, text) in &self.files {
            let Some(froms) = self.origins(file, &pages) else {
                continue;
            };
            let markup = if file.ends_with(".svelte") {
                blank(&blank(text, "<script", "</script>"), "<style", "</style>")
            } else {
                String::new()
            };
            let tags = markup::tags(&markup);
            let mut site = Site {
                app: self,
                file,
                text,
                markup: &markup,
                tags: &tags,
                froms,
                spans: Vec::new(),
                pages: &pages,
                checks: &checks,
            };
            site.spans = markup::spans(&site);
            nav::transitions(&site, flow);
            http::actions(&site, scan, &clients, flow);
        }
        flow.screens.extend(pages.into_iter().map(|p| p.screen));
    }

    /// A screen per `+page.svelte`, without access yet.
    fn pages(&self) -> Vec<Page> {
        let mut pages = Vec::new();
        for (file, text) in &self.files {
            let Some((dir, _)) = self.route_file(file).filter(|(_, n)| is_page(n)) else {
                continue;
            };
            let (route, variants) = route_of(dir);
            let screen = Screen {
                weak: false,
                id: format!("{}:{route}", self.name),
                name: heading(text).unwrap_or_else(|| route.clone()),
                kind: ScreenKind::Page,
                route: Some(route),
                application: Some(self.name.clone()),
                file: file.clone(),
                line: 1,
                access: archguard_core::AccessState::Undeclared,
                source: SOURCE.into(),
            };
            let dir = dir.to_string();
            pages.push(Page {
                screen,
                variants,
                dir,
            });
        }
        pages
    }

    /// The requirements of every `+layout`/`+page` load file, by file.
    fn loads(
        &self,
        checks: &Checks,
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> BTreeMap<&str, Vec<AccessState>> {
        let is_load = |name: &str| {
            LOADS
                .iter()
                .any(|e| name == format!("+layout{e}") || name == format!("+page{e}"))
        };
        (self.files.iter())
            .filter(|(file, _)| self.route_file(file).is_some_and(|(_, n)| is_load(n)))
            .map(|(file, text)| {
                let found = guard::requirements(self, checks, (file, text), out);
                (file.as_str(), found)
            })
            .collect()
    }

    /// The layout directories whose loads run for `page`, root first: the chain of
    /// directories, cut at `+layout@x.svelte` and `+page@x.svelte` resets (`x` names an
    /// ancestor directory, empty the root). An unknown target keeps the whole chain.
    fn layout_chain(&self, page: &Page, out: &mut Vec<UiFlowDiagnostic>) -> Vec<String> {
        let mut dirs = vec![String::new()];
        dirs.extend(
            page.dir
                .match_indices('/')
                .map(|(i, _)| page.dir[..i].to_string()),
        );
        if !page.dir.is_empty() {
            dirs.push(page.dir.clone());
        }
        let reset = |name: &str| {
            let (_, rest) = name.split_once('@')?;
            rest.strip_suffix(".svelte").map(str::to_string)
        };
        let cut = |kept: &mut Vec<String>, target: &str, out: &mut Vec<UiFlowDiagnostic>| {
            let at = kept.iter().position(|d| match target {
                "" => d.is_empty(),
                t => !d.is_empty() && d.rsplit('/').next() == Some(t),
            });
            match at {
                Some(i) => kept.truncate(i + 1),
                None => {
                    let message = format!(
                        "Layout reset to '{target}' names no ancestor directory; the whole layout chain is kept."
                    );
                    diag(out, &page.screen.file, 1, message);
                }
            }
        };
        let mut kept: Vec<String> = Vec::new();
        for d in &dirs {
            let sep = if d.is_empty() { "" } else { "/" };
            let prefix = format!("{}{d}{sep}+layout@", self.routes);
            let layout = self.files.keys().find_map(|f| {
                let name = f.strip_prefix(&prefix)?;
                reset(&format!("+layout@{name}")).filter(|_| !name.contains('/'))
            });
            if let Some(target) = layout {
                cut(&mut kept, &target, out);
            }
            kept.push(d.clone());
        }
        let name = self.route_file(&page.screen.file).map_or("", |(_, n)| n);
        if let Some(target) = reset(name) {
            cut(&mut kept, &target, out);
        }
        kept
    }

    /// The load files of `page`, outermost layout first, then the page's own.
    fn load_files(&self, page: &Page, chain: &[String]) -> (Vec<String>, Vec<String>) {
        let files = |dir: &str, kind: &str| -> Vec<String> {
            let sep = if dir.is_empty() { "" } else { "/" };
            LOADS
                .map(|e| format!("{}{dir}{sep}{kind}{e}", self.routes))
                .into()
        };
        let layouts = chain.iter().flat_map(|d| files(d, "+layout")).collect();
        (layouts, files(&page.dir, "+page"))
    }

    /// Whether `page` is prerendered: the nearest `export const prerender` of its
    /// own loads and its layouts wins.
    fn prerendered(&self, layouts: &[String], pages: &[String]) -> bool {
        let nearest = pages.iter().chain(layouts.iter().rev());
        for file in nearest {
            let Some(text) = self.files.get(file) else {
                continue;
            };
            let declared = words(text, "prerender")
                .into_iter()
                .find(|&p| text[..p].trim_end().ends_with("const") && text[..p].contains("export"));
            if let Some(p) = declared {
                let value = declared_value(text, "prerender");
                return value.is_some_and(|v| v.source(text) == "true") && p > 0;
            }
        }
        false
    }

    /// The access of `page`: the layouts' loads from the root down, the page's own
    /// loads and the hooks; weak when a layout's load contributes a condition. A
    /// prerendered page is not covered by `handle`.
    fn page_access(
        &self,
        page: &Page,
        loads: &BTreeMap<&str, Vec<AccessState>>,
        hooks: &[hooks::Rule],
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> (AccessState, bool) {
        let chain = self.layout_chain(page, out);
        let (layouts, own) = self.load_files(page, &chain);
        let of = |files: &[String]| -> Vec<AccessState> {
            let found = files.iter().filter_map(|f| loads.get(f.as_str()));
            found.flatten().cloned().collect()
        };
        let (from_layouts, from_page) = (of(&layouts), of(&own));
        let weak = from_layouts.iter().any(|s| s.condition().is_some());
        let route = page.screen.route.as_deref().unwrap_or("/");
        let mut hook = hooks::state(hooks, route, &format!("/{}", page.dir));
        if hook != AccessState::Undeclared && self.prerendered(&layouts, &own) {
            let message =
                "Prerendered page: handle does not run for it, hooks rules do not cover it.";
            note(out, &page.screen.file, 1, message);
            hook = AccessState::Undeclared;
        }
        let layers = from_layouts.into_iter().chain(from_page).chain([hook]);
        (fold(layers), weak)
    }

    /// The screens a file's links and calls start from; none for server files and
    /// `+page` scripts without a `+page.svelte`.
    fn origins(&self, file: &str, pages: &[Page]) -> Option<Vec<Origin>> {
        match self.route_file(file) {
            Some((dir, name)) if is_page(name) || name.starts_with("+page.") => {
                let page = pages.iter().find(|p| p.dir == dir)?;
                Some(vec![Origin::Screen(page.screen.id.clone())])
            }
            Some((_, name)) if name.starts_with("+server.") => None,
            _ if file.starts_with(&format!("{}src/hooks.server.", self.prefix)) => None,
            _ => Some(vec![Origin::AnyScreen {
                application: self.name.clone(),
            }]),
        }
    }
}

fn is_page(name: &str) -> bool {
    name == "+page.svelte" || (name.starts_with("+page@") && name.ends_with(".svelte"))
}

/// A literal string field `key` of the object `object: { … }` in a config file:
/// `Some(Err(source))` when the value is no literal.
fn config_value(text: &str, object: &str, key: &str) -> Option<Result<String, String>> {
    let b = text.as_bytes();
    words(text, object).into_iter().find_map(|pos| {
        let colon = skip_ws(b, pos + object.len());
        let open = skip_ws(b, colon + 1);
        if b.get(colon) != Some(&b':') || b.get(open) != Some(&b'{') {
            return None;
        }
        let fields = Parser { s: text, i: open }.object(0);
        let (_, v) = fields.into_iter().find(|(k, _)| k == key)?;
        Some(match v.value {
            Value::Str(s) => Ok(s),
            _ => Err(v.source(text)),
        })
    })
}

/// Text of the first `<h1>` when it is plain text.
fn heading(text: &str) -> Option<String> {
    let start = text.find("<h1")?;
    let inner = &text[start + text[start..].find('>')? + 1..];
    let inner = inner[..inner.find("</h1")?].trim();
    (!inner.is_empty() && !inner.contains(['{', '<'])).then(|| inner.to_string())
}

/// A screen with every route variant its optional segments allow.
struct Page {
    screen: Screen,
    variants: Vec<String>,
    /// Directory below the routes directory, `""` for the root.
    dir: String,
}

/// The canonical route (optional segments left out) and every variant of a route
/// directory.
fn route_of(dir: &str) -> (String, Vec<String>) {
    let mut variants: Vec<Vec<String>> = vec![vec![]];
    let mut optional = 0;
    for seg in dir.split('/').filter(|s| !s.is_empty()) {
        if seg.starts_with('(') && seg.ends_with(')') {
            continue;
        }
        let name = |inner: &str| format!(":{}", inner.split('=').next().unwrap_or(inner));
        let opt = seg.strip_prefix("[[").and_then(|s| s.strip_suffix("]]"));
        let part = if let Some(inner) = opt {
            name(inner)
        } else if seg.starts_with("[...") && seg.ends_with(']') {
            "*".into()
        } else if let Some(inner) = seg.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
            && !inner.contains(['[', ']'])
        {
            name(inner)
        } else if seg.contains('[') {
            PARAM.into()
        } else {
            seg.into()
        };
        if opt.is_none() {
            variants.iter_mut().for_each(|v| v.push(part.clone()));
        } else if optional < MAX_OPTIONAL {
            optional += 1;
            let with: Vec<Vec<String>> = (variants.iter())
                .map(|v| v.iter().cloned().chain([part.clone()]).collect())
                .collect();
            variants.extend(with);
        }
    }
    let paths: Vec<String> = (variants.iter())
        .map(|v| format!("/{}", v.join("/")))
        .collect();
    (paths[0].clone(), paths)
}

/// `[views.access]` applied to checks and guard calls.
struct Checks<'a> {
    access: &'a Access,
}

impl Checks<'_> {
    /// The condition that check `text` is true; `Err` when it is not readable.
    fn check(&self, text: &str) -> Result<Option<Condition>, String> {
        let t = unparen(text);
        if let Some(rest) = t.strip_prefix('!')
            && !rest.starts_with('=')
        {
            return self.check(rest).map(negate).map_err(|_| t.to_string());
        }
        let name = t.replace("?.", ".");
        if is_name(&name) {
            return access_condition(self.access, &name, &[])
                .map(|s| s.condition().cloned())
                .map_err(|_| t.to_string());
        }
        match call_parts(t) {
            Some((f, args)) if self.access.contains_key(f) => {
                access_condition(self.access, f, &args)
                    .map(|s| s.condition().cloned())
                    .map_err(|_| t.to_string())
            }
            _ => Err(t.to_string()),
        }
    }

    /// `all` of the checks; an unreadable one is `unknown` and a gap at `line`.
    fn checks(
        &self,
        texts: &[String],
        (file, line): (&str, u32),
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Option<Condition> {
        let mut conditions = Vec::new();
        for text in texts {
            match self.check(text) {
                Ok(c) => conditions.extend(c),
                Err(t) => {
                    let message = format!(
                        "Check {t} is not a name or listed in [views.access]; the condition is unknown."
                    );
                    diag(out, file, line, message);
                    conditions.push(Condition::Unknown(t));
                }
            }
        }
        all(conditions)
    }
}

/// The file being scanned, the screens it acts on and its trigger spans.
struct Site<'a> {
    app: &'a App,
    file: &'a str,
    text: &'a str,
    /// The text with scripts and styles blanked; empty for scripts.
    markup: &'a str,
    tags: &'a [markup::Tag],
    froms: Vec<Origin>,
    spans: Vec<markup::Span>,
    pages: &'a [Page],
    checks: &'a Checks<'a>,
}

impl Site<'_> {
    /// Triggers of the code at `at`; `[None]` when no trigger span holds it.
    fn triggers(&self, at: usize) -> Vec<Option<String>> {
        let mut labels: Vec<Option<String>> = (self.spans.iter())
            .filter(|s| (s.start..s.end).contains(&at))
            .map(|s| Some(s.label.clone()))
            .collect();
        labels.dedup();
        if labels.is_empty() {
            labels.push(None);
        }
        labels
    }

    /// Local names bound to `name` imported from `module`.
    fn imported(&self, module: &str, name: &str) -> Vec<String> {
        (imports(self.text).into_iter())
            .filter(|i| i.module == module && i.imported.as_deref() == Some(name))
            .map(|i| i.local)
            .collect()
    }

    /// The path of a target expression: a literal, a template literal or a route helper.
    fn path(&self, value: &Value) -> Option<String> {
        match value {
            Value::Str(s) => Some(s.clone()),
            Value::Tpl(t) => fill(t, |hole| self.hole(hole)),
            Value::Raw(r) => helper::call(self.app, self.file, self.text, r),
            _ => None,
        }
    }

    fn hole(&self, hole: &str) -> Option<String> {
        let hole = hole.trim();
        if self
            .imported("$app/paths", "base")
            .iter()
            .any(|b| b == hole)
        {
            return Some(self.app.base.clone());
        }
        helper::call(self.app, self.file, self.text, hole)
    }

    /// The screen an absolute path resolves to, else why it does not; `None` for
    /// an external URL or an anchor.
    fn resolve(&self, path: &str) -> Option<Result<Target, String>> {
        let path = path.split(['?', '#']).next().unwrap_or("");
        let external = ["//", "mailto:", "tel:", "javascript:"];
        if path.is_empty() || path.contains("://") || external.iter().any(|p| path.starts_with(p)) {
            return None;
        }
        let base = &self.app.base;
        let path = match path.strip_prefix(base.as_str()) {
            Some("") if !base.is_empty() => "/",
            Some(rest) if !base.is_empty() && rest.starts_with('/') => rest,
            _ => path,
        };
        if !path.starts_with('/') {
            let message = format!(
                "Relative navigation {path} depends on the current URL; its target is unknown."
            );
            return Some(Err(message));
        }
        let candidates = (self.pages.iter())
            .flat_map(|p| p.variants.iter().map(move |v| (&p.screen.id, v.as_str())));
        Some(match best_route(path, candidates) {
            Ok(id) => Ok(Target::Screen(id.clone())),
            Err(tied) if tied.is_empty() => Err(format!(
                "Navigation target {path} matches no route of application {}.",
                self.app.name
            )),
            Err(tied) => Err(format!(
                "Navigation target {path} matches several routes equally: {}.",
                tied.iter()
                    .map(|id| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        })
    }
}

/// Offset of the `(` of a call of `name` at `pos`, when it is one.
fn call_open(text: &str, pos: usize, name: &str) -> Option<usize> {
    let open = skip_ws(text.as_bytes(), pos + name.len());
    let call = global_call(text, pos) && !text[..pos].ends_with('#');
    (text.as_bytes().get(open) == Some(&b'(') && call).then_some(open)
}

/// Arguments of the call whose `(` is at `open`.
fn args(text: &str, open: usize) -> Vec<Item> {
    Parser { s: text, i: open }.array(0)
}

/// Route helpers one hop away (see the module documentation).
mod helper {
    use super::*;

    /// The path a helper call `expr` returns.
    pub(super) fn call(app: &App, file: &str, text: &str, expr: &str) -> Option<String> {
        let (name, _) = call_parts(expr.trim())?;
        let (obj, member) = match name.split_once('.') {
            Some((o, m)) if !m.contains('.') => (o, Some(m)),
            None => (name, None),
            _ => return None,
        };
        let (mtext, local) = if declared(text, obj).is_some() {
            (text, obj.to_string())
        } else {
            let import = imports(text).into_iter().find(|i| i.local == obj)?;
            let exported = import.imported.filter(|n| n != "*")?;
            (app.module(file, &import.module)?.1, exported)
        };
        let def = declared(mtext, &local)?;
        let source = match member {
            Some(m) => {
                let Value::Obj(fields) = &def.value else {
                    return None;
                };
                fields.iter().find(|(k, _)| k == m)?.1.source(mtext)
            }
            None => def.source(mtext),
        };
        path(mtext, returned(&source)?)
    }

    pub(super) use crate::lang_ts_decl::declared_value as declared;

    /// The returned expression of a function source: an arrow's expression, or the
    /// final `return` of its (or a function's) block.
    pub(super) fn returned(source: &str) -> Option<&str> {
        let body = match find_top(source, "=>") {
            Some(i) => source[i + 2..].trim(),
            None if source.starts_with('{') => source,
            None => return None,
        };
        if !body.starts_with('{') {
            return Some(body.trim_end_matches([';', ',']).trim());
        }
        let inner = body[1..body.rfind('}')?].trim_end();
        let inner = inner.strip_suffix(';').unwrap_or(inner);
        let at = *words(inner, "return").last()?;
        let expr = inner[at + "return".len()..].trim();
        (!expr.contains(';')).then_some(expr)
    }

    /// A literal or template literal, plus query-only parts; holes become `:param`, a
    /// dropped query part leaves a `?` so that callers cut what follows.
    pub(super) fn path(mtext: &str, expr: &str) -> Option<String> {
        let parts = split_plus(expr);
        let first = Parser { s: parts[0], i: 0 }.value(0).value.literal()?;
        let tails = &parts[1..];
        if !tails.iter().all(|t| query_only(mtext, t, 1)) {
            return None;
        }
        Some(if tails.is_empty() {
            first
        } else {
            format!("{first}?")
        })
    }

    /// Whether the expression only adds a query or fragment.
    fn query_only(mtext: &str, expr: &str, depth: usize) -> bool {
        let e = unparen(expr);
        if let Some(q) = find_top(e, " ? ") {
            return find_top(&e[q + 3..], " : ").is_some_and(|c| {
                let (yes, no) = (&e[q + 3..q + 3 + c], &e[q + 3 + c + 3..]);
                query_only(mtext, yes, depth) && query_only(mtext, no, depth)
            });
        }
        match (Parser { s: e, i: 0 }).value(0).value {
            Value::Str(s) => s.is_empty() || s.starts_with(['?', '#']),
            Value::Tpl(t) => t.starts_with(['?', '#']),
            _ if depth == 0 => false,
            _ => call_parts(e).is_some_and(|(f, _)| {
                is_ident(f)
                    && (declared(mtext, f).map(|d| d.source(mtext))).is_some_and(|s| {
                        returned(&s).is_some_and(|r| query_only(mtext, r, depth - 1))
                    })
            }),
        }
    }
}

/// `if` guards around a `redirect(…)` and the requirements of `load` functions.
mod guard {
    use super::*;

    /// The `if` condition whose `)` is at `close`.
    fn if_cond(text: &str, close: usize) -> Option<String> {
        let open = matching_back(text.as_bytes(), close)?;
        let kw = trailing_ident(text[..open].trim_end());
        (kw.as_deref() == Some("if")).then(|| text[open + 1..close].trim().to_string())
    }

    /// Offsets of the `{` of every block holding `pos`, innermost first.
    pub(super) fn enclosing(text: &str, pos: usize) -> Vec<usize> {
        let mut stack = Vec::new();
        scan_brackets(text, 0, (b'{', b'}'), |i, c, _| {
            if i >= pos {
                return true;
            }
            if c == b'{' {
                stack.push(i);
            } else {
                stack.pop();
            }
            false
        });
        stack.reverse();
        stack
    }

    /// The `if` conditions between the call at `pos` and its function; `Err` when a
    /// block other than `if`/`try` holds it.
    pub(super) fn guards(text: &str, pos: usize) -> Result<Vec<String>, String> {
        let mut conds = Vec::new();
        let mut before = text[..pos].trim_end();
        for kw in ["throw", "await", "return"] {
            if let Some(s) = before.strip_suffix(kw)
                && !s.ends_with(is_ident_char)
            {
                before = s.trim_end();
            }
        }
        if before.ends_with(')') {
            conds.extend(if_cond(text, before.len() - 1));
        }
        if before.ends_with("=>") {
            return callback(text, before.len() - 2).map(|()| conds);
        }
        for open in enclosing(text, pos) {
            let prev = text[..open].trim_end();
            if prev.ends_with("=>") {
                return callback(text, prev.len() - 2).map(|()| conds);
            }
            if prev.ends_with("else") {
                return Err("redirect in an else block".into());
            }
            if prev.ends_with(')') {
                match if_cond(text, prev.len() - 1) {
                    Some(c) => conds.push(c),
                    None => {
                        let paren = matching_back(text.as_bytes(), prev.len() - 1)
                            .ok_or("unbalanced parentheses")?;
                        let kw = trailing_ident(text[..paren].trim_end()).unwrap_or_default();
                        if ["for", "while", "switch", "catch"].contains(&kw.as_str()) {
                            return Err(format!("redirect in a {kw} block"));
                        }
                        break;
                    }
                }
            }
        }
        Ok(conds)
    }

    /// `Err` when the arrow whose `=>` starts at `arrow` is a call argument (a
    /// callback such as `.catch(() => redirect(…))`, which runs only sometimes).
    fn callback(text: &str, arrow: usize) -> Result<(), String> {
        let params = text[..arrow].trim_end();
        let start = match params.strip_suffix(')') {
            Some(_) => matching_back(text.as_bytes(), params.len() - 1).unwrap_or(0),
            None => params.len() - trailing_ident(params).map_or(0, |p| p.len()),
        };
        let before = text[..start].trim_end();
        let before = before.strip_suffix("async").unwrap_or(before).trim_end();
        // `f(() => …)` or `f(x, () => …)`, not a parenthesized `(async () => …)`.
        let argument = before.ends_with(',')
            || (before.strip_suffix('(')).is_some_and(|callee| {
                let callee = callee.trim_end();
                callee.ends_with([')', ']'])
                    || trailing_ident(callee).is_some_and(|w| w != "return")
            });
        match argument {
            true => Err("redirect in a callback".into()),
            false => Ok(()),
        }
    }

    /// The guard condition of the `redirect(` at `pos`: none without a guard.
    pub(super) fn condition(
        checks: &Checks,
        (file, text, pos): (&str, &str, usize),
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> (bool, Option<Condition>) {
        let line = line_of(text, pos);
        match guards(text, pos) {
            Ok(texts) if texts.is_empty() => (false, None),
            Ok(texts) => (true, checks.checks(&texts, (file, line), out)),
            Err(why) => {
                let message = format!("{why}; its condition is unknown.");
                diag(out, file, line, message);
                (true, Some(Condition::Unknown(why)))
            }
        }
    }

    /// The source range of the function or constant `name`: the expression of a
    /// `const`/`let`, the body of a `function`.
    pub(super) fn decl_range(text: &str, name: &str, exported: bool) -> Option<(usize, usize)> {
        let b = text.as_bytes();
        words(text, name).into_iter().find_map(|pos| {
            let before = text[..pos].trim_end();
            if exported && !before.contains("export") {
                return None;
            }
            if before.ends_with("const") || before.ends_with("let") {
                let eq = pos + text[pos..].find('=')?;
                let item = Parser { s: text, i: eq + 1 }.raw(skip_ws(b, eq + 1));
                return Some((item.at, item.end));
            }
            if before.ends_with("function") {
                let open = pos + text[pos..].find('(')?;
                let body = close(text, open) + text[close(text, open)..].find('{')?;
                return Some((body, close(text, body)));
            }
            None
        })
    }

    /// The braces of the function written at `at..end` (`{…}` of a function body, an
    /// arrow with a block or a `function` expression); an arrow with an expression
    /// body gives an empty range; `None` when it is no function.
    pub(super) fn body_range(text: &str, (at, end): (usize, usize)) -> Option<(usize, usize)> {
        let s = text.get(at..end)?;
        if s.starts_with('{') {
            return Some((at, end));
        }
        if let Some(arrow) = s.find("=>") {
            let i = skip_ws(text.as_bytes(), at + arrow + 2);
            return Some(match text.as_bytes().get(i) {
                Some(b'{') => (i, close(text, i)),
                _ => (at, at),
            });
        }
        let kw = s.find("function")?;
        let open = at + kw + s[kw..].find('(')?;
        let body = close(text, open) + text[close(text, open)..].find('{')?;
        Some((body, close(text, body)))
    }

    /// The `load` function's source range.
    fn load_range(text: &str) -> Option<(usize, usize)> {
        decl_range(text, "load", true)
    }

    /// Calls `f(…)` written as a statement (the result is dropped), in `range`:
    /// the name, the offset of the name and of its `(`.
    fn statement_calls(text: &str, (start, end): (usize, usize)) -> Vec<(String, usize, usize)> {
        const KEYWORDS: [&str; 9] = [
            "if", "for", "while", "switch", "catch", "return", "function", "typeof", "throw",
        ];
        let b = text.as_bytes();
        let mut found = Vec::new();
        let mut i = start;
        while i < end {
            if !is_ident_char(b[i] as char) || (i > start && is_ident_char(b[i - 1] as char)) {
                i += 1;
                continue;
            }
            let name = ident_at(text, i);
            let after = skip_ws(b, i + name.len());
            let mut before = text[start..i].trim_end();
            if let Some(s) = before.strip_suffix("await")
                && !s.ends_with(is_ident_char)
            {
                before = s.trim_end();
            }
            let statement = before.is_empty() || before.ends_with([';', '{', '}']);
            if statement && b.get(after) == Some(&b'(') && !KEYWORDS.contains(&name) {
                found.push((name.to_string(), i, after));
            }
            i += name.len().max(1);
        }
        found
    }

    /// What the `load` function of a `+layout`/`+page` file requires, in text order:
    /// each requirement is `Protected`, or `Open` for a guard `[views.access]` lists
    /// as open.
    pub(super) fn requirements(
        app: &App,
        checks: &Checks,
        (file, text): (&str, &str),
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Vec<AccessState> {
        let Some(range) = load_range(text) else {
            return vec![];
        };
        scan(app, checks, (file, text, range), true, out)
            .into_iter()
            .map(|(_, s)| s)
            .collect()
    }

    type Found = Vec<(usize, AccessState)>;

    /// The requirements inside `range` of `text`, by offset. `hop`: helpers called
    /// as statements are read one level deep.
    fn scan(
        app: &App,
        checks: &Checks,
        (file, text, range): (&str, &str, (usize, usize)),
        hop: bool,
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Found {
        let at = (file, text, range);
        let mut found = blocking_calls(checks, at, out);
        found.extend(listed_calls(checks, at, out));
        if hop {
            found.extend(helper_calls(app, checks, at, out));
        }
        found.sort_by_key(|(pos, _)| *pos);
        found
    }

    /// `redirect(…)`/`error(…)` of `@sveltejs/kit` under their guards.
    fn blocking_calls(
        checks: &Checks,
        (file, text, (start, end)): (&str, &str, (usize, usize)),
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Found {
        let mut found = Vec::new();
        let blockers = (imports(text).into_iter())
            .filter(|i| {
                i.module == "@sveltejs/kit"
                    && matches!(i.imported.as_deref(), Some("redirect" | "error"))
            })
            .map(|i| i.local);
        for name in blockers {
            for pos in words(text, &name)
                .into_iter()
                .filter(|p| (start..end).contains(p))
            {
                if call_open(text, pos, &name).is_none() {
                    continue;
                }
                if let (true, Some(c)) = condition(checks, (file, text, pos), out) {
                    // a guard that never holds never blocks: an explicit open marker
                    let state = negate(Some(c)).map_or(AccessState::Open, AccessState::Protected);
                    found.push((pos, state));
                }
            }
        }
        found
    }

    /// Calls of guards `[views.access]` lists.
    fn listed_calls(
        checks: &Checks,
        (file, text, (start, end)): (&str, &str, (usize, usize)),
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Found {
        use AccessState::Protected as P;
        let mut found = Vec::new();
        for (name, meaning) in checks.access.iter().filter(|(n, _)| is_ident(n)) {
            for pos in words(text, name)
                .into_iter()
                .filter(|p| (start..end).contains(p))
            {
                let Some(open) = call_open(text, pos, name) else {
                    continue;
                };
                let call = &text[pos..close(text, open)];
                let fixed = !matches!(
                    meaning,
                    AccessMeaning::RoleFromArguments
                        | AccessMeaning::PolicyFromArguments
                        | AccessMeaning::FromArgumentsOr(..)
                );
                let options = (args(text, open).iter()).any(|a| matches!(a.value, Value::Obj(_)));
                let message = if fixed && options {
                    format!(
                        "Guard call {call} passes an options object; one fixed meaning from [views.access] cannot hold for every call site, the condition is unknown."
                    )
                } else {
                    match access_condition(checks.access, name, &args(text, open)) {
                        Ok(s) => {
                            found.push((pos, s));
                            continue;
                        }
                        Err(n) => format!(
                            "Guard call {n} has no string literal arguments for its $arg meaning; the condition is unknown."
                        ),
                    }
                };
                diag(out, file, line_of(text, pos), message);
                found.push((pos, P(Condition::Unknown(call.into()))));
            }
        }
        found
    }

    /// Helpers called as statements, read one hop deep or unknown.
    fn helper_calls(
        app: &App,
        checks: &Checks,
        (file, text, range): (&str, &str, (usize, usize)),
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Found {
        let mut found = Vec::new();
        for (name, pos, open) in statement_calls(text, range) {
            if checks.access.contains_key(&name) {
                continue;
            }
            let call = text[pos..close(text, open)].to_string();
            match helper_range(app, (file, text), &name) {
                Helper::None => {}
                Helper::Unreadable => {
                    let message = format!(
                        "Helper {call} is not found or not a function in the scanned sources; whether it guards is unknown."
                    );
                    diag(out, file, line_of(text, pos), message);
                    found.push((pos, AccessState::Protected(Condition::Unknown(call))));
                }
                Helper::At(hfile, htext, hrange) => {
                    let inner = scan(app, checks, (&hfile, &htext, hrange), false, out);
                    found.extend(inner.into_iter().map(|(_, s)| (pos, s)));
                }
            }
        }
        found
    }

    enum Helper {
        /// Not a helper of the application (a parameter, a package function).
        None,
        Unreadable,
        At(String, String, (usize, usize)),
    }

    /// The declaration of the function `name` called in `file`: its own, or one
    /// of the module it is imported from.
    fn helper_range(app: &App, (file, text): (&str, &str), name: &str) -> Helper {
        let Some(import) = imports(text).into_iter().find(|i| i.local == name) else {
            return match decl_range(text, name, false) {
                Some(range) => Helper::At(file.into(), text.into(), range),
                None => Helper::None,
            };
        };
        let local = import.module.starts_with('.') || import.module.starts_with("$lib");
        let found = app.module(file, &import.module);
        if !local && found.is_none() {
            return Helper::None;
        }
        let range = import
            .imported
            .as_deref()
            .filter(|i| *i != "*")
            .and_then(|i| {
                let (f, t) = found?;
                Some((f, t, decl_range(t, i, false)?))
            });
        match range {
            Some((f, t, range)) => Helper::At(f.into(), t.into(), range),
            None => Helper::Unreadable,
        }
    }
}

/// `hooks.server` rules of the `handle` function.
mod hooks {
    use super::*;

    enum Op {
        Prefix(String),
        Exact(String),
        In(Vec<String>),
    }

    /// One conjunct that selects routes: on the pathname, or on `route.id` (with
    /// its groups); `neg` for `!`/`!==`.
    struct Sel {
        id: bool,
        neg: bool,
        op: Op,
    }

    impl Sel {
        fn holds(&self, route: &str, id: &str) -> bool {
            let s = if self.id { id } else { route };
            let hit = match &self.op {
                Op::Prefix(p) => s.starts_with(p.as_str()),
                Op::Exact(p) => s == p,
                Op::In(list) => list.iter().any(|p| p == s),
            };
            hit != self.neg
        }

        /// `!list.includes(…)`: failing it means the route is on a read allowlist.
        fn allowlist(&self) -> bool {
            self.neg && matches!(self.op, Op::In(_))
        }
    }

    enum Kind {
        /// The routes the selectors match are blocked unless the requirement holds
        /// (`None`: the rule needs nothing).
        Block(Option<Condition>),
        /// `return resolve(event)`: the matched routes skip every later rule.
        Pass,
    }

    pub(super) struct Rule {
        sels: Vec<Sel>,
        kind: Kind,
    }

    /// Whether `s` is the route id (`true`) or the pathname (`false`).
    fn subject(s: &str) -> Option<bool> {
        let s = s.trim().trim_end_matches('?').replace("?.", ".");
        if s.ends_with("route.id") {
            Some(true)
        } else if s == "pathname" || s.ends_with(".pathname") {
            Some(false)
        } else {
            None
        }
    }

    fn literal(s: &str) -> Option<String> {
        match (Parser { s: s.trim(), i: 0 }).value(0).value {
            Value::Str(v) => Some(v),
            _ => None,
        }
    }

    /// The strings of the array literal `list`, or of the constant it names in `text`.
    fn string_list(text: &str, list: &str) -> Option<Vec<String>> {
        let items = match (Parser {
            s: list.trim(),
            i: 0,
        })
        .value(0)
        .value
        {
            Value::Arr(items) => items,
            Value::Raw(name) if is_name(&name) => match declared_value(text, &name)?.value {
                Value::Arr(items) => items,
                _ => return None,
            },
            _ => return None,
        };
        (items.into_iter())
            .map(|i| match i.value {
                Value::Str(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    /// The part of an `if` condition `part` as a route selector.
    fn selector(text: &str, part: &str) -> Option<Sel> {
        let t = unparen(part);
        let (neg, t) = match t.strip_prefix('!') {
            Some(r) if !r.starts_with('=') => (true, unparen(r)),
            _ => (false, t),
        };
        if let Some((left, rest)) = t.split_once(".startsWith(") {
            let op = Op::Prefix(literal(rest.strip_suffix(')')?)?);
            return Some(Sel {
                id: subject(left)?,
                neg,
                op,
            });
        }
        if let Some((list, rest)) = t.split_once(".includes(") {
            let op = Op::In(string_list(text, list)?);
            return Some(Sel {
                id: subject(rest.strip_suffix(')')?)?,
                neg,
                op,
            });
        }
        let compare = [("!==", true), ("!=", true), ("===", false), ("==", false)];
        let (left, right, flip) = compare
            .iter()
            .find_map(|(op, flip)| t.split_once(op).map(|(l, r)| (l, r, *flip)))?;
        Some(Sel {
            id: subject(left)?,
            neg: neg ^ flip,
            op: Op::Exact(literal(right)?),
        })
    }

    /// Reads the rules of every `handle` of the application's hooks files.
    pub(super) fn rules(app: &App, access: &Access, out: &mut Vec<UiFlowDiagnostic>) -> Vec<Rule> {
        let checks = Checks { access };
        let mut reader = Reader {
            app,
            checks: &checks,
            out,
            rules: Vec::new(),
        };
        for name in [
            "hooks.server.ts",
            "hooks.server.js",
            "hooks.server/index.ts",
        ]
        .into_iter()
        .chain(["hooks.server/index.js"])
        {
            let file = format!("{}src/{name}", app.prefix);
            if let Some(text) = app.files.get(&file) {
                reader.named(&file, text, "handle", true);
            }
        }
        reader.rules
    }

    struct Reader<'a> {
        app: &'a App,
        checks: &'a Checks<'a>,
        out: &'a mut Vec<UiFlowDiagnostic>,
        rules: Vec<Rule>,
    }

    impl Reader<'_> {
        /// Everything is unknown: a rule that blocks unless the unreadable part holds.
        fn unknown(&mut self, file: &str, line: u32, why: &str) {
            let message = format!("{why}; every route it could cover is unknown.");
            diag(self.out, file, line, message);
            let kind = Kind::Block(Some(Condition::Unknown(why.into())));
            self.rules.push(Rule { sels: vec![], kind });
        }

        /// Reads the handle `name` declared in (or re-exported from) `file`; `follow`
        /// allows one hop through an import. `false` when the file has none.
        fn named(&mut self, file: &str, text: &str, name: &str, follow: bool) -> bool {
            if let Some(range) = guard::decl_range(text, name, false) {
                let source = &text[range.0..range.1];
                if let Some(rest) = source.strip_prefix("sequence")
                    && rest.trim_start().starts_with('(')
                {
                    let open = range.0 + source.find('(').unwrap_or(0);
                    for arg in (Parser { s: text, i: open }).array(0) {
                        self.operand(file, text, &arg, follow);
                    }
                } else {
                    self.function(file, text, range);
                }
                return true;
            }
            let reexport = words(text, "export").into_iter().find_map(|p| {
                let stmt = text[p..].split(';').next()?.lines().next()?;
                let (names, from) = stmt.split_once(" from ")?;
                let hit = names.contains('{') && !words(names, name).is_empty();
                let spec = from.trim().trim_matches(['\'', '"', ';']).to_string();
                hit.then(|| (spec, line_of(text, p)))
            });
            let Some((module, line)) = reexport else {
                return false;
            };
            self.external(file, line, &module, name, follow);
            true
        }

        /// The handle `imported` of module `spec`, imported by `file`.
        fn external(&mut self, file: &str, line: u32, spec: &str, imported: &str, follow: bool) {
            let target = self.app.module(file, spec).filter(|_| follow);
            let Some((mfile, mtext)) = target else {
                let why =
                    format!("handle `{imported}` of '{spec}' is not found in the scanned sources");
                return self.unknown(file, line, &why);
            };
            // Auth.js: `handle` only attaches `locals.auth()`; the library protects nothing.
            if imported == "handle" && mtext.contains("SvelteKitAuth(") {
                return;
            }
            if !self.named(mfile, mtext, imported, false) {
                let why = format!("handle `{imported}` is not declared in {mfile}");
                self.unknown(file, line, &why);
            }
        }

        /// One argument of `sequence(…)`.
        fn operand(&mut self, file: &str, text: &str, arg: &Item, follow: bool) {
            let source = arg.source(text);
            let line = line_of(text, arg.at);
            if is_name(&source) {
                if guard::decl_range(text, &source, false).is_some() {
                    self.named(file, text, &source, follow);
                } else if let Some(i) = imports(text).into_iter().find(|i| i.local == source) {
                    let imported = i.imported.unwrap_or_default();
                    self.external(file, line, &i.module, &imported, follow);
                } else {
                    let why = format!("handle `{source}` is not declared");
                    self.unknown(file, line, &why);
                }
            } else if source.contains("=>") || source.contains("function") {
                self.function(file, text, (arg.at, arg.end));
            } else {
                let why = format!("handle `{source}` is not a function");
                self.unknown(file, line, &why);
            }
        }

        /// The rules of the function written at `range`.
        fn function(&mut self, file: &str, text: &str, range: (usize, usize)) {
            let Some((open, end)) = guard::body_range(text, range) else {
                let why = "handle is not a function".to_string();
                return self.unknown(file, line_of(text, range.0), &why);
            };
            if end <= open {
                return;
            }
            let mut seen = Vec::new();
            self.block(file, text, (open, end), &[], &mut seen);
            for pos in blocking(text, (open, end)) {
                if !seen.contains(&pos) {
                    let why = "A redirect, error or response of handle is not under an `if` the rules read";
                    self.unknown(file, line_of(text, pos), why);
                }
            }
        }

        /// The `if` statements directly in the block `{…}` of `text`; `prefix` are the
        /// conditions of the blocks around it.
        fn block(
            &mut self,
            file: &str,
            text: &str,
            (open, end): (usize, usize),
            prefix: &[String],
            seen: &mut Vec<usize>,
        ) {
            let b = text.as_bytes();
            for pos in words(text, "if") {
                if pos <= open || pos >= end || guard::enclosing(text, pos).first() != Some(&open) {
                    continue;
                }
                let before = text[..pos].trim_end();
                if !(before.ends_with([';', '{', '}']) && !before.ends_with("else")) {
                    continue;
                }
                let paren = skip_ws(b, pos + 2);
                if b.get(paren) != Some(&b'(') {
                    continue;
                }
                let cond_end = close(text, paren);
                let cond = text[paren + 1..cond_end - 1].trim().to_string();
                let at = skip_ws(b, cond_end);
                let braced = b.get(at) == Some(&b'{');
                let body_end = if braced {
                    close(text, at)
                } else {
                    text[at..].find(';').map_or(text.len(), |e| at + e + 1)
                };
                let direct: Vec<usize> = (blocking(text, (at, body_end)).into_iter())
                    .filter(|&p| !braced || guard::enclosing(text, p).first() == Some(&at))
                    .collect();
                let mut conds = prefix.to_vec();
                conds.push(cond);
                let line = line_of(text, pos);
                if !direct.is_empty() {
                    seen.extend(direct);
                    self.rule(file, text, &conds, (line, false));
                } else if passes(&text[at..body_end]) {
                    self.rule(file, text, &conds, (line, true));
                }
                if braced {
                    self.block(file, text, (at, body_end), &conds, seen);
                }
            }
        }

        /// A rule from the `&&` conjuncts of `conds`: route selectors and checks.
        fn rule(&mut self, file: &str, text: &str, conds: &[String], (line, pass): (u32, bool)) {
            let (mut sels, mut rest) = (Vec::new(), Vec::new());
            for part in conds.iter().flat_map(|c| split_top(c, "&&")) {
                match selector(text, part) {
                    Some(s) => sels.push(s),
                    None => rest.push(unparen(part).to_string()),
                }
            }
            if pass {
                // an early return that is not purely a route selection proves nothing
                if rest.is_empty() {
                    self.rules.push(Rule {
                        sels,
                        kind: Kind::Pass,
                    });
                }
                return;
            }
            let requirement = match rest.is_empty() {
                true => Some(Condition::Never),
                false => negate(self.checks.checks(&rest, (file, line), self.out)),
            };
            self.rules.push(Rule {
                sels,
                kind: Kind::Block(requirement),
            });
        }
    }

    /// `return resolve(…)` and nothing else.
    fn passes(body: &str) -> bool {
        let t = body.trim().trim_start_matches('{').trim_end_matches('}');
        let t = t.trim().trim_end_matches(';');
        t.strip_prefix("return").is_some_and(|r| {
            let r = r.trim_start();
            let r = r.strip_prefix("await ").unwrap_or(r);
            r.starts_with("resolve(") && r.ends_with(')')
        })
    }

    /// Offsets of the blocking calls in `range`: `redirect(…)`, `error(…)`, and a
    /// `Response`/`json(…)` with a 40x status.
    fn blocking(text: &str, (start, end): (usize, usize)) -> Vec<usize> {
        let mut found = Vec::new();
        for f in ["redirect", "error", "Response", "json"] {
            for pos in words(text, f) {
                if !(start..end).contains(&pos) || text[..pos].ends_with('.') {
                    continue;
                }
                let Some(open) = call_open(text, pos, f) else {
                    continue;
                };
                let blocks = match f {
                    "Response" | "json" => {
                        let call = &text[open..close(text, open)];
                        words(call, "status").into_iter().any(|s| {
                            let rest = call[s + 6..].trim_start_matches([':', '=', ' ']);
                            rest.starts_with("40")
                        })
                    }
                    _ => true,
                };
                if blocks {
                    found.push(pos);
                }
            }
        }
        found
    }

    /// The state hooks give the route `route` (route id `id`): the conditions of
    /// every rule that selects it, up to a rule that lets it pass; `Open` only when
    /// a read allowlist exempts it from a blocking rule and nothing else applies.
    pub(super) fn state(rules: &[Rule], route: &str, id: &str) -> AccessState {
        let (mut conditions, mut allowed) = (Vec::new(), false);
        for rule in rules {
            let fails: Vec<&Sel> = (rule.sels.iter()).filter(|s| !s.holds(route, id)).collect();
            match &rule.kind {
                Kind::Pass if fails.is_empty() => break,
                Kind::Block(Some(c)) if fails.is_empty() => conditions.push(c.clone()),
                Kind::Block(Some(_)) if fails.iter().all(|s| s.allowlist()) => allowed = true,
                _ => {}
            }
        }
        AccessState::from_parts(all(conditions), allowed)
    }
}

/// `+server` exports and `+page.server` form actions.
mod endpoints {
    use super::*;

    pub(super) fn extract(app: &App, rules: &[hooks::Rule], endpoints: &mut Vec<EndpointAccess>) {
        for (file, text) in &app.files {
            let Some((dir, name)) = app.route_file(file) else {
                continue;
            };
            let (route, _) = route_of(dir);
            let found = if name.starts_with("+server.") {
                server(text, &route)
            } else if name.starts_with("+page.server.") {
                actions(text, &route)
            } else {
                continue;
            };
            let access = hooks::state(rules, &route, &format!("/{dir}"));
            endpoints.extend(found.into_iter().map(|(method, path, pos)| EndpointAccess {
                method,
                path,
                access: access.clone(),
                application: Some(app.name.clone()),
                file: file.clone(),
                line: line_of(text, pos),
                source: SOURCE.into(),
                handler: None,
                sends: Vec::new(),
            }));
        }
    }

    /// `export const GET = …`, `export function POST(…)`, …; `fallback` is `ANY`.
    fn server(text: &str, route: &str) -> Vec<(String, String, usize)> {
        let methods = HTTP_METHODS.map(str::to_ascii_uppercase).into_iter();
        let mut found = Vec::new();
        for m in methods.chain(["fallback".to_string()]) {
            for pos in words(text, &m) {
                let before = text[..pos].trim_end();
                let decl = ["const", "let", "function"]
                    .iter()
                    .any(|k| before.ends_with(k));
                if decl && before.contains("export") {
                    let method = if m == "fallback" {
                        ANY_METHOD.into()
                    } else {
                        m.clone()
                    };
                    found.push((method, route.to_string(), pos));
                }
            }
        }
        found
    }

    /// `export const actions = { default, x }`: `POST <route>` and `POST <route>?/x`.
    fn actions(text: &str, route: &str) -> Vec<(String, String, usize)> {
        let Some(Item {
            value: Value::Obj(fields),
            ..
        }) = helper::declared(text, "actions")
        else {
            return vec![];
        };
        (fields.into_iter())
            .map(|(key, item)| {
                let path = match key.as_str() {
                    "default" => route.to_string(),
                    _ => format!("{route}?/{key}"),
                };
                ("POST".to_string(), path, item.at)
            })
            .collect()
    }
}

/// Svelte markup: start tags, attributes, labels and trigger spans.
mod markup {
    use super::*;

    pub(super) struct Tag {
        pub(super) at: usize,
        pub(super) name: String,
        pub(super) attrs: Vec<Attr>,
        /// Offset after `>`.
        pub(super) end: usize,
        pub(super) self_closing: bool,
    }

    pub(super) struct Attr {
        pub(super) name: String,
        pub(super) value: AttrValue,
        /// Offset of the value's content.
        pub(super) at: usize,
    }

    pub(super) enum AttrValue {
        /// Quoted text; `{…}` parts are expressions.
        Text(String),
        /// `{…}` content.
        Expr(String),
        Bare,
    }

    impl Tag {
        pub(super) fn attr(&self, name: &str) -> Option<&Attr> {
            self.attrs.iter().find(|a| a.name == name)
        }
    }

    impl AttrValue {
        /// A literal, template or helper call value as `lang_ts` reads it.
        pub(super) fn value(&self) -> Value {
            match self {
                AttrValue::Expr(e) => Parser { s: e, i: 0 }.value(0).value,
                AttrValue::Text(t) if t.contains('{') => Value::Tpl(t.replace('{', "${")),
                AttrValue::Text(t) => Value::Str(t.clone()),
                AttrValue::Bare => Value::Raw(String::new()),
            }
        }

        pub(super) fn source(&self) -> &str {
            match self {
                AttrValue::Expr(s) | AttrValue::Text(s) => s.trim(),
                AttrValue::Bare => "",
            }
        }
    }

    pub(super) fn tags(markup: &str) -> Vec<Tag> {
        let b = markup.as_bytes();
        (markup.match_indices('<'))
            .filter(|(at, _)| b.get(at + 1).is_some_and(u8::is_ascii_alphabetic))
            .map(|(at, _)| tag_at(markup, at))
            .collect()
    }

    /// One step through a start tag's attributes.
    enum Step {
        End(usize, bool),
        Attr(Attr, usize),
        Skip(usize),
    }

    /// The start tag whose `<` is at `at`.
    fn tag_at(markup: &str, at: usize) -> Tag {
        let len = markup[at + 1..]
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(markup.len() - at - 1);
        let name = markup[at + 1..at + 1 + len].to_string();
        let (mut i, mut attrs) = (at + 1 + len, Vec::new());
        loop {
            match attribute(markup, i) {
                Step::End(end, self_closing) => {
                    return Tag {
                        at,
                        name,
                        attrs,
                        end,
                        self_closing,
                    };
                }
                Step::Attr(attr, next) => {
                    attrs.push(attr);
                    i = next;
                }
                Step::Skip(next) => i = next,
            }
        }
    }

    /// The attribute (or tag end) after whitespace at `i`.
    fn attribute(markup: &str, i: usize) -> Step {
        let b = markup.as_bytes();
        let i = skip_ws(b, i);
        match b.get(i) {
            None => return Step::End(b.len(), false),
            Some(b'>') => return Step::End(i + 1, false),
            Some(b'/') if b.get(i + 1) == Some(&b'>') => return Step::End(i + 2, true),
            Some(b'{') => {
                // `{name}` shorthand or `{...spread}`.
                let (value, at, next) = value_at(markup, i);
                let name = value.source().to_string();
                return Step::Attr(Attr { name, value, at }, next);
            }
            _ => {}
        }
        let n = markup[i..]
            .find(|c: char| c.is_whitespace() || matches!(c, '=' | '>' | '/'))
            .unwrap_or(markup.len() - i);
        if n == 0 {
            return Step::Skip(i + markup[i..].chars().next().map_or(1, char::len_utf8));
        }
        let name = markup[i..i + n].to_string();
        let eq = skip_ws(b, i + n);
        if b.get(eq) != Some(&b'=') {
            return Step::Attr(
                Attr {
                    name,
                    value: AttrValue::Bare,
                    at: eq,
                },
                eq,
            );
        }
        let (value, at, next) = value_at(markup, skip_ws(b, eq + 1));
        Step::Attr(Attr { name, value, at }, next)
    }

    /// The value at `i`, the offset of its content and the offset after it.
    fn value_at(markup: &str, i: usize) -> (AttrValue, usize, usize) {
        let b = markup.as_bytes();
        match b.get(i) {
            Some(b'{') => {
                let e = close(markup, i);
                let inner = markup.get(i + 1..e.saturating_sub(1)).unwrap_or("");
                (AttrValue::Expr(inner.into()), i + 1, e)
            }
            Some(&q @ (b'"' | b'\'')) => {
                // The closing quote outside `{…}`: `title="{a ? "x" : "y"}"`.
                let (mut j, mut depth) = (i + 1, 0usize);
                while j < b.len() && !(b[j] == q && depth == 0) {
                    depth = match b[j] {
                        b'{' => depth + 1,
                        b'}' => depth.saturating_sub(1),
                        _ => depth,
                    };
                    j += 1;
                }
                let text = AttrValue::Text(markup[i + 1..j].into());
                (text, i + 1, (j + 1).min(b.len()))
            }
            _ => {
                let e = markup[i..]
                    .find(|c: char| c.is_whitespace() || c == '>')
                    .map_or(markup.len(), |e| i + e);
                (AttrValue::Text(markup[i..e].into()), i, e)
            }
        }
    }

    /// `aria-label`, `data-testid` or, unless `attrs_only`, the plain text of the element.
    pub(super) fn label(markup: &str, tag: &Tag, attrs_only: bool) -> Option<String> {
        let named = ["aria-label", "data-testid"]
            .into_iter()
            .find_map(|key| match tag.attr(key) {
                Some(Attr {
                    value: AttrValue::Text(s),
                    ..
                }) if !s.trim().is_empty() && !s.contains('{') => Some(s.trim().to_string()),
                _ => None,
            });
        if named.is_some() || tag.self_closing || attrs_only {
            return named;
        }
        let inner = markup.get(tag.end..)?;
        plain_text(&inner[..inner.find("</").unwrap_or(inner.len())])
    }

    /// Text outside nested tags and `{…}` expressions, whitespace collapsed.
    fn plain_text(inner: &str) -> Option<String> {
        let (mut label, mut in_tag, mut braces) = (String::new(), false, 0usize);
        for c in inner.chars() {
            // `<` and `>` inside `{…}` (`x => x > 1`) are code, not tag bounds.
            match c {
                '{' => braces += 1,
                '}' => braces = braces.saturating_sub(1),
                _ if braces > 0 => {}
                '<' => in_tag = true,
                '>' => in_tag = false,
                c if !in_tag => label.push(c),
                _ => {}
            }
        }
        let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
        (!label.is_empty()).then_some(label)
    }

    /// Code from `start` to `end` runs when the element labeled `label` fires.
    pub(super) struct Span {
        pub(super) start: usize,
        pub(super) end: usize,
        pub(super) label: String,
    }

    /// The handler an event expression names or calls: `save`, `save(x)`,
    /// `() => save(x)`, `async () => await save(x)`.
    fn handler(expr: &str) -> Option<&str> {
        let mut s = expr.trim();
        if let Some(i) = find_top(s, "=>") {
            s = s[i + 2..].trim_start();
            s = s.strip_prefix("await ").unwrap_or(s).trim_start();
        }
        let name = ident_at(s, 0);
        let after = s[name.len()..].trim_start();
        (!name.is_empty() && (after.is_empty() || after.starts_with('('))).then_some(name)
    }

    /// The body of `function name(…) {…}` or of the value `const name = …` binds.
    fn body(text: &str, name: &str) -> Option<(usize, usize)> {
        let b = text.as_bytes();
        words(text, name)
            .into_iter()
            .filter(|&p| !text[..p].ends_with('.'))
            .find_map(|pos| {
                let before = text[..pos].trim_end();
                if before.ends_with("function") {
                    let open = pos + text[pos..].find('(')?;
                    let at = close(text, open) + text[close(text, open)..].find('{')?;
                    return Some((at, close(text, at)));
                }
                if !["const", "let", "var"].iter().any(|k| before.ends_with(k)) {
                    return None;
                }
                let eq = skip_ws(b, pos + name.len());
                (b.get(eq) == Some(&b'=')).then(|| {
                    let item = Parser { s: text, i: eq + 1 }.raw(skip_ws(b, eq + 1));
                    (item.at, item.end)
                })
            })
    }

    pub(super) fn spans(site: &Site) -> Vec<Span> {
        let mut out = Vec::new();
        for tag in site.tags {
            for attr in &tag.attrs {
                let event = attr.name.split('|').next().unwrap_or("");
                let AttrValue::Expr(expr) = &attr.value else {
                    continue;
                };
                if !TRIGGERS.contains(&event) {
                    continue;
                }
                let Some(label) = label(site.markup, tag, false) else {
                    continue;
                };
                let own = Some((attr.at, attr.at + expr.len()));
                let named = handler(expr).and_then(|h| body(site.text, h));
                for (start, end) in [own, named].into_iter().flatten() {
                    out.push(Span {
                        start,
                        end,
                        label: label.clone(),
                    });
                }
            }
        }
        out
    }
}

/// Links, `goto` and `redirect`.
mod nav {
    use super::*;

    struct Nav {
        at: usize,
        source: String,
        path: Option<String>,
        /// A link's own label; calls take theirs from the trigger spans.
        label: Option<String>,
        condition: Option<Condition>,
    }

    pub(super) fn transitions(site: &Site, flow: &mut UiFlow) {
        let mut navs = links(site);
        navs.extend(calls(site, &mut flow.diagnostics));
        for nav in navs {
            push(site, nav, flow);
        }
    }

    /// Elements with an `href`.
    fn links(site: &Site) -> Vec<Nav> {
        (site.tags.iter())
            .filter_map(|tag| {
                let href = tag.attr("href")?;
                Some(Nav {
                    at: tag.at,
                    source: href.value.source().to_string(),
                    path: site.path(&href.value.value()),
                    label: markup::label(site.markup, tag, false),
                    condition: None,
                })
            })
            .collect()
    }

    /// `goto(path)` and `redirect(status, path)` under its guard.
    fn calls(site: &Site, out: &mut Vec<UiFlowDiagnostic>) -> Vec<Nav> {
        let (file, text) = (site.file, site.text);
        let gotos = site.imported("$app/navigation", "goto");
        let redirects = site.imported("@sveltejs/kit", "redirect");
        let named = (gotos.iter().map(|g| (g, false))).chain(redirects.iter().map(|r| (r, true)));
        let mut navs = Vec::new();
        for (name, redirect) in named {
            for pos in words(text, name) {
                let Some(open) = call_open(text, pos, name) else {
                    continue;
                };
                let Some(arg) = args(text, open).into_iter().nth(usize::from(redirect)) else {
                    continue;
                };
                let condition = match redirect {
                    true => guard::condition(site.checks, (file, text, pos), out).1,
                    false => None,
                };
                navs.push(Nav {
                    at: pos,
                    source: arg.source(text),
                    path: site.path(&arg.value),
                    label: None,
                    condition,
                });
            }
        }
        navs
    }

    /// One transition per origin and trigger; an unreadable target is a gap.
    fn push(site: &Site, nav: Nav, flow: &mut UiFlow) {
        let found = match &nav.path {
            Some(path) => site.resolve(path),
            None => Some(Err(format!(
                "Navigation target is not a literal: {}",
                nav.source
            ))),
        };
        let Some(found) = found else { return };
        let line = line_of(site.text, nav.at);
        let target = found.unwrap_or_else(|message| {
            diag(&mut flow.diagnostics, site.file, line, message);
            Target::Unresolved(nav.source.clone())
        });
        let triggers = match &nav.label {
            Some(label) => vec![Some(label.clone())],
            None => site.triggers(nav.at),
        };
        for from in &site.froms {
            flow.transitions
                .extend(triggers.iter().map(|trigger| Transition {
                    weak: false,
                    from: from.clone(),
                    target: target.clone(),
                    trigger: trigger.clone(),
                    file: site.file.into(),
                    line,
                    condition: archguard_core::AccessState::from_parts(
                        nav.condition.clone(),
                        false,
                    ),
                    source: SOURCE.into(),
                }));
        }
    }
}

/// `fetch`, axios, API client functions and forms.
mod http {
    use super::*;

    /// An action at an offset, with the element's own label for forms.
    type Found = (usize, Call, Option<String>);

    pub(super) fn actions(site: &Site, scan: &Scan, clients: &Clients, flow: &mut UiFlow) {
        let out = &mut flow.diagnostics;
        let mut calls = fetches(site, clients, out);
        calls.extend(api_calls(site, scan, out));
        forms(site, &mut calls, out);
        for (pos, call, label) in calls {
            let triggers = match label {
                Some(l) => vec![Some(l)],
                None => site.triggers(pos),
            };
            for from in &site.froms {
                flow.actions.extend(triggers.iter().map(|trigger| Action {
                    weak: false,
                    screen: from.clone(),
                    trigger: trigger.clone(),
                    call: call.clone(),
                    file: site.file.into(),
                    line: line_of(site.text, pos),
                    condition: archguard_core::AccessState::Undeclared,
                    source: SOURCE.into(),
                }));
            }
        }
    }

    /// The call; an unknown one becomes `Call::Unresolved` and a gap.
    fn record(
        site: &Site,
        pos: usize,
        found: Result<Call, String>,
        source: String,
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Found {
        let call = found.unwrap_or_else(|why| {
            let message = format!("HTTP call {why}; the call is unknown: {source}");
            diag(out, site.file, line_of(site.text, pos), message);
            Call::Unresolved(source)
        });
        (pos, call, None)
    }

    /// `fetch(url, { method })`, axios and axios instances.
    fn fetches(site: &Site, clients: &Clients, out: &mut Vec<UiFlowDiagnostic>) -> Vec<Found> {
        let (file, text) = (site.file, site.text);
        let mut calls = Vec::new();
        for pos in words(text, "fetch") {
            let Some(open) = call_open(text, pos, "fetch") else {
                continue;
            };
            let args = args(text, open);
            let Some(url) = args.first() else { continue };
            let found = http_call(fetch_method(text, args.get(1)), url.value.literal(), None);
            calls.push(record(site, pos, found, url.source(text), out));
        }
        let module = |spec: &str| site.app.module(file, spec).map(|(m, _)| m);
        for (recv, base) in clients.receivers(file, text, module) {
            for pos in words(text, &recv) {
                let Some((method, url)) = client_call(text, pos, &recv) else {
                    continue;
                };
                let method = Ok(method.to_ascii_uppercase());
                let found = http_call(method, url.value.literal(), base.as_ref());
                calls.push(record(site, pos, found, url.source(text), out));
            }
        }
        calls
    }

    /// Calls of functions imported from a scanned API client package.
    fn api_calls(site: &Site, scan: &Scan, out: &mut Vec<UiFlowDiagnostic>) -> Vec<Found> {
        let text = site.text;
        let mut calls = Vec::new();
        for import in imports(text) {
            let Some(sdk) = package_of(&import.module).and_then(|p| scan.sdks.get(p)) else {
                continue;
            };
            for (pos, name) in called(text, &import.local, import.imported.as_deref()) {
                match sdk.get(name) {
                    Some(Ok(call)) => calls.push((pos, call.clone(), None)),
                    Some(Err(why)) => {
                        let message = format!(
                            "API client function {name} ({}) does not give a literal method and path: {why}",
                            import.module
                        );
                        diag(out, site.file, line_of(text, pos), message);
                        calls.push((pos, Call::Unresolved(name.into()), None));
                    }
                    None => {}
                }
            }
        }
        calls
    }

    /// Offsets and exported names of the calls `local(…)` (a named import) or
    /// `local.name(…)` (a namespace import).
    fn called<'t>(text: &'t str, local: &str, imported: Option<&'t str>) -> Vec<(usize, &'t str)> {
        let namespace = imported == Some("*");
        let mut found = Vec::new();
        for pos in words(text, local) {
            let (name, at) = match imported {
                Some("*") if text[pos + local.len()..].starts_with('.') => {
                    let at = pos + local.len() + 1;
                    (ident_at(text, at), at)
                }
                Some(n) if n != "*" => (n, pos),
                _ => continue,
            };
            let open = skip_ws(text.as_bytes(), at + ident_at(text, at).len());
            let call = text.as_bytes().get(open) == Some(&b'(');
            if call && (namespace || global_call(text, pos)) {
                found.push((pos, name));
            }
        }
        found
    }

    /// `<form method="post" action="…">`: the screen's own route for `?/x` and none.
    fn forms(
        site: &Site,
        calls: &mut Vec<(usize, Call, Option<String>)>,
        out: &mut Vec<UiFlowDiagnostic>,
    ) {
        let own = match site.froms.as_slice() {
            [Origin::Screen(id)] => site
                .pages
                .iter()
                .find(|p| &p.screen.id == id)
                .and_then(|p| p.screen.route.clone()),
            _ => None,
        };
        for tag in site.tags.iter().filter(|t| t.name == "form") {
            let post = tag
                .attr("method")
                .is_some_and(|m| m.value.source().eq_ignore_ascii_case("post"));
            if !post {
                continue;
            }
            let action = tag.attr("action").map_or("", |a| a.value.source());
            let path = match (action, &own) {
                ("", Some(route)) => Some(route.clone()),
                (a, Some(route)) if a.starts_with("?/") => Some(format!("{route}{a}")),
                (a, _) if a.starts_with('/') && !a.contains('{') => Some(a.to_string()),
                _ => None,
            };
            let label = markup::label(site.markup, tag, true);
            let call = path.map_or_else(
                || {
                    let message = format!(
                        "Form action {action:?} is not a literal path or a ?/action of a page; the call is unknown."
                    );
                    diag(out, site.file, line_of(site.text, tag.at), message);
                    Call::Unresolved(action.to_string())
                },
                |path| Call::Http { method: "POST".into(), path },
            );
            calls.push((tag.at, call, label));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{
        AccessState, Call, Condition, DiagnosticSeverity, EndpointAccess, Origin, Target,
    };

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn routes(flow: &UiFlow) -> Vec<(String, String, Option<String>)> {
        (flow.screens.iter())
            .map(|s| (s.id.clone(), s.name.clone(), s.route.clone()))
            .collect()
    }

    fn access(flow: &UiFlow, id: &str) -> Option<Condition> {
        let screen = flow.screens.iter().find(|s| s.id == id);
        screen.expect("screen").access.condition().cloned()
    }

    type Row = (Origin, Target, Option<String>, Option<Condition>);

    fn transitions(flow: &UiFlow) -> Vec<Row> {
        let mut got: Vec<Row> = (flow.transitions.iter())
            .map(|t| {
                let (from, target) = (t.from.clone(), t.target.clone());
                (
                    from,
                    target,
                    t.trigger.clone(),
                    t.condition.condition().cloned(),
                )
            })
            .collect();
        got.sort();
        got
    }

    fn actions(flow: &UiFlow) -> Vec<(Origin, Call, Option<String>)> {
        let mut got: Vec<_> = (flow.actions.iter())
            .map(|a| (a.screen.clone(), a.call.clone(), a.trigger.clone()))
            .collect();
        got.sort();
        got
    }

    fn gaps(flow: &UiFlow) -> Vec<(String, u32)> {
        (flow.diagnostics.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .map(|d| (d.file.clone(), d.line))
            .collect()
    }

    fn page(id: &str) -> Origin {
        Origin::Screen(id.into())
    }

    fn any() -> Origin {
        Origin::AnyScreen {
            application: "web".into(),
        }
    }

    fn to(id: &str) -> Target {
        Target::Screen(id.into())
    }

    fn http(method: &str, path: &str) -> Call {
        Call::Http {
            method: method.into(),
            path: path.into(),
        }
    }

    fn custom(name: &str) -> Condition {
        Condition::Custom(name.into())
    }

    fn not(c: Condition) -> Condition {
        Condition::Not(Box::new(c))
    }

    fn sorted(mut rows: Vec<Row>) -> Vec<Row> {
        rows.sort();
        rows
    }

    const KIT: &str = r#"{"devDependencies":{"@sveltejs/kit":"^2.0.0"}}"#;

    #[test]
    fn page_files_become_screens_with_sveltekit_routes() {
        let dir = project(&[
            ("web/package.json", KIT),
            ("web/src/routes/+page.svelte", "<h1>Welcome</h1>\n"),
            ("web/src/routes/+layout.svelte", "<slot />\n"),
            (
                "web/src/routes/about/+page.svelte",
                "<h1>{$t('about')}</h1>\n",
            ),
            (
                "web/src/routes/(app)/items/[id]/+page.svelte",
                "<p>item</p>\n",
            ),
            (
                "web/src/routes/(app)/items/new/+page.svelte",
                "<p>new</p>\n",
            ),
            (
                "web/src/routes/blog/[slug=word]/[[lang]]/+page@.svelte",
                "<p>post</p>\n",
            ),
            (
                "web/src/routes/docs/[...path]/+page.svelte",
                "<p>docs</p>\n",
            ),
            ("web/src/routes/f/[a]-[b]/+page.svelte", "<p>pair</p>\n"),
            (
                "web/src/routes/redirect-only/+page.ts",
                "export const load = () => {};\n",
            ),
            ("web/src/routes/page.spec.ts", "<p>test</p>\n"),
            ("web/src/lib/+page.svelte", "<p>not routed</p>\n"),
            ("plain/package.json", r#"{"dependencies":{"svelte":"^5"}}"#),
            ("plain/src/routes/+page.svelte", "<p>no kit</p>\n"),
        ]);
        let flow = extract(dir.path());
        let s = |id: &str, name: &str, route: &str| (id.into(), name.into(), Some(route.into()));
        assert_eq!(
            routes(&flow),
            vec![
                s("web:/", "Welcome", "/"),
                s("web:/about", "/about", "/about"),
                s("web:/blog/:slug", "/blog/:slug", "/blog/:slug"),
                s("web:/docs/*", "/docs/*", "/docs/*"),
                s("web:/f/:param", "/f/:param", "/f/:param"),
                s("web:/items/:id", "/items/:id", "/items/:id"),
                s("web:/items/new", "/items/new", "/items/new"),
            ]
        );
        assert_eq!(flow.screens[0].file, "web/src/routes/+page.svelte");
        assert!(flow.screens.iter().all(|s| s.source == "svelte"));
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn svelte_config_names_the_routes_directory_and_base_path() {
        let dir = project(&[
            ("app/package.json", KIT),
            (
                "app/svelte.config.js",
                "export default { kit: { files: { routes: 'src/pages' }, paths: { base: '/shop' } } };\n",
            ),
            (
                "app/src/pages/+page.svelte",
                "<a href=\"/shop/cart\">Cart</a>\n",
            ),
            (
                "app/src/pages/cart/+page.svelte",
                "<script>import { base } from '$app/paths';</script>\n<a href=\"{base}/\">Home</a>\n",
            ),
            ("app/src/routes/+page.svelte", "<p>ignored</p>\n"),
        ]);
        let flow = extract(dir.path());
        let ids: Vec<&str> = flow.screens.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["app:/", "app:/cart"]);
        let got: Vec<(Origin, Target)> = (flow.transitions.iter())
            .map(|t| (t.from.clone(), t.target.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                (page("app:/"), to("app:/cart")),
                (page("app:/cart"), to("app:/")),
            ]
        );
    }

    const ROUTE_HELPER: &str = "const asQuery = (params?: Record<string, string>) => {
  const items = Object.entries(params ?? {}).map(([k, v]) => `${k}=${v}`);
  return items.length === 0 ? '' : `?${items.join('&')}`;
};

export const Route = {
  photos: (params?: { at?: string }) => '/photos' + asQuery(params),
  viewItem: ({ id }: { id: string }) => `/items/${id}`,
  search: (dto?: object) => {
    const query = JSON.stringify(dto);
    return `/search` + asQuery({ query });
  },
  map: (p?: { z: number }) => '/map' + (p ? `#${p.z}` : ''),
  shared: ({ slug, key }: { slug?: string; key: string }) => (slug ? `/s/${slug}` : `/share/${key}`),
};
";

    const ALBUMS: &str = r##"<script lang="ts">
  import { goto } from '$app/navigation';
  import { Route } from '#lib/route.ts';
  const open = async (id: string) => {
    await goto(Route.viewItem({ id }));
  };
  function later() {
    goto(`${Route.search({ q })}&x=1`);
  }
</script>

<!-- <a href="/commented">x</a> -->
<a href={Route.photos({ at: id })} data-testid="photos-link">All photos</a>
<a href="/items/{item.id}">Item</a>
<Button href={Route.map()} aria-label="Map">Map</Button>
<a href="/items/new?ref=albums#top">New</a>
<a href="https://immich.app">Site</a>
<a href="#main">Skip</a>
<a href="">Empty</a>
<a href={Route.shared(link)}>Shared</a>
<a href={link.href}>Dynamic</a>
<a href="edit">Relative</a>
<button onclick={() => open(album.id)} aria-label="Open album">Open</button>
<button on:click={later}>Later</button>
<button onclick={() => goto('/missing')}>Missing</button>
"##;

    #[test]
    fn links_goto_and_route_helpers_resolve_with_triggers() {
        let dir = project(&[
            (
                "web/package.json",
                r##"{"imports":{"#lib/*":"./src/lib/*"},"devDependencies":{"@sveltejs/kit":"2"}}"##,
            ),
            ("web/src/lib/route.ts", ROUTE_HELPER),
            (
                "web/src/routes/+page.svelte",
                "<a href=\"/photos/{asset.id}\">Photos</a>\n",
            ),
            (
                "web/src/routes/photos/[[assetId=id]]/+page.svelte",
                "<p/>\n",
            ),
            ("web/src/routes/items/[id]/+page.svelte", "<p/>\n"),
            ("web/src/routes/items/new/+page.svelte", "<p/>\n"),
            ("web/src/routes/search/+page.svelte", "<p/>\n"),
            ("web/src/routes/map/+page.svelte", "<p/>\n"),
            ("web/src/routes/(user)/albums/+page.svelte", ALBUMS),
            (
                "web/src/lib/Nav.svelte",
                "<script>import { goto as go } from '$app/navigation';</script>\n<a href=\"/\">Home</a>\n<button onclick={() => go('/search')}>Find</button>\n<a href=\"/search\"><Icon title={(x) => x > 1} />Go</a>\n",
            ),
        ]);
        let flow = extract(dir.path());
        let albums = page("web:/albums");
        let lbl = |s: &str| Some(s.to_string());
        let unresolved = |s: &str| Target::Unresolved(s.into());
        let want = vec![
            (page("web:/"), to("web:/photos"), lbl("Photos"), None),
            (albums.clone(), to("web:/photos"), lbl("photos-link"), None),
            (albums.clone(), to("web:/items/:id"), lbl("Item"), None),
            (albums.clone(), to("web:/map"), lbl("Map"), None),
            (albums.clone(), to("web:/items/new"), lbl("New"), None),
            (
                albums.clone(),
                unresolved("Route.shared(link)"),
                lbl("Shared"),
                None,
            ),
            (
                albums.clone(),
                unresolved("link.href"),
                lbl("Dynamic"),
                None,
            ),
            (albums.clone(), unresolved("edit"), lbl("Relative"), None),
            (
                albums.clone(),
                to("web:/items/:id"),
                lbl("Open album"),
                None,
            ),
            (albums.clone(), to("web:/search"), lbl("Later"), None),
            (
                albums.clone(),
                unresolved("'/missing'"),
                lbl("Missing"),
                None,
            ),
            (any(), to("web:/"), lbl("Home"), None),
            (any(), to("web:/search"), lbl("Find"), None),
            (any(), to("web:/search"), lbl("Go"), None),
        ];
        assert_eq!(transitions(&flow), sorted(want));
        let photos = flow.screens.iter().find(|s| s.id == "web:/photos");
        assert_eq!(photos.unwrap().route.as_deref(), Some("/photos"));
        let file = "web/src/routes/(user)/albums/+page.svelte";
        let want: Vec<(String, u32)> = [20, 21, 22, 25].map(|l| (file.to_string(), l)).into();
        assert_eq!(gaps(&flow), want);
    }

    const ROOT_LOAD: &str = "import { redirect } from '@sveltejs/kit';
export const load = async () => {
  try {
    if (authManager.authenticated) {
      redirect(307, '/photos');
    }
    if (!config.isInitialized) throw redirect(307, '/auth/register');
  } catch (e) {
    throw e;
  }
};
";

    const ADMIN_LAYOUT: &str = "import { redirect } from '@sveltejs/kit';
export async function load({ locals }) {
  await authenticate(url, { admin: true });
  if (!locals.user?.isAdmin) {
    redirect(303, '/auth/login');
  }
  if (locals.user.roles.includes('x')) redirect(303, '/');
}
";

    const HOOKS: &str = "import { redirect } from '@sveltejs/kit';
export const handle = async ({ event, resolve }) => {
  if (event.url.pathname.startsWith('/photos') && !event.locals.session) {
    redirect(303, '/auth/login');
  }
  return resolve(event);
};
";

    fn guard_project(config: &str) -> tempfile::TempDir {
        project(&[
            (".archguard/archguard.toml", config),
            ("web/package.json", KIT),
            ("web/src/routes/+page.svelte", "<p/>\n"),
            ("web/src/routes/+page.ts", ROOT_LOAD),
            ("web/src/routes/auth/register/+page.svelte", "<p/>\n"),
            ("web/src/routes/auth/login/+page.svelte", "<p/>\n"),
            ("web/src/routes/photos/+page.svelte", "<p/>\n"),
            ("web/src/routes/admin/+layout.server.ts", ADMIN_LAYOUT),
            ("web/src/routes/admin/users/+page.svelte", "<p/>\n"),
            (
                "web/src/routes/admin/+page.ts",
                "import { redirect } from '@sveltejs/kit';\nexport const load = async () => {\n  await authenticate(url);\n  redirect(307, '/admin/users');\n};\n",
            ),
            ("web/src/routes/admin/+page.svelte", "<p/>\n"),
            ("web/src/hooks.server.ts", HOOKS),
        ])
    }

    #[test]
    fn load_redirect_guards_and_hooks_give_access_and_conditions() {
        let dir = guard_project("");
        let flow = extract(dir.path());
        let authed = custom("authManager.authenticated");
        let initialized = custom("config.isInitialized");
        assert_eq!(
            access(&flow, "web:/"),
            Some(Condition::All(vec![
                not(authed.clone()),
                initialized.clone()
            ]))
        );
        let admin = custom("locals.user.isAdmin");
        let unknown = Condition::Unknown("locals.user.roles.includes('x')".into());
        let admin_access = Some(Condition::All(vec![admin.clone(), not(unknown.clone())]));
        assert_eq!(access(&flow, "web:/admin"), admin_access);
        assert_eq!(access(&flow, "web:/admin/users"), admin_access);
        let session = custom("event.locals.session");
        assert_eq!(access(&flow, "web:/photos"), Some(session));
        assert_eq!(access(&flow, "web:/auth/login"), None);
        let want = vec![
            (page("web:/"), to("web:/photos"), None, Some(authed)),
            (
                page("web:/"),
                to("web:/auth/register"),
                None,
                Some(not(initialized)),
            ),
            (any(), to("web:/auth/login"), None, Some(not(admin))),
            (any(), to("web:/"), None, Some(unknown)),
            (page("web:/admin"), to("web:/admin/users"), None, None),
        ];
        assert_eq!(transitions(&flow), sorted(want));
        let layout = "web/src/routes/admin/+layout.server.ts";
        assert_eq!(gaps(&flow), vec![(layout.to_string(), 7)]);
    }

    #[test]
    fn views_access_maps_checks_and_listed_guard_calls() {
        let config = "[views.access]\n\"authManager.authenticated\" = \"authenticated\"\n\
                      authenticate = \"role:admin\"\n\"locals.user.isAdmin\" = \"open\"\n";
        let dir = guard_project(config);
        let flow = extract(dir.path());
        let want = Condition::All(vec![
            not(Condition::Authenticated),
            custom("config.isInitialized"),
        ]);
        assert_eq!(access(&flow, "web:/"), Some(want));
        let unknown = Condition::Unknown("locals.user.roles.includes('x')".into());
        // the layout's call passes an options object: one fixed meaning cannot hold for it
        let call = Condition::Unknown("authenticate(url, { admin: true })".into());
        let users = vec![call, not(unknown)];
        let admin = [users.clone(), vec![Condition::Role("admin".into())]].concat();
        assert_eq!(access(&flow, "web:/admin"), Some(Condition::All(admin)));
        assert_eq!(
            access(&flow, "web:/admin/users"),
            Some(Condition::All(users))
        );
    }

    #[test]
    fn callbacks_unreadable_checks_and_private_methods_stay_unknown() {
        let layout = "import { redirect } from '@sveltejs/kit';
export const load = (async ({ params }) => {
  if (!UUID_REGEX.test(params.id)) {
    redirect(307, '/');
  }
  await q.catch(() => redirect(307, '/'));
}) satisfies LayoutLoad;
";
        let manager = "import { goto } from '$app/navigation';
class Manager {
  #goto(href: string) {
    return goto(href);
  }
}
";
        let dir = project(&[
            ("web/package.json", KIT),
            ("web/src/routes/+page.svelte", "<p/>\n"),
            ("web/src/routes/+layout.ts", layout),
            ("web/src/lib/manager.ts", manager),
        ]);
        let flow = extract(dir.path());
        let check = Condition::Unknown("!UUID_REGEX.test(params.id)".into());
        let callback = Condition::Unknown("redirect in a callback".into());
        let want = Condition::All(vec![not(check.clone()), not(callback.clone())]);
        assert_eq!(access(&flow, "web:/"), Some(want));
        let want = vec![
            (any(), to("web:/"), None, Some(check)),
            (any(), to("web:/"), None, Some(callback)),
            (any(), Target::Unresolved("href".into()), None, None),
        ];
        assert_eq!(transitions(&flow), sorted(want));
        let (routes, lib) = ("web/src/routes/+layout.ts", "web/src/lib/manager.ts");
        let want = [(lib, 4), (routes, 4), (routes, 6)].map(|(f, l)| (f.to_string(), l));
        assert_eq!(gaps(&flow), want.to_vec());
    }

    const SDK_PKG: &str = r#"{"name":"@acme/sdk","version":"1.0.0"}"#;
    const SDK: &str = r#"import * as Oazapfts from "@oazapfts/runtime";
export const defaults: Oazapfts.Defaults<Oazapfts.CustomHeaders> = {
    headers: {},
    baseUrl: "/api"
};
export function getAllAlbums({ shared }: { shared?: boolean; }, opts?: Oazapfts.RequestOpts) {
    return oazapfts.ok(oazapfts.fetchJson<{
        status: 200;
        data: AlbumResponseDto[];
    }>(`/albums${QS.query(QS.explode({ shared }))}`, {
        ...opts
    }));
}
export function deleteAlbum({ id }: { id: string; }, opts?: Oazapfts.RequestOpts) {
    return oazapfts.ok(oazapfts.fetchText(`/albums/${encodeURIComponent(id)}`, {
        ...opts,
        method: "DELETE"
    }));
}
export function dynamicCall({ path }: { path: string; }, opts?: Oazapfts.RequestOpts) {
    return oazapfts.ok(oazapfts.fetchJson(path, { ...opts }));
}
export const getBaseUrl = () => defaults.baseUrl;
"#;

    const SDK_PAGE: &str = r#"<script lang="ts">
  import { getAllAlbums, deleteAlbum, dynamicCall, getBaseUrl } from '@acme/sdk';
  import * as sdk from '@acme/sdk';
  const albums = await getAllAlbums({ shared: true });
  const remove = async (id) => {
    await deleteAlbum({ id });
  };
  dynamicCall({ path });
  const url = getBaseUrl();
  fetch('/api/health');
  sdk.getAllAlbums({});
</script>

<button aria-label="Delete" onclick={() => remove(a.id)}>x</button>
<form method="POST" action="?/rename" use:enhance><button>Go</button></form>
<form method="post"><button>Default</button></form>
<form method="POST" action="/login?/signin"></form>
"#;

    const API_HOOKS: &str = "export const handle = async ({ event, resolve }) => {
  if (event.url.pathname.startsWith('/api') && !event.locals.user) {
    error(401, 'no');
  }
  return resolve(event);
};
";

    #[test]
    fn fetch_sdk_functions_server_endpoints_and_form_actions() {
        let dir = project(&[
            ("packages/sdk/package.json", SDK_PKG),
            ("packages/sdk/src/fetch-client.ts", SDK),
            (
                "packages/sdk/src/index.ts",
                "export * from './fetch-client.js';\n",
            ),
            ("web/package.json", KIT),
            ("web/src/routes/albums/+page.svelte", SDK_PAGE),
            (
                "web/src/routes/albums/+page.server.ts",
                "export const actions = {\n  default: async () => {},\n  rename: async ({ request }) => {},\n};\n",
            ),
            (
                "web/src/routes/api/items/[id]/+server.ts",
                "export const GET = async () => json([]);\nexport async function DELETE() {}\nexport const fallback = () => {};\nexport const helper = 1;\n",
            ),
            ("web/src/hooks.server.ts", API_HOOKS),
            (
                "web/src/lib/Form.svelte",
                "<form method=\"POST\" action=\"?/save\"></form>\n",
            ),
        ]);
        let flow = extract(dir.path());
        let albums = page("web:/albums");
        let lbl = |s: &str| Some(s.to_string());
        let mut want = vec![
            (albums.clone(), http("GET", "/api/albums"), None),
            (albums.clone(), http("GET", "/api/albums"), None),
            (
                albums.clone(),
                http("DELETE", "/api/albums/:param"),
                lbl("Delete"),
            ),
            (albums.clone(), Call::Unresolved("dynamicCall".into()), None),
            (albums.clone(), http("GET", "/api/health"), None),
            (albums.clone(), http("POST", "/albums?/rename"), None),
            (albums.clone(), http("POST", "/albums"), None),
            (albums.clone(), http("POST", "/login?/signin"), None),
            (any(), Call::Unresolved("?/save".into()), None),
        ];
        want.sort();
        assert_eq!(actions(&flow), want);
        let ep =
            |method: &str, path: &str, file: &str, line: u32, access: AccessState| EndpointAccess {
                method: method.into(),
                path: path.into(),
                access,
                application: Some("web".into()),
                file: format!("web/src/routes/{file}"),
                line,
                source: "svelte".into(),
                handler: None,
                sends: vec![],
            };
        let (actions_file, server) = ("albums/+page.server.ts", "api/items/[id]/+server.ts");
        let protected = AccessState::Protected(custom("event.locals.user"));
        let undeclared = AccessState::Undeclared;
        let mut want = vec![
            ep("POST", "/albums", actions_file, 2, undeclared.clone()),
            ep("POST", "/albums?/rename", actions_file, 3, undeclared),
            ep("GET", "/api/items/:id", server, 1, protected.clone()),
            ep("DELETE", "/api/items/:id", server, 2, protected.clone()),
            ep("ANY", "/api/items/:id", server, 3, protected),
        ];
        want.sort();
        assert_eq!(flow.endpoints, want);
        let mut gap_files: Vec<String> = gaps(&flow).into_iter().map(|g| g.0).collect();
        gap_files.dedup();
        assert_eq!(
            gap_files,
            [
                "web/src/lib/Form.svelte",
                "web/src/routes/albums/+page.svelte"
            ]
        );
    }

    #[test]
    fn no_application_without_sveltekit() {
        let dir = project(&[
            ("site/package.json", r#"{"dependencies":{"svelte":"5"}}"#),
            ("site/src/routes/+page.svelte", "<a href=\"/x\">x</a>\n"),
        ]);
        assert_eq!(extract(dir.path()), UiFlow::default());
    }

    use AccessState::{Open, Protected, Undeclared};

    fn state(flow: &UiFlow, id: &str) -> AccessState {
        let screen = flow.screens.iter().find(|s| s.id == id);
        screen.expect("screen").access.clone()
    }

    fn weak(flow: &UiFlow, id: &str) -> bool {
        flow.screens.iter().find(|s| s.id == id).unwrap().weak
    }

    fn endpoint(flow: &UiFlow, method: &str, path: &str) -> AccessState {
        let mut found = flow.endpoints.iter();
        let e = found.find(|e| e.method == method && e.path == path);
        e.unwrap_or_else(|| panic!("endpoint {method} {path}"))
            .access
            .clone()
    }

    fn unknown(text: &str) -> Condition {
        Condition::Unknown(text.into())
    }

    const PAGE: &str = "<p/>\n";
    const GUARD_LOAD: &str = "import { redirect } from '@sveltejs/kit';
export const load = ({ locals }) => {
  if (!locals.user) redirect(307, '/login');
};
";

    fn kit(files: &[(&str, &str)]) -> UiFlow {
        kit_cfg("", files)
    }

    fn kit_cfg(config: &str, files: &[(&str, &str)]) -> UiFlow {
        let mut all = vec![
            (".archguard/archguard.toml", config),
            ("web/package.json", KIT),
        ];
        all.extend_from_slice(files);
        extract(project(&all).path())
    }

    #[test]
    fn d3_error_in_load_is_a_guard_like_redirect() {
        let load = "import { error } from '@sveltejs/kit';
export const load = ({ locals }) => {
  if (!locals.user) error(401, 'x');
};
";
        let flow = kit(&[
            ("web/src/routes/a/+page.svelte", PAGE),
            ("web/src/routes/a/+page.server.ts", load),
        ]);
        assert_eq!(state(&flow, "web:/a"), Protected(custom("locals.user")));
    }

    #[test]
    fn d7_redirect_in_else_stays_unknown_with_a_gap() {
        let load = "import { redirect } from '@sveltejs/kit';
export const load = ({ locals }) => {
  if (locals.user) { ok(); } else { redirect(307, '/login'); }
};
";
        let flow = kit(&[
            ("web/src/routes/a/+page.svelte", PAGE),
            ("web/src/routes/a/+page.server.ts", load),
        ]);
        let want = Protected(Condition::Not(Box::new(unknown(
            "redirect in an else block",
        ))));
        assert_eq!(state(&flow, "web:/a"), want);
        assert!(!flow.complete);
    }

    const APP_LAYOUT: (&str, &str) = ("web/src/routes/(app)/+layout.server.ts", GUARD_LOAD);

    #[test]
    fn d8_layout_guard_reaches_group_pages_only_and_is_weak() {
        let flow = kit(&[
            APP_LAYOUT,
            ("web/src/routes/(app)/x/+page.svelte", PAGE),
            ("web/src/routes/(marketing)/y/+page.svelte", PAGE),
        ]);
        assert_eq!(state(&flow, "web:/x"), Protected(custom("locals.user")));
        assert!(weak(&flow, "web:/x"));
        assert_eq!(state(&flow, "web:/y"), Undeclared);
        assert!(!weak(&flow, "web:/y"));
    }

    #[test]
    fn d9_page_and_layout_resets_skip_the_group_layout() {
        let flow = kit(&[
            APP_LAYOUT,
            ("web/src/routes/(app)/x/+page.svelte", PAGE),
            ("web/src/routes/(app)/x/embed/+page@.svelte", PAGE),
            ("web/src/routes/(app)/z/+layout@.svelte", PAGE),
            ("web/src/routes/(app)/z/+page.svelte", PAGE),
            ("web/src/routes/(app)/w/+page@(app).svelte", PAGE),
            ("web/src/routes/(app)/v/+page@nowhere.svelte", PAGE),
        ]);
        let guarded = Protected(custom("locals.user"));
        assert_eq!(state(&flow, "web:/x"), guarded);
        assert_eq!(state(&flow, "web:/x/embed"), Undeclared);
        assert_eq!(state(&flow, "web:/z"), Undeclared);
        assert_eq!(state(&flow, "web:/w"), guarded);
        // an unknown reset target is a gap and keeps the whole chain
        assert_eq!(state(&flow, "web:/v"), guarded);
        assert!(!flow.complete);
    }

    const AUTH_HELPER: &str = "import { redirect } from '@sveltejs/kit';
export async function authenticate(url) {
  if (!auth.ok) redirect(307, '/login');
}
";

    fn helper_load(import: &str, call: &str) -> String {
        format!("{import}\nexport const load = async ({{ url }}) => {{\n  {call}\n}};\n")
    }

    #[test]
    fn d14_helper_is_followed_one_hop_or_unknown_with_a_gap() {
        let load = helper_load(
            "import { authenticate } from '$lib/auth';",
            "await authenticate(url);",
        );
        let flow = kit(&[
            ("web/src/routes/a/+page.svelte", PAGE),
            ("web/src/routes/a/+page.ts", &load),
            ("web/src/routes/login/+page.svelte", PAGE),
            ("web/src/lib/auth.ts", AUTH_HELPER),
        ]);
        assert_eq!(state(&flow, "web:/a"), Protected(custom("auth.ok")));
        assert!(flow.complete);
        // helper not found: never silent
        let flow = kit(&[
            ("web/src/routes/a/+page.svelte", PAGE),
            ("web/src/routes/a/+page.ts", &load),
        ]);
        assert_eq!(
            state(&flow, "web:/a"),
            Protected(unknown("authenticate(url)"))
        );
        assert!(!flow.complete);
        // a helper without any guard adds nothing and no gap
        let flow = kit(&[
            ("web/src/routes/a/+page.svelte", PAGE),
            ("web/src/routes/a/+page.ts", &load),
            (
                "web/src/lib/auth.ts",
                "export async function authenticate(url) { log(url); }\n",
            ),
        ]);
        assert_eq!(state(&flow, "web:/a"), Undeclared);
        assert!(flow.complete);
    }

    #[test]
    fn d15_listed_guard_with_an_options_object_is_unknown() {
        let call = |args: &str| {
            helper_load(
                "import { authenticate } from '$lib/auth';",
                &format!("await authenticate({args});"),
            )
        };
        let (admin, public, plain) = (
            call("url, { admin: true }"),
            call("url, { public: true }"),
            call("url"),
        );
        let flow = kit_cfg(
            "[views.access]\nauthenticate = \"authenticated\"\n",
            &[
                ("web/src/routes/a/+page.svelte", PAGE),
                ("web/src/routes/a/+page.ts", &public),
                ("web/src/routes/b/+page.svelte", PAGE),
                ("web/src/routes/b/+page.ts", &admin),
                ("web/src/routes/c/+page.svelte", PAGE),
                ("web/src/routes/c/+page.ts", &plain),
            ],
        );
        let want_a = Protected(unknown("authenticate(url, { public: true })"));
        assert_eq!(state(&flow, "web:/a"), want_a);
        let want_b = Protected(unknown("authenticate(url, { admin: true })"));
        assert_eq!(state(&flow, "web:/b"), want_b);
        assert_eq!(state(&flow, "web:/c"), Protected(Condition::Authenticated));
        assert!(!flow.complete);
    }

    fn hooks_project(hooks: &str, extra: &[(&str, &str)]) -> UiFlow {
        let get = "export const GET = () => {};\n";
        let mut files = vec![
            ("web/src/hooks.server.ts", hooks),
            ("web/src/routes/+page.svelte", PAGE),
            ("web/src/routes/login/+page.svelte", PAGE),
            ("web/src/routes/admin/+page.svelte", PAGE),
            ("web/src/routes/administrators/+page.svelte", PAGE),
            ("web/src/routes/(app)/dash/+page.svelte", PAGE),
            ("web/src/routes/admin/api/+server.ts", get),
            (
                "web/src/routes/admin/x/+page.server.ts",
                "export const actions = { default: async () => {} };\n",
            ),
            ("web/src/routes/api/public/feed/+server.ts", get),
            ("web/src/routes/api/private/+server.ts", get),
        ];
        files.extend_from_slice(extra);
        kit(&files)
    }

    #[test]
    fn d16_path_prefix_rule_covers_screens_endpoints_and_actions() {
        let hooks = "export const handle = async ({ event, resolve }) => {
  if (event.url.pathname.startsWith('/admin') && !event.locals.user) redirect(303, '/login');
  return resolve(event);
};
";
        let flow = hooks_project(hooks, &[]);
        let want = Protected(custom("event.locals.user"));
        for id in ["web:/admin", "web:/administrators"] {
            assert_eq!(state(&flow, id), want, "{id}");
        }
        assert_eq!(state(&flow, "web:/"), Undeclared);
        assert_eq!(endpoint(&flow, "GET", "/admin/api"), want);
        assert_eq!(endpoint(&flow, "POST", "/admin/x"), want);
        assert_eq!(endpoint(&flow, "GET", "/api/private"), Undeclared);
    }

    #[test]
    fn d17_early_return_excludes_routes_from_later_rules() {
        let hooks = "export const handle = async ({ event, resolve }) => {
  if (event.url.pathname.startsWith('/api/public')) return resolve(event);
  if (!event.locals.user) error(401);
  return resolve(event);
};
";
        let flow = hooks_project(hooks, &[]);
        let want = Protected(custom("event.locals.user"));
        assert_eq!(endpoint(&flow, "GET", "/api/public/feed"), Undeclared);
        assert_eq!(endpoint(&flow, "GET", "/api/private"), want);
        assert_eq!(state(&flow, "web:/admin"), want);
    }

    #[test]
    fn d18_read_allowlist_is_open_and_never_beats_another_rule() {
        let hooks = "const pub = ['/', '/login'];
export const handle = async ({ event, resolve }) => {
  if (!event.locals.user && !pub.includes(event.route.id)) redirect(303, '/login');
  return resolve(event);
};
";
        let flow = hooks_project(hooks, &[]);
        assert_eq!(state(&flow, "web:/"), Open);
        assert_eq!(state(&flow, "web:/login"), Open);
        let want = Protected(custom("event.locals.user"));
        assert_eq!(state(&flow, "web:/admin"), want);
        // a second, unreadable rule keeps the allowlisted routes protected
        let both = "const pub = ['/', '/login'];
export const handle = async ({ event, resolve }) => {
  if (!event.locals.user && !pub.includes(event.route.id)) redirect(303, '/login');
  if (!ok(event)) return new Response('no', { status: 403 });
  return resolve(event);
};
";
        let flow = hooks_project(both, &[]);
        let want = Protected(Condition::Not(Box::new(unknown("!ok(event)"))));
        assert_eq!(state(&flow, "web:/"), want);
        assert!(!flow.complete);
    }

    #[test]
    fn d19_route_id_selector_matches_groups() {
        let hooks = "export const handle = async ({ event, resolve }) => {
  if (event.route.id?.startsWith('/(app)') && !event.locals.user) redirect(303, '/login');
  return resolve(event);
};
";
        let flow = hooks_project(hooks, &[]);
        let want = Protected(custom("event.locals.user"));
        assert_eq!(state(&flow, "web:/dash"), want);
        assert_eq!(state(&flow, "web:/admin"), Undeclared);
    }

    #[test]
    fn d20_authjs_handle_reexport_protects_nothing() {
        let auth = "import SvelteKitAuth from '@auth/sveltekit';
export const { handle, signIn, signOut } = SvelteKitAuth({ providers: [] });
";
        let flow = hooks_project(
            "export { handle } from './auth';\n",
            &[("web/src/auth.ts", auth)],
        );
        assert_eq!(state(&flow, "web:/admin"), Undeclared);
        assert!(flow.complete);
        // a re-export that cannot be read is unknown
        let flow = hooks_project("export { handle } from './nowhere';\n", &[]);
        let state = state(&flow, "web:/admin");
        assert!(matches!(state, Protected(Condition::Unknown(_))));
        assert!(!flow.complete);
    }

    #[test]
    fn d21_sequence_follows_imported_handles_one_hop() {
        let hooks = "import { sequence } from '@sveltejs/kit/hooks';
import { authz } from '$lib/server/authz';
const authn = async ({ event, resolve }) => { event.locals.user = u(); return resolve(event); };
export const handle = sequence(authn, authz);
";
        let authz = "export const authz = async ({ event, resolve }) => {
  if (event.url.pathname.startsWith('/admin') && !event.locals.user) redirect(303, '/login');
  return resolve(event);
};
";
        let flow = hooks_project(hooks, &[("web/src/lib/server/authz.ts", authz)]);
        let want = Protected(custom("event.locals.user"));
        assert_eq!(state(&flow, "web:/admin"), want);
        assert_eq!(state(&flow, "web:/"), Undeclared);
        assert!(flow.complete);
        let flow = hooks_project(hooks, &[]);
        let state = state(&flow, "web:/");
        assert!(matches!(state, Protected(Condition::Unknown(_))));
        assert!(!flow.complete);
    }

    #[test]
    fn d22_unreadable_blocking_response_is_unknown_everywhere() {
        let hooks = "export const handle = async ({ event, resolve }) => {
  if (!ok(event)) return new Response('no', { status: 403 });
  return resolve(event);
};
";
        let flow = hooks_project(hooks, &[]);
        let want = Protected(Condition::Not(Box::new(unknown("!ok(event)"))));
        assert_eq!(state(&flow, "web:/"), want);
        assert_eq!(endpoint(&flow, "GET", "/api/private"), want);
        assert!(!flow.complete);
    }

    #[test]
    fn d23_prerendered_pages_are_not_covered_by_handle() {
        let hooks = "export const handle = async ({ event, resolve }) => {
  if (event.url.pathname.startsWith('/admin') && !event.locals.user) redirect(303, '/login');
  return resolve(event);
};
";
        let flow = hooks_project(
            hooks,
            &[
                ("web/src/routes/admin/help/+page.svelte", PAGE),
                (
                    "web/src/routes/admin/help/+page.ts",
                    "export const prerender = true;\n",
                ),
            ],
        );
        assert_eq!(state(&flow, "web:/admin/help"), Undeclared);
        let want = Protected(custom("event.locals.user"));
        assert_eq!(state(&flow, "web:/admin"), want);
        let note = (flow.diagnostics.iter()).find(|d| d.message.contains("Prerendered"));
        assert!(note.is_some_and(|d| d.severity == DiagnosticSeverity::Note));
    }

    #[test]
    fn d27_blocking_calls_outside_handle_are_no_rule() {
        let hooks = "export const handleError = ({ error }) => {
  if (error) error(500);
};
const helper = () => { if (x) error(500); };
export const handle = async ({ event, resolve }) => resolve(event);
";
        let flow = hooks_project(hooks, &[]);
        assert_eq!(state(&flow, "web:/admin"), Undeclared);
        assert!(flow.complete);
    }

    #[test]
    fn a_blocking_call_the_rules_cannot_place_is_unknown() {
        let hooks = "export const handle = async ({ event, resolve }) => {
  switch (event.url.pathname) { case '/x': redirect(303, '/'); }
  return resolve(event);
};
";
        let flow = hooks_project(hooks, &[]);
        let state = state(&flow, "web:/");
        assert!(matches!(state, Protected(Condition::Unknown(_))));
        assert!(!flow.complete);
    }

    #[test]
    fn a_listed_open_guard_in_a_load_makes_the_screen_open() {
        let load = helper_load("import { allowAll } from '$lib/auth';", "await allowAll();");
        let flow = kit_cfg(
            "[views.access]\nallowAll = \"open\"\n",
            &[
                ("web/src/routes/a/+page.svelte", PAGE),
                ("web/src/routes/a/+page.ts", &load),
            ],
        );
        assert_eq!(state(&flow, "web:/a"), Open);
    }
}
