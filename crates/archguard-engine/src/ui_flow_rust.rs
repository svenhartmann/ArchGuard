//! Rust UI frameworks other than Slint for ui-flow/v1 (ADR 0021): Tauri
//! commands and windows, Dioxus, Leptos, Yew, egui and iced. A token level
//! scanner over `lang_rust`'s blanked text, no Rust parser; only literals are
//! read, everything else is a gap or a note.
//!
//! Scope: a Rust file counts when its nearest `Cargo.toml` with a `[package]`
//! depends on `tauri`, `dioxus`, `leptos`, `yew`, `iced`, `eframe` or `egui`
//! (in `[dependencies]` or `[target.*.dependencies]`) and is not one of that
//! framework's own crates: the package is the framework itself or is named
//! `<framework>-…`/`<framework>_…` without `example` in the name
//! (`dioxus-router`, `leptos_router`, `tauri-plugin-sample` are internals;
//! `dioxus-examples` is not). A file below no package counts when it names a
//! framework path (`use dioxus::…`); it is its own application, named after
//! its path without `.rs` (`main.rs`, `lib.rs`, `mod.rs`: its folder's path,
//! `06-routing/router`), as the single-file examples of these repositories
//! are; the path keeps it apart from a package folder of the same name. The
//! application of a
//! package is its folder name; a `src-tauri` folder takes its parent's name.
//! Test folders, `target`, `examples` below the root and comments are skipped.
//!
//! Dioxus and Yew: an `enum` with `#[derive(Routable)]`. A variant after
//! `#[route("/p")]` (Dioxus, optional second argument: the component) or
//! `#[at("/p")]` (Yew) is a `page`, id `<app>:<Enum>::<Variant>`, named after
//! the component (Dioxus: the variant unless given; Yew: the component in
//! `Enum::Variant => html! { <Comp/> }`). `#[nest("/x")]`…`#[end_nest]`
//! prefixes routes; `#[layout]`, `#[redirect]` and `#[not_found]` add no
//! screen; `#[child]` is a gap. `:..rest` (Dioxus) is the catch-all `*`; a
//! query (`?:q`) or hash (`#:h`) part is dropped. Transitions: Dioxus
//! `Link { to: …, "label" }`, Yew `<Link<Route> to={…}>{ "label" }</Link<…>>`,
//! and `.push(…)`/`.replace(…)` with a `Route::Variant` argument or on
//! `navigator()`, `router()` or a variable bound to `use_navigator()`.
//!
//! Leptos: `<Route path=… view=Comp/>` and `<ProtectedRoute …/>` inside
//! `<ParentRoute path=…>`/`<ProtectedParentRoute>` are pages, id
//! `<app>:<route>`, named after the view component (the route when the view is
//! a closure); parent routes are layouts. A path is `path!("…")`, a string,
//! or `StaticSegment`/`ParamSegment`/`WildcardSegment`/`OptionalParamSegment`
//! alone or in a tuple; an optional segment keeps its parameter form, and a
//! route whose form without optional segments is `/` is a start screen.
//! `condition=f`, `condition=move || f()` of a protected route is `f`'s
//! `[views.access]` meaning, else `custom(f)`; any other condition is
//! `unknown` with a gap; children inherit it. Transitions: `<A href=…>`,
//! `<Redirect path=…/>` and `navigate("…", …)` with `let navigate =
//! use_navigate();`. A literal, or `format!("/u/{id}")` with holes as
//! parameters; a relative link resolves below the route of its screen.
//!
//! Server functions (Leptos and Dioxus): `#[server]` on an `fn` is an endpoint
//! `POST <prefix>/<name>`: prefix `/api` unless `prefix = "…"` or the second
//! positional argument says otherwise, name the `endpoint = "…"` or the
//! function name; `input = GetUrl` or a `Get…` encoding makes it `GET`. The
//! real default URL carries a hash suffix, which is not computed: the path is
//! the join key only. Access is `undeclared` (checks live in the body). A
//! call of the function, or `::<StructName>` (`ServerAction::<AddTodo>`), in
//! the same application is an action with that method and path.
//!
//! egui and iced: the application's root type (`impl eframe::App for X`,
//! `impl Application for X`, `impl Sandbox for X`, or `X::update`/`X::view`
//! passed to `application(…)`/`run(…)`/`daemon(…)`) holds the screen state: a
//! field whose type is an `enum` of the application with at least two
//! variants, and whose field or type name contains `screen`, `page`, `view`,
//! `tab` or `route` (case ignored). Each variant is a `view`, id
//! `<app>:<Enum>::<Variant>`. Start: `field: Enum::V` in an `fn` named `new`,
//! `default`, `boot` or `init`, else the `#[default]` variant, else the
//! variant `impl Default for Enum` returns; none is a note. Transitions:
//! `….field = Enum::V`; `….field = x` where `x` is bound by the match arm
//! `Message::GoTo(x) =>` makes every `Message::GoTo(Enum::V)` a transition;
//! `selectable_value(&mut ….field, Enum::V, "label")` and `radio_value`.
//! Anything else assigned is unresolved. All start at `any_screen`. The
//! trigger is the literal of the nearest `…button("label")` or
//! `Button::new("label")` in the `if` header around the code or in the
//! widget chain holding the message, or for a unit message arm
//! (`Message::Next =>`) the one label all its `on_…(Message::Next)` sites
//! share.
//!
//! Tauri: a function in `tauri::generate_handler![…]` is an endpoint
//! `INVOKE <command>`: the pseudo method `INVOKE` and the bare command name as
//! path, so the frontend's `invoke('<command>', …)` (imported from
//! `@tauri-apps/api/core` or the v1 `@tauri-apps/api/tauri`, in `.ts`,
//! `.tsx`, `.js`, `.jsx`, `.mjs`, `.svelte` and `.vue` files below the
//! application folder) joins it as an `http` action with the same method and
//! path from `any_screen`. Without an app ACL (see below) a command is
//! `undeclared`: Tauri lets every window call it. Under one it is guarded by the capability files `capabilities/**`: each that
//! grants `allow-<command>` (dashes for underscores) and does not deny it is
//! `policy(capability:<identifier>)`, several are `any`; none is `never`,
//! unless a capability names an app permission set (`default`) that is not
//! read: then `unknown` with a gap. Windows: `app.windows` (v1:
//! `tauri.windows`) of `tauri.conf.json` with their `label` (default `main`)
//! are `window` screens, id `<app>:window:<label>`, started unless `create`
//! is `false`; `WebviewWindowBuilder::new(_, "label", _)`, `WindowBuilder::new`
//! and `WebviewWindow::builder` add windows, started inside `.setup(…)`, else
//! a transition from `any_screen`. A label that is not literal is a gap.
//!
//! Tauri capabilities: `capabilities/**` (`json`, `toml`; `json5` only with the
//! cargo feature `config-json5`, read as far as it is JSON), one capability, a
//! list or `{ "capabilities": [ … ] }` per file; with `app.security.capabilities`
//! in the configuration (`tauri.conf.json`, `.json5`, `Tauri.toml`) only the
//! listed ones are active, inline ones included, and one not found is a gap.
//! App permissions and sets of `permissions/**` (`[[permission]]`, `[[set]]`,
//! `[default]`) are expanded; a deny in any active capability of an
//! overlapping window beats an allow. A non-empty `AppManifest::commands` or
//! any permission file switches the app ACL on: every app command is then
//! guarded (`never` without a grant), not only the listed ones.
//!
//! Leptos: a component that holds route tags and is placed by a `<Name/>` tag
//! (`<ContactRoutes/>`) gets its routes below the placing route, with its path
//! and condition; a component never placed that way counts from the root.
//! Dioxus `RouterConfig::on_update` and a Yew `switch` arm that branches
//! (`if`, `match`, match guard) make the router's or the arm's page `unknown`
//! with a gap, or the `[views.access]` meaning of a function used as the condition.
//!
//! Not read: Dioxus `#[get]`/`#[post]` server routes, Tauri `platforms`,
//! `remote` and per-window action checks, scopes, Leptos route components
//! placed in a form other than a tag, Yew wrapper components,
//! navigation through helper functions, and messages that carry the screen
//! through more than one variable.
use crate::extract_ctx::{ExtractContext, finish, with_root_config};
use crate::lang_rust::{FnItem, fn_items, literal_value};
use crate::lang_text::offset;
use archguard_core::{Screen, UiFlow};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const SOURCE: &str = "rust-ui";
const MAX_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 5_000;

/// Screens and transitions of every Rust UI application below `root`.
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run; Leptos conditions take `[views.access]`.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    let apps = files::discover(ctx.root, &mut flow);
    if let (Some(e), false) = (&ctx.config_error, apps.is_empty()) {
        let message = format!("Configuration not read ({e}); [views.access] is not applied.");
        nav::diag(&mut flow, "", 0, message, false);
    }
    for app in &apps {
        run_app(ctx, app, &mut flow);
    }
    flow.start_screens.sort();
    flow.start_screens.dedup();
    finish(&mut flow);
    flow
}

fn run_app(ctx: &ExtractContext, app: &App, flow: &mut UiFlow) {
    let mut known = Known::default();
    routable::screens(app, &ctx.config.views.access, flow, &mut known);
    leptos::screens(app, &ctx.config.views.access, flow, &mut known);
    let keys = state::screens(app, flow, &mut known);
    tauri::run(ctx.root, app, flow, &mut known);
    for src in &app.rust {
        let cx = Cx {
            app,
            src,
            known: &known,
        };
        routable::links(cx, flow);
        leptos::links(cx, flow);
    }
    state::transitions(app, &keys, &known, flow);
    server::run(app, &known, flow);
    flow.screens.extend(known.screens);
}

/// One Rust source: comments blanked (`code`), literal contents too (`mask`).
pub(crate) struct Src {
    rel: String,
    code: String,
    mask: String,
    fns: Vec<FnItem>,
}

impl Src {
    fn new(rel: String, text: &str) -> Self {
        let mask = crate::lang_rust::mask(text);
        let fns = fn_items(&mask);
        let code = crate::lang_rust::code(text);
        Src {
            rel,
            code,
            mask,
            fns,
        }
    }

    fn line(&self, at: usize) -> u32 {
        crate::lang_text::line(&self.code, at)
    }

    /// The source text of `part`, a slice of `mask`.
    fn text(&self, part: &str) -> &str {
        let at = offset(&self.mask, part);
        self.code.get(at..at + part.len()).unwrap_or("").trim()
    }

    /// The string literal `part` (a slice of `mask`) spans.
    fn lit(&self, part: &str) -> Option<String> {
        literal_value(self.text(part))
    }
}

/// One file of one application, with what the screen rules found.
#[derive(Clone, Copy)]
pub(crate) struct Cx<'a> {
    app: &'a App,
    src: &'a Src,
    known: &'a Known,
}

/// One application: its Rust sources and, for Tauri, its web sources.
pub(crate) struct App {
    name: String,
    /// The package folder; `""` for a file below no package.
    dir: String,
    rust: Vec<Src>,
    /// (path, text) of the frontend of a Tauri application.
    web: Vec<(String, String)>,
    /// `tauri.conf.json`, `build.rs` and `capabilities/*.json` of a Tauri package.
    tauri_files: Vec<String>,
    tauri: bool,
    /// The `tauri` dependency has the cargo feature `config-json5`.
    json5: bool,
}

/// What the screen rules found in one application.
#[derive(Default)]
pub(crate) struct Known {
    screens: Vec<Screen>,
    /// (enum, variant) -> screen id, routable and screen-state enums.
    variants: BTreeMap<(String, String), String>,
    /// Enums with `#[derive(Routable)]`.
    routable: BTreeSet<String>,
}

mod files {
    use super::*;
    use crate::extract_ctx::{ancestors, dir_name, read_text, walk};
    use crate::services::support::test_path;

    /// Dependency name and framework.
    const FRAMEWORKS: [(&str, &str); 7] = [
        ("tauri", "tauri"),
        ("dioxus", "dioxus"),
        ("leptos", "leptos"),
        ("yew", "yew"),
        ("iced", "iced"),
        ("eframe", "eframe"),
        ("egui", "egui"),
    ];
    const WEB: [&str; 7] = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".svelte", ".vue"];

    /// A package: application name and the frameworks it uses.
    struct Package {
        name: String,
        frameworks: BTreeSet<&'static str>,
        json5: bool,
    }

    /// The applications below `root`, in path order.
    pub(crate) fn discover(root: &Path, flow: &mut UiFlow) -> Vec<App> {
        let walk = match walk(root) {
            Ok(walk) => walk,
            Err(error) => {
                nav::diag(
                    flow,
                    "",
                    0,
                    format!("Rust files not scanned: {error}"),
                    true,
                );
                return vec![];
            }
        };
        let mut files: Vec<String> = walk.files.into_iter().filter(|r| !test_path(r)).collect();
        files.sort();
        let packages: BTreeMap<String, Package> = (files.iter())
            .filter(|r| *r == "Cargo.toml" || r.ends_with("/Cargo.toml"))
            .filter_map(|r| Some((parent(r).to_string(), package(root, r, flow)?)))
            .collect();
        let mut rust: Vec<&String> = files.iter().filter(|r| r.ends_with(".rs")).collect();
        rust.retain(|r| owner(r, &packages).is_none_or(|(_, p)| !p.frameworks.is_empty()));
        if let Some(first) = rust.get(MAX_FILES) {
            let message = format!("More than {MAX_FILES} Rust files; the rest was not read.");
            nav::diag(flow, first, 0, message, true);
            rust.truncate(MAX_FILES);
        }
        let mut apps: BTreeMap<String, App> = BTreeMap::new();
        for rel in rust {
            add_source(root, rel, &packages, &mut apps, flow);
        }
        for app in apps.values_mut().filter(|a| a.tauri) {
            tauri_files(root, app, &files, flow);
        }
        apps.into_values().collect()
    }

    fn add_source(
        root: &Path,
        rel: &str,
        packages: &BTreeMap<String, Package>,
        apps: &mut BTreeMap<String, App>,
        flow: &mut UiFlow,
    ) {
        let Some(text) = read(root, rel, flow) else {
            return;
        };
        let src = Src::new(rel.to_string(), &text);
        let (dir, name, tauri, json5) = match owner(rel, packages) {
            Some((dir, p)) => (
                dir.to_string(),
                p.name.clone(),
                p.frameworks.contains("tauri"),
                p.json5,
            ),
            None if names_framework(&src.mask) => (String::new(), file_app(rel), false, false),
            None => return,
        };
        let key = format!("{dir}\n{name}");
        let app = apps.entry(key).or_insert_with(|| App {
            name,
            dir,
            rust: vec![],
            web: vec![],
            tauri_files: vec![],
            tauri,
            json5,
        });
        app.rust.push(src);
    }

    /// The package of `Cargo.toml` at `rel`; none for a workspace without one.
    fn package(root: &Path, rel: &str, flow: &mut UiFlow) -> Option<Package> {
        let text = read(root, rel, flow)?;
        let Ok(manifest) = toml::from_str::<toml::Value>(&text) else {
            let message = "Cargo.toml does not parse; its package counts as no Rust UI.";
            nav::diag(flow, rel, 0, message.into(), false);
            return Some(Package {
                name: String::new(),
                frameworks: BTreeSet::new(),
                json5: false,
            });
        };
        let package = manifest.get("package")?.get("name")?.as_str()?.to_string();
        let mut tables = vec![manifest.get("dependencies")];
        if let Some(targets) = manifest.get("target").and_then(|t| t.as_table()) {
            tables.extend(targets.values().map(|t| t.get("dependencies")));
        }
        let uses = |dep: &str| (tables.iter().flatten()).any(|t| t.get(dep).is_some());
        let frameworks = (FRAMEWORKS.iter())
            .filter(|(dep, fw)| uses(dep) && !own(&package, fw))
            .map(|(_, fw)| *fw)
            .collect();
        let dir = parent(rel);
        let name = match dir_name(root, dir) {
            n if n == "src-tauri" => dir_name(root, parent(dir)),
            n => n,
        };
        let features =
            (tables.iter().flatten()).filter_map(|t| t.get("tauri")?.get("features")?.as_array());
        let json5 = (features.flatten()).any(|f| f.as_str() == Some("config-json5"));
        Some(Package {
            name,
            frameworks,
            json5,
        })
    }

    /// A crate of the framework itself (see the module documentation).
    pub(super) fn own(package: &str, framework: &str) -> bool {
        let prefixed = [format!("{framework}-"), format!("{framework}_")]
            .iter()
            .any(|p| package.starts_with(p.as_str()));
        package == framework || (prefixed && !package.contains("example"))
    }

    /// `framework::` in a file below no package.
    fn names_framework(mask: &str) -> bool {
        FRAMEWORKS.iter().any(|(dep, _)| {
            let b = mask.as_bytes();
            (crate::lang_rust::words(mask, dep).into_iter())
                .any(|at| b[at + dep.len()..].starts_with(b"::"))
        })
    }

    /// The application of a file below no package: its path without `.rs`,
    /// for `main.rs`, `lib.rs` and `mod.rs` its folder's path.
    fn file_app(rel: &str) -> String {
        let stem = rel.trim_end_matches(".rs");
        let folder = ["/main", "/lib", "/mod"]
            .iter()
            .find_map(|f| stem.strip_suffix(f));
        folder.unwrap_or(stem).to_string()
    }

    fn owner<'a>(
        rel: &str,
        packages: &'a BTreeMap<String, Package>,
    ) -> Option<(&'a str, &'a Package)> {
        ancestors(rel).find_map(|dir| packages.get_key_value(dir).map(|(k, p)| (k.as_str(), p)))
    }

    /// Tauri configuration files of the package and the frontend sources
    /// below the application folder (the package's parent for `src-tauri`).
    fn tauri_files(root: &Path, app: &mut App, files: &[String], flow: &mut UiFlow) {
        let under = |dir: &str, rel: &str| dir.is_empty() || rel.starts_with(&format!("{dir}/"));
        let local = |rel: &str| {
            rel.strip_prefix(&format!("{}/", app.dir))
                .unwrap_or(rel)
                .to_string()
        };
        let config = |rel: &&String| {
            let l = local(rel);
            under(&app.dir, rel)
                && (matches!(
                    l.as_str(),
                    "tauri.conf.json" | "tauri.conf.json5" | "Tauri.toml" | "build.rs"
                ) || (l.starts_with("capabilities/")
                    && [".json", ".json5", ".toml"].iter().any(|e| l.ends_with(e)))
                    || (l.starts_with("permissions/")
                        && [".json", ".toml"].iter().any(|e| l.ends_with(e))))
        };
        app.tauri_files = files.iter().filter(config).cloned().collect();
        let front = if app.dir.ends_with("src-tauri") {
            parent(&app.dir)
        } else {
            app.dir.as_str()
        }
        .to_string();
        let web = (files.iter()).filter(|r| {
            under(&front, r) && WEB.iter().any(|e| r.ends_with(e)) && !r.ends_with(".d.ts")
        });
        let web: Vec<String> = web.cloned().collect();
        for rel in web {
            if let Some(text) = read(root, &rel, flow) {
                app.web.push((rel, text));
            }
        }
    }

    fn parent(rel: &str) -> &str {
        rel.rsplit_once('/').map_or("", |(dir, _)| dir)
    }

    pub(super) fn read(root: &Path, rel: &str, flow: &mut UiFlow) -> Option<String> {
        let text = read_text(root, rel, MAX_BYTES);
        if text.is_err() {
            let message = "File not read: larger than 1 MiB, not a regular file or not readable; its screens and navigation are unknown.";
            nav::diag(flow, rel, 0, message.into(), true);
        }
        text.ok()
    }
}

/// Targets, origins and diagnostics shared by the framework rules.
mod nav {
    use super::*;
    use crate::lang_text::{next_nonws, split_top};
    use archguard_core::{
        DiagnosticSeverity, Origin, ScreenKind, Target, Transition, UiFlowDiagnostic, best_route,
    };

    /// Diagnostic at `file:line`; a gap also marks the flow incomplete.
    pub(crate) fn diag(flow: &mut UiFlow, file: &str, line: u32, message: String, gap: bool) {
        let severity = if gap {
            DiagnosticSeverity::Gap
        } else {
            DiagnosticSeverity::Note
        };
        (flow.diagnostics).push(UiFlowDiagnostic {
            file: file.into(),
            line,
            message,
            severity,
        });
    }

    /// `name`'s `[views.access]` meaning as a route condition: `None` when
    /// not configured, else the condition (`Some(None)` for an open marker).
    pub(crate) fn configured(
        access: &BTreeMap<String, archguard_core::AccessMeaning>,
        name: &str,
        place: (&Src, usize),
        flow: &mut UiFlow,
    ) -> Option<Option<archguard_core::Condition>> {
        use archguard_core::{AccessMeaning, Condition};
        let meaning = access.get(name)?.at_use_site(false);
        Some(match meaning {
            AccessMeaning::Open => None,
            AccessMeaning::Authenticated => Some(Condition::Authenticated),
            AccessMeaning::Never => Some(Condition::Never),
            AccessMeaning::Role(r) => Some(Condition::Role(r.clone())),
            AccessMeaning::Policy(p) => Some(Condition::Policy(p.clone())),
            _ => {
                let message = format!(
                    "[views.access] gives `{name}` a $arg meaning, but a route condition has no arguments."
                );
                diag(flow, &place.0.rel, place.0.line(place.1), message, true);
                Some(Condition::Unknown(name.into()))
            }
        })
    }

    /// A screen of this application.
    pub(crate) fn screen(
        app: &App,
        src: &Src,
        at: usize,
        id: &str,
        name: &str,
        kind: ScreenKind,
    ) -> Screen {
        Screen {
            weak: false,
            id: format!("{}:{id}", app.name),
            name: name.into(),
            kind,
            route: None,
            application: Some(app.name.clone()),
            file: src.rel.clone(),
            line: src.line(at),
            access: archguard_core::AccessState::Undeclared,
            source: SOURCE.into(),
        }
    }

    pub(crate) fn any(app: &App) -> Origin {
        Origin::AnyScreen {
            application: app.name.clone(),
        }
    }

    /// Code at `at` starts at the page its `fn` renders, when exactly one
    /// page and one `fn` of the application have that name; else at any
    /// screen.
    pub(crate) fn origin(cx: Cx, at: usize) -> Origin {
        let Some(f) = crate::lang_rust::fn_at(&cx.src.fns, at) else {
            return any(cx.app);
        };
        let fns = (cx.app.rust.iter())
            .flat_map(|s| &s.fns)
            .filter(|g| g.name == f.name);
        if fns.count() != 1 {
            return any(cx.app);
        }
        let pages: Vec<&Screen> = (cx.known.screens.iter())
            .filter(|s| s.kind == ScreenKind::Page && s.name == f.name)
            .collect();
        match pages.as_slice() {
            [page] => Origin::Screen(page.id.clone()),
            _ => any(cx.app),
        }
    }

    /// A transition at `at`; an unresolved target adds a gap.
    pub(crate) fn transition(
        flow: &mut UiFlow,
        src: &Src,
        at: usize,
        from: Origin,
        target: Target,
        trigger: Option<String>,
    ) {
        let line = src.line(at);
        if let Target::Unresolved(text) = &target {
            let message =
                format!("`{text}` is no literal route or known screen; the target is unknown.");
            diag(flow, &src.rel, line, message, true);
        }
        flow.transitions.push(Transition {
            weak: false,
            from,
            target,
            trigger,
            file: src.rel.clone(),
            line,
            condition: archguard_core::AccessState::Undeclared,
            source: SOURCE.into(),
        });
    }

    /// The page whose route matches `path` best.
    pub(crate) fn path_target(known: &Known, path: &str) -> Target {
        let routes =
            (known.screens.iter()).filter_map(|s| Some((s.id.as_str(), s.route.as_deref()?)));
        match best_route(path, routes) {
            Ok(id) => Target::Screen(id.to_string()),
            Err(_) => Target::Unresolved(path.to_string()),
        }
    }

    /// `Enum::Variant` (also `&crate::Enum::Variant { … }`) of a known enum.
    pub(crate) fn variant_target(known: &Known, expr: &str) -> Option<Target> {
        let expr = expr.trim().trim_start_matches('&').trim();
        let end = expr
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
            .unwrap_or(expr.len());
        let segments: Vec<&str> = expr[..end].split("::").collect();
        let [.., e, v] = segments.as_slice() else {
            return None;
        };
        let enumerated = known.variants.keys().any(|(k, _)| k == e);
        let found = known.variants.get(&(e.to_string(), v.to_string()));
        let rest = expr[end..].trim_start();
        let literal = rest.is_empty() || rest.starts_with(['{', '(', ')', ',']);
        enumerated.then(|| match found {
            Some(id) if literal => Target::Screen(id.clone()),
            _ => Target::Unresolved(expr.to_string()),
        })
    }

    /// A URL value: a literal, `"…".into()`/`.to_string()`/`String::from`,
    /// or `format!("…", …)` with every `{…}` hole a parameter.
    pub(crate) fn url_value(src: &Src, expr: &str) -> Option<String> {
        let mut e = expr.trim();
        for suffix in [".into()", ".to_string()", ".to_owned()"] {
            e = e.strip_suffix(suffix).unwrap_or(e);
        }
        if let Some(inner) = e
            .strip_prefix("String::from(")
            .and_then(|r| r.strip_suffix(')'))
        {
            e = inner;
        }
        if let Some(v) = src.lit(e) {
            return Some(v);
        }
        let b = e.as_bytes();
        let open = next_nonws(b, "format!".len()).filter(|_| e.starts_with("format!"))?;
        let close = crate::lang_text::matching(b, open)?;
        let first = split_top(&e[open + 1..close], ",")[0];
        Some(holes(&src.lit(first)?))
    }

    /// `{x}` holes as `:param` segments, `{{`/`}}` as braces.
    fn holes(template: &str) -> String {
        let (mut out, mut chars) = (String::new(), template.chars().peekable());
        while let Some(c) = chars.next() {
            match (c, chars.peek()) {
                ('{', Some('{')) | ('}', Some('}')) => out.push(chars.next().unwrap_or(c)),
                ('{', _) => {
                    chars.by_ref().find(|&c| c == '}');
                    out.push_str(crate::lang_ts::PARAM);
                }
                _ => out.push(c),
            }
        }
        out
    }

    /// An address outside the application (`https://…`, `mailto:`).
    pub(crate) fn external(url: &str) -> bool {
        url.contains("://") || url.starts_with("mailto:") || url.starts_with("tel:")
    }

    /// The route of `parts` joined: `:..rest` and `*rest` become `*`, a
    /// trailing `?` of a parameter is dropped, query and hash parts too.
    pub(crate) fn route_of<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
        let joined = parts.into_iter().collect::<Vec<_>>().join("/");
        let path = joined.split(['?', '#']).next().unwrap_or("");
        let segments: Vec<&str> = (path.split('/'))
            .filter(|s| !s.is_empty())
            .map(|s| match s {
                s if s.starts_with(":..") || s.starts_with('*') => "*",
                s => s.strip_suffix('?').unwrap_or(s),
            })
            .collect();
        format!("/{}", segments.join("/"))
    }

    /// The text of a tag or call argument as written, for messages.
    pub(crate) fn shown(src: &Src, part: &str) -> String {
        crate::lang_text::collapse(src.text(part))
    }
}

/// Labels of buttons and tags.
mod label {
    use super::*;
    use crate::lang_rust::string_at;
    use crate::lang_text::{is_id, matching, prev_nonws, prev_word};

    /// Start of the expression chain holding `at`: back to `;`, `,` or an
    /// open bracket, skipping the arguments of method calls (`.on_press(`).
    pub(crate) fn chain_start(mask: &str, at: usize) -> usize {
        let b = mask.as_bytes();
        let mut depth = 0usize;
        for k in (0..at).rev() {
            match b[k] {
                b')' | b']' | b'}' => depth += 1,
                b'(' | b'[' | b'{' if depth > 0 => depth -= 1,
                b'(' if method_call(mask, k) => {}
                b'(' | b'[' | b'{' => return k + 1,
                b';' | b',' if depth == 0 => return k + 1,
                _ => {}
            }
        }
        0
    }

    fn method_call(mask: &str, open: usize) -> bool {
        let word = prev_word(mask, open);
        let start = prev_nonws(mask.as_bytes(), open).map_or(0, |e| e + 1 - word.len());
        !word.is_empty() && start > 0 && mask.as_bytes()[start - 1] == b'.'
    }

    /// The last `…button("x")` or `Button::new("x")` in `from..to`.
    pub(crate) fn button_in(src: &Src, from: usize, to: usize) -> Option<String> {
        let b = src.mask.as_bytes();
        (from..to.min(b.len())).rev().find_map(|k| {
            if b[k] != b'(' {
                return None;
            }
            let word = prev_word(&src.mask, k);
            let button = word.to_ascii_lowercase().ends_with("button")
                || (word == "new" && src.mask[..k].trim_end().ends_with("Button::new"));
            button
                .then(|| string_at(&src.code, k + 1))
                .flatten()
                .map(|(v, _)| v)
        })
    }

    /// The button in the header of the block around `at` (`if ui.button("x")
    /// .clicked() { … }`).
    pub(crate) fn block(src: &Src, at: usize) -> Option<String> {
        let b = src.mask.as_bytes();
        let mut depth = 0usize;
        let open = (0..at).rev().find(|&k| match b[k] {
            b'}' => {
                depth += 1;
                false
            }
            b'{' if depth > 0 => {
                depth -= 1;
                false
            }
            c => c == b'{',
        })?;
        button_in(src, chain_start(&src.mask, open), open)
    }

    /// The end of the tag whose name ends before `from`: offset of its `>`
    /// and whether it closes itself (`/>`).
    pub(crate) fn tag_end(mask: &str, from: usize) -> Option<(usize, bool)> {
        let b = mask.as_bytes();
        let mut k = from;
        while k < b.len() {
            match b[k] {
                b'(' | b'[' | b'{' | b'<' => k = matching(b, k)?,
                b'>' if !matches!(b[k - 1], b'=' | b'-') => return Some((k, b[k - 1] == b'/')),
                _ => {}
            }
            k += 1;
        }
        None
    }

    /// `name=value` attributes of a tag in `from..to`: (name, value) slices of
    /// `mask`; a value runs to the next attribute.
    pub(crate) fn tag_attrs(mask: &str, from: usize, to: usize) -> Vec<(&str, &str)> {
        let b = mask.as_bytes();
        let mut starts = vec![];
        let mut depth = 0i32;
        for k in from..to {
            match b[k] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b'=' if depth == 0
                    && !matches!(b.get(k + 1), Some(b'=' | b'>'))
                    && is_id(b[k - 1]) =>
                {
                    let start = k - prev_word(mask, k).len();
                    if start > 0 && b[start - 1].is_ascii_whitespace() {
                        starts.push((start, k));
                    }
                }
                _ => {}
            }
        }
        let ends = (starts.iter().skip(1).map(|s| s.0)).chain([to]);
        (starts.iter().zip(ends))
            .map(|(&(s, eq), end)| {
                (
                    &mask[s..eq],
                    mask[eq + 1..end].trim().trim_end_matches('/').trim(),
                )
            })
            .collect()
    }

    /// The literal text between a tag's `>` and `</name`: `"x"` or `{ "x" }`.
    pub(crate) fn content(src: &Src, gt: usize, name: &str) -> Option<String> {
        let close = src.mask[gt..].find(&format!("</{name}"))? + gt;
        let inner = src.mask[gt + 1..close].trim();
        let inner = (inner.strip_prefix('{').and_then(|i| i.strip_suffix('}'))).unwrap_or(inner);
        src.lit(inner.trim())
    }
}

/// Dioxus and Yew: `#[derive(Routable)]` enums.
mod routable {
    use super::*;
    use crate::lang_rust::{Attr, attr_at, attrs_before, words};
    use crate::lang_text::{ident_end, is_id, matching, next_nonws, split_top};
    use archguard_core::{ScreenKind, Target};

    /// An attribute or a variant in an enum body, in source order.
    pub(crate) enum Part {
        Attr(Attr),
        Variant(String, usize),
    }

    /// Name and body braces of the `enum` at `at`.
    pub(crate) fn enum_body(mask: &str, at: usize) -> Option<(String, usize, usize)> {
        let b = mask.as_bytes();
        let start = next_nonws(b, at + 4)?;
        let name = mask[start..ident_end(b, start)].to_string();
        let open = start + mask[start..].find(['{', ';'])?;
        (b[open] == b'{' && !name.is_empty()).then(|| Some((name, open, matching(b, open)?)))?
    }

    /// The attributes and variants of an enum body.
    pub(crate) fn parts(mask: &str, open: usize, close: usize) -> Vec<Part> {
        let b = mask.as_bytes();
        let (mut out, mut i) = (vec![], open + 1);
        while let Some(k) = next_nonws(b, i).filter(|&k| k < close) {
            if let Some(a) = attr_at(mask, k) {
                out.push(Part::Attr(a));
                i = a.end + 1;
            } else if is_id(b[k]) {
                let end = ident_end(b, k);
                out.push(Part::Variant(mask[k..end].to_string(), k));
                i = past_variant(b, end, close);
            } else {
                i = k + 1;
            }
        }
        out
    }

    fn past_variant(b: &[u8], mut k: usize, close: usize) -> usize {
        while k < close {
            match b[k] {
                b'{' | b'(' => k = matching(b, k).unwrap_or(close),
                b',' => return k + 1,
                _ => {}
            }
            k += 1;
        }
        close
    }

    /// The pages of every routable enum.
    pub(crate) fn screens(
        app: &App,
        access: &BTreeMap<String, archguard_core::AccessMeaning>,
        flow: &mut UiFlow,
        known: &mut Known,
    ) {
        for src in &app.rust {
            for at in words(&src.mask, "enum") {
                let routable = attrs_before(&src.mask, at).iter().any(|a| {
                    a.name(&src.mask) == "derive"
                        && a.args
                            .is_some_and(|(o, c)| !words(&src.mask[o..c], "Routable").is_empty())
                });
                if let Some((name, open, close)) = enum_body(&src.mask, at).filter(|_| routable) {
                    known.routable.insert(name.clone());
                    routes(app, src, &name, parts(&src.mask, open, close), flow, known);
                }
            }
        }
        components(app, known);
        super::guard::guards(app, access, flow, known);
    }

    fn routes(
        app: &App,
        src: &Src,
        name: &str,
        parts: Vec<Part>,
        flow: &mut UiFlow,
        known: &mut Known,
    ) {
        let (mut nest, mut route) = (Vec::<String>::new(), None);
        for part in parts {
            match part {
                Part::Attr(a) => {
                    let args = attr_args(src, &a);
                    let first = || args.first().and_then(|p| src.lit(p));
                    match a.name(&src.mask) {
                        "nest" => nest.push(first().unwrap_or_else(|| unreadable(flow, src, &a))),
                        "end_nest" => drop(nest.pop()),
                        "route" | "at" => {
                            let path = first().unwrap_or_else(|| unreadable(flow, src, &a));
                            route = Some((path, args.get(1).map(|c| c.trim().to_string())));
                        }
                        "child" => {
                            let message = "A child router is not followed; its pages are unknown.";
                            nav::diag(flow, &src.rel, src.line(a.at), message.into(), true);
                        }
                        _ => {}
                    }
                }
                Part::Variant(variant, at) => {
                    let Some((path, comp)) = route.take() else {
                        continue;
                    };
                    let id = format!("{name}::{variant}");
                    let mut screen = nav::screen(
                        app,
                        src,
                        at,
                        &id,
                        comp.as_deref().unwrap_or(&variant),
                        ScreenKind::Page,
                    );
                    screen.route = Some(nav::route_of(
                        nest.iter().map(String::as_str).chain([path.as_str()]),
                    ));
                    known
                        .variants
                        .insert((name.to_string(), variant), screen.id.clone());
                    known.screens.push(screen);
                }
            }
        }
    }

    fn attr_args<'a>(src: &'a Src, a: &Attr) -> Vec<&'a str> {
        a.args
            .map_or(vec![], |(o, c)| split_top(&src.mask[o + 1..c], ","))
    }

    fn unreadable(flow: &mut UiFlow, src: &Src, a: &Attr) -> String {
        let message = "The route attribute holds no string literal; the route is unknown.";
        nav::diag(flow, &src.rel, src.line(a.at), message.into(), true);
        crate::lang_ts::PARAM.into()
    }

    /// Yew: `Enum::Variant => html! { <Comp … /> }` names the page's component.
    fn components<'a>(app: &'a App, known: &mut Known) {
        let ids: BTreeMap<String, (String, String)> = (known.variants.iter())
            .filter(|((e, _), _)| known.routable.contains(e))
            .map(|((e, v), id)| (id.clone(), (e.clone(), v.clone())))
            .collect();
        for screen in known.screens.iter_mut() {
            let Some((e, v)) = ids.get(&screen.id) else {
                continue;
            };
            let pattern = format!("{e}::{v}");
            let after = |src: &'a Src| {
                let ats = words(&src.mask, &pattern).into_iter();
                ats.filter_map(|at| arm_component(&src.mask, at + pattern.len()))
                    .collect::<Vec<_>>()
            };
            let found: BTreeSet<&str> = app.rust.iter().flat_map(after).collect();
            if let [comp] = found.into_iter().collect::<Vec<_>>().as_slice() {
                screen.name = comp.to_string();
            }
        }
    }

    fn arm_component(mask: &str, mut k: usize) -> Option<&str> {
        let b = mask.as_bytes();
        k = next_nonws(b, k)?;
        if matches!(b[k], b'{' | b'(') {
            k = next_nonws(b, matching(b, k)? + 1)?;
        }
        let rest = mask[k..]
            .strip_prefix("=>")?
            .trim_start()
            .strip_prefix("html!")?;
        let rest = rest
            .trim_start()
            .strip_prefix('{')?
            .trim_start()
            .strip_prefix('<')?;
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        rest.starts_with(|c: char| c.is_ascii_uppercase())
            .then(|| &rest[..end])
    }

    /// Dioxus `Link { to: … }`, Yew `<Link<R> to={…}>` and router pushes.
    pub(crate) fn links(cx: Cx, flow: &mut UiFlow) {
        let b = cx.src.mask.as_bytes();
        for at in words(&cx.src.mask, "Link") {
            let Some(k) = next_nonws(b, at + 4) else {
                continue;
            };
            if b[k] == b'{' {
                dioxus_link(cx, flow, at, k);
            } else if at > 0 && b[at - 1] == b'<' {
                yew_link(cx, flow, at);
            }
        }
        pushes(cx, flow);
    }

    fn dioxus_link(cx: Cx, flow: &mut UiFlow, at: usize, open: usize) {
        let src = cx.src;
        let Some(close) = matching(src.mask.as_bytes(), open) else {
            return;
        };
        let parts = split_top(&src.mask[open + 1..close], ",");
        let to = (parts.iter()).find_map(|p| {
            p.trim()
                .strip_prefix("to")
                .and_then(|r| r.trim_start().strip_prefix(':'))
        });
        let label = (parts.iter()).find_map(|p| src.lit(p.trim()).filter(|l| !l.contains('{')));
        if let Some(to) = to {
            link(cx, flow, at, to, label);
        }
    }

    fn yew_link(cx: Cx, flow: &mut UiFlow, at: usize) {
        let src = cx.src;
        let b = src.mask.as_bytes();
        let mut from = at + 4;
        if b.get(from) == Some(&b'<') {
            from = matching(b, from).map_or(from, |g| g + 1);
        }
        let Some((gt, closed)) = label::tag_end(&src.mask, from) else {
            return;
        };
        let attrs = label::tag_attrs(&src.mask, from, gt);
        let Some((_, to)) = attrs.into_iter().find(|(n, _)| *n == "to") else {
            return;
        };
        let to = to
            .strip_prefix('{')
            .and_then(|t| t.strip_suffix('}'))
            .unwrap_or(to);
        let trigger = (!closed).then(|| label::content(src, gt, "Link")).flatten();
        link(cx, flow, at, to, trigger);
    }

    fn link(cx: Cx, flow: &mut UiFlow, at: usize, to: &str, trigger: Option<String>) {
        let target = match (
            nav::variant_target(cx.known, to),
            nav::url_value(cx.src, to),
        ) {
            (Some(t), _) => t,
            (None, Some(url)) if nav::external(&url) => return,
            (None, Some(url)) => nav::path_target(cx.known, &url),
            (None, None) => Target::Unresolved(nav::shown(cx.src, to)),
        };
        nav::transition(flow, cx.src, at, nav::origin(cx, at), target, trigger);
    }

    /// `.push(x)`/`.replace(x)`: a known enum variant anywhere, any value on
    /// a navigator.
    fn pushes(cx: Cx, flow: &mut UiFlow) {
        let src = cx.src;
        let b = src.mask.as_bytes();
        let navigators = bound(src, &["use_navigator", "navigator", "router"]);
        for method in ["push", "replace"] {
            for at in words(&src.mask, method) {
                let open = next_nonws(b, at + method.len()).filter(|&k| b[k] == b'(');
                let close =
                    (open.filter(|_| at > 0 && b[at - 1] == b'.')).and_then(|o| matching(b, o));
                let (Some(open), Some(close)) = (open, close) else {
                    continue;
                };
                let arg = &src.mask[open + 1..close];
                let target = match nav::variant_target(cx.known, arg) {
                    Some(t) => t,
                    None if !on_navigator(&src.mask[..at - 1], &navigators) => continue,
                    None => match nav::url_value(src, arg) {
                        Some(url) => nav::path_target(cx.known, &url),
                        None => Target::Unresolved(nav::shown(src, arg)),
                    },
                };
                nav::transition(flow, src, at, nav::origin(cx, at), target, None);
            }
        }
    }

    /// The receiver before `.push` is `navigator()`, `router()`,
    /// `use_navigator()` or a variable bound to one of them.
    fn on_navigator(before: &str, navigators: &[String]) -> bool {
        let receiver = before.trim_end();
        let calls = ["navigator()", "router()", "use_navigator()"];
        let word = crate::lang_text::prev_word(receiver, receiver.len());
        calls.iter().any(|r| receiver.ends_with(r)) || navigators.iter().any(|n| n == word)
    }

    /// Variables bound by `let x = f()` for one of `calls` (`.unwrap()` allowed).
    pub(crate) fn bound(src: &Src, calls: &[&str]) -> Vec<String> {
        let b = src.mask.as_bytes();
        let mut out = vec![];
        for call in calls {
            for at in words(&src.mask, call) {
                let before = src.mask[..at].trim_end();
                let called = b.get(at + call.len()) == Some(&b'(');
                let Some(before) = before.strip_suffix('=').filter(|_| called) else {
                    continue;
                };
                let before = before.trim_end();
                let name = crate::lang_text::prev_word(before, before.len());
                let head = before[..before.len() - name.len()].trim_end();
                if !name.is_empty() && (head.ends_with("let") || head.ends_with("let mut")) {
                    out.push(name.to_string());
                }
            }
        }
        out
    }
}

/// Guards the Dioxus and Yew routing code puts on routable pages.
mod guard {
    use super::*;
    use crate::lang_rust::words;
    use crate::lang_text::{is_id, matching, next_nonws};

    /// (enum, variant, screen id) of every routable page, and the conditions
    /// the routing code adds to a screen id.
    type Pages = Vec<(String, String, String)>;
    type Marks = BTreeSet<(String, archguard_core::Condition)>;

    /// Guards the routing code puts on pages: a Dioxus `RouterConfig`
    /// `on_update` callback (any page of its router), a Yew `switch` arm
    /// that branches (`Route::X => if cond { … } else { … }`, `match`, or a
    /// match guard). Recognised but not readable is `unknown` with a gap;
    /// a condition naming a `[views.access]` function takes its meaning.
    pub(crate) fn guards(
        app: &App,
        access: &BTreeMap<String, archguard_core::AccessMeaning>,
        flow: &mut UiFlow,
        known: &mut Known,
    ) {
        use archguard_core::AccessState;
        let mut marks = Marks::new();
        let pages: Pages = (known.variants.iter())
            .filter(|((e, _), _)| known.routable.contains(e))
            .map(|((e, v), id)| (e.clone(), v.clone(), id.clone()))
            .collect();
        for src in &app.rust {
            on_update(src, &pages, flow, &mut marks);
            for (e, v, id) in &pages {
                let pattern = format!("{e}::{v}");
                for at in words(&src.mask, &pattern) {
                    if let Some(c) = arm_condition(src, (at, &pattern), access, flow) {
                        marks.insert((id.clone(), c));
                    }
                }
            }
        }
        for (id, condition) in marks {
            if let Some(screen) = known.screens.iter_mut().find(|s| s.id == id) {
                let own = std::mem::take(&mut screen.access);
                screen.access = own.and(AccessState::Protected(condition));
            }
        }
    }

    /// `.on_update(…)`: every page of the router it configures is unknown.
    fn on_update(src: &Src, pages: &Pages, flow: &mut UiFlow, marks: &mut Marks) {
        let b = src.mask.as_bytes();
        for at in words(&src.mask, "on_update") {
            if at == 0 || b[at - 1] != b'.' {
                continue;
            }
            let named = router_enum(&src.mask, at);
            let message = "A router `on_update` callback can redirect every navigation; the access of its pages is unknown.";
            nav::diag(flow, &src.rel, src.line(at), message.into(), true);
            for (e, _, id) in pages {
                if named.as_ref().is_none_or(|n| n == e) {
                    let unknown = archguard_core::Condition::Unknown("on_update".into());
                    marks.insert((id.clone(), unknown));
                }
            }
        }
    }

    /// The condition of the arm of `pattern` at offset `at`; `None` for no
    /// guard or an open marker.
    fn arm_condition(
        src: &Src,
        (at, pattern): (usize, &str),
        access: &BTreeMap<String, archguard_core::AccessMeaning>,
        flow: &mut UiFlow,
    ) -> Option<archguard_core::Condition> {
        let (cond_at, cond) = arm_guard(&src.mask, at + pattern.len())?;
        let text = cond.trim().trim_end_matches("()");
        let path = (text.chars()).all(|c| c.is_alphanumeric() || c == '_' || c == ':');
        let name = text.rsplit("::").next().unwrap_or(text);
        match path
            .then(|| nav::configured(access, name, (src, at), flow))
            .flatten()
        {
            Some(read) => read,
            None => {
                let shown = nav::shown(src, cond);
                let message = format!(
                    "The guard `{shown}` of route `{pattern}` is not read; its access is unknown."
                );
                nav::diag(flow, &src.rel, src.line(cond_at), message, true);
                Some(archguard_core::Condition::Unknown(shown))
            }
        }
    }

    /// The enum of the `Router::<Enum> { … }` block holding offset `at`.
    fn router_enum(mask: &str, at: usize) -> Option<String> {
        let b = mask.as_bytes();
        let found = words(&mask[..at], "Router").into_iter().rev();
        found.into_iter().find_map(|w| {
            let rest = mask[w + 6..].strip_prefix("::<")?;
            let end = rest.find('>')?;
            let open = next_nonws(b, w + 6 + 3 + end + 1).filter(|&k| b[k] == b'{')?;
            (matching(b, open)? > at).then(|| rest[..end].trim().to_string())
        })
    }

    /// The condition of the arm whose pattern ended at `k`: (offset, text) of
    /// an `if`/`match` header after `=>`, or a match guard before it.
    fn arm_guard(mask: &str, mut k: usize) -> Option<(usize, &str)> {
        let b = mask.as_bytes();
        k = next_nonws(b, k)?;
        if matches!(b[k], b'{' | b'(') {
            k = next_nonws(b, matching(b, k)? + 1)?;
        }
        if mask[k..].starts_with("if") && !b.get(k + 2).is_some_and(|&c| is_id(c)) {
            let end = mask[k..].find("=>")? + k;
            return Some((k + 2, &mask[k + 2..end]));
        }
        let rest = mask[k..].strip_prefix("=>")?;
        let k = mask.len() - rest.len();
        header(mask, next_nonws(b, k)?)
    }

    /// `if cond {` or `match scrutinee {` at `k`, or the first one in a block.
    fn header(mask: &str, k: usize) -> Option<(usize, &str)> {
        let b = mask.as_bytes();
        let word = |at: usize, w: &str| {
            mask[at..].starts_with(w) && !b.get(at + w.len()).is_some_and(|&c| is_id(c))
        };
        if b[k] == b'{' {
            let close = matching(b, k)?;
            let at = words(&mask[..close], "if")
                .into_iter()
                .chain(words(&mask[..close], "match"))
                .filter(|&a| a > k)
                .min()?;
            return header(mask, at);
        }
        let skip = if word(k, "if") {
            2
        } else if word(k, "match") {
            0
        } else {
            return None;
        };
        let mut i = k + skip;
        while i < mask.len() && b[i] != b'{' {
            i = match b[i] {
                b'(' | b'[' => matching(b, i)? + 1,
                _ => i + 1,
            };
        }
        Some((k + skip, &mask[k + skip..i]))
    }
}

/// Leptos `<Routes>` trees and links.
mod leptos {
    use super::*;
    use crate::lang_rust::words;
    use crate::lang_text::{matching, next_nonws, split_top};
    use archguard_core::{AccessMeaning, Condition, Origin, ScreenKind, Target};

    const LEAVES: [&str; 2] = ["Route", "ProtectedRoute"];
    const PARENTS: [&str; 2] = ["ParentRoute", "ProtectedParentRoute"];

    /// One open parent route: its path, the path without optional segments,
    /// and its condition.
    #[derive(Clone)]
    struct Level {
        path: String,
        required: String,
        condition: Option<Condition>,
    }

    /// The route tags of one file.
    struct Tree<'a> {
        app: &'a App,
        src: &'a Src,
        access: &'a BTreeMap<String, AccessMeaning>,
    }

    /// Components that hold route tags and are placed by a `<Name/>` tag:
    /// name -> (source index, body braces). Their routes belong below the
    /// route they are placed in (`<ContactRoutes/>`), not to the root.
    type Places = BTreeMap<String, (usize, (usize, usize))>;

    fn route_tags(mask: &str, (open, close): (usize, usize)) -> bool {
        let found = |tag: &str| !tags(&mask[open..close], &format!("<{tag}")).is_empty();
        LEAVES.iter().chain(&PARENTS).any(|t| found(t))
    }

    fn places(app: &App) -> Places {
        let mut out = Places::new();
        for (i, src) in app.rust.iter().enumerate() {
            for f in fn_items(&src.mask) {
                let placed = app
                    .rust
                    .iter()
                    .any(|s| !tags(&s.mask, &format!("<{}", f.name)).is_empty());
                if placed && route_tags(&src.mask, f.body) {
                    out.entry(f.name).or_insert((i, f.body));
                }
            }
        }
        out
    }

    pub(crate) fn screens(
        app: &App,
        access: &BTreeMap<String, AccessMeaning>,
        flow: &mut UiFlow,
        known: &mut Known,
    ) {
        let places = places(app);
        for (i, src) in app.rust.iter().enumerate() {
            let skip: Vec<(usize, usize)> = (places.values())
                .filter(|p| p.0 == i)
                .map(|p| p.1)
                .collect();
            let mut place = Place {
                app,
                access,
                places: &places,
                active: vec![],
            };
            place.walk((i, (0, src.mask.len())), &skip, &[], flow, known);
        }
    }

    struct Place<'a> {
        app: &'a App,
        access: &'a BTreeMap<String, AccessMeaning>,
        places: &'a Places,
        /// Components being walked; a component placed inside itself is skipped.
        active: Vec<String>,
    }

    impl Place<'_> {
        /// The route tags of `scope` (source index, range) below the levels
        /// `base`; a placed component's routes are walked at its tag.
        fn walk(
            &mut self,
            (i, range): (usize, (usize, usize)),
            skip: &[(usize, usize)],
            base: &[Level],
            flow: &mut UiFlow,
            known: &mut Known,
        ) {
            let (app, places) = (self.app, self.places);
            let src = &app.rust[i];
            let inside = |at: usize| {
                range.0 <= at && at < range.1 && !skip.iter().any(|&(o, c)| o <= at && at < c)
            };
            // (offset, tag, kind): 0 closes a parent, 1 opens a route, 2 places a component.
            let mut events: Vec<(usize, &str, u8)> = vec![];
            for tag in LEAVES.iter().chain(&PARENTS) {
                let found = tags(&src.mask, &format!("<{tag}"));
                events.extend(found.into_iter().map(|at| (at, *tag, 1)));
            }
            for tag in PARENTS {
                let found = tags(&src.mask, &format!("</{tag}"));
                events.extend(found.into_iter().map(|at| (at, tag, 0)));
            }
            for name in places.keys() {
                let found = tags(&src.mask, &format!("<{name}"));
                events.extend(found.into_iter().map(|at| (at, name.as_str(), 2)));
            }
            events.retain(|e| inside(e.0));
            events.sort();
            let access = self.access;
            let tree = Tree { app, src, access };
            let mut stack: Vec<Level> = base.to_vec();
            for (at, tag, kind) in events {
                match kind {
                    0 => {
                        if stack.len() > base.len() {
                            stack.pop();
                        }
                    }
                    1 => {
                        if let Some(level) = tree.route(at, tag, &stack, flow, known) {
                            stack.push(level);
                        }
                    }
                    _ if self.active.iter().any(|n| n == tag) => {}
                    _ => {
                        self.active.push(tag.to_string());
                        self.walk(places[tag], &[], &stack, flow, known);
                        self.active.pop();
                    }
                }
            }
        }
    }

    /// Offsets of `<Tag` with no identifier byte after and none or `:` before
    /// (`Link<Route>` and `Router::<Route>` are type arguments).
    fn tags(mask: &str, tag: &str) -> Vec<usize> {
        let b = mask.as_bytes();
        let id = |k: Option<&u8>| k.is_some_and(|&c| crate::lang_text::is_id(c));
        let typed = |at: usize| at > 0 && (id(b.get(at - 1)) || b[at - 1] == b':');
        (mask.match_indices(tag))
            .map(|(at, _)| at)
            .filter(|&at| !id(b.get(at + tag.len())) && !typed(at))
            .collect()
    }

    impl Tree<'_> {
        /// A route tag: a page for a leaf, the level of an open parent.
        fn route(
            &self,
            at: usize,
            tag: &str,
            stack: &[Level],
            flow: &mut UiFlow,
            known: &mut Known,
        ) -> Option<Level> {
            let src = self.src;
            let from = at + 1 + tag.len();
            let (gt, closed) = label::tag_end(&src.mask, from)?;
            let attrs = label::tag_attrs(&src.mask, from, gt);
            let get = |name: &str| attrs.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
            let Some((path, required)) = get("path").and_then(|p| path(src, p)) else {
                let message = format!("The path of <{tag}> is not readable; its route is unknown.");
                nav::diag(flow, &src.rel, src.line(at), message, true);
                let param = crate::lang_ts::PARAM.to_string();
                return PARENTS.contains(&tag).then(|| Level {
                    path: param.clone(),
                    required: param,
                    condition: None,
                });
            };
            let own = get("condition").and_then(|c| self.condition(c, flow, at));
            let conditions: Vec<Condition> = (stack.iter().filter_map(|l| l.condition.clone()))
                .chain(own)
                .collect();
            let level = Level {
                path: nav::route_of(stack.iter().map(|l| l.path.as_str()).chain([path.as_str()])),
                required: nav::route_of(
                    (stack.iter().map(|l| l.required.as_str())).chain([required.as_str()]),
                ),
                condition: match conditions.len() {
                    0 => None,
                    1 => conditions.into_iter().next(),
                    _ => Some(Condition::All(conditions)),
                },
            };
            if PARENTS.contains(&tag) && !closed {
                return Some(level);
            }
            let view = get("view").filter(|v| v.chars().all(|c| c.is_alphanumeric() || c == '_'));
            self.page(at, level, view, flow, known);
            None
        }

        fn page(
            &self,
            at: usize,
            level: Level,
            view: Option<&str>,
            flow: &mut UiFlow,
            known: &mut Known,
        ) {
            let name = view.unwrap_or(&level.path);
            let mut screen =
                nav::screen(self.app, self.src, at, &level.path, name, ScreenKind::Page);
            if level.required == "/" && level.path != "/" {
                flow.start_screens.push(screen.id.clone());
            }
            (screen.route, screen.access) = (
                Some(level.path),
                archguard_core::AccessState::from_parts(level.condition, false),
            );
            if !known.screens.iter().any(|s| s.id == screen.id) {
                known.screens.push(screen);
            }
        }

        /// `condition=f` or `move || f()`: `f`'s `[views.access]` meaning,
        /// else `custom(f)`; anything else unknown with a gap.
        fn condition(&self, value: &str, flow: &mut UiFlow, at: usize) -> Option<Condition> {
            let src = self.src;
            let text = value.trim().trim_start_matches("move").trim_start();
            let text = text.strip_prefix("||").unwrap_or(text).trim();
            let text = text.strip_suffix("()").unwrap_or(text);
            let name = text.rsplit("::").next().unwrap_or(text);
            let path = text
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == ':');
            let gap = |message: String, flow: &mut UiFlow| {
                nav::diag(flow, &src.rel, src.line(at), message, true);
            };
            if name.is_empty() || !path {
                let shown = nav::shown(src, value);
                let message = format!(
                    "The route condition `{shown}` names no function; its access is unknown."
                );
                gap(message, flow);
                return Some(Condition::Unknown(shown));
            }
            match nav::configured(self.access, name, (src, at), flow) {
                None => Some(Condition::Custom(name.into())),
                Some(condition) => condition,
            }
        }
    }

    /// The path of a route and its form without optional segments.
    fn path(src: &Src, value: &str) -> Option<(String, String)> {
        let value = value.trim();
        let lit = match value.strip_prefix("path!") {
            Some(rest) => src.lit(rest.trim().strip_prefix('(')?.strip_suffix(')')?),
            None => src.lit(value),
        };
        if let Some(lit) = lit {
            let required = lit
                .split('/')
                .filter(|s| !(s.starts_with(':') && s.ends_with('?')));
            let required = required.collect::<Vec<_>>().join("/");
            return Some((lit, required));
        }
        let inner = (value.strip_prefix('(').and_then(|v| v.strip_suffix(')'))).unwrap_or(value);
        let (mut all, mut required) = (vec![], vec![]);
        for part in split_top(inner, ",")
            .into_iter()
            .filter(|s| !s.trim().is_empty())
        {
            let (kind, arg) = part.trim().split_once('(')?;
            let arg = src.lit(arg.strip_suffix(')')?)?;
            let segment = match kind.trim() {
                "StaticSegment" => arg,
                "ParamSegment" | "OptionalParamSegment" => format!(":{arg}"),
                "WildcardSegment" => "*".into(),
                _ => return None,
            };
            if kind.trim() != "OptionalParamSegment" {
                required.push(segment.clone());
            }
            all.push(segment);
        }
        Some((all.join("/"), required.join("/")))
    }

    /// `<A href=…>`, `<Redirect path=…/>` and `navigate("…")`.
    pub(crate) fn links(cx: Cx, flow: &mut UiFlow) {
        let src = cx.src;
        for (tag, attr) in [("A", "href"), ("Redirect", "path")] {
            for at in tags(&src.mask, &format!("<{tag}")) {
                let from = at + 1 + tag.len();
                let Some((gt, closed)) = label::tag_end(&src.mask, from) else {
                    continue;
                };
                let attrs = label::tag_attrs(&src.mask, from, gt);
                if let Some((_, value)) = attrs.into_iter().find(|(n, _)| *n == attr) {
                    let content = !closed && tag == "A";
                    let trigger = content.then(|| label::content(src, gt, tag)).flatten();
                    link(cx, flow, at, value, trigger);
                }
            }
        }
        let b = src.mask.as_bytes();
        for name in routable::bound(src, &["use_navigate"]) {
            for at in words(&src.mask, &name) {
                let open = next_nonws(b, at + name.len()).filter(|&k| b[k] == b'(');
                let Some((open, close)) = open.and_then(|o| Some((o, matching(b, o)?))) else {
                    continue;
                };
                let first = split_top(&src.mask[open + 1..close], ",")[0];
                link(cx, flow, at, first, None);
            }
        }
    }

    fn link(cx: Cx, flow: &mut UiFlow, at: usize, value: &str, trigger: Option<String>) {
        let from = nav::origin(cx, at);
        let target = match (nav::url_value(cx.src, value), &from) {
            (Some(url), _) if nav::external(&url) => return,
            (Some(url), _) if url.starts_with('/') => nav::path_target(cx.known, &url),
            (Some(url), Origin::Screen(id)) => {
                let screen = cx.known.screens.iter().find(|s| &s.id == id);
                let base = screen.and_then(|s| s.route.clone()).unwrap_or_default();
                nav::path_target(cx.known, &format!("{base}/{url}"))
            }
            (Some(url), _) => Target::Unresolved(url),
            (None, _) => Target::Unresolved(nav::shown(cx.src, value)),
        };
        nav::transition(flow, cx.src, at, from, target, trigger);
    }
}

/// Leptos and Dioxus `#[server]` functions.
mod server {
    use super::*;
    use crate::lang_rust::{attrs_before, words};
    use crate::lang_text::{next_nonws, split_top};
    use archguard_core::{AccessState, Action, Call, EndpointAccess};

    /// A server function: name, struct name, method, path.
    struct ServerFn {
        name: String,
        type_name: String,
        method: &'static str,
        path: String,
    }

    pub(crate) fn run(app: &App, known: &Known, flow: &mut UiFlow) {
        let mut found = vec![];
        for src in &app.rust {
            for f in &src.fns {
                let attrs = attrs_before(&src.mask, f.at);
                let Some(attr) = attrs.iter().find(|a| a.name(&src.mask) == "server") else {
                    continue;
                };
                let args = attr
                    .args
                    .map_or(vec![], |(o, c)| split_top(&src.mask[o + 1..c], ","));
                let server = server_fn(src, &f.name, &args);
                flow.endpoints.push(EndpointAccess {
                    method: server.method.into(),
                    path: server.path.clone(),
                    access: AccessState::Undeclared,
                    application: Some(app.name.clone()),
                    file: src.rel.clone(),
                    line: src.line(f.at),
                    source: SOURCE.into(),
                    handler: Some(f.name.clone()),
                    sends: vec![],
                });
                found.push(server);
            }
        }
        for src in &app.rust {
            for server in &found {
                actions(Cx { app, src, known }, server, flow);
            }
        }
    }

    fn server_fn(src: &Src, name: &str, args: &[&str]) -> ServerFn {
        let named = |key: &str| {
            args.iter().find_map(|a| {
                let (k, v) = a.split_once('=')?;
                (k.trim() == key).then(|| v.trim())
            })
        };
        let positional: Vec<&str> = args
            .iter()
            .copied()
            .filter(|a| !a.contains(['=', ':']) && !a.trim().is_empty())
            .collect();
        let type_name = positional
            .first()
            .map(|t| t.trim().to_string())
            .unwrap_or_else(|| pascal(name));
        let prefix = named("prefix")
            .or(positional.get(1).copied())
            .and_then(|p| src.lit(p))
            .unwrap_or_else(|| "/api".into());
        let endpoint = named("endpoint")
            .and_then(|e| src.lit(e))
            .unwrap_or_else(|| name.to_string());
        let encoding = named("input")
            .map(str::to_string)
            .or_else(|| positional.get(2).and_then(|e| src.lit(e)));
        let method = if encoding.is_some_and(|e| e.trim().starts_with("Get")) {
            "GET"
        } else {
            "POST"
        };
        let path = format!(
            "{}/{}",
            prefix.trim_end_matches('/'),
            endpoint.trim_start_matches('/')
        );
        ServerFn {
            name: name.into(),
            type_name,
            method,
            path,
        }
    }

    fn pascal(name: &str) -> String {
        (name.split('_'))
            .map(|w| {
                let mut c = w.chars();
                c.next()
                    .map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
            })
            .collect()
    }

    /// Calls of `server` (`name(…)` or `::<TypeName`) inside other functions.
    fn actions(cx: Cx, server: &ServerFn, flow: &mut UiFlow) {
        let src = cx.src;
        let b = src.mask.as_bytes();
        let calls = words(&src.mask, &server.name).into_iter().filter(|&at| {
            let call = next_nonws(b, at + server.name.len()).is_some_and(|k| b[k] == b'(');
            let before = src.mask[..at].trim_end();
            call && !before.ends_with("fn") && !before.ends_with('.')
        });
        let turbofish = words(&src.mask, &server.type_name)
            .into_iter()
            .filter(|&at| src.mask[..at].trim_end().ends_with("::<"));
        for at in calls.chain(turbofish) {
            let inside = crate::lang_rust::fn_at(&src.fns, at);
            if inside.is_none_or(|f| f.name == server.name) {
                continue;
            }
            flow.actions.push(Action {
                weak: false,
                screen: nav::origin(cx, at),
                trigger: None,
                call: Call::Http {
                    method: server.method.into(),
                    path: server.path.clone(),
                },
                file: src.rel.clone(),
                line: src.line(at),
                condition: archguard_core::AccessState::Undeclared,
                source: SOURCE.into(),
            });
        }
    }
}

/// egui and iced: an enum field of the root type holds the screen.
mod state {
    use super::routable::{Part, enum_body, parts};
    use super::*;
    use crate::lang_rust::words;
    use crate::lang_text::{ident_end, matching, next_nonws, prev_word, split_top};
    use archguard_core::{ScreenKind, Target};

    const NAMES: [&str; 5] = ["screen", "page", "view", "tab", "route"];
    const ROOT_TRAITS: [&str; 3] = ["App", "Application", "Sandbox"];

    /// A screen state: the field and its enum.
    pub(crate) struct Key {
        field: String,
        enumeration: String,
    }

    pub(crate) fn screens(app: &App, flow: &mut UiFlow, known: &mut Known) -> Vec<Key> {
        let roots = roots(app);
        let mut enums: BTreeMap<String, (&Src, Vec<Part>)> = BTreeMap::new();
        for src in &app.rust {
            for at in words(&src.mask, "enum") {
                if let Some((name, open, close)) = enum_body(&src.mask, at) {
                    enums
                        .entry(name)
                        .or_insert_with(|| (src, parts(&src.mask, open, close)));
                }
            }
        }
        let mut keys = vec![];
        for (field, ty) in app.rust.iter().flat_map(|s| fields(s, &roots)) {
            let Some((src, parts)) = enums.get(&ty) else {
                continue;
            };
            let variants: Vec<(&str, usize)> = (parts.iter())
                .filter_map(|p| {
                    if let Part::Variant(v, at) = p {
                        Some((v.as_str(), *at))
                    } else {
                        None
                    }
                })
                .collect();
            let named = [&field, &ty]
                .iter()
                .any(|n| NAMES.iter().any(|k| n.to_lowercase().contains(k)));
            if variants.len() < 2 || !named || keys.iter().any(|k: &Key| k.field == field) {
                continue;
            }
            for (v, at) in &variants {
                let screen = nav::screen(app, src, *at, &format!("{ty}::{v}"), v, ScreenKind::View);
                known
                    .variants
                    .insert((ty.clone(), v.to_string()), screen.id.clone());
                known.screens.push(screen);
            }
            let key = Key {
                field,
                enumeration: ty,
            };
            start(app, &key, parts, src, flow, known);
            keys.push(key);
        }
        keys
    }

    /// The root types of the application.
    fn roots(app: &App) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = (app.rust.iter())
            .flat_map(|src| impl_roots(src).into_iter().chain(call_roots(src)))
            .collect();
        out.retain(|r| !r.is_empty());
        out
    }

    /// `impl eframe::App for X`, `impl Application for X`, `impl Sandbox for X`.
    fn impl_roots(src: &Src) -> Vec<String> {
        let header = |at: usize| {
            let end = src.mask[at..].find(['{', ';']).map_or(at, |e| at + e);
            let (tr, ty) = src.mask[at + "impl".len()..end].split_once(" for ")?;
            let tr = tr.trim().rsplit("::").next().unwrap_or("").trim();
            let mut ty = ty.split(|c: char| !(c.is_alphanumeric() || c == '_'));
            ROOT_TRAITS
                .contains(&tr)
                .then(|| ty.find(|s| !s.is_empty()).unwrap_or("").to_string())
        };
        words(&src.mask, "impl")
            .into_iter()
            .filter_map(header)
            .collect()
    }

    /// `X` of `X::update`/`X::view` passed to `application(…)`, `run(…)`,
    /// `daemon(…)`.
    fn call_roots(src: &Src) -> Vec<String> {
        let b = src.mask.as_bytes();
        let mut out = vec![];
        for call in ["application", "run", "daemon"] {
            for at in words(&src.mask, call) {
                let open = next_nonws(b, at + call.len()).filter(|&k| b[k] == b'(');
                let Some(close) = open.and_then(|o| matching(b, o)) else {
                    continue;
                };
                let args = &src.mask[at..close];
                let named = ["::update", "::view"]
                    .into_iter()
                    .flat_map(|m| args.match_indices(m));
                out.extend(named.map(|(k, _)| prev_word(args, k).to_string()));
            }
        }
        out
    }

    /// (field, type) of the root structs in `src`.
    fn fields(src: &Src, roots: &BTreeSet<String>) -> Vec<(String, String)> {
        let b = src.mask.as_bytes();
        let mut out = vec![];
        for at in words(&src.mask, "struct") {
            let Some(start) = next_nonws(b, at + 6) else {
                continue;
            };
            let name = &src.mask[start..ident_end(b, start)];
            let open = src.mask[start..].find(['{', ';', '(']).map(|o| start + o);
            let Some((open, close)) = open
                .filter(|&o| b[o] == b'{' && roots.contains(name))
                .and_then(|o| Some((o, matching(b, o)?)))
            else {
                continue;
            };
            for field in split_top(&src.mask[open + 1..close], ",") {
                let field = field.trim().trim_start_matches("pub").trim();
                if let Some((f, ty)) = field.rsplit_once(':').filter(|(f, _)| !f.contains(':')) {
                    let f = f.split_whitespace().last().unwrap_or("");
                    out.push((f.to_string(), ty.trim().to_string()));
                }
            }
        }
        out
    }

    /// The start view: `field: Enum::V` in `new`/`default`/`boot`/`init`,
    /// else `#[default]`, else what `impl Default for Enum` returns.
    fn start(app: &App, key: &Key, parts: &[Part], src: &Src, flow: &mut UiFlow, known: &Known) {
        let init = app.rust.iter().find_map(|s| {
            words(&s.mask, &key.field).into_iter().find_map(|at| {
                let inside = crate::lang_rust::fn_at(&s.fns, at)?;
                let rest = s.mask[at + key.field.len()..]
                    .trim_start()
                    .strip_prefix(':')?;
                let starts = ["new", "default", "boot", "init"].contains(&inside.name.as_str());
                starts.then(|| variant(known, key, rest)).flatten()
            })
        });
        let marked = parts.windows(2).find_map(|w| match w {
            [Part::Attr(a), Part::Variant(v, _)] if a.name(&src.mask) == "default" => known
                .variants
                .get(&(key.enumeration.clone(), v.clone()))
                .cloned(),
            _ => None,
        });
        let start = init.or(marked).or_else(|| default_impl(app, key, known));
        match start {
            Some(id) => flow.start_screens.push(id),
            None => {
                let message = format!(
                    "The start value of `{}` is not a literal variant; its start view is unknown.",
                    key.field
                );
                let at = match parts.first() {
                    Some(Part::Variant(_, at)) => *at,
                    _ => 0,
                };
                nav::diag(flow, &src.rel, src.line(at), message, false);
            }
        }
    }

    /// The screen of a literal `Enum::V` (or `Self::V` inside the enum's impl) at the start of `text`.
    fn variant(known: &Known, key: &Key, text: &str) -> Option<String> {
        let text = text.trim_start();
        let path = &text[..text
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
            .unwrap_or(text.len())];
        let (e, v) = path.rsplit_once("::")?;
        let e = e.rsplit("::").next().unwrap_or(e);
        (e == key.enumeration || e == "Self").then(|| {
            known
                .variants
                .get(&(key.enumeration.clone(), v.to_string()))
                .cloned()
        })?
    }

    fn default_impl(app: &App, key: &Key, known: &Known) -> Option<String> {
        app.rust.iter().find_map(|s| {
            words(&s.mask, "impl").into_iter().find_map(|at| {
                let open = at + s.mask[at..].find('{')?;
                let header: Vec<&str> = s.mask[at..open].split_whitespace().collect();
                if header != ["impl", "Default", "for", key.enumeration.as_str()] {
                    return None;
                }
                let body = &s.mask[open..matching(s.mask.as_bytes(), open)?];
                let found: BTreeSet<String> = (body.match_indices("::"))
                    .filter_map(|(k, _)| variant(known, key, &body[k - prev_word(body, k).len()..]))
                    .collect();
                (found.len() == 1)
                    .then(|| found.into_iter().next())
                    .flatten()
            })
        })
    }

    pub(crate) fn transitions(app: &App, keys: &[Key], known: &Known, flow: &mut UiFlow) {
        for key in keys {
            let mut messages = BTreeSet::new();
            for src in &app.rust {
                let cx = Cx { app, src, known };
                assignments(cx, key, flow, &mut messages);
                selections(cx, key, flow);
            }
            for message in &messages {
                for src in &app.rust {
                    message_sites(Cx { app, src, known }, key, message, flow);
                }
            }
        }
    }

    /// `….field = value`.
    fn assignments(cx: Cx, key: &Key, flow: &mut UiFlow, messages: &mut BTreeSet<String>) {
        let src = cx.src;
        let b = src.mask.as_bytes();
        for at in words(&src.mask, &key.field) {
            let eq = next_nonws(b, at + key.field.len())
                .filter(|&k| b[k] == b'=' && !matches!(b.get(k + 1), Some(b'=' | b'>')));
            let Some(eq) = eq.filter(|_| at > 0 && b[at - 1] == b'.') else {
                continue;
            };
            let expr = statement(&src.mask[eq + 1..]);
            let target = match variant(cx.known, key, expr) {
                Some(id) => Target::Screen(id),
                None => match plain_name(expr).and_then(|x| arm_binding(src, at, x)) {
                    Some(message) => {
                        messages.insert(message);
                        continue;
                    }
                    None => Target::Unresolved(nav::shown(src, expr)),
                },
            };
            let arm = || arm_message(src, at).and_then(|m| shared_label(cx.app, &m));
            let trigger = label::block(src, at).or_else(arm);
            nav::transition(flow, src, at, nav::any(cx.app), target, trigger);
        }
    }

    /// The expression up to `;`, `,` or `}` outside brackets.
    fn statement(rest: &str) -> &str {
        let end = (split_top(rest, ";")[0].len()).min(split_top(rest, ",")[0].len());
        let expr = &rest[..end];
        expr.split('}').next().unwrap_or(expr).trim()
    }

    fn plain_name(expr: &str) -> Option<&str> {
        let plain = !expr.is_empty() && expr.chars().all(|c| c.is_alphanumeric() || c == '_');
        plain.then_some(expr)
    }
    /// `Path::Variant(x) =>` before `at` in its `fn`: `Path::Variant`.
    fn arm_binding(src: &Src, at: usize, x: &str) -> Option<String> {
        let f = crate::lang_rust::fn_at(&src.fns, at)?;
        let text = &src.mask[f.body.0..at];
        let pattern = format!("({x})");
        let k = text
            .rmatch_indices(&pattern)
            .map(|(k, _)| k)
            .find(|&k| text[k + pattern.len()..].trim_start().starts_with("=>"))?;
        let path = path_before(text, k);
        path.contains("::").then(|| path.to_string())
    }

    /// The `A::B` (no parentheses) of the nearest `A::B =>` before `at` in its `fn`.
    fn arm_message(src: &Src, at: usize) -> Option<String> {
        let f = crate::lang_rust::fn_at(&src.fns, at)?;
        let text = &src.mask[f.body.0..at];
        let k = text.rfind("=>")?;
        let path = path_before(text, k);
        (path.contains("::") && text[..k].trim_end().ends_with(path)).then(|| path.to_string())
    }

    fn path_before(text: &str, end: usize) -> &str {
        let head = text[..end].trim_end();
        let start = head
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
            .map_or(0, |s| s + 1);
        &head[start..]
    }

    /// The one button label all sites of the unit message `m` share.
    fn shared_label(app: &App, m: &str) -> Option<String> {
        let mut labels = BTreeSet::new();
        for src in &app.rust {
            for (at, _) in src.mask.match_indices(m) {
                let next = src.mask[at + m.len()..].trim_start();
                if next.starts_with(')') && !src.mask[..at].ends_with("::") {
                    labels.extend(label::button_in(src, label::chain_start(&src.mask, at), at));
                }
            }
        }
        (labels.len() == 1)
            .then(|| labels.into_iter().next())
            .flatten()
    }

    /// `selectable_value(&mut ….field, Enum::V, "label")`, `radio_value(…)`.
    fn selections(cx: Cx, key: &Key, flow: &mut UiFlow) {
        let src = cx.src;
        let b = src.mask.as_bytes();
        for call in ["selectable_value", "radio_value"] {
            for at in words(&src.mask, call) {
                let open = next_nonws(b, at + call.len()).filter(|&k| b[k] == b'(');
                let Some((open, close)) = open.and_then(|o| Some((o, matching(b, o)?))) else {
                    continue;
                };
                let args = split_top(&src.mask[open + 1..close], ",");
                let [field, value, text, ..] = args.as_slice() else {
                    continue;
                };
                if !field.trim().ends_with(&format!(".{}", key.field)) {
                    continue;
                }
                let target = variant(cx.known, key, value).map_or_else(
                    || Target::Unresolved(nav::shown(src, value)),
                    Target::Screen,
                );
                nav::transition(
                    flow,
                    src,
                    at,
                    nav::any(cx.app),
                    target,
                    src.lit(text.trim()),
                );
            }
        }
    }

    /// `Message::GoTo(Enum::V)` outside its arm pattern.
    fn message_sites(cx: Cx, key: &Key, message: &str, flow: &mut UiFlow) {
        let src = cx.src;
        let b = src.mask.as_bytes();
        for (at, _) in src.mask.match_indices(message) {
            let open = next_nonws(b, at + message.len())
                .filter(|&k| b[k] == b'(' && (at == 0 || !crate::lang_text::is_id(b[at - 1])));
            let Some((open, close)) = open.and_then(|o| Some((o, matching(b, o)?))) else {
                continue;
            };
            if src.mask[close + 1..].trim_start().starts_with("=>") {
                continue;
            }
            let arg = &src.mask[open + 1..close];
            let target = variant(cx.known, key, arg)
                .map_or_else(|| Target::Unresolved(nav::shown(src, arg)), Target::Screen);
            let trigger = label::button_in(src, label::chain_start(&src.mask, at), at);
            nav::transition(flow, src, at, nav::any(cx.app), target, trigger);
        }
    }
}

/// Tauri commands, capabilities, windows and the frontend's `invoke`.
mod tauri {
    use super::*;
    use crate::lang_rust::{attrs_before, macro_bodies, words};
    use crate::lang_text::{matching, next_nonws, split_top};
    use archguard_core::{
        AccessState, Action, Call, Condition, EndpointAccess, ScreenKind, Target,
    };
    use serde_json::Value;

    pub(crate) const INVOKE: &str = "INVOKE";

    pub(crate) fn run(root: &Path, app: &App, flow: &mut UiFlow, known: &mut Known) {
        if !app.tauri {
            return;
        }
        let read = |name: &str, flow: &mut UiFlow| {
            let rel = app.tauri_files.iter().find(|r| {
                r.ends_with(name) && (r.len() == name.len() || r.ends_with(&format!("/{name}")))
            })?;
            Some((rel.clone(), files::read(root, rel, flow)?))
        };
        let manifest = read("build.rs", flow)
            .map(|(_, text)| app_manifest(&text))
            .unwrap_or_default();
        let conf = ["tauri.conf.json", "tauri.conf.json5", "Tauri.toml"]
            .iter()
            .find_map(|name| read(name, flow));
        let conf = conf.map(|(rel, text)| {
            let value = parse(app, &rel, &text);
            (rel, text, value)
        });
        let acl = Acl::read(root, app, &manifest, conf.as_ref(), flow);
        commands(app, &acl, flow);
        if let Some(conf) = &conf {
            conf_windows(app, conf, flow, known);
        }
        for src in &app.rust {
            builders(app, src, flow, known);
        }
        web::invokes(app, flow);
    }

    /// A configuration or capability file as JSON; `json5` is read as far as
    /// it is JSON (no JSON5 parser in the build).
    fn parse(app: &App, rel: &str, text: &str) -> Result<Value, String> {
        if rel.ends_with(".toml") {
            let toml = toml::from_str::<toml::Value>(text).map_err(|e| e.to_string())?;
            return serde_json::to_value(toml).map_err(|e| e.to_string());
        }
        if rel.ends_with(".json5") && !app.json5 {
            return Err("json5 file without the `config-json5` feature of tauri".into());
        }
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    /// Commands named in `AppManifest::new().commands(&[…])`.
    fn app_manifest(text: &str) -> BTreeSet<String> {
        let mask = crate::lang_rust::mask(text);
        let code = crate::lang_rust::code(text);
        let Some(at) = words(&mask, "AppManifest").first().copied() else {
            return BTreeSet::new();
        };
        let Some(c) = words(&mask[at..], "commands").first().map(|c| c + at) else {
            return BTreeSet::new();
        };
        let Some(open) = mask[c..].find('[').map(|o| o + c) else {
            return BTreeSet::new();
        };
        let close = matching(mask.as_bytes(), open).unwrap_or(open);
        (split_top(&mask[open + 1..close], ","))
            .into_iter()
            .filter_map(|p| literal_value(&code[offset(&mask, p)..offset(&mask, p) + p.len()]))
            .collect()
    }

    /// A capability: identifier, windows, permissions.
    #[derive(Clone)]
    struct Cap {
        id: String,
        windows: Vec<String>,
        perms: Vec<String>,
    }

    /// A capability, or why it is not read.
    type Capability = Result<Cap, String>;

    /// An app permission (`commands.allow/deny`) or set (`includes`).
    #[derive(Default)]
    struct Def {
        allow: Vec<String>,
        deny: Vec<String>,
        includes: Vec<String>,
    }

    /// The app ACL: active capabilities (file or inline) and app permissions.
    struct Acl {
        /// (file, capability) of every active capability.
        caps: Vec<(String, Capability)>,
        defs: BTreeMap<String, Def>,
        /// A build manifest or a permission file exists: app commands are checked.
        active: bool,
    }

    /// The parsed configuration: (file, text, value or parse error).
    type Conf = (String, String, Result<Value, String>);

    fn strings(v: Option<&Value>) -> Vec<String> {
        let list = v.and_then(Value::as_array).into_iter().flatten();
        list.filter_map(|p| p.as_str().or_else(|| p.get("identifier")?.as_str()))
            .map(str::to_string)
            .collect()
    }

    /// One capability, a list of them, or `{ "capabilities": [ … ] }`.
    fn capabilities_of(v: &Value) -> Vec<Capability> {
        let nested = v.get("capabilities").filter(|c| c.is_array());
        match (v, nested) {
            (Value::Array(list), _) => list.iter().flat_map(capabilities_of).collect(),
            (_, Some(list)) if v.get("identifier").is_none() => capabilities_of(list),
            _ => vec![capability(v)],
        }
    }

    fn capability(v: &Value) -> Capability {
        let id = (v.get("identifier").and_then(Value::as_str)).ok_or("no literal identifier")?;
        if !v.get("permissions").is_some_and(Value::is_array) {
            return Err("no permissions list".into());
        }
        Ok(Cap {
            id: id.to_string(),
            windows: strings(v.get("windows")),
            perms: strings(v.get("permissions")),
        })
    }

    /// Whether two window label lists can name the same window; an empty
    /// list (webviews only) counts as overlapping.
    fn overlap(a: &[String], b: &[String]) -> bool {
        a.is_empty() || b.is_empty() || a.iter().any(|x| b.iter().any(|y| glob(x, y) || glob(y, x)))
    }

    /// `*` matches any text.
    fn glob(pattern: &str, text: &str) -> bool {
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.len() == 1 {
            return pattern == text;
        }
        let Some(mut rest) = text.strip_prefix(parts[0]) else {
            return false;
        };
        for p in &parts[1..parts.len() - 1] {
            let Some(at) = rest.find(p) else {
                return false;
            };
            rest = &rest[at + p.len()..];
        }
        rest.ends_with(parts[parts.len() - 1])
    }

    /// Capabilities of every file, permissions, and whether a permission file exists.
    type Files = (Vec<(String, Capability)>, BTreeMap<String, Def>, bool);

    impl Acl {
        fn read(
            root: &Path,
            app: &App,
            manifest: &BTreeSet<String>,
            conf: Option<&Conf>,
            flow: &mut UiFlow,
        ) -> Acl {
            let (files, defs, permission_files) = Acl::files(root, app, flow);
            let caps = Acl::active(files, conf, flow);
            if let Some((rel, _, Err(e))) = conf {
                let message = format!(
                    "Configuration not read ({e}); the active capability list is unknown, every capability file counts."
                );
                nav::diag(flow, rel, 0, message, true);
            }
            Acl {
                caps,
                active: !manifest.is_empty() || permission_files,
                defs,
            }
        }

        /// Every capability file and permission file of the package.
        fn files(root: &Path, app: &App, flow: &mut UiFlow) -> Files {
            let (mut files, mut defs, mut permission_files) = (vec![], BTreeMap::new(), false);
            for rel in &app.tauri_files {
                let in_dir = |d: &str| rel.contains(&format!("{d}/"));
                if !in_dir("capabilities") && !in_dir("permissions") {
                    continue;
                }
                let Some(text) = files::read(root, rel, flow) else {
                    continue;
                };
                let value = parse(app, rel, &text);
                if in_dir("capabilities") {
                    match &value {
                        Ok(v) => {
                            files.extend(capabilities_of(v).into_iter().map(|c| (rel.clone(), c)))
                        }
                        Err(e) => files.push((rel.clone(), Err(e.clone()))),
                    }
                } else {
                    permission_files = true;
                    match &value {
                        Ok(v) => definitions(v, &mut defs),
                        Err(e) => {
                            let message = format!(
                                "Permission file not read ({e}); its permissions are unknown."
                            );
                            nav::diag(flow, rel, 0, message, true);
                        }
                    }
                }
            }
            for (rel, cap) in &files {
                if let Err(e) = cap {
                    let message =
                        format!("Capability not read ({e}); its grants and denies are unknown.");
                    nav::diag(flow, rel, 0, message, true);
                }
            }
            (files, defs, permission_files)
        }

        /// The capabilities in force: all files, or only those the
        /// configuration lists (inline ones included).
        fn active(
            files: Vec<(String, Capability)>,
            conf: Option<&Conf>,
            flow: &mut UiFlow,
        ) -> Vec<(String, Capability)> {
            let listed = conf.and_then(|(rel, _, v)| {
                let entries = v.as_ref().ok()?.pointer("/app/security/capabilities")?;
                Some((rel, entries.as_array()?))
            });
            let Some((conf_rel, entries)) = listed else {
                return files;
            };
            let mut active = vec![];
            for e in entries {
                let Some(id) = e.as_str() else {
                    let inline = capabilities_of(e).into_iter();
                    active.extend(inline.map(|c| (conf_rel.clone(), c)));
                    continue;
                };
                let before = active.len();
                let named = |c: &Capability| c.as_ref().is_ok_and(|c| c.id == id);
                active.extend(files.iter().filter(|(_, c)| named(c)).cloned());
                if active.len() == before {
                    let message = format!(
                        "Capability `{id}` is listed in the configuration but not found in a capability file."
                    );
                    nav::diag(flow, conf_rel, 0, message.clone(), true);
                    active.push((conf_rel.clone(), Err(message)));
                }
            }
            active
        }
    }

    /// `[[permission]]`, `[[set]]` and `[default]` of a permission file.
    fn definitions(v: &Value, defs: &mut BTreeMap<String, Def>) {
        let list = |key: &str| {
            v.get(key)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        for p in list("permission") {
            if let Some(id) = p.get("identifier").and_then(Value::as_str) {
                let def = defs.entry(id.to_string()).or_default();
                def.allow.extend(strings(p.pointer("/commands/allow")));
                def.deny.extend(strings(p.pointer("/commands/deny")));
            }
        }
        for s in list("set") {
            if let Some(id) = s.get("identifier").and_then(Value::as_str) {
                let def = defs.entry(id.to_string()).or_default();
                def.includes.extend(strings(s.get("permissions")));
            }
        }
        if let Some(d) = v.get("default") {
            let def = defs.entry("default".into()).or_default();
            def.includes.extend(strings(d.get("permissions")));
        }
    }

    fn commands(app: &App, acl: &Acl, flow: &mut UiFlow) {
        let mut defined = BTreeMap::new();
        for src in &app.rust {
            for f in &src.fns {
                let attrs = attrs_before(&src.mask, f.at);
                if (attrs.iter()).any(|a| matches!(a.path(&src.mask), "tauri::command" | "command"))
                {
                    let site = (src.rel.clone(), src.line(f.at));
                    defined.entry(f.name.clone()).or_insert(site);
                }
            }
        }
        for src in &app.rust {
            for (open, close) in macro_bodies(&src.mask, "generate_handler") {
                let plugin = plugin(src, open, close);
                let paths = split_top(&src.mask[open + 1..close], ",")
                    .into_iter()
                    .map(str::trim);
                for path in paths.filter(|p| !p.is_empty()) {
                    let path = path.rsplit(']').next().unwrap_or(path).trim();
                    let name = path.rsplit("::").next().unwrap_or(path).to_string();
                    let (file, line) =
                        (defined.get(&name).cloned()).unwrap_or((src.rel.clone(), src.line(open)));
                    // Plugin commands always need a permission; app commands once an app ACL exists.
                    let guarded = plugin.is_some() || acl.active;
                    let command = (name, plugin.clone());
                    let access = access(&command, guarded, acl, flow, (&file, line));
                    flow.endpoints.push(EndpointAccess {
                        method: INVOKE.into(),
                        path: match &command.1 {
                            Some(p) => format!("plugin:{p}|{}", command.0),
                            None => command.0,
                        },
                        access,
                        application: Some(app.name.clone()),
                        file,
                        line,
                        source: SOURCE.into(),
                        handler: Some(path.to_string()),
                        sends: vec![],
                    });
                }
            }
        }
    }

    /// The plugin of a handler list: `#![plugin(app_menu)]` at its start,
    /// underscores read as dashes.
    fn plugin(src: &Src, open: usize, close: usize) -> Option<String> {
        let at = open + 1 + src.mask[open + 1..close].find('#')?;
        let attr = crate::lang_rust::attr_at(&src.mask, at)?;
        let (o, c) = attr.args.filter(|_| attr.name(&src.mask) == "plugin")?;
        Some(src.mask[o + 1..c].trim().replace('_', "-"))
    }

    /// (command name, plugin) of one handler entry.
    type Command = (String, Option<String>);

    fn access(
        command: &Command,
        guarded: bool,
        acl: &Acl,
        flow: &mut UiFlow,
        (file, line): (&str, u32),
    ) -> AccessState {
        if !guarded {
            return AccessState::Undeclared;
        }
        let prefix = command
            .1
            .as_ref()
            .map_or(String::new(), |p| format!("{p}:"));
        let allow = format!("{prefix}allow-{}", command.0.replace('_', "-"));
        let deny = allow.replacen("allow-", "deny-", 1);
        let set = |p: &str| {
            (p.strip_prefix(prefix.as_str())).is_some_and(|r| {
                !r.contains(':') && !r.starts_with("allow-") && !r.starts_with("deny-")
            })
        };
        let (mut grants, mut denies, mut unread) = (vec![], vec![], vec![]);
        for (rel, cap) in &acl.caps {
            match cap {
                Ok(cap) => {
                    let (grant, denied, sets) = judge(cap, acl, command, (&allow, &deny), &set);
                    unread.extend(sets.into_iter().map(|id| format!("{rel}: {id}")));
                    grants.extend(grant.then_some(cap));
                    denies.extend(denied.then_some(cap));
                }
                Err(e) => unread.push(format!("{rel}: {e}")),
            }
        }
        // A deny in any active capability of an overlapping window wins.
        grants.retain(|g| !denies.iter().any(|d| overlap(&g.windows, &d.windows)));
        let mut granting: Vec<Condition> = (grants.iter())
            .map(|c| Condition::Policy(format!("capability:{}", c.id)))
            .collect();
        match (granting.len(), unread.is_empty()) {
            (1, _) => AccessState::Protected(granting.remove(0)),
            (0, true) => AccessState::Protected(Condition::Never),
            (0, false) => {
                let message = format!(
                    "No capability grants `{allow}` directly; permission sets are not read: {}.",
                    unread.join(", ")
                );
                nav::diag(flow, file, line, message, true);
                AccessState::Protected(Condition::Unknown(format!("capability for {allow}")))
            }
            _ => AccessState::Protected(Condition::Any(granting)),
        }
    }

    /// Whether `cap` allows and denies the command (sets and app permissions
    /// expanded), and the set-like permissions it names that are not read.
    fn judge(
        cap: &Cap,
        acl: &Acl,
        command: &Command,
        (allow, deny): (&str, &str),
        set: &dyn Fn(&str) -> bool,
    ) -> (bool, bool, Vec<String>) {
        let (mut seen, mut todo) = (BTreeSet::new(), cap.perms.iter().collect::<Vec<_>>());
        let (mut grant, mut denied, mut unread) = (false, false, vec![]);
        while let Some(id) = todo.pop() {
            if !seen.insert(id) {
                continue;
            }
            if id == allow {
                grant = true;
            } else if id == deny {
                denied = true;
            } else if let Some(def) = acl.defs.get(id) {
                if command.1.is_none() {
                    grant |= def.allow.contains(&command.0);
                    denied |= def.deny.contains(&command.0);
                }
                todo.extend(&def.includes);
            } else if set(id) {
                unread.push(id.clone());
            }
        }
        (grant, denied, unread)
    }

    /// `app.windows` (v1 `tauri.windows`) of `tauri.conf.json`.
    fn conf_windows(app: &App, (rel, text, conf): &Conf, flow: &mut UiFlow, known: &mut Known) {
        let Ok(conf) = conf else {
            return;
        };
        let windows = (conf
            .pointer("/app/windows")
            .or(conf.pointer("/tauri/windows")))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
        let src = Src::new(rel.to_string(), text);
        for w in windows {
            let label = w.get("label").and_then(Value::as_str).unwrap_or("main");
            let at = text
                .find(&format!("\"{label}\""))
                .or_else(|| text.find("\"windows\""))
                .unwrap_or(0);
            let id = window(app, &src, at, label, known);
            if w.get("create").and_then(Value::as_bool) != Some(false) {
                flow.start_screens.push(id);
            }
        }
    }

    /// The window screen `label`, added once; its id.
    fn window(app: &App, src: &Src, at: usize, label: &str, known: &mut Known) -> String {
        let screen = nav::screen(
            app,
            src,
            at,
            &format!("window:{label}"),
            label,
            ScreenKind::Window,
        );
        let id = screen.id.clone();
        if !known.screens.iter().any(|s| s.id == id) {
            known.screens.push(screen);
        }
        id
    }

    /// `WebviewWindowBuilder::new(_, "label", _)` and the like.
    fn builders(app: &App, src: &Src, flow: &mut UiFlow, known: &mut Known) {
        let b = src.mask.as_bytes();
        let setups: Vec<(usize, usize)> = (words(&src.mask, "setup").into_iter())
            .filter(|&at| at > 0 && b[at - 1] == b'.')
            .filter_map(|at| {
                let open = next_nonws(b, at + 5).filter(|&k| b[k] == b'(')?;
                Some((open, matching(b, open)?))
            })
            .collect();
        let calls = [
            "WebviewWindowBuilder::new",
            "WindowBuilder::new",
            "WebviewWindow::builder",
        ];
        for call in calls {
            for (at, _) in src.mask.match_indices(call) {
                let open = next_nonws(b, at + call.len()).filter(|&k| b[k] == b'(');
                let open = open.filter(|_| at == 0 || !crate::lang_text::is_id(b[at - 1]));
                let Some((open, close)) = open.and_then(|o| Some((o, matching(b, o)?))) else {
                    continue;
                };
                let label = split_top(&src.mask[open + 1..close], ",")
                    .get(1)
                    .copied()
                    .unwrap_or("");
                let started = (setups.iter())
                    .any(|&(o, c)| o < at && at < c && closure_bodies(&src.mask, o, at) <= 1);
                opened(app, src, (at, label, started), flow, known);
            }
        }
    }

    /// A window built at `at` with `label`: started in the setup, else opened
    /// from any screen.
    fn opened(
        app: &App,
        src: &Src,
        (at, label, started): (usize, &str, bool),
        flow: &mut UiFlow,
        known: &mut Known,
    ) {
        match (src.lit(label), started) {
            (Some(l), true) => flow.start_screens.push(window(app, src, at, &l, known)),
            (Some(l), false) => {
                let target = Target::Screen(window(app, src, at, &l, known));
                nav::transition(flow, src, at, nav::any(app), target, None);
            }
            (None, true) => {
                let message = format!(
                    "The window label `{}` is not literal; this window is unknown.",
                    nav::shown(src, label)
                );
                nav::diag(flow, &src.rel, src.line(at), message, true);
            }
            (None, false) => {
                let target = Target::Unresolved(nav::shown(src, label));
                nav::transition(flow, src, at, nav::any(app), target, None);
            }
        }
    }

    /// Closure bodies (`|…| {`) open at `at`, counted from `from`: more than
    /// the setup closure itself means the window opens on an event.
    fn closure_bodies(mask: &str, from: usize, at: usize) -> usize {
        let b = mask.as_bytes();
        let mut stack = vec![];
        for k in from..at {
            match b[k] {
                b'{' => {
                    stack.push(crate::lang_text::prev_nonws(b, k).is_some_and(|p| b[p] == b'|'))
                }
                b'}' => drop(stack.pop()),
                _ => {}
            }
        }
        stack.into_iter().filter(|closure| *closure).count()
    }

    /// The frontend side.
    mod web {
        use super::*;
        use crate::lang_ts::{Parser, Value, line_of, skip_generic};
        use crate::lang_ts_decl::{imports, mask};
        use crate::lang_ts_http::global_call;

        const MODULES: [&str; 2] = ["@tauri-apps/api/core", "@tauri-apps/api/tauri"];

        /// `invoke('command', …)` with `invoke` imported from the Tauri API.
        pub(crate) fn invokes(app: &App, flow: &mut UiFlow) {
            for (rel, text) in &app.web {
                let masked = mask(text, true);
                let names: Vec<String> = (imports(&masked).into_iter())
                    .filter(|i| MODULES.contains(&i.module.as_str()))
                    .filter_map(|i| match i.imported.as_deref() {
                        Some("invoke") => Some(i.local),
                        Some("*") => Some(format!("{}.invoke", i.local)),
                        _ => None,
                    })
                    .collect();
                for name in &names {
                    calls(app, rel, &masked, name, flow);
                }
            }
        }

        fn calls(app: &App, rel: &str, masked: &str, name: &str, flow: &mut UiFlow) {
            let b = masked.as_bytes();
            for (at, _) in masked.match_indices(name) {
                let after = at + name.len();
                let fits = (at == 0 || !crate::lang_text::is_id(b[at - 1]))
                    && b.get(after).is_none_or(|&c| !crate::lang_text::is_id(c));
                let mut open = crate::lang_ts::skip_ws(b, after);
                if b.get(open) == Some(&b'<') {
                    open = crate::lang_ts::skip_ws(b, skip_generic(b, open));
                }
                if !fits
                    || b.get(open) != Some(&b'(')
                    || (!name.contains('.') && !global_call(masked, at))
                {
                    continue;
                }
                let first = Parser { s: masked, i: open }.array(0).into_iter().next();
                let call = match &first {
                    Some(item) => match &item.value {
                        Value::Str(command) => Call::Http {
                            method: INVOKE.into(),
                            path: command.clone(),
                        },
                        _ => Call::Unresolved(item.source(masked)),
                    },
                    None => Call::Unresolved(String::new()),
                };
                let line = line_of(masked, at);
                if let Call::Unresolved(text) = &call {
                    let message =
                        format!("`{text}` names no literal Tauri command; the call is unknown.");
                    nav::diag(flow, rel, line, message, true);
                }
                flow.actions.push(Action {
                    weak: false,
                    screen: nav::any(app),
                    trigger: None,
                    call,
                    file: rel.to_string(),
                    line,
                    condition: archguard_core::AccessState::Undeclared,
                    source: SOURCE.into(),
                });
            }
        }
    }
}

#[cfg(test)]
mod test_support {
    use super::*;
    use archguard_core::{Origin, Target};

    pub(super) fn extract_files(files: &[(&str, &str)]) -> UiFlow {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        extract(dir.path())
    }

    pub(super) fn cargo(name: &str, dep: &str) -> String {
        format!("[package]\nname = \"{name}\"\n[dependencies]\n{dep} = \"1\"\n")
    }

    /// (id, route) of every screen.
    pub(super) fn routes(flow: &UiFlow) -> Vec<(&str, Option<&str>)> {
        (flow.screens.iter())
            .map(|s| (s.id.as_str(), s.route.as_deref()))
            .collect()
    }

    /// (from, target, trigger, line) of every transition.
    pub(super) fn moves(flow: &UiFlow) -> Vec<(String, String, Option<&str>, u32)> {
        let from = |o: &Origin| match o {
            Origin::Screen(id) => id.clone(),
            Origin::AnyScreen { application } => format!("{application}:*"),
        };
        let target = |t: &Target| match t {
            Target::Screen(id) => id.clone(),
            Target::Unresolved(text) => format!("? {text}"),
        };
        let mut out: Vec<_> = (flow.transitions.iter())
            .map(|t| {
                (
                    from(&t.from),
                    target(&t.target),
                    t.trigger.as_deref(),
                    t.line,
                )
            })
            .collect();
        out.sort_by_key(|m| m.3);
        out
    }

    pub(super) fn gaps(flow: &UiFlow) -> Vec<(&str, u32, &str)> {
        (flow.diagnostics.iter())
            .filter(|d| d.severity == archguard_core::DiagnosticSeverity::Gap)
            .map(|d| (d.file.as_str(), d.line, d.message.as_str()))
            .collect()
    }
}

#[cfg(test)]
mod tests_router {
    use super::test_support::*;

    const DIOXUS: &str = r#"use dioxus::prelude::*;

#[derive(Routable, Clone, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(NavBar)]
        #[route("/")]
        Home {},
        #[nest("/blog")]
            #[route("/")]
            BlogList {},
            #[route("/:name", Post)]
            BlogPost { name: String },
        #[end_nest]
    #[end_layout]
    #[redirect("/old", || Route::Home {})]
    #[route("/:..route")]
    PageNotFound { route: Vec<String> },
}

#[component]
fn NavBar() -> Element {
    rsx! {
        Link { to: Route::Home {}, "Home" }
        Link { to: Route::BlogList {}, class: "nav", "Blog" }
        document::Link { href: "style.css" }
    }
}

#[component]
fn BlogList() -> Element {
    rsx! {
        Link { to: Route::BlogPost { name: "a".into() }, "Read {name}" }
        Link { to: "/blog/b", "Second" }
        button { onclick: move |_| { navigator().push(Route::Home {}); }, "Back" }
    }
}

#[component]
fn Post(name: String) -> Element {
    let nav = use_navigator();
    nav.replace(target());
    items.push(name);
    rsx! { Link { to: "https://dioxuslabs.com", "Docs" } }
}
"#;

    #[test]
    fn dioxus_routable_enum_gives_pages_and_links() {
        let flow = extract_files(&[
            ("blog/Cargo.toml", &cargo("blog", "dioxus")),
            ("blog/src/main.rs", DIOXUS),
        ]);
        assert_eq!(
            routes(&flow),
            [
                ("blog:Route::BlogList", Some("/blog")),
                ("blog:Route::BlogPost", Some("/blog/:name")),
                ("blog:Route::Home", Some("/")),
                ("blog:Route::PageNotFound", Some("/*")),
            ]
        );
        let post = flow
            .screens
            .iter()
            .find(|s| s.id == "blog:Route::BlogPost")
            .unwrap();
        assert_eq!((post.name.as_str(), post.line), ("Post", 13));
        let any = "blog:*".to_string();
        let list = "blog:Route::BlogList".to_string();
        assert_eq!(
            moves(&flow),
            [
                (any.clone(), "blog:Route::Home".into(), Some("Home"), 24),
                (any.clone(), list.clone(), Some("Blog"), 25),
                (list.clone(), "blog:Route::BlogPost".into(), None, 33),
                (
                    list.clone(),
                    "blog:Route::BlogPost".into(),
                    Some("Second"),
                    34
                ),
                (list, "blog:Route::Home".into(), None, 35),
                ("blog:Route::BlogPost".into(), "? target()".into(), None, 42),
            ]
        );
        assert_eq!(gaps(&flow).len(), 1, "{:#?}", flow.diagnostics);
        assert!(!flow.complete);
    }

    const YEW: &str = r#"use yew::prelude::*;
use yew_router::prelude::*;

#[derive(Clone, Routable, PartialEq)]
enum Route {
    #[at("/")]
    Home,
    #[at("/posts/:id")]
    Post { id: u32 },
    #[not_found]
    #[at("/404")]
    NotFound,
}

fn switch(route: Route) -> Html {
    match route {
        Route::Home => html! { <HomePage /> },
        Route::Post { id } => html! { <PostView id={id} /> },
        Route::NotFound => html! { <h1>{ "404" }</h1> },
    }
}

#[function_component(HomePage)]
fn home_page() -> Html {
    let navigator = use_navigator().unwrap();
    let onclick = Callback::from(move |_| navigator.push(&Route::NotFound));
    html! {
        <Link<Route> to={Route::Post { id: 1 }}>{ "First post" }</Link<Route>>
    }
}
"#;

    #[test]
    fn yew_routes_take_their_component_from_the_switch() {
        let flow = extract_files(&[
            ("web/Cargo.toml", &cargo("web", "yew")),
            ("web/src/main.rs", YEW),
        ]);
        let names: Vec<(&str, &str, Option<&str>)> = (flow.screens.iter())
            .map(|s| (s.id.as_str(), s.name.as_str(), s.route.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                ("web:Route::Home", "HomePage", Some("/")),
                ("web:Route::NotFound", "NotFound", Some("/404")),
                ("web:Route::Post", "PostView", Some("/posts/:id")),
            ]
        );
        assert_eq!(
            moves(&flow),
            [
                ("web:*".into(), "web:Route::NotFound".into(), None, 26),
                (
                    "web:*".into(),
                    "web:Route::Post".into(),
                    Some("First post"),
                    28
                ),
            ]
        );
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn framework_crates_and_unrelated_packages_are_no_apps() {
        let routable =
            "use dioxus::prelude::*;\n#[derive(Routable)]\nenum R { #[route(\"/\")] A {} }\n";
        let flow = extract_files(&[
            (
                "packages/router/Cargo.toml",
                &cargo("dioxus-router", "dioxus"),
            ),
            ("packages/router/src/lib.rs", routable),
            ("cli/Cargo.toml", &cargo("cli", "serde")),
            ("cli/src/main.rs", routable),
            ("demo/simple.rs", routable),
            (
                "demo/plain.rs",
                "#[derive(Routable)]\nenum R { #[route(\"/\")] A {} }\n",
            ),
            ("site/Cargo.toml", &cargo("dioxus-examples", "dioxus")),
            ("site/src/main.rs", routable),
        ]);
        assert_eq!(
            routes(&flow),
            [("demo/simple:R::A", Some("/")), ("site:R::A", Some("/"))]
        );
        assert!(
            super::files::own("leptos_router", "leptos") && super::files::own("tauri", "tauri")
        );
        assert!(
            !super::files::own("dioxus-examples", "dioxus") && !super::files::own("api", "tauri")
        );
    }
}

#[cfg(test)]
mod tests_leptos {
    use super::test_support::*;
    use archguard_core::{AccessState, Call, Condition};

    const APP: &str = r#"use leptos::prelude::*;
use leptos_router::{components::*, path};

#[component]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <nav><A href="/about">"About"</A><A href="https://leptos.dev">"Docs"</A></nav>
            <Routes fallback=|| "Not found.">
                <Route path=path!("about") view=About/>
                <ProtectedRoute path=path!("admin") condition=is_admin redirect_path=|| "/" view=Admin/>
                <ProtectedRoute path=path!("settings") condition=move || Some(logged_in.get()) redirect_path=|| "/" view=Settings/>
                <ParentRoute path=path!("") view=ContactList>
                    <Route path=path!("/") view=|| "Select a contact."/>
                    <Route path=path!("/:id") view=Contact/>
                    <Route path=path!("/:id/edit") view=EditContact/>
                </ParentRoute>
                <Route path=(StaticSegment("users"), ParamSegment("id")) view=User/>
                <Route path=OptionalParamSegment("stories") view=Stories/>
                <Route path=path!("old") view=|| view! { <Redirect path="/about"/> }/>
            </Routes>
        </Router>
    }
}

#[component]
fn Contact() -> impl IntoView {
    let navigate = use_navigate();
    navigate("/users/7", Default::default());
    let contacts = get_contacts(String::new());
    let add = ServerAction::<AddContact>::new();
    view! {
        <A href=format!("/users/{}", id)>"Author"</A>
        <A href="edit">"Edit"</A>
        <A href=link_to(id)>"Somewhere"</A>
    }
}

#[server]
pub async fn get_contacts(search: String) -> Result<Vec<String>, ServerFnError> {
    Ok(vec![])
}

#[server(endpoint = "add", input = GetUrl)]
pub async fn add_contact(name: String) -> Result<(), ServerFnError> {
    Ok(())
}
"#;

    fn flow() -> archguard_core::UiFlow {
        extract_files(&[
            ("contacts/Cargo.toml", &cargo("contacts", "leptos")),
            ("contacts/src/lib.rs", APP),
            (
                ".archguard/archguard.toml",
                "[views.access]\nis_admin = \"role:Admin\"\n",
            ),
        ])
    }

    #[test]
    fn leptos_route_tree_gives_pages_with_access() {
        let flow = flow();
        assert_eq!(
            routes(&flow),
            [
                ("contacts:/", Some("/")),
                ("contacts:/:id", Some("/:id")),
                ("contacts:/:id/edit", Some("/:id/edit")),
                ("contacts:/:stories", Some("/:stories")),
                ("contacts:/about", Some("/about")),
                ("contacts:/admin", Some("/admin")),
                ("contacts:/old", Some("/old")),
                ("contacts:/settings", Some("/settings")),
                ("contacts:/users/:id", Some("/users/:id")),
            ]
        );
        let access = |id: &str| {
            flow.screens
                .iter()
                .find(|s| s.id == id)
                .unwrap()
                .access
                .clone()
        };
        assert_eq!(
            access("contacts:/admin"),
            archguard_core::AccessState::Protected(Condition::Role("Admin".into()))
        );
        assert!(matches!(
            access("contacts:/settings"),
            archguard_core::AccessState::Protected(Condition::Unknown(_))
        ));
        assert_eq!(
            flow.screens
                .iter()
                .find(|s| s.id == "contacts:/:id")
                .unwrap()
                .name,
            "Contact"
        );
        assert_eq!(flow.start_screens, ["contacts:/:stories"]);
        let gaps = gaps(&flow);
        assert!(
            gaps.iter()
                .any(|g| g.1 == 12 && g.2.contains("Some(logged_in.get())")),
            "{gaps:#?}"
        );
    }

    #[test]
    fn leptos_links_resolve_literals_formats_and_relative_hrefs() {
        let flow = flow();
        let contact = "contacts:/:id".to_string();
        assert_eq!(
            moves(&flow),
            [
                (
                    "contacts:*".into(),
                    "contacts:/about".into(),
                    Some("About"),
                    8
                ),
                ("contacts:*".into(), "contacts:/about".into(), None, 20),
                (contact.clone(), "contacts:/users/:id".into(), None, 29),
                (
                    contact.clone(),
                    "contacts:/users/:id".into(),
                    Some("Author"),
                    33
                ),
                (
                    contact.clone(),
                    "contacts:/:id/edit".into(),
                    Some("Edit"),
                    34
                ),
                (contact, "? link_to(id)".into(), Some("Somewhere"), 35),
            ]
        );
    }

    #[test]
    fn leptos_server_functions_are_endpoints_their_calls_actions() {
        let flow = flow();
        let endpoints: Vec<(&str, &str, &AccessState)> = (flow.endpoints.iter())
            .map(|e| (e.method.as_str(), e.path.as_str(), &e.access))
            .collect();
        assert_eq!(
            endpoints,
            [
                ("GET", "/api/add", &AccessState::Undeclared),
                ("POST", "/api/get_contacts", &AccessState::Undeclared),
            ]
        );
        let mut calls: Vec<(&Call, u32)> = flow.actions.iter().map(|a| (&a.call, a.line)).collect();
        calls.sort_by_key(|c| c.1);
        let http = |m: &str, p: &str| Call::Http {
            method: m.into(),
            path: p.into(),
        };
        assert_eq!(
            calls,
            [
                (&http("POST", "/api/get_contacts"), 30),
                (&http("GET", "/api/add"), 31)
            ]
        );
        let joined = archguard_core::join_actions_to_endpoints(&flow);
        assert!(
            joined.iter().all(|(_, hits)| hits.len() == 1),
            "{joined:#?}"
        );
    }
}

#[cfg(test)]
mod tests_state {
    use super::test_support::*;

    const ICED: &str = r#"use iced::widget::{button, column};

pub fn main() -> iced::Result {
    iced::application(App::default, App::update, App::view).run()
}

struct App {
    screen: Screen,
    layout: Layout,
}

#[derive(Clone, Copy)]
enum Screen { Home, Settings, About }

enum Layout { Row, Column }

enum Message { GoTo(Screen), Next, Back }

impl App {
    fn update(&mut self, message: Message) {
        match message {
            Message::GoTo(screen) => self.screen = screen,
            Message::Next => self.screen = Screen::About,
            Message::Back => self.screen = self.screen.previous(),
        }
    }

    fn view(&self) -> Element<Message> {
        column![
            button("Settings").on_press(Message::GoTo(Screen::Settings)),
            padded_button("Next").on_press(Message::Next),
        ]
    }
}

impl Default for App {
    fn default() -> Self {
        Self { screen: Screen::Home, layout: Layout::Row }
    }
}
"#;

    #[test]
    fn iced_screen_field_gives_views_and_message_transitions() {
        let flow = extract_files(&[
            ("tour/Cargo.toml", &cargo("tour", "iced")),
            ("tour/src/main.rs", ICED),
        ]);
        assert_eq!(
            routes(&flow),
            [
                ("tour:Screen::About", None),
                ("tour:Screen::Home", None),
                ("tour:Screen::Settings", None)
            ]
        );
        assert_eq!(flow.start_screens, ["tour:Screen::Home"]);
        let any = "tour:*".to_string();
        assert_eq!(
            moves(&flow),
            [
                (any.clone(), "tour:Screen::About".into(), Some("Next"), 23),
                (any.clone(), "? self.screen.previous()".into(), None, 24),
                (any, "tour:Screen::Settings".into(), Some("Settings"), 30),
            ]
        );
        assert_eq!(gaps(&flow).len(), 1);
    }

    const EGUI: &str = r#"use eframe::egui;

#[derive(Default)]
struct MyApp { tab: Tab, count: u8 }

#[derive(Default, PartialEq)]
enum Tab {
    #[default]
    Overview,
    Graph,
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.selectable_value(&mut self.tab, Tab::Graph, "Graph");
            if ui.button("Back").clicked() {
                self.tab = Tab::Overview;
            }
        });
    }
}
"#;

    #[test]
    fn egui_tab_field_reads_selectable_values_and_button_blocks() {
        let flow = extract_files(&[
            ("viewer/Cargo.toml", &cargo("viewer", "eframe")),
            ("viewer/src/main.rs", EGUI),
        ]);
        assert_eq!(
            routes(&flow),
            [("viewer:Tab::Graph", None), ("viewer:Tab::Overview", None)]
        );
        assert_eq!(flow.start_screens, ["viewer:Tab::Overview"]);
        assert_eq!(
            moves(&flow),
            [
                (
                    "viewer:*".into(),
                    "viewer:Tab::Graph".into(),
                    Some("Graph"),
                    16
                ),
                (
                    "viewer:*".into(),
                    "viewer:Tab::Overview".into(),
                    Some("Back"),
                    18
                ),
            ]
        );
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }
}

#[cfg(test)]
mod tests_tauri {
    use super::test_support::*;
    use archguard_core::{AccessState, Call, Condition, ScreenKind};

    const LIB: &str = r#"mod cmd;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {name}")
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            WebviewWindowBuilder::new(app, "splash", WebviewUrl::default()).build()?;
            app.on_menu_event(|app, e| { WebviewWindowBuilder::new(app, "about", WebviewUrl::default()); });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![greet, cmd::delete_all, cmd::export])
        .run(tauri::generate_context!())
        .expect("error");
}

fn open(app: &AppHandle, n: u8) {
    WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("s.html".into())).build();
    WebviewWindowBuilder::new(app, format!("w{n}"), WebviewUrl::default()).build();
}
"#;

    const CMD: &str = "#[tauri::command]\npub fn delete_all() {}\n#[command]\npub fn export() {}\n";
    const MENU: &str = "pub fn init() -> TauriPlugin<R> {\n    Builder::new(\"app-menu\")\n        .invoke_handler(tauri::generate_handler![\n            #![plugin(app_menu)]\n            toggle,\n        ])\n        .build()\n}\n#[command]\nfn toggle() {}\n";
    const BUILD: &str = r#"fn main() {
    tauri_build::try_build(tauri_build::Attributes::new()
        .plugin("menu", tauri_build::InlinedPlugin::new().commands(&["toggle"]))
        .app_manifest(tauri_build::AppManifest::new().commands(&["delete_all", "export"])))
        .unwrap();
}"#;
    const FRONT: &str = r#"import { invoke } from '@tauri-apps/api/core'
import * as core from '@tauri-apps/api/core'
// invoke('commented')
export async function go(cmd: string) {
  await invoke<string>('greet', { name: 'x' })
  await core.invoke('delete_all')
  await invoke(cmd)
  await invoke('plugin:app-menu|toggle')
}
"#;

    fn flow() -> archguard_core::UiFlow {
        extract_files(&[
            ("notes/package.json", "{}"),
            ("notes/src/main.ts", FRONT),
            ("notes/src-tauri/Cargo.toml", &cargo("notes", "tauri")),
            ("notes/src-tauri/build.rs", BUILD),
            ("notes/src-tauri/src/lib.rs", LIB),
            ("notes/src-tauri/src/cmd.rs", CMD),
            ("notes/src-tauri/src/menu.rs", MENU),
            (
                "notes/src-tauri/tauri.conf.json",
                r#"{"app": {"windows": [{"title": "Notes"}, {"label": "hidden", "create": false}]}}"#,
            ),
            (
                "notes/src-tauri/capabilities/main.json",
                r#"{"identifier": "main", "windows": ["main"], "permissions": ["core:default", "allow-delete-all", "app-menu:allow-toggle"]}"#,
            ),
            (
                "notes/src-tauri/capabilities/extra.json",
                r#"{"identifier": "extra", "permissions": [{"identifier": "allow-delete-all"}]}"#,
            ),
        ])
    }

    #[test]
    fn tauri_commands_are_invoke_endpoints_guarded_by_capabilities() {
        let flow = flow();
        let endpoints: Vec<(&str, &str, &AccessState, &str, u32)> = (flow.endpoints.iter())
            .map(|e| {
                (
                    e.method.as_str(),
                    e.path.as_str(),
                    &e.access,
                    e.file.as_str(),
                    e.line,
                )
            })
            .collect();
        let policy = |id: &str| Condition::Policy(format!("capability:{id}"));
        assert_eq!(
            endpoints,
            [
                (
                    "INVOKE",
                    "delete_all",
                    &AccessState::Protected(Condition::Any(vec![policy("extra"), policy("main")])),
                    "notes/src-tauri/src/cmd.rs",
                    2
                ),
                (
                    "INVOKE",
                    "export",
                    &AccessState::Protected(Condition::Never),
                    "notes/src-tauri/src/cmd.rs",
                    4
                ),
                (
                    "INVOKE",
                    "greet",
                    &AccessState::Protected(Condition::Never),
                    "notes/src-tauri/src/lib.rs",
                    4
                ),
                (
                    "INVOKE",
                    "plugin:app-menu|toggle",
                    &AccessState::Protected(policy("main")),
                    "notes/src-tauri/src/menu.rs",
                    10
                ),
            ]
        );
        assert!(
            flow.endpoints
                .iter()
                .all(|e| e.application.as_deref() == Some("notes"))
        );
    }

    #[test]
    fn frontend_invokes_join_the_commands() {
        let flow = flow();
        let mut calls: Vec<(&Call, u32)> = flow.actions.iter().map(|a| (&a.call, a.line)).collect();
        calls.sort_by_key(|c| c.1);
        let invoke = |p: &str| Call::Http {
            method: "INVOKE".into(),
            path: p.into(),
        };
        assert_eq!(
            calls,
            [
                (&invoke("greet"), 5),
                (&invoke("delete_all"), 6),
                (&Call::Unresolved("cmd".into()), 7),
                (&invoke("plugin:app-menu|toggle"), 8)
            ]
        );
        let joined = archguard_core::join_actions_to_endpoints(&flow);
        let hits: Vec<(&str, usize)> = (joined.iter())
            .map(|(a, hits)| (a.file.as_str(), hits.len()))
            .collect();
        assert_eq!(hits, [("notes/src/main.ts", 1); 3]);
    }

    #[test]
    fn tauri_windows_from_config_and_builders() {
        let flow = flow();
        let windows: Vec<(&str, &ScreenKind)> = flow
            .screens
            .iter()
            .map(|s| (s.id.as_str(), &s.kind))
            .collect();
        assert_eq!(
            windows,
            [
                ("notes:window:about", &ScreenKind::Window),
                ("notes:window:hidden", &ScreenKind::Window),
                ("notes:window:main", &ScreenKind::Window),
                ("notes:window:settings", &ScreenKind::Window),
                ("notes:window:splash", &ScreenKind::Window),
            ]
        );
        assert_eq!(
            flow.start_screens,
            ["notes:window:main", "notes:window:splash"]
        );
        assert_eq!(
            moves(&flow),
            [
                ("notes:*".into(), "notes:window:about".into(), None, 12),
                ("notes:*".into(), "notes:window:settings".into(), None, 21),
                ("notes:*".into(), "? format!(\"w{n}\")".into(), None, 22),
            ]
        );
    }
}

#[cfg(test)]
mod tests_tauri_acl {
    use super::test_support::*;
    use archguard_core::{AccessState, Condition};

    const LIB: &str = "#[tauri::command]\nfn greet() {}\n#[tauri::command]\nfn other() {}\npub fn run() {\n    tauri::Builder::default().invoke_handler(tauri::generate_handler![greet, other]);\n}\n";
    const BUILD: &str = "fn main() { tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[\"greet\"]))).unwrap(); }";
    const CAP_TOML: &str =
        "identifier = \"main\"\nwindows = [\"main\"]\npermissions = [\"allow-greet\"]\n";
    const CAP_JSON5: &str =
        r#"{"identifier": "x", "windows": ["main"], "permissions": ["allow-greet"]}"#;

    fn cap(id: &str, perms: &str) -> String {
        format!(r#"{{"identifier": "{id}", "windows": ["main"], "permissions": {perms}}}"#)
    }

    /// The access of `command` in a Tauri package with `extra` files, and
    /// whether the flow has a gap. `dep` replaces the `tauri` dependency line.
    fn access(dep: &str, extra: &[(&str, &str)], command: &str) -> (AccessState, bool) {
        let dep = if dep.is_empty() { "tauri = \"2\"" } else { dep };
        let cargo = format!("[package]\nname = \"app\"\n[dependencies]\n{dep}\n");
        let mut files: Vec<(String, String)> = vec![
            ("app/src-tauri/Cargo.toml".into(), cargo),
            ("app/src-tauri/src/lib.rs".into(), LIB.into()),
        ];
        for (p, t) in extra {
            files.retain(|(q, _)| *q != format!("app/src-tauri/{p}"));
            files.push((format!("app/src-tauri/{p}"), t.to_string()));
        }
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect();
        let flow = extract_files(&refs);
        let e = flow.endpoints.iter().find(|e| e.path == command).unwrap();
        (e.access.clone(), !gaps(&flow).is_empty())
    }

    fn policy(id: &str) -> AccessState {
        AccessState::Protected(Condition::Policy(format!("capability:{id}")))
    }

    #[test]
    fn tf1_no_manifest_no_permissions_is_undeclared() {
        assert_eq!(access("", &[], "greet").0, AccessState::Undeclared);
    }

    #[test]
    fn tf2_toml_capability() {
        let files = [("build.rs", BUILD), ("capabilities/main.toml", CAP_TOML)];
        assert_eq!(access("", &files, "greet"), (policy("main"), false));
    }

    #[test]
    fn tf3_capability_in_subfolder() {
        let c = cap("main", r#"["allow-greet"]"#);
        let files = [("build.rs", BUILD), ("capabilities/desktop/main.json", &c)];
        assert_eq!(access("", &files, "greet"), (policy("main"), false));
    }

    #[test]
    fn tf4_capability_list_in_one_file() {
        let all = format!(
            r#"{{"capabilities": [{}, {}]}}"#,
            cap("a", r#"["allow-greet"]"#),
            cap("b", "[]")
        );
        let files = [("build.rs", BUILD), ("capabilities/all.json", &all)];
        assert_eq!(access("", &files, "greet"), (policy("a"), false));
        let top = format!("[{}]", cap("a", r#"["allow-greet"]"#));
        let files = [("build.rs", BUILD), ("capabilities/all.json", &top)];
        assert_eq!(access("", &files, "greet"), (policy("a"), false));
    }

    #[test]
    fn tf5_only_the_listed_capabilities_are_active() {
        let (a, b) = (
            cap("a", r#"["allow-greet"]"#),
            cap("b", r#"["allow-greet"]"#),
        );
        let conf = r#"{"app": {"security": {"capabilities": ["b"]}}}"#;
        let files = [
            ("build.rs", BUILD),
            ("capabilities/a.json", &a),
            ("capabilities/b.json", &b),
            ("tauri.conf.json", conf),
        ];
        assert_eq!(access("", &files, "greet"), (policy("b"), false));
    }

    #[test]
    fn a_listed_capability_that_is_not_found_is_unknown_with_a_gap() {
        let conf = r#"{"app": {"security": {"capabilities": ["ghost"]}}}"#;
        let files = [("build.rs", BUILD), ("tauri.conf.json", conf)];
        let (state, gap) = access("", &files, "greet");
        assert!(gap);
        assert!(matches!(
            state,
            AccessState::Protected(Condition::Unknown(_))
        ));
    }

    #[test]
    fn tf6_inline_capability_in_the_config() {
        let conf = r#"{"app": {"security": {"capabilities": [{"identifier": "inline", "windows": ["*"], "permissions": ["allow-greet"]}]}}}"#;
        let files = [("build.rs", BUILD), ("tauri.conf.json", conf)];
        assert_eq!(access("", &files, "greet"), (policy("inline"), false));
    }

    #[test]
    fn tf7_deny_in_another_capability_wins() {
        let (a, b) = (
            cap("a", r#"["allow-greet"]"#),
            cap("b", r#"["deny-greet"]"#),
        );
        let files = [
            ("build.rs", BUILD),
            ("capabilities/a.json", &a),
            ("capabilities/b.json", &b),
        ];
        let never = AccessState::Protected(Condition::Never);
        assert_eq!(access("", &files, "greet"), (never, false));
    }

    #[test]
    fn tf8_app_permission_set() {
        let a = cap("a", r#"["my-set"]"#);
        let sets = "[[set]]\nidentifier = \"my-set\"\npermissions = [\"allow-greet\"]\n";
        let files = [
            ("build.rs", BUILD),
            ("capabilities/a.json", &a),
            ("permissions/sets.toml", sets),
        ];
        assert_eq!(access("", &files, "greet"), (policy("a"), false));
    }

    #[test]
    fn tf9_permission_files_switch_the_app_acl_on() {
        let perm = "[[permission]]\nidentifier = \"greeting\"\ncommands.allow = [\"greet\"]\n";
        let c = cap("main", r#"["greeting"]"#);
        let files = [
            ("capabilities/main.json", c.as_str()),
            ("permissions/custom.toml", perm),
        ];
        assert_eq!(access("", &files, "greet"), (policy("main"), false));
        let never = AccessState::Protected(Condition::Never);
        assert_eq!(access("", &files, "other"), (never, false));
    }

    #[test]
    fn tf10_plugin_set_not_in_the_repo_is_unknown() {
        let lib = LIB.replace(
            "generate_handler![greet, other]",
            "generate_handler![#![plugin(fs)] read_file]",
        );
        let c = cap("main", r#"["fs:default"]"#);
        let files = [("capabilities/main.json", c.as_str()), ("src/lib.rs", &lib)];
        let (state, gap) = access("", &files, "plugin:fs|read_file");
        assert!(gap);
        let unknown = Condition::Unknown("capability for fs:allow-read-file".into());
        assert_eq!(state, AccessState::Protected(unknown));
    }

    #[test]
    fn tf13_json5_capability_needs_the_cargo_feature() {
        let files = [("build.rs", BUILD), ("capabilities/x.json5", CAP_JSON5)];
        let (state, gap) = access("tauri = \"2\"", &files, "greet");
        assert!(gap);
        assert!(matches!(
            state,
            AccessState::Protected(Condition::Unknown(_))
        ));
        let dep = "tauri = { version = \"2\", features = [\"config-json5\"] }";
        assert_eq!(access(dep, &files, "greet"), (policy("x"), false));
    }
}

#[cfg(test)]
mod tests_rust_access {
    use super::test_support::*;
    use archguard_core::{AccessState, Condition};

    fn access(flow: &archguard_core::UiFlow, id: &str) -> AccessState {
        (flow.screens.iter().find(|s| s.id == id))
            .unwrap_or_else(|| panic!("no screen {id}: {:?}", routes(flow)))
            .access
            .clone()
    }

    const ADMIN_APP: &str = r#"use leptos::prelude::*;
use leptos_router::{components::*, path};
mod admin;

#[component]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <Routes fallback=|| "Not found.">
                <Route path=path!("home") view=Home/>
                <ProtectedParentRoute path=path!("admin") condition=is_admin redirect_path=|| "/" view=Layout>
                    <AdminRoutes/>
                </ProtectedParentRoute>
            </Routes>
        </Router>
    }
}
"#;
    const ADMIN_ROUTES: &str = r#"use leptos::prelude::*;
use leptos_router::{components::*, path};

#[component(transparent)]
pub fn AdminRoutes() -> impl MatchNestedRoutes {
    view! {
        <Route path=path!("logs") view=Logs/>
    }
}
"#;

    #[test]
    fn lf2_route_component_inherits_the_placing_parents_path_and_condition() {
        let flow = extract_files(&[
            ("web/Cargo.toml", &cargo("web", "leptos")),
            ("web/src/lib.rs", ADMIN_APP),
            ("web/src/admin.rs", ADMIN_ROUTES),
        ]);
        assert_eq!(
            routes(&flow),
            [
                ("web:/admin/logs", Some("/admin/logs")),
                ("web:/home", Some("/home"))
            ]
        );
        let custom = Condition::Custom("is_admin".into());
        assert_eq!(
            access(&flow, "web:/admin/logs"),
            AccessState::Protected(custom)
        );
        assert_eq!(access(&flow, "web:/home"), AccessState::Undeclared);
    }

    #[test]
    fn lf3_a_closure_condition_is_unknown_with_a_gap() {
        let app = ADMIN_APP.replace("condition=is_admin", "condition=move || Some(true)");
        let flow = extract_files(&[
            ("web/Cargo.toml", &cargo("web", "leptos")),
            ("web/src/lib.rs", &app),
            ("web/src/admin.rs", ADMIN_ROUTES),
        ]);
        let unknown = Condition::Unknown("move || Some(true)".into());
        assert_eq!(
            access(&flow, "web:/admin/logs"),
            AccessState::Protected(unknown)
        );
        assert_eq!(gaps(&flow).len(), 1);
    }

    const DIOXUS: &str = r#"use dioxus::prelude::*;

#[derive(Routable, Clone, PartialEq)]
enum Route {
    #[route("/")]
    Home {},
    #[route("/admin")]
    Admin {},
}

fn app() -> Element {
    rsx! { Router::<Route> { config: move || RouterConfig::default().on_update(guard) } }
}
"#;

    #[test]
    fn df3_on_update_makes_every_page_unknown() {
        let flow = extract_files(&[
            ("site/Cargo.toml", &cargo("site", "dioxus")),
            ("site/src/main.rs", DIOXUS),
        ]);
        let unknown = AccessState::Protected(Condition::Unknown("on_update".into()));
        assert_eq!(access(&flow, "site:Route::Home"), unknown);
        assert_eq!(access(&flow, "site:Route::Admin"), unknown);
        assert_eq!(gaps(&flow).len(), 1);
        let plain = DIOXUS.replace(".on_update(guard)", "");
        let flow = extract_files(&[
            ("site/Cargo.toml", &cargo("site", "dioxus")),
            ("site/src/main.rs", &plain),
        ]);
        assert_eq!(access(&flow, "site:Route::Home"), AccessState::Undeclared);
    }

    const YEW: &str = r#"use yew::prelude::*;
use yew_router::prelude::*;

#[derive(Routable, PartialEq, Clone)]
enum Route {
    #[at("/")]
    Home,
    #[at("/admin")]
    Admin,
    #[at("/login")]
    Login,
}

fn switch(route: Route) -> Html {
    match route {
        Route::Home => html! { <Home /> },
        Route::Admin => if is_admin() { html! { <Admin /> } } else { html! { <Redirect<Route> to={Route::Login} /> } },
        Route::Login => html! { <Login /> },
    }
}
"#;

    #[test]
    fn a_yew_match_guard_is_read_like_an_if_arm() {
        let yew = YEW.replace(
            "Route::Admin => if is_admin() { html! { <Admin /> } } else { html! { <Redirect<Route> to={Route::Login} /> } },",
            "Route::Admin if is_admin() => html! { <Admin /> },",
        );
        let flow = extract_files(&[
            ("shop/Cargo.toml", &cargo("shop", "yew")),
            ("shop/src/main.rs", &yew),
        ]);
        let unknown = Condition::Unknown("is_admin()".into());
        assert_eq!(
            access(&flow, "shop:Route::Admin"),
            AccessState::Protected(unknown)
        );
    }

    #[test]
    fn yf1_a_yew_guard_arm_is_unknown_or_the_configured_meaning() {
        let files = |extra: &'static str| {
            extract_files(&[
                ("shop/Cargo.toml", &cargo("shop", "yew")),
                ("shop/src/main.rs", YEW),
                (".archguard/archguard.toml", extra),
            ])
        };
        let flow = files("");
        let unknown = Condition::Unknown("is_admin()".into());
        assert_eq!(
            access(&flow, "shop:Route::Admin"),
            AccessState::Protected(unknown)
        );
        assert_eq!(access(&flow, "shop:Route::Home"), AccessState::Undeclared);
        assert_eq!(access(&flow, "shop:Route::Login"), AccessState::Undeclared);
        assert_eq!(gaps(&flow).len(), 1);
        let flow = files("[views.access]\nis_admin = \"role:admin\"\n");
        let role = Condition::Role("admin".into());
        assert_eq!(
            access(&flow, "shop:Route::Admin"),
            AccessState::Protected(role)
        );
        assert!(gaps(&flow).is_empty());
    }
}
