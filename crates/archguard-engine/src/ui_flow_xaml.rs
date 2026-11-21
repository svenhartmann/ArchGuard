//! XAML extractor for ui-flow/v1 (ADR 0021 phase 6): .NET MAUI Shell first,
//! then WPF, WinUI and Avalonia. A tag and token level scanner over `.xaml`,
//! `.axaml` and `.cs` files, no XAML or C# parser.
//!
//! Applications: a `.csproj` with `<UseMaui>true</UseMaui>`,
//! `<UseWPF>true</UseWPF>`, a `Microsoft.WindowsAppSDK` (WinUI) or `Avalonia`
//! package reference. A file belongs to the project of its nearest `.csproj`;
//! files of other projects, `bin`, `obj` and test projects are skipped.
//!
//! Screens (one per XAML file, id `<app>:<path in project>`, name = `x:Class`):
//! - Root `ContentPage`, `TabbedPage`, `FlyoutPage`, `NavigationPage`,
//!   `CarouselPage`, `Page` -> `page`; `Window` -> `window`; a root of a class
//!   declared in the project (`class PageBase : ContentPage`) follows its
//!   base classes.
//! - `UserControl` (WPF, WinUI, Avalonia) -> `view`, only when it is shown
//!   through a typed `<DataTemplate DataType=...>` or inside a
//!   `<ContentControl>`. MAUI `ContentView`s are components, no screens.
//! - A page only ever opened by `PushModalAsync`, or a window only by
//!   `ShowDialog`, is a `dialog`.
//!
//! Routes (MAUI only; WPF, WinUI and Avalonia screens have none):
//! - `<ShellContent Route="x" ContentTemplate="{DataTemplate v:XPage}"/>` in
//!   `<FlyoutItem>`/`<TabBar>`/`<Tab>`/`<ShellItem>`/`<ShellSection>`: the
//!   `Route`s of the hierarchy joined, absolute (`//Main/Catalog`).
//! - `Routing.RegisterRoute("x", typeof(XPage))`, also `nameof(XPage)`.
//! - With a Shell, a page with neither gets the note "not reachable through
//!   Shell routing".
//!
//! Navigation, transitions in text order:
//! - `GoToAsync("x")`, `"//a/b"`, `"x?id=1"` (query dropped), `$"x?id={id}"`,
//!   `nameof(XPage)`, and a ternary of two literals (two transitions). `..`
//!   (back) is no transition. Absolute targets match the longest Shell route,
//!   the rest must be registered routes; relative targets are registered
//!   routes (the last segment is the target). Holes in the path stay unresolved.
//! - A navigation wrapper: a method whose first parameter `string p` appears
//!   in its body next to a `GoToAsync(` call (eShop's
//!   `MauiNavigationService.NavigateToAsync`). Its calls count as `GoToAsync`
//!   (one hop); navigation inside wrapper bodies is not reported.
//! - `PushAsync(new XPage(..))`, `PushModalAsync(new XPage(..))`,
//!   `.Navigate(typeof(XPage))` / `.Navigate(new XPage())` (WinUI `Frame`,
//!   WPF `NavigationService`), `new XWindow()` with `.Show()`, `.Activate()`
//!   or `.ShowDialog()` chained or on the variable it was assigned to.
//!
//! Origin of code: a code-behind (`X.xaml.cs`) belongs to its screen. Code of
//! a class named `...ViewModel` belongs to the screen bound to it:
//! explicitly by `x:DataType` on the root element, a `<X.DataContext>` or
//! `<X.BindingContext>` property element, a typed `DataTemplate` around a
//! view, or a code-behind that assigns `BindingContext`/`DataContext` and
//! names the view model (constructor injection, `new XViewModel()`); without
//! any explicit binding by the naming convention `XPage`/`XView`/`XWindow` <->
//! `XViewModel` (also `XWindowViewModel`). A view model bound to no screen or
//! to several starts at `any_screen` of its application, with a note. Other
//! code (App, Shell, services) starts at `any_screen`.
//!
//! Triggers: navigation inside a code-behind method `M` takes the label of
//! the elements with `Clicked`/`Click`/`Tapped`/`Pressed="M"`; inside a view
//! model method `M`, of the elements in the bound screens whose `...Command`
//! attribute binds `MCommand` (`[RelayCommand]` on `M`/`MAsync`) or a
//! `XCommand` property initialized with `new ...(M ...)`. The label is the
//! element's `AutomationId`, else its literal `Text` or `Content`. One
//! transition per labelled element; none without one.
//!
//! Access: no standard attribute exists. An `if (...)` around a navigation or
//! call whose condition uses a configured guard (`IsAuthenticated`,
//! `HasRole(...)`) becomes `Custom` (negated with `!`); else none.
//!
//! Actions: in view models and code-behinds, calls `Get|Post|Put|Delete|
//! Patch...Async` on a receiver declared `HttpClient` or named `*http*`, with
//! a literal, interpolated (holes of string constants of the file filled
//! in, others `{p}`) or string-constant URL; otherwise `Call::Unresolved`.
//! One hop: a call of a method of a service class (any other class of the
//! project) adds that method's direct HTTP calls at the caller's screen.
//!
//! Start screen (`UiFlow.start_screens`, one per application, since no XAML
//! screen is routed `/`): MAUI with a Shell, the page of the first
//! `ShellContent` of the first Shell file in document order. Otherwise the
//! `StartupUri` of the `Application` XAML (relative to its folder), else the
//! first screen class created with `new` in `OnStartup` (WPF), `OnLaunched`
//! (WinUI), `OnFrameworkInitializationCompleted` (Avalonia) or `CreateWindow`
//! (MAUI without a Shell: `new Window(new MainPage())`). None found: a note on
//! the project folder.
//!
//! Not recognized: commented-out code, routes or targets from variables,
//! `else` branches of guards, C#-only pages without XAML, `ContentControl`
//! switching by view model (`CurrentView = new XViewModel()`), the older MAUI
//! `MainPage = …` assignment, services reached over more than one hop.
use crate::extract_ctx::{ExtractContext, ancestors, dir_name, read_text, walk, with_root_config};
use crate::lang_markup as xml;
use crate::services::support::test_path;
use archguard_core::{
    Action, Call, Condition, DiagnosticSeverity, Origin, Screen, ScreenKind, Target, Transition,
    UiFlow, UiFlowDiagnostic,
};
use navigation::{calls, navigations, resolve, string_value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use text::{balanced, blank, has_word, ident_start, is_call, line, literal, scopes, split_args};
use xml::{attr, attributes, tags};

const SOURCE: &str = "xaml";
const MAX_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 5_000;
const VERBS: [&str; 5] = ["Get", "Post", "Put", "Delete", "Patch"];
/// Configured guard methods (ADR 0021 decision 6).
const GUARDS: [&str; 2] = ["IsAuthenticated", "HasRole"];
const EVENTS: [&str; 4] = ["Clicked", "Click", "Tapped", "Pressed"];
/// Application overrides that create the first window or page (WPF, WinUI,
/// Avalonia, MAUI without a Shell).
const STARTUP: [&str; 4] = [
    "OnStartup",
    "OnLaunched",
    "OnFrameworkInitializationCompleted",
    "CreateWindow",
];
const SHELL_ITEMS: [&str; 6] = [
    "FlyoutItem",
    "TabBar",
    "Tab",
    "ShellItem",
    "ShellSection",
    "ShellContent",
];

/// One application project: its name, whether it is MAUI, and its files as
/// (root-relative path, text with comments blanked).
struct App {
    name: String,
    /// Project directory relative to the root, `""` at the root.
    dir: String,
    maui: bool,
    xaml: Vec<(String, String)>,
    cs: Vec<(String, String)>,
}

/// One XAML screen.
struct Doc {
    id: String,
    class: String,
    kind: ScreenKind,
    file: String,
    line: u32,
    /// XAML text index in `App::xaml`.
    xaml: usize,
}

/// What the rules know about one application.
struct Index<'a> {
    app: &'a App,
    docs: Vec<Doc>,
    /// Absolute Shell routes (`//a/b`) and registered routes, each with its doc.
    shell: Vec<(String, usize)>,
    registered: Vec<(String, usize)>,
    /// View model class -> bound docs.
    bound: BTreeMap<String, Vec<usize>>,
    /// Code-behind file index -> doc.
    behind: BTreeMap<usize, usize>,
    wrappers: BTreeSet<String>,
    /// Method name -> bodies in service classes.
    services: BTreeMap<String, Vec<Body>>,
    /// The doc the application starts with.
    start: Option<usize>,
}

/// A method body: cs file index and (start, end) offsets.
type Body = (usize, (usize, usize));

/// What a navigation names: a route (`None` when not readable) or a page type.
enum Goal {
    Route(Option<String>),
    Type { class: String, modal: bool },
}

/// Screens, transitions and actions of every XAML application below `root`.
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    for app in discover(ctx.root, &mut flow) {
        analyze(&app, &mut flow);
    }
    flow.diagnostics.sort();
    flow.diagnostics.dedup();
    flow
}

fn discover(root: &Path, flow: &mut UiFlow) -> Vec<App> {
    let walk = match walk(root) {
        Ok(walk) => walk,
        Err(error) => {
            report(flow, "", 0, format!("XAML files not scanned: {error}"));
            return vec![];
        }
    };
    flow.diagnostics.extend(
        walk.notes
            .into_iter()
            .map(|(file, message)| UiFlowDiagnostic {
                file,
                line: 0,
                message,
                severity: DiagnosticSeverity::Note,
            }),
    );
    let (mut projects, mut files) = (BTreeMap::new(), Vec::new());
    for rel in walk.files {
        if test_path(&rel) {
            continue;
        }
        if rel.ends_with(".csproj") {
            let dir = rel.rsplit_once('/').map_or("", |(dir, _)| dir).to_string();
            let text = read(root, &rel, flow).unwrap_or_default();
            projects.insert(dir, framework(&text));
        } else if [".xaml", ".axaml", ".cs"].iter().any(|e| rel.ends_with(e)) {
            files.push(rel);
        }
    }
    let mut apps: BTreeMap<String, App> = BTreeMap::new();
    files.retain(|rel| matches!(owner(rel, &projects), Some((_, Some(_)))));
    files.sort();
    if files.len() > MAX_FILES {
        let message = format!("More than {MAX_FILES} XAML and C# files; the rest was not read.");
        report(flow, "", 0, message);
        files.truncate(MAX_FILES);
    }
    for rel in files {
        let Some((dir, Some(maui))) = owner(&rel, &projects) else {
            continue;
        };
        let Some(text) = read(root, &rel, flow) else {
            continue;
        };
        let name = dir_name(root, dir);
        let app = apps.entry(dir.to_string()).or_insert_with(|| App {
            name,
            dir: dir.to_string(),
            maui,
            xaml: vec![],
            cs: vec![],
        });
        if rel.ends_with(".cs") {
            app.cs.push((rel, crate::lang_csharp::code(&text)));
        } else {
            app.xaml.push((rel, blank(&text, "<!--", "-->")));
        }
    }
    apps.into_values().collect()
}

/// `Some(true)` for MAUI, `Some(false)` for WPF, WinUI and Avalonia.
fn framework(csproj: &str) -> Option<bool> {
    let text: String = csproj
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase();
    if text.contains("<usemaui>true</usemaui>") {
        Some(true)
    } else {
        let desktop = [
            "<usewpf>true</usewpf>",
            "microsoft.windowsappsdk",
            "include=\"avalonia",
        ];
        desktop.iter().any(|m| text.contains(m)).then_some(false)
    }
}

/// The nearest project directory of `rel` and its framework.
fn owner<'a>(
    rel: &str,
    projects: &'a BTreeMap<String, Option<bool>>,
) -> Option<(&'a str, Option<bool>)> {
    ancestors(rel).find_map(|dir| {
        (projects.get_key_value(dir)).map(|(key, framework)| (key.as_str(), *framework))
    })
}

fn read(root: &Path, rel: &str, flow: &mut UiFlow) -> Option<String> {
    let text = read_text(root, rel, MAX_BYTES);
    if text.is_err() {
        let message = "File not read: larger than 1 MiB, not a regular file or not readable; its screens and navigation are unknown.";
        report(flow, rel, 0, message.into());
    }
    text.ok()
}

/// Diagnostic that also marks the flow incomplete.
fn report(flow: &mut UiFlow, file: &str, line: u32, message: String) {
    flow.complete = false;
    diagnostic(flow, file, line, message, DiagnosticSeverity::Gap);
}

fn note(flow: &mut UiFlow, file: &str, line: u32, message: String) {
    diagnostic(flow, file, line, message, DiagnosticSeverity::Note);
}

fn diagnostic(
    flow: &mut UiFlow,
    file: &str,
    line: u32,
    message: String,
    severity: DiagnosticSeverity,
) {
    flow.diagnostics.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message,
        severity,
    });
}

/// Per doc: (opened modally or as dialog, opened otherwise).
type Opened = Vec<(bool, bool)>;

fn analyze(app: &App, flow: &mut UiFlow) {
    let ix = index::build(app, flow);
    let mut transitions = Vec::new();
    let mut opened = vec![(false, false); ix.docs.len()];
    for (ci, (_, text)) in app.cs.iter().enumerate() {
        let methods = scopes(text);
        transitions.extend(file_transitions(&ix, ci, &methods, &mut opened, flow));
        actions::collect(&ix, ci, &methods, flow);
    }
    screens(&ix, &opened, flow);
    flow.transitions.extend(transitions);
}

/// The transitions of cs file `ci`, in text order.
fn file_transitions(
    ix: &Index,
    ci: usize,
    methods: &[text::Scope],
    opened: &mut Opened,
    flow: &mut UiFlow,
) -> Vec<Transition> {
    let (file, text) = &ix.app.cs[ci];
    let mut found = navigations(ix, text, methods);
    found.sort_by_key(|(at, ..)| *at);
    let mut transitions = Vec::new();
    for (at, source, goal) in found {
        let line = line(text, at);
        let target = match resolve(ix, &goal, &source) {
            Ok(None) => continue,
            Ok(Some((doc, modal))) => {
                opened[doc].0 |= modal;
                opened[doc].1 |= !modal;
                Target::Screen(ix.docs[doc].id.clone())
            }
            Err(message) => {
                report(flow, file, line, message);
                Target::Unresolved(source)
            }
        };
        let context = origin(ix, ci, at, flow);
        for trigger in triggers(ix, ci, methods, at, &context) {
            transitions.push(Transition {
                weak: false,
                from: context.0.clone(),
                target: target.clone(),
                trigger,
                file: file.clone(),
                line,
                condition: archguard_core::AccessState::from_parts(guard(text, methods, at), false),
                source: SOURCE.into(),
            });
        }
    }
    transitions
}

/// One screen per doc; a page or window opened only modally is a dialog.
fn screens(ix: &Index, opened: &Opened, flow: &mut UiFlow) {
    let app = ix.app;
    match ix.start {
        Some(doc) => flow.start_screens.push(ix.docs[doc].id.clone()),
        None if !ix.docs.is_empty() => {
            let message = format!(
                "No start screen found for {}: no Shell, `StartupUri` or window created in {}.",
                app.name,
                STARTUP.join("/")
            );
            note(flow, &app.dir, 0, message);
        }
        None => {}
    }
    for (i, doc) in ix.docs.iter().enumerate() {
        let route = (ix.shell.iter().chain(&ix.registered))
            .find(|(_, d)| *d == i)
            .map(|(r, _)| r.clone());
        let routed_in_shell = ix.shell.iter().any(|(_, d)| *d == i);
        let kind = match (doc.kind, opened[i]) {
            (ScreenKind::Page | ScreenKind::Window, (true, false)) if !routed_in_shell => {
                ScreenKind::Dialog
            }
            (kind, _) => kind,
        };
        if app.maui && !ix.shell.is_empty() && route.is_none() && kind == ScreenKind::Page {
            let message = format!(
                "{} has no Shell route: not reachable through Shell routing, only by pushing it.",
                doc.class
            );
            note(flow, &doc.file, doc.line, message);
        }
        flow.screens.push(Screen {
            weak: false,
            id: doc.id.clone(),
            name: doc.class.clone(),
            kind,
            route,
            application: Some(app.name.clone()),
            file: doc.file.clone(),
            line: doc.line,
            access: archguard_core::AccessState::Undeclared,
            source: SOURCE.into(),
        });
    }
}

/// Navigation calls in C# and their targets.
mod navigation {
    use super::*;

    /// A navigation found in C#: offset, source text, goal.
    type Found = (usize, String, Goal);

    /// Every navigation in one C# file, outside wrapper bodies.
    pub(super) fn navigations(ix: &Index, text: &str, methods: &[text::Scope]) -> Vec<Found> {
        let inside_wrapper = |at: usize| {
            methods
                .iter()
                .any(|m| ix.wrappers.contains(&m.name) && m.body.0 < at && at < m.body.1)
        };
        let mut found = route_navigations(ix, text);
        found.extend(type_navigations(text));
        found.retain(|(at, ..)| !inside_wrapper(*at));
        found.extend(windows(ix, text, methods));
        found
    }

    /// `GoToAsync` and wrapper calls.
    fn route_navigations(ix: &Index, text: &str) -> Vec<Found> {
        let names = ix.wrappers.iter().map(String::as_str).chain(["GoToAsync"]);
        let mut found = Vec::new();
        for name in names {
            for (at, arg) in calls(text, name) {
                let routes = route_args(&arg).into_iter();
                found.extend(routes.map(|(source, path)| (at, source, Goal::Route(path))));
            }
        }
        found
    }

    /// `PushAsync`, `PushModalAsync` and `Navigate` with a page type.
    fn type_navigations(text: &str) -> Vec<Found> {
        let mut found = Vec::new();
        for (name, modal) in [
            ("PushAsync", false),
            ("PushModalAsync", true),
            ("Navigate", false),
        ] {
            for (at, arg) in calls(text, name) {
                let typed = || arg.strip_prefix("typeof(")?.strip_suffix(')');
                let class = created(&arg).or_else(|| (name == "Navigate").then(typed)?);
                let class = class.map_or("", |c| c.rsplit('.').next().unwrap_or(c));
                let goal = Goal::Type {
                    class: class.to_string(),
                    modal,
                };
                found.push((at, arg, goal));
            }
        }
        found
    }

    /// `new XWindow()` shown by `Show`, `Activate` or `ShowDialog`.
    fn windows(ix: &Index, text: &str, methods: &[text::Scope]) -> Vec<Found> {
        let mut found = Vec::new();
        for doc in ix.docs.iter().filter(|d| d.kind == ScreenKind::Window) {
            for (at, _) in text.match_indices(&format!("new {}(", doc.class)) {
                if let Some(modal) = shown(text, methods, at, &doc.class) {
                    let source = format!("new {}()", doc.class);
                    let goal = Goal::Type {
                        class: doc.class.clone(),
                        modal,
                    };
                    found.push((at, source, goal));
                }
            }
        }
        found
    }

    /// Every call `name(` in text: (offset of the name, first argument).
    pub(super) fn calls(text: &str, name: &str) -> Vec<(usize, String)> {
        text.match_indices(name)
            .filter(|&(at, _)| is_call(text, at, name))
            .map(|(at, _)| {
                let open = at + name.len() + text[at + name.len()..].find('(').unwrap_or(0);
                let args = balanced(&text[open + 1..], ')');
                (at, split_args(args).into_iter().next().unwrap_or_default())
            })
            .collect()
    }

    /// Class name of a `new X(...)` argument.
    fn created(arg: &str) -> Option<&str> {
        let rest = arg.strip_prefix("new ")?.trim_start();
        let name = &rest[..rest.find(['(', '{', '<']).unwrap_or(rest.len())];
        (!name.is_empty()).then_some(name.trim())
    }

    /// For `new XWindow(` at `at`: `Some(true)` when shown by `ShowDialog`,
    /// `Some(false)` by `Show`/`Activate`, `None` when not shown here.
    fn shown(text: &str, methods: &[text::Scope], at: usize, class: &str) -> Option<bool> {
        let open = at + "new ".len() + class.len();
        let after = &text[open + 1 + balanced(&text[open + 1..], ')').len() + 1..];
        let after = after.trim_start();
        let kind = |rest: &str| {
            if rest.starts_with(".ShowDialog") {
                Some(true)
            } else if rest.starts_with(".Show(") || rest.starts_with(".Activate(") {
                Some(false)
            } else {
                None
            }
        };
        if let Some(found) = kind(after) {
            return Some(found);
        }
        // `var w = new XWindow(); w.ShowDialog();` within the same method.
        let before = text[..at].trim_end().strip_suffix('=')?.trim_end();
        let var = &before[ident_start(before)..];
        let end = methods
            .iter()
            .filter(|m| m.body.0 < at && at < m.body.1)
            .map(|m| m.body.1)
            .min()
            .unwrap_or(text.len());
        let rest = &text[at..end];
        rest.match_indices(var)
            .filter(|(i, _)| !rest[..*i].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
            .find_map(|(i, _)| kind(&rest[i + var.len()..]))
    }

    /// Route candidates of a navigation argument: (source text, path or `None`).
    fn route_args(arg: &str) -> Vec<(String, Option<String>)> {
        // `c ? "a" : "b"`: both branches when both are literal.
        if let Some((_, branches)) = arg.split_once(" ? ")
            && let Some((a, b)) = branches.split_once(" : ")
            && let (Some(a), Some(b)) = (literal(a.trim()), literal(b.trim()))
        {
            return vec![(format!("\"{a}\""), Some(a)), (format!("\"{b}\""), Some(b))];
        }
        vec![(arg.to_string(), string_value(arg, &|_| None))]
    }

    /// Value of a C# string expression: a literal, `nameof(X)`, a constant via
    /// `lookup`, or an interpolation whose holes are `nameof`/constants or `{p}`.
    pub(super) fn string_value(
        arg: &str,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Option<String> {
        let nameof = |s: &str| {
            let inner = s.strip_prefix("nameof(")?.strip_suffix(')')?;
            Some(inner.rsplit('.').next().unwrap_or(inner).trim().to_string())
        };
        if let Some(body) = arg.strip_prefix('$') {
            let body = literal(body)?;
            let mut out = String::new();
            let mut rest = body.as_str();
            while let Some(open) = rest.find('{') {
                out.push_str(&rest[..open]);
                let hole = balanced(&rest[open + 1..], '}');
                let value = hole.split(':').next().unwrap_or(hole).trim();
                out.push_str(
                    &nameof(value)
                        .or_else(|| lookup(value))
                        .unwrap_or_else(|| "{p}".into()),
                );
                rest = rest.get(open + hole.len() + 2..).unwrap_or("");
            }
            out.push_str(rest);
            return Some(out);
        }
        literal(arg).or_else(|| nameof(arg)).or_else(|| lookup(arg))
    }

    /// The doc a goal opens and whether it opens modally; `Ok(None)` for back.
    pub(super) fn resolve(
        ix: &Index,
        goal: &Goal,
        source: &str,
    ) -> Result<Option<(usize, bool)>, String> {
        let unresolved =
            || format!("Navigation target {source} is not a route or page of this application.");
        match goal {
            Goal::Type { class, modal } => (ix.docs.iter())
                .position(|d| d.class == *class && !class.is_empty())
                .map(|d| Some((d, *modal)))
                .ok_or_else(unresolved),
            Goal::Route(None) => Err(unresolved()),
            Goal::Route(Some(path)) => {
                let mut path = path.split('?').next().unwrap_or("").trim();
                while let Some(rest) = path.strip_prefix("..") {
                    path = rest.trim_start_matches('/');
                }
                if path.is_empty() {
                    return Ok(None);
                }
                if path.contains("{p}") {
                    return Err(unresolved());
                }
                let (start, rest) = match path.strip_prefix("//") {
                    Some(_) => {
                        let (route, doc) = (ix.shell.iter())
                            .filter(|(r, _)| path == r || path.starts_with(&format!("{r}/")))
                            .max_by_key(|(r, _)| r.len())
                            .ok_or_else(unresolved)?;
                        (Some(*doc), path[route.len()..].trim_start_matches('/'))
                    }
                    None => (None, path.trim_start_matches('/')),
                };
                if rest.is_empty() {
                    return Ok(start.map(|d| (d, false)));
                }
                let registered = |route: &str| {
                    let docs: BTreeSet<usize> = (ix.registered.iter())
                        .filter(|(r, _)| r == route)
                        .map(|(_, d)| *d)
                        .collect();
                    (docs.len() == 1).then(|| *docs.first().unwrap_or(&0))
                };
                let last = registered(rest).or_else(|| {
                    let docs: Option<Vec<usize>> = rest.split('/').map(registered).collect();
                    docs?.pop()
                });
                last.map(|d| Some((d, false))).ok_or_else(unresolved)
            }
        }
    }
}

/// Origin of code at `at` in cs file `ci`, with the docs whose XAML holds its triggers.
type Context = (Origin, Vec<usize>, Trigger);

/// How elements trigger the method: an event handler or a command.
#[derive(Clone, Copy, PartialEq)]
enum Trigger {
    Event,
    Command,
    None,
}

fn origin(ix: &Index, ci: usize, at: usize, flow: &mut UiFlow) -> Context {
    let any = || Origin::AnyScreen {
        application: ix.app.name.clone(),
    };
    if let Some(&doc) = ix.behind.get(&ci) {
        return (
            Origin::Screen(ix.docs[doc].id.clone()),
            vec![doc],
            Trigger::Event,
        );
    }
    let (file, text) = &ix.app.cs[ci];
    let class = index::class_at(text, at);
    let Some(class) = class.filter(|c| c.ends_with("ViewModel")) else {
        return (any(), vec![], Trigger::None);
    };
    let docs = ix.bound.get(&class).cloned().unwrap_or_default();
    if let [doc] = docs[..] {
        return (
            Origin::Screen(ix.docs[doc].id.clone()),
            docs,
            Trigger::Command,
        );
    }
    let bound: Vec<&str> = docs.iter().map(|&d| ix.docs[d].id.as_str()).collect();
    let message = match bound.len() {
        0 => format!(
            "{class} is bound to no screen; its navigation and calls start at any screen of {}.",
            ix.app.name
        ),
        _ => format!(
            "{class} is bound to several screens ({}); its navigation and calls start at any screen of {}.",
            bound.join(", "),
            ix.app.name
        ),
    };
    let line = text
        .find(&format!("class {class}"))
        .map_or(0, |at| line(text, at));
    note(flow, file, line, message);
    (any(), docs, Trigger::Command)
}

/// Labels of the elements that trigger the method around `at`; `[None]` when
/// no labelled element is found.
fn triggers(
    ix: &Index,
    ci: usize,
    methods: &[text::Scope],
    at: usize,
    context: &Context,
) -> Vec<Option<String>> {
    let text = &ix.app.cs[ci].1;
    let method = methods
        .iter()
        .filter(|m| m.name != "if" && m.body.0 < at && at < m.body.1)
        .max_by_key(|m| m.body.0);
    let (Some(method), (_, docs, kind)) = (method, context) else {
        return vec![None];
    };
    let commands = commands(text, method);
    let fires = |(name, value): (&str, &str)| match kind {
        Trigger::Event => EVENTS.contains(&name) && value == method.name,
        Trigger::Command => {
            name.ends_with("Command") && commands.iter().any(|c| has_word(value, c))
        }
        Trigger::None => false,
    };
    let mut labels = Vec::new();
    for &doc in docs {
        let xaml = &ix.app.xaml[ix.docs[doc].xaml].1;
        let firing = tags(xaml).filter(|tag| attributes(tag.attrs).into_iter().any(fires));
        for label in firing.filter_map(|tag| label(tag.attrs)) {
            if !labels.contains(&Some(label.to_string())) {
                labels.push(Some(label.to_string()));
            }
        }
    }
    if labels.is_empty() {
        labels.push(None);
    }
    labels
}

/// The element's `AutomationId`, else its literal `Text` or `Content`.
fn label(attrs: &str) -> Option<&str> {
    ["AutomationId", "Text", "Content"]
        .iter()
        .find_map(|n| attr(attrs, n).filter(|v| !v.is_empty() && !v.starts_with('{')))
}

/// Command property names that run `method`.
fn commands(text: &str, method: &text::Scope) -> Vec<String> {
    let mut found = Vec::new();
    let head = &text[..method.at];
    let member = head.rfind([';', '}', '{']).map_or(head, |i| &head[i..]);
    if member.contains("[RelayCommand") {
        let base = method.name.strip_suffix("Async").unwrap_or(&method.name);
        found.push(format!("{base}Command"));
    }
    for (at, _) in text.match_indices("Command") {
        let name = &text[ident_start(&text[..at])..at + "Command".len()];
        let after = text[at + "Command".len()..].trim_start();
        let Some(init) = after
            .strip_prefix("=>")
            .or_else(|| after.strip_prefix('=').filter(|r| !r.starts_with('=')))
        else {
            continue;
        };
        let init = balanced(init, ';');
        if init.trim_start().starts_with("new") && has_word(init, &method.name) {
            found.push(name.to_string());
        }
    }
    found
}

/// Guard condition of the `if` blocks around `at`.
fn guard(text: &str, scopes: &[text::Scope], at: usize) -> Option<Condition> {
    let mut found = Vec::new();
    for scope in scopes
        .iter()
        .filter(|s| s.name == "if" && s.body.0 < at && at < s.body.1)
    {
        let cond = &text[scope.cond.0..scope.cond.1];
        for name in GUARDS {
            for (i, _) in cond
                .match_indices(name)
                .filter(|&(i, _)| text::word_at(cond, i, name))
            {
                let rest = &cond[i + name.len()..];
                let call = match rest.trim_start().strip_prefix('(') {
                    Some(args) => format!("{name}({})", balanced(args, ')').trim()),
                    None => name.to_string(),
                };
                let custom = Condition::Custom(call);
                // `!IsAuthenticated`, `!_session.IsAuthenticated`.
                let chain = cond[..i]
                    .trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '.');
                found.push(if chain.trim_end().ends_with('!') {
                    Condition::Not(Box::new(custom))
                } else {
                    custom
                });
            }
        }
    }
    match found.len() {
        0 => None,
        1 => found.pop(),
        _ => Some(Condition::All(found)),
    }
}

/// Screens, routes, bindings, wrappers and services of one application.
mod index {
    use super::*;

    /// Base classes known to the rules, by kind.
    fn kind_of(root: &str, bases: &BTreeMap<String, String>) -> Option<&'static str> {
        let mut name = root;
        for _ in 0..8 {
            match name {
                "ContentPage" | "TabbedPage" | "FlyoutPage" | "NavigationPage" | "CarouselPage"
                | "Page" => return Some("page"),
                "Window" => return Some("window"),
                "UserControl" => return Some("control"),
                "Shell" => return Some("shell"),
                _ => name = bases.get(name)?,
            }
        }
        None
    }

    /// Base class by class, and the view model class names.
    fn hierarchy(app: &App) -> (BTreeMap<String, String>, BTreeSet<String>) {
        let mut bases = BTreeMap::new();
        let mut view_models = BTreeSet::new();
        for (_, class, base) in app.cs.iter().flat_map(|(_, text)| classes(text)) {
            if class.ends_with("ViewModel") {
                view_models.insert(class.clone());
            }
            if let Some(base) = base {
                bases.insert(class, base);
            }
        }
        (bases, view_models)
    }

    /// The screens and the indexes of the Shell XAML files.
    fn documents(
        app: &App,
        bases: &BTreeMap<String, String>,
        shown: &BTreeMap<String, Option<String>>,
    ) -> (Vec<Doc>, Vec<usize>) {
        let (mut docs, mut shells) = (Vec::new(), Vec::new());
        for (xi, (file, text)) in app.xaml.iter().enumerate() {
            let Some(root) = tags(text).next() else {
                continue;
            };
            let local = root.name.rsplit(':').next().unwrap_or(root.name);
            let class = attr(root.attrs, "x:Class").map_or_else(
                || {
                    let name = file.rsplit('/').next().unwrap_or(file);
                    &name[..name.find('.').unwrap_or(name.len())]
                },
                |c| c.rsplit('.').next().unwrap_or(c),
            );
            let kind = match kind_of(local, bases) {
                Some("page") => ScreenKind::Page,
                Some("window") => ScreenKind::Window,
                Some("control") if shown.contains_key(class) => ScreenKind::View,
                Some("shell") => {
                    shells.push(xi);
                    continue;
                }
                _ => continue,
            };
            let inner = match app.dir.as_str() {
                "" => file.as_str(),
                dir => file.strip_prefix(&format!("{dir}/")).unwrap_or(file),
            };
            docs.push(Doc {
                id: format!("{}:{inner}", app.name),
                class: class.to_string(),
                kind,
                file: file.clone(),
                line: line(text, root.at),
                xaml: xi,
            });
        }
        (docs, shells)
    }

    pub(super) fn build<'a>(app: &'a App, flow: &mut UiFlow) -> Index<'a> {
        let (bases, view_models) = hierarchy(app);
        let shown = if app.maui {
            BTreeMap::new()
        } else {
            templated(app)
        };
        let (docs, shells) = documents(app, &bases, &shown);
        let mut ix = Index {
            app,
            docs,
            shell: vec![],
            registered: vec![],
            bound: BTreeMap::new(),
            behind: BTreeMap::new(),
            wrappers: BTreeSet::new(),
            services: BTreeMap::new(),
            start: None,
        };
        ix.start = start(&ix, &shells);
        for xi in shells {
            shell_routes(&mut ix, xi);
        }
        registered_routes(&mut ix, flow);
        for (ci, (file, _)) in app.cs.iter().enumerate() {
            let xaml = file.strip_suffix(".cs").unwrap_or("");
            if let Some(doc) = ix.docs.iter().position(|d| d.file == xaml) {
                ix.behind.insert(ci, doc);
            }
        }
        bindings(&mut ix, &view_models, &shown);
        callables(&mut ix);
        ix
    }

    /// The doc the application starts with; see the module doc.
    fn start(ix: &Index, shells: &[usize]) -> Option<usize> {
        let doc = |class: &str| ix.docs.iter().position(|d| d.class == class);
        let local =
            |name: &'static str| move |t: &xml::Tag| t.name.rsplit(':').next() == Some(name);
        if let Some(&xi) = shells.first() {
            let content = tags(&ix.app.xaml[xi].1).find(local("ShellContent"))?;
            return doc(template_class(attr(content.attrs, "ContentTemplate")?));
        }
        for (file, text) in &ix.app.xaml {
            let Some(root) = tags(text).next().filter(local("Application")) else {
                continue;
            };
            if let Some(uri) = attr(root.attrs, "StartupUri") {
                let uri = uri.trim_start_matches('/');
                let target = match file.rsplit_once('/') {
                    Some((dir, _)) => format!("{dir}/{uri}"),
                    None => uri.to_string(),
                };
                return ix.docs.iter().position(|d| d.file == target);
            }
        }
        let methods = ix.app.cs.iter().flat_map(|(_, text)| {
            let bodies = scopes(text)
                .into_iter()
                .filter(|s| STARTUP.contains(&s.name.as_str()));
            bodies.map(move |s| &text[s.body.0..s.body.1])
        });
        methods
            .flat_map(|body| body.match_indices("new ").map(move |(at, _)| (body, at)))
            .filter(|(body, at)| !body[..*at].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
            .find_map(|(body, at)| {
                let rest = &body[at + "new ".len()..];
                doc(&rest[..rest
                    .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len())])
            })
    }

    /// `LoginPage` of `{DataTemplate views:LoginPage}`.
    fn template_class(template: &str) -> &str {
        let class = template
            .trim_start_matches("{DataTemplate")
            .trim_end_matches('}')
            .trim();
        class.rsplit(':').next().unwrap_or(class)
    }

    /// `class Name ... : Base`: (offset, name, first base without namespace or generics).
    pub(super) fn classes(text: &str) -> Vec<(usize, String, Option<String>)> {
        let mut found = Vec::new();
        for (at, _) in text.match_indices("class ") {
            if text[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let rest = &text[at + "class ".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                continue;
            }
            let mut rest = rest[name.len()..].trim_start();
            if let Some(generic) = rest.strip_prefix('<') {
                rest = generic
                    .get(balanced(generic, '>').len() + 1..)
                    .unwrap_or("")
                    .trim_start();
            }
            if let Some(args) = rest.strip_prefix('(') {
                rest = args
                    .get(balanced(args, ')').len() + 1..)
                    .unwrap_or("")
                    .trim_start();
            }
            let base = rest.strip_prefix(':').map(|b| {
                let b = b.trim_start();
                let b = &b[..b
                    .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
                    .unwrap_or(b.len())];
                b.rsplit('.').next().unwrap_or(b).to_string()
            });
            found.push((at, name, base.filter(|b| !b.is_empty())));
        }
        found
    }

    /// The class declared last before `at`.
    pub(super) fn class_at(text: &str, at: usize) -> Option<String> {
        classes(text)
            .into_iter()
            .take_while(|(c, ..)| *c < at)
            .last()
            .map(|(_, n, _)| n)
    }

    /// Desktop user controls shown by a typed `DataTemplate` (with its view
    /// model) or inside a `ContentControl`.
    fn templated(app: &App) -> BTreeMap<String, Option<String>> {
        let mut shown = BTreeMap::new();
        for (_, text) in &app.xaml {
            for (open, close, tag) in xml::blocks(text, &["DataTemplate", "ContentControl"]) {
                let vm = attr(tag.attrs, "DataType").map(|t| {
                    let t = t.trim_start_matches("{x:Type").trim_end_matches('}').trim();
                    t.rsplit(':').next().unwrap_or(t).to_string()
                });
                if tag.name.ends_with("DataTemplate") && vm.is_none() {
                    continue;
                }
                for inner in tags(&text[open..close]).skip(1) {
                    let name = inner.name.rsplit(':').next().unwrap_or(inner.name);
                    shown.entry(name.to_string()).or_insert(vm.clone());
                }
            }
        }
        shown
    }

    fn shell_routes(ix: &mut Index, xi: usize) {
        let text = &ix.app.xaml[xi].1;
        let mut stack: Vec<(&str, Option<&str>)> = Vec::new();
        for tag in xml::all_tags(text) {
            let local = tag.name.rsplit(':').next().unwrap_or(tag.name);
            if tag.closing {
                if let Some(i) = stack.iter().rposition(|(n, _)| *n == local) {
                    stack.truncate(i);
                }
                continue;
            }
            let route = attr(tag.attrs, "Route").filter(|_| SHELL_ITEMS.contains(&local));
            if local == "ShellContent"
                && let Some(template) = attr(tag.attrs, "ContentTemplate")
            {
                let class = template_class(template);
                let segments: Vec<&str> =
                    stack.iter().filter_map(|(_, r)| *r).chain(route).collect();
                if let Some(doc) = ix.docs.iter().position(|d| d.class == class)
                    && !segments.is_empty()
                {
                    ix.shell.push((format!("//{}", segments.join("/")), doc));
                }
            }
            if !tag.self_closing {
                stack.push((local, route));
            }
        }
    }

    fn registered_routes(ix: &mut Index, flow: &mut UiFlow) {
        for (file, text) in &ix.app.cs {
            for (at, _) in text.match_indices("RegisterRoute(") {
                let args = split_args(balanced(&text[at + "RegisterRoute(".len()..], ')'));
                let [route, page, ..] = &args[..] else {
                    continue;
                };
                let class = page
                    .strip_prefix("typeof(")
                    .and_then(|p| p.strip_suffix(')'))
                    .unwrap_or("");
                let class = class.rsplit('.').next().unwrap_or(class);
                let doc = ix.docs.iter().position(|d| d.class == class);
                match (string_value(route, &|_| None), doc) {
                    (Some(route), Some(doc)) if !route.contains("{p}") => {
                        ix.registered.push((route, doc))
                    }
                    (_, None) => report(
                        flow,
                        file,
                        line(text, at),
                        format!(
                            "Registered page {page} has no XAML page in this application; navigation to it stays unresolved."
                        ),
                    ),
                    (_, Some(_)) => report(
                        flow,
                        file,
                        line(text, at),
                        format!("Route {route} is not a literal; it is no route of the screen."),
                    ),
                }
            }
        }
    }

    /// View model names doc `d` binds explicitly: root `x:DataType`, a
    /// `DataContext`/`BindingContext` property element, a typed template, or
    /// a code-behind assigning the context and naming the view model.
    fn explicit_bindings(
        ix: &Index,
        d: usize,
        view_models: &BTreeSet<String>,
        shown: &BTreeMap<String, Option<String>>,
    ) -> BTreeSet<String> {
        let local = |name: &str| name.rsplit(':').next().unwrap_or(name).to_string();
        let doc = &ix.docs[d];
        let xaml = &ix.app.xaml[doc.xaml].1;
        let mut names = BTreeSet::new();
        let root = tags(xaml).next();
        names.extend(root.and_then(|r| attr(r.attrs, "x:DataType")).map(local));
        for property in [".DataContext>", ".BindingContext>"] {
            let inner = xaml
                .find(property)
                .and_then(|i| tags(&xaml[i + property.len()..]).next());
            names.extend(inner.map(|t| local(t.name)));
        }
        names.extend(shown.get(&doc.class).cloned().flatten());
        let behind = (ix.behind.iter())
            .find(|(_, x)| **x == d)
            .map(|(c, _)| &ix.app.cs[*c].1)
            .filter(|t| t.contains("BindingContext") || t.contains("DataContext"));
        if let Some(text) = behind {
            names.extend(view_models.iter().filter(|vm| has_word(text, vm)).cloned());
        }
        names
    }

    /// View model -> docs: explicit bindings first, the naming convention for
    /// docs and view models that have none.
    fn bindings(
        ix: &mut Index,
        view_models: &BTreeSet<String>,
        shown: &BTreeMap<String, Option<String>>,
    ) {
        let mut bound: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut explicit = BTreeSet::new();
        for d in 0..ix.docs.len() {
            let names = explicit_bindings(ix, d, view_models, shown);
            for name in names.into_iter().filter(|n| view_models.contains(n)) {
                explicit.insert(d);
                bound.entry(name).or_default().push(d);
            }
        }
        for (d, doc) in ix
            .docs
            .iter()
            .enumerate()
            .filter(|(d, _)| !explicit.contains(d))
        {
            let stem = ["Page", "View", "Window"]
                .iter()
                .find_map(|s| doc.class.strip_suffix(s))
                .unwrap_or(&doc.class);
            for vm in [
                format!("{stem}ViewModel"),
                format!("{}ViewModel", doc.class),
            ] {
                if view_models.contains(&vm) && !bound.contains_key(&vm) {
                    bound.entry(vm).or_default().push(d);
                    break;
                }
            }
        }
        ix.bound = bound;
    }

    /// Navigation wrappers and the methods of service classes.
    fn callables(ix: &mut Index) {
        for (ci, (_, text)) in ix.app.cs.iter().enumerate() {
            let classes = classes(text);
            for m in scopes(text).into_iter().filter(|m| m.name != "if") {
                let param = split_args(&text[m.cond.0..m.cond.1])
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                let words: Vec<&str> = param.split_whitespace().collect();
                let body = &text[m.body.0..m.body.1];
                if let [.., ty, name] = words[..]
                    && ty == "string"
                    && body.contains("GoToAsync(")
                    && has_word(body, name)
                {
                    ix.wrappers.insert(m.name.clone());
                }
                let class = classes
                    .iter()
                    .take_while(|(c, ..)| *c < m.at)
                    .last()
                    .map(|(_, n, _)| n.as_str());
                if !ix.behind.contains_key(&ci) && !class.is_some_and(|c| c.ends_with("ViewModel"))
                {
                    ix.services
                        .entry(m.name.clone())
                        .or_default()
                        .push((ci, m.body));
                }
            }
        }
    }
}

/// HTTP actions of view models and code-behinds, with one service hop.
mod actions {
    use super::*;

    /// (caller offset, cs file of the call, call offset, call).
    type Found = (usize, usize, usize, Call);

    /// Direct calls of screen-side code in file `ci` and those one service hop away.
    fn found(ix: &Index, ci: usize) -> Vec<Found> {
        let text = &ix.app.cs[ci].1;
        let screen_side = |at: usize| {
            ix.behind.contains_key(&ci)
                || index::class_at(text, at).is_some_and(|c| c.ends_with("ViewModel"))
        };
        let direct = http(text, (0, text.len())).into_iter();
        let mut found: Vec<Found> = direct
            .filter(|(at, _)| screen_side(*at))
            .map(|(at, call)| (at, ci, at, call))
            .collect();
        for (name, bodies) in &ix.services {
            let hops = calls(text, name).into_iter().map(|(at, _)| at);
            for at in hops.filter(|&at| screen_side(at) && text[..at].ends_with('.')) {
                for &(si, body) in bodies {
                    let calls = http(&ix.app.cs[si].1, body).into_iter();
                    found.extend(calls.map(|(call_at, call)| (at, si, call_at, call)));
                }
            }
        }
        found.sort_by_key(|(at, si, call_at, _)| (*at, *si, *call_at));
        found
    }

    pub(super) fn collect(ix: &Index, ci: usize, methods: &[text::Scope], flow: &mut UiFlow) {
        let text = &ix.app.cs[ci].1;
        for (at, si, call_at, call) in found(ix, ci) {
            let (call_file, call_text) = &ix.app.cs[si];
            let line = line(call_text, call_at);
            if let Call::Unresolved(arg) = &call {
                let message =
                    format!("HTTP call path {arg} is not a literal; its endpoint is unknown.");
                report(flow, call_file, line, message);
            }
            let context = origin(ix, ci, at, flow);
            let condition = guard(text, methods, at);
            for trigger in triggers(ix, ci, methods, at, &context) {
                let action = Action {
                    weak: false,
                    screen: context.0.clone(),
                    trigger,
                    call: call.clone(),
                    file: call_file.clone(),
                    line,
                    condition: archguard_core::AccessState::from_parts(condition.clone(), false),
                    source: SOURCE.into(),
                };
                if !flow.actions.contains(&action) {
                    flow.actions.push(action);
                }
            }
        }
    }

    /// Direct HTTP client calls within `span`: (offset, call).
    fn http(text: &str, (from, to): (usize, usize)) -> Vec<(usize, Call)> {
        let consts = constants(text);
        let lookup = |name: &str| consts.get(name).cloned();
        let mut found = Vec::new();
        for (at, _) in text[from..to].match_indices("Async") {
            let at = from + at;
            let before = &text[..at];
            let start = ident_start(before);
            let Some(verb) = VERBS.iter().find(|v| before[start..].starts_with(*v)) else {
                continue;
            };
            let Some(object) = before[..start].strip_suffix('.') else {
                continue;
            };
            let receiver = &object[ident_start(object)..];
            let declared = text.match_indices("HttpClient").any(|(i, _)| {
                let rest = text[i + "HttpClient".len()..]
                    .trim_start_matches('?')
                    .trim_start();
                rest.starts_with(receiver)
                    && !rest[receiver.len()..]
                        .starts_with(|c: char| c.is_alphanumeric() || c == '_')
            });
            if receiver.is_empty() || !(declared || receiver.to_ascii_lowercase().contains("http"))
            {
                continue;
            }
            let mut rest = &text[at + "Async".len()..];
            if let Some(generic) = rest.strip_prefix('<') {
                rest = generic
                    .get(balanced(generic, '>').len() + 1..)
                    .unwrap_or("");
            }
            let Some(args) = rest.strip_prefix('(') else {
                continue;
            };
            let arg = split_args(balanced(args, ')'))
                .into_iter()
                .next()
                .unwrap_or_default();
            let call = match string_value(&arg, &lookup) {
                // A leading hole is an unknown base address, no path.
                Some(path) if !path.starts_with("{p}") => Call::Http {
                    method: verb.to_uppercase(),
                    path: shape(match path.split_once("://") {
                        Some((_, host)) => host.split_once('/').map_or("", |(_, p)| p),
                        None => &path,
                    }),
                },
                _ => Call::Unresolved(arg),
            };
            found.push((start, call));
        }
        found
    }

    /// `string NAME = "literal";` fields and constants of one file.
    fn constants(text: &str) -> BTreeMap<String, String> {
        let mut found = BTreeMap::new();
        for (at, _) in text.match_indices("string ") {
            let rest = &text[at + "string ".len()..];
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let after = rest.trim_start()[name.len()..].trim_start();
            if let Some(value) = after.strip_prefix('=')
                && let Some(value) = literal(balanced(value, ';').trim())
                && !name.is_empty()
            {
                found.insert(name, value);
            }
        }
        found
    }

    /// Route shape: query and fragment dropped, leading `/`, holes `{p}`.
    fn shape(path: &str) -> String {
        let path = path.split(['?', '#']).next().unwrap_or("");
        let segments: Vec<&str> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| if s.contains('{') { "{p}" } else { s })
            .collect();
        format!("/{}", segments.join("/"))
    }
}

/// C# token scanning without knowledge of the model.
mod text {
    pub(super) use crate::lang_csharp::{
        balanced, ident_start, is_ident as ident, literal, split_args,
    };
    pub(super) use crate::lang_markup::blank;
    pub(super) use crate::lang_text::line;

    /// `word` at `at` in `hay` is a whole identifier.
    pub(super) fn word_at(hay: &str, at: usize, word: &str) -> bool {
        !hay[..at].ends_with(ident) && !hay[at + word.len()..].starts_with(ident)
    }

    pub(super) fn has_word(hay: &str, word: &str) -> bool {
        hay.match_indices(word).any(|(i, _)| word_at(hay, i, word))
    }

    /// `name` at `at` is called, not declared: followed by `(` and preceded
    /// by `.`, `=>`, an operator or a keyword such as `await`, not by a type.
    pub(super) fn is_call(text: &str, at: usize, name: &str) -> bool {
        if !word_at(text, at, name) || !text[at + name.len()..].trim_start().starts_with('(') {
            return false;
        }
        let head = text[..at].trim_end();
        if head.ends_with('.') || head.ends_with("=>") {
            return true;
        }
        let prev = &head[ident_start(head)..];
        if prev.is_empty() {
            return !head.ends_with(['>', ']']);
        }
        [
            "await", "return", "else", "in", "is", "throw", "yield", "case",
        ]
        .contains(&prev)
    }

    /// A method or `if` with its parameter (or condition) span and body span.
    pub(super) struct Scope {
        pub(super) name: String,
        /// Offset of the name.
        pub(super) at: usize,
        pub(super) cond: (usize, usize),
        pub(super) body: (usize, usize),
    }

    const KEYWORDS: [&str; 18] = [
        "for", "foreach", "while", "switch", "catch", "using", "lock", "fixed", "when", "nameof",
        "typeof", "sizeof", "default", "base", "this", "return", "await", "async",
    ];
    /// Words before a name that make it a call or creation, not a declaration.
    const NOT_TYPES: [&str; 12] = [
        "new", "await", "return", "else", "in", "is", "as", "throw", "yield", "case", "out", "ref",
    ];

    /// Method declarations (block or expression body) and `if` statements.
    pub(super) fn scopes(text: &str) -> Vec<Scope> {
        let mut found = Vec::new();
        for (open, _) in text.match_indices('(') {
            let Some(start) = scope_name(text, open) else {
                continue;
            };
            let name = &text[start..];
            let name = &name[..name.find(|c: char| !ident(c)).unwrap_or(name.len())];
            let close = open + 1 + balanced(&text[open + 1..], ')').len();
            if let Some(body) = scope_body(text, close, name == "if") {
                found.push(Scope {
                    name: name.into(),
                    at: start,
                    cond: (open + 1, close),
                    body,
                });
            }
        }
        found
    }

    /// Offset of the method or `if` name before the `(` at `open`, when the
    /// text there declares a method (a type before the name) or is an `if`.
    fn scope_name(text: &str, open: usize) -> Option<usize> {
        let mut head = text[..open].trim_end();
        if head.ends_with('>') && !head.ends_with("=>") {
            head = head[..head.rfind('<')?].trim_end();
        }
        let start = ident_start(head);
        let name = &head[start..];
        let usable = !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && !KEYWORDS.contains(&name);
        if !usable {
            return None;
        }
        if name == "if" {
            return Some(start);
        }
        let before = head[..start].trim_end();
        let prev = &before[ident_start(before)..];
        let typed = before.ends_with(|c: char| ident(c) || c == ']' || c == '?')
            || (before.ends_with('>') && !before.ends_with("=>"));
        (typed && !NOT_TYPES.contains(&prev)).then_some(start)
    }

    /// Body span after the `)` at `close`: a block, an expression body or,
    /// for `if`, one statement. Skips `: base(...)` and `where` clauses.
    fn scope_body(text: &str, close: usize, is_if: bool) -> Option<(usize, usize)> {
        let after = text.get(close + 1..)?;
        let mut at = close + 1 + (after.len() - after.trim_start().len());
        let rest = &text[at..];
        let init = rest.strip_prefix(':').map(str::trim_start);
        let constructor = init.is_some_and(|r| r.starts_with("base(") || r.starts_with("this("));
        if !is_if && (constructor || rest.starts_with("where ")) {
            let i = rest.find(['{', ';'])?;
            rest[i..].starts_with('{').then_some(())?;
            at += i;
        }
        let rest = &text[at..];
        if let Some(inner) = rest.strip_prefix('{') {
            Some((at, at + 1 + balanced(inner, '}').len()))
        } else if is_if || rest.starts_with("=>") {
            Some((at, at + balanced(rest, ';').len()))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{
        Call, Condition, DiagnosticSeverity, Origin, Screen, ScreenKind, Target, Transition,
    };

    fn write(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn screen<'a>(flow: &'a UiFlow, id: &str) -> &'a Screen {
        flow.screens
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no screen {id}: {:#?}", flow.screens))
    }

    fn from<'a>(flow: &'a UiFlow, id: &str) -> Vec<&'a Transition> {
        let origin = Origin::Screen(id.into());
        flow.transitions
            .iter()
            .filter(|t| t.from == origin)
            .collect()
    }

    fn to(id: &str) -> Target {
        Target::Screen(id.into())
    }

    const MAUI_PROJ: &str =
        "<Project><PropertyGroup><UseMaui>true</UseMaui></PropertyGroup></Project>";

    const SHELL: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<Shell x:Class="Shop.AppShell" xmlns:views="clr-namespace:Shop.Views">
    <Shell.Resources><ResourceDictionary /></Shell.Resources>
    <!-- <ShellContent Route="Ghost" ContentTemplate="{DataTemplate views:GhostPage}" /> -->
    <FlyoutItem>
        <ShellContent ContentTemplate="{DataTemplate views:LoginPage}" Route="Login" />
    </FlyoutItem>
    <TabBar Route="Main">
        <Tab Route="Shop">
            <ShellContent
                Title="CATALOG"
                ContentTemplate="{DataTemplate views:CatalogPage}"
                Route="Catalog" />
        </Tab>
    </TabBar>
</Shell>
"#;

    const SHELL_CS: &str = r#"public partial class AppShell : Shell
{
    public AppShell()
    {
        //Routing.RegisterRoute("Ghost", typeof(GhostPage));
        Routing.RegisterRoute("Detail", typeof(DetailPage));
        Routing.RegisterRoute(nameof(EditPage), typeof(EditPage));
        Routing.RegisterRoute("Modal", typeof(ModalPage));
        InitializeComponent();
    }
}
"#;

    fn page(class: &str, body: &str) -> String {
        format!(
            "<ContentPage x:Class=\"Shop.Views.{class}\" xmlns:vm=\"clr-namespace:Shop.ViewModels\">\n{body}\n</ContentPage>\n"
        )
    }

    const CATALOG_CS: &str = r#"public partial class CatalogPage
{
    public CatalogPage(CatalogViewModel viewModel)
    {
        BindingContext = viewModel;
        InitializeComponent();
    }

    async void OnBack(object sender, EventArgs e) => await Shell.Current.GoToAsync("//Login");

    async Task Jumps(int id, string target)
    {
        await Shell.Current.GoToAsync("Detail?id=4");
        await Shell.Current.GoToAsync($"Detail?id={id}");
        await Shell.Current.GoToAsync($"{nameof(EditPage)}?id={id}");
        await Shell.Current.GoToAsync("..");
        await Shell.Current.GoToAsync(target);
        await Shell.Current.GoToAsync($"items/{id}");
        await Navigation.PushModalAsync(new ModalPage());
        await Navigation.PushAsync(new DetailPage(id));
        if (IsAuthenticated)
        {
            await Shell.Current.GoToAsync("//Main/Shop/Catalog");
        }
    }
}
"#;

    const NAV_SERVICE: &str = r#"public interface INavigationService
{
    Task NavigateToAsync(string route, IDictionary<string, object> routeParameters = null);
}

public class MauiNavigationService : INavigationService
{
    public Task InitializeAsync(bool signedIn) =>
        NavigateToAsync(signedIn ? "//Main/Shop/Catalog" : "//Login");

    public Task NavigateToAsync(string route, IDictionary<string, object> routeParameters = null)
    {
        var shellNavigation = new ShellNavigationState(route);
        return Shell.Current.GoToAsync(shellNavigation);
    }

    public Task PopAsync() => Shell.Current.GoToAsync("..");
}
"#;

    const CATALOG_VM: &str = r#"public partial class CatalogViewModel : ObservableObject
{
    private const string ItemsBase = "api/items";
    private readonly HttpClient _client;
    private readonly INavigationService _nav;
    private readonly IOrderService _orders;

    public ICommand ShowModalCommand => new Command(async () => await ShowModal());

    [RelayCommand]
    private async Task OpenDetailAsync()
    {
        await _nav.NavigateToAsync("Detail", new Dictionary<string, object> { ["id"] = 1 });
    }

    private async Task ShowModal()
    {
        if (HasRole("Admin"))
        {
            await _nav.NavigateToAsync(nameof(EditPage));
        }
    }

    [RelayCommand]
    private async Task LoadAsync(int id)
    {
        var items = await _client.GetFromJsonAsync<Item[]>($"{ItemsBase}/{id}");
        await _client.DeleteAsync(MakeUrl(id));
        await _orders.LoadOrdersAsync();
    }
}
"#;

    const ORDER_SERVICE: &str = r#"public class OrderService : IOrderService
{
    private static readonly string OrdersUrl = "https://shop.example/api/orders";
    private readonly HttpClient _http;

    public async Task LoadOrdersAsync()
    {
        await _http.GetAsync(OrdersUrl);
    }
}
"#;

    const DETAIL_VM: &str = r#"public class DetailViewModel
{
    public async Task CloseAsync() => await Shell.Current.GoToAsync("//Main/Shop/Catalog");
}
"#;

    const SHARED_VM: &str = r#"public class SharedViewModel
{
    public Task HomeAsync() => Shell.Current.GoToAsync("//Login");
}
"#;

    fn maui() -> tempfile::TempDir {
        let catalog = page(
            "CatalogPage",
            r#"    <Button AutomationId="open-detail" Text="Open" Command="{Binding OpenDetailCommand}" />
    <Button Text="Back to login" Clicked="OnBack" />
    <ToolbarItem Text="ADMIN" Command="{Binding ShowModalCommand}" />
    <Button Command="{Binding LoadCommand}" />"#,
        );
        let edit = page("EditPage", "");
        let edit = edit.replace(
            "<ContentPage x:Class=\"Shop.Views.EditPage\"",
            "<ContentPage x:Class=\"Shop.Views.EditPage\" x:DataType=\"vm:SharedViewModel\"",
        );
        let files: Vec<(&str, String)> = vec![
            ("Shop/Shop.csproj", MAUI_PROJ.into()),
            ("Shop/AppShell.xaml", SHELL.into()),
            ("Shop/AppShell.xaml.cs", SHELL_CS.into()),
            ("Shop/Views/LoginPage.xaml", page("LoginPage", "")),
            ("Shop/Views/CatalogPage.xaml", catalog),
            ("Shop/Views/CatalogPage.xaml.cs", CATALOG_CS.into()),
            ("Shop/Views/DetailPage.xaml", page("DetailPage", "")),
            ("Shop/Views/EditPage.xaml", edit),
            ("Shop/Views/ModalPage.xaml", page("ModalPage", "")),
            (
                "Shop/Views/OrphanPage.xaml",
                "<local:PageBase x:Class=\"Shop.Views.OrphanPage\">\n</local:PageBase>\n".into(),
            ),
            (
                "Shop/Views/PageBase.cs",
                "public abstract class PageBase : ContentPage { }\n".into(),
            ),
            (
                "Shop/Views/StatsPage.xaml",
                page("StatsPage", "").replace(
                    "<ContentPage x:",
                    "<ContentPage x:DataType=\"vm:SharedViewModel\" x:",
                ),
            ),
            (
                "Shop/Controls/Badge.xaml",
                "<ContentView x:Class=\"Shop.Controls.Badge\" />\n".into(),
            ),
            ("Shop/Services/NavigationService.cs", NAV_SERVICE.into()),
            ("Shop/Services/OrderService.cs", ORDER_SERVICE.into()),
            ("Shop/ViewModels/CatalogViewModel.cs", CATALOG_VM.into()),
            ("Shop/ViewModels/DetailViewModel.cs", DETAIL_VM.into()),
            ("Shop/ViewModels/SharedViewModel.cs", SHARED_VM.into()),
            ("Shop/bin/Debug/Old.xaml", page("OldPage", "")),
            ("Shop/obj/Gen.xaml", page("GenPage", "")),
            ("Shop.Tests/Shop.Tests.csproj", MAUI_PROJ.into()),
            ("Shop.Tests/FakePage.xaml", page("FakePage", "")),
            ("Lib/Lib.csproj", "<Project />".into()),
            ("Lib/LibPage.xaml", page("LibPage", "")),
        ];
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
        write(&files)
    }

    #[test]
    fn shell_hierarchy_and_registered_routes_become_page_screens() {
        let flow = extract(maui().path());
        let route = |id: &str| screen(&flow, id).route.clone();
        assert_eq!(
            route("Shop:Views/LoginPage.xaml").as_deref(),
            Some("//Login")
        );
        assert_eq!(
            route("Shop:Views/CatalogPage.xaml").as_deref(),
            Some("//Main/Shop/Catalog")
        );
        assert_eq!(
            route("Shop:Views/DetailPage.xaml").as_deref(),
            Some("Detail")
        );
        assert_eq!(
            route("Shop:Views/EditPage.xaml").as_deref(),
            Some("EditPage")
        );
        let catalog = screen(&flow, "Shop:Views/CatalogPage.xaml");
        assert_eq!(
            (catalog.name.as_str(), catalog.kind, catalog.line),
            ("CatalogPage", ScreenKind::Page, 1)
        );
        assert_eq!(catalog.application.as_deref(), Some("Shop"));
        assert_eq!(catalog.file, "Shop/Views/CatalogPage.xaml");
        assert_eq!(catalog.source, "xaml");
        // bin, obj, tests, non-app projects, content views and the shell are no screens.
        let mut ids: Vec<&str> = flow.screens.iter().map(|s| s.id.as_str()).collect();
        ids.sort();
        assert_eq!(
            ids,
            [
                "Shop:Views/CatalogPage.xaml",
                "Shop:Views/DetailPage.xaml",
                "Shop:Views/EditPage.xaml",
                "Shop:Views/LoginPage.xaml",
                "Shop:Views/ModalPage.xaml",
                "Shop:Views/OrphanPage.xaml",
                "Shop:Views/StatsPage.xaml",
            ]
        );
    }

    #[test]
    fn page_without_shell_route_gets_a_note() {
        let flow = extract(maui().path());
        let orphan = screen(&flow, "Shop:Views/OrphanPage.xaml");
        assert_eq!((orphan.kind, &orphan.route), (ScreenKind::Page, &None));
        let note = flow
            .diagnostics
            .iter()
            .find(|d| d.file == "Shop/Views/OrphanPage.xaml")
            .unwrap();
        assert!(note.message.contains("not reachable through Shell routing"));
        assert_eq!(note.severity, DiagnosticSeverity::Note);
        // The commented-out `RegisterRoute("Ghost", ...)` is no route and no gap.
        assert!(
            flow.diagnostics
                .iter()
                .all(|d| d.file != "Shop/AppShell.xaml.cs"),
            "{:#?}",
            flow.diagnostics
        );
    }

    #[test]
    fn go_to_async_resolves_absolute_relative_query_and_interpolated_targets() {
        let flow = extract(maui().path());
        let jumps: Vec<(&Target, u32)> = from(&flow, "Shop:Views/CatalogPage.xaml")
            .into_iter()
            .filter(|t| t.file == "Shop/Views/CatalogPage.xaml.cs")
            .map(|t| (&t.target, t.line))
            .collect();
        assert_eq!(
            jumps,
            [
                (&to("Shop:Views/LoginPage.xaml"), 9),
                (&to("Shop:Views/DetailPage.xaml"), 13),
                (&to("Shop:Views/DetailPage.xaml"), 14),
                (&to("Shop:Views/EditPage.xaml"), 15),
                (&Target::Unresolved("target".into()), 17),
                (&Target::Unresolved("$\"items/{id}\"".into()), 18),
                (&to("Shop:Views/ModalPage.xaml"), 19),
                (&to("Shop:Views/DetailPage.xaml"), 20),
                (&to("Shop:Views/CatalogPage.xaml"), 23),
            ]
        );
        for line in [17, 18] {
            let gap = flow
                .diagnostics
                .iter()
                .find(|d| d.file == "Shop/Views/CatalogPage.xaml.cs" && d.line == line)
                .unwrap();
            assert_eq!(gap.severity, DiagnosticSeverity::Gap);
        }
        assert!(!flow.complete);
    }

    #[test]
    fn page_only_pushed_modally_is_a_dialog() {
        let flow = extract(maui().path());
        assert_eq!(
            screen(&flow, "Shop:Views/ModalPage.xaml").kind,
            ScreenKind::Dialog
        );
        // Also pushed modelessly and routed: stays a page.
        assert_eq!(
            screen(&flow, "Shop:Views/DetailPage.xaml").kind,
            ScreenKind::Page
        );
    }

    #[test]
    fn clicked_handler_and_guard_shape_the_transition() {
        let flow = extract(maui().path());
        let catalog = from(&flow, "Shop:Views/CatalogPage.xaml");
        let back = catalog.iter().find(|t| t.line == 9).unwrap();
        assert_eq!(back.trigger.as_deref(), Some("Back to login"));
        let guarded = catalog
            .iter()
            .find(|t| t.file.ends_with(".xaml.cs") && t.line == 23)
            .unwrap();
        assert_eq!(
            guarded.condition,
            archguard_core::AccessState::Protected(Condition::Custom("IsAuthenticated".into()))
        );
        assert_eq!(back.condition, archguard_core::AccessState::Undeclared);
    }

    #[test]
    fn view_model_commands_navigate_through_the_wrapper_from_the_bound_page() {
        let flow = extract(maui().path());
        let vm = "Shop/ViewModels/CatalogViewModel.cs";
        let found: Vec<(&Target, Option<&str>, Option<&Condition>)> =
            from(&flow, "Shop:Views/CatalogPage.xaml")
                .into_iter()
                .filter(|t| t.file == vm)
                .map(|t| (&t.target, t.trigger.as_deref(), t.condition.condition()))
                .collect();
        assert_eq!(
            found,
            [
                (&to("Shop:Views/DetailPage.xaml"), Some("open-detail"), None),
                (
                    &to("Shop:Views/EditPage.xaml"),
                    Some("ADMIN"),
                    Some(&Condition::Custom("HasRole(\"Admin\")".into()))
                ),
            ]
        );
        // The wrapper's own forwarding call and its back navigation are no transitions.
        assert!(
            flow.transitions
                .iter()
                .all(|t| !t.file.ends_with("NavigationService.cs") || t.line == 9)
        );
        // A ternary of two literals gives two transitions from every screen.
        let any = Origin::AnyScreen {
            application: "Shop".into(),
        };
        let init: Vec<&Target> = flow
            .transitions
            .iter()
            .filter(|t| t.from == any && t.file.ends_with("NavigationService.cs"))
            .map(|t| &t.target)
            .collect();
        assert_eq!(
            init,
            [
                &to("Shop:Views/CatalogPage.xaml"),
                &to("Shop:Views/LoginPage.xaml")
            ]
        );
    }

    #[test]
    fn view_model_binding_by_naming_convention_and_ambiguity() {
        let flow = extract(maui().path());
        // DetailViewModel has no explicit binding: Views/DetailPage by convention.
        let close = from(&flow, "Shop:Views/DetailPage.xaml");
        assert_eq!(close.len(), 1);
        assert_eq!(close[0].target, to("Shop:Views/CatalogPage.xaml"));
        // SharedViewModel is bound by two pages: any screen plus a note.
        let shared = flow
            .transitions
            .iter()
            .find(|t| t.file == "Shop/ViewModels/SharedViewModel.cs")
            .unwrap();
        assert_eq!(
            shared.from,
            Origin::AnyScreen {
                application: "Shop".into()
            }
        );
        let note = flow
            .diagnostics
            .iter()
            .find(|d| d.file == "Shop/ViewModels/SharedViewModel.cs")
            .unwrap();
        assert_eq!(note.severity, DiagnosticSeverity::Note);
        assert!(note.message.contains("SharedViewModel"), "{}", note.message);
    }

    #[test]
    fn http_calls_in_view_models_and_one_service_hop_become_actions() {
        let flow = extract(maui().path());
        let http = |method: &str, path: &str| Call::Http {
            method: method.into(),
            path: path.into(),
        };
        let found: Vec<(&Origin, &Call, &str, u32)> = flow
            .actions
            .iter()
            .map(|a| (&a.screen, &a.call, a.file.as_str(), a.line))
            .collect();
        let catalog = Origin::Screen("Shop:Views/CatalogPage.xaml".into());
        assert_eq!(
            found,
            [
                (
                    &catalog,
                    &http("GET", "/api/items/{p}"),
                    "Shop/ViewModels/CatalogViewModel.cs",
                    27
                ),
                (
                    &catalog,
                    &Call::Unresolved("MakeUrl(id)".into()),
                    "Shop/ViewModels/CatalogViewModel.cs",
                    28
                ),
                (
                    &catalog,
                    &http("GET", "/api/orders"),
                    "Shop/Services/OrderService.cs",
                    8
                ),
            ]
        );
        assert!(flow.actions.iter().all(|a| a.source == "xaml"));
        let gap = flow
            .diagnostics
            .iter()
            .find(|d| d.file.ends_with("CatalogViewModel.cs") && d.line == 28)
            .unwrap();
        assert_eq!(gap.severity, DiagnosticSeverity::Gap);
    }

    const WPF_PROJ: &str =
        "<Project><PropertyGroup><UseWPF>true</UseWPF></PropertyGroup></Project>";

    const MAIN_WINDOW: &str = r#"<Window x:Class="Desk.MainWindow" xmlns:v="clr-namespace:Desk.Views" xmlns:vm="clr-namespace:Desk.ViewModels">
    <Window.Resources>
        <DataTemplate DataType="{x:Type vm:DashboardViewModel}">
            <v:DashboardView />
        </DataTemplate>
    </Window.Resources>
    <StackPanel>
        <Button Content="Settings" Click="OpenSettings" />
        <Button AutomationId="about" Content="About" Click="OpenAbout" />
        <Frame x:Name="MainFrame" />
    </StackPanel>
</Window>
"#;

    const MAIN_WINDOW_CS: &str = r#"public partial class MainWindow : Window
{
    private void OpenSettings(object sender, RoutedEventArgs e)
    {
        new SettingsWindow().ShowDialog();
    }

    private void OpenAbout(object sender, RoutedEventArgs e)
    {
        var about = new AboutWindow();
        about.Show();
        MainFrame.Navigate(typeof(HomePage));
        NavigationService.Navigate(new HelpPage());
    }
}
"#;

    fn wpf() -> tempfile::TempDir {
        write(&[
            ("Desk.csproj", WPF_PROJ),
            (
                "App.xaml",
                "<Application x:Class=\"Desk.App\" StartupUri=\"MainWindow.xaml\" />\n",
            ),
            ("MainWindow.xaml", MAIN_WINDOW),
            ("MainWindow.xaml.cs", MAIN_WINDOW_CS),
            (
                "SettingsWindow.xaml",
                "<Window x:Class=\"Desk.SettingsWindow\" />\n",
            ),
            (
                "AboutWindow.xaml",
                "<Window x:Class=\"Desk.AboutWindow\" />\n",
            ),
            (
                "Views/HomePage.xaml",
                "<Page x:Class=\"Desk.Views.HomePage\" />\n",
            ),
            (
                "Views/HelpPage.xaml",
                "<Page x:Class=\"Desk.Views.HelpPage\" />\n",
            ),
            (
                "Views/DashboardView.xaml",
                "<UserControl x:Class=\"Desk.Views.DashboardView\" />\n",
            ),
            (
                "Views/UnusedView.xaml",
                "<UserControl x:Class=\"Desk.Views.UnusedView\" />\n",
            ),
        ])
    }

    #[test]
    fn wpf_windows_dialogs_pages_and_template_views() {
        let dir = wpf();
        let flow = extract(dir.path());
        let app = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let id = |rel: &str| format!("{app}:{rel}");
        let kind = |rel: &str| screen(&flow, &id(rel)).kind;
        assert_eq!(kind("MainWindow.xaml"), ScreenKind::Window);
        assert_eq!(kind("SettingsWindow.xaml"), ScreenKind::Dialog);
        assert_eq!(kind("AboutWindow.xaml"), ScreenKind::Window);
        assert_eq!(kind("Views/HomePage.xaml"), ScreenKind::Page);
        assert_eq!(kind("Views/DashboardView.xaml"), ScreenKind::View);
        assert!(flow.screens.iter().all(|s| s.route.is_none()));
        assert!(
            !flow
                .screens
                .iter()
                .any(|s| s.id.ends_with("UnusedView.xaml"))
        );
        let found: Vec<(Target, Option<&str>, u32)> = from(&flow, &id("MainWindow.xaml"))
            .into_iter()
            .map(|t| (t.target.clone(), t.trigger.as_deref(), t.line))
            .collect();
        assert_eq!(
            found,
            [
                (to(&id("SettingsWindow.xaml")), Some("Settings"), 5),
                (to(&id("AboutWindow.xaml")), Some("about"), 10),
                (to(&id("Views/HomePage.xaml")), Some("about"), 12),
                (to(&id("Views/HelpPage.xaml")), Some("about"), 13),
            ]
        );
        // No routing in WPF: no Shell note.
        assert!(flow.complete, "{:#?}", flow.diagnostics);
        assert!(flow.diagnostics.is_empty(), "{:#?}", flow.diagnostics);
    }

    #[test]
    fn start_screens_from_shell_startup_uri_and_startup_code() {
        use archguard_core::{Assignment, ScreenReach, screen_reaches};
        let anonymous = Assignment::default();
        // MAUI: the first ShellContent of the first Shell item.
        let flow = extract(maui().path());
        assert_eq!(flow.start_screens, ["Shop:Views/LoginPage.xaml"]);
        let reaches = screen_reaches(&flow, &anonymous);
        assert_eq!(
            reaches["Shop:Views/LoginPage.xaml"],
            ScreenReach::Undeclared
        );
        assert!(!reaches.values().any(|r| *r == ScreenReach::NoStartScreen));
        // WPF: `StartupUri` of App.xaml.
        let dir = wpf();
        let flow = extract(dir.path());
        let app = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let main = format!("{app}:MainWindow.xaml");
        assert_eq!(flow.start_screens, std::slice::from_ref(&main));
        let reaches = screen_reaches(&flow, &anonymous);
        assert_eq!(reaches[&main], ScreenReach::Undeclared);
        assert_eq!(
            reaches[&format!("{app}:SettingsWindow.xaml")],
            ScreenReach::Undeclared
        );
        // Avalonia and WinUI: the window created in the startup override.
        let avalonia = write(&[
            (
                "Desk/Desk.csproj",
                "<Project><ItemGroup><PackageReference Include=\"Avalonia\" /></ItemGroup></Project>",
            ),
            ("Desk/App.axaml", "<Application x:Class=\"Desk.App\" />\n"),
            (
                "Desk/App.axaml.cs",
                r#"public partial class App : Application
{
    public void Helper() { var w = new OtherWindow(); }
    public override void OnFrameworkInitializationCompleted()
    {
        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime desktop)
            desktop.MainWindow = new MainWindow { DataContext = new MainWindowViewModel() };
    }
}
"#,
            ),
            (
                "Desk/MainWindow.axaml",
                "<Window x:Class=\"Desk.MainWindow\" />\n",
            ),
            (
                "Desk/OtherWindow.axaml",
                "<Window x:Class=\"Desk.OtherWindow\" />\n",
            ),
            (
                "Win/Win.csproj",
                "<Project><ItemGroup><PackageReference Include=\"Microsoft.WindowsAppSDK\" /></ItemGroup></Project>",
            ),
            (
                "Win/App.xaml.cs",
                "public partial class App : Application\n{\n    protected override void OnLaunched(LaunchActivatedEventArgs args)\n    {\n        m_window = new ShellWindow();\n        m_window.Activate();\n    }\n}\n",
            ),
            (
                "Win/ShellWindow.xaml",
                "<Window x:Class=\"Win.ShellWindow\" />\n",
            ),
            ("Hybrid/Hybrid.csproj", MAUI_PROJ),
            (
                "Hybrid/App.xaml.cs",
                "public partial class App : Application\n{\n    protected override Window CreateWindow(IActivationState? state)\n    {\n        return new Window(new MainPage()) { Title = \"Shop\" };\n    }\n}\n",
            ),
            (
                "Hybrid/MainPage.xaml",
                "<ContentPage x:Class=\"Hybrid.MainPage\" />\n",
            ),
            ("Bare/Bare.csproj", WPF_PROJ),
            ("Bare/App.xaml", "<Application x:Class=\"Bare.App\" />\n"),
            (
                "Bare/MainWindow.xaml",
                "<Window x:Class=\"Bare.MainWindow\" />\n",
            ),
        ]);
        let flow = extract(avalonia.path());
        assert_eq!(
            flow.start_screens,
            [
                "Desk:MainWindow.axaml",
                "Hybrid:MainPage.xaml",
                "Win:ShellWindow.xaml"
            ]
        );
        // Nothing found: a note, and the simulation says so.
        let note = (flow.diagnostics.iter())
            .find(|d| d.message.contains("No start screen found"))
            .expect("note for Bare");
        assert_eq!(
            (note.file.as_str(), note.severity),
            ("Bare", DiagnosticSeverity::Note)
        );
        assert_eq!(
            screen_reaches(&flow, &anonymous)["Bare:MainWindow.xaml"],
            ScreenReach::NoStartScreen
        );
    }

    #[test]
    fn no_application_project_means_an_empty_complete_flow() {
        let dir = write(&[
            ("Lib/Lib.csproj", "<Project />"),
            (
                "Lib/MainWindow.xaml",
                "<Window x:Class=\"Lib.MainWindow\" />\n",
            ),
        ]);
        assert_eq!(extract(dir.path()), UiFlow::default());
    }
}
