//! Slint extractor for ui-flow/v1 (ADR 0021 phase 6): one window, state
//! decides the view. A brace and token level scanner over `.slint` and `.rs`
//! files, no Slint or Rust parser.
//!
//! Applications: the `.slint` files below one `Cargo.toml` (or below the root
//! without one) form an application named after that folder. Rust code is
//! read from the packages whose `Cargo.toml` names `slint`; it may act on any
//! application. `target`, `examples`, test folders and comments are skipped.
//!
//! Screens (route none, id `<app>:<identity>`):
//! - `component X inherits Window` -> `window` (id `<app>:X`);
//!   `inherits Dialog` -> `dialog`.
//! - `id := PopupWindow { }`, or `id := X { }` with `X inherits PopupWindow`,
//!   inside a window -> `dialog` (id `<app>:<Window>.<id>`, name `id` or `X`).
//! - Views chosen by state, read only inside a window's body:
//!   `if <state> == <value>: X { }` (also `id := X`, the sides swapped). The
//!   state is a property of the window (`view`, `root.view`), a property of a
//!   `global` (`Nav.page`), or a field of a model the window repeats over
//!   (`for p[i] in root.panes: … if p.view == "a"`, identity `panes.view`).
//!   The value is a string, integer or `Enum.variant` literal of a declared
//!   `enum`. Each branch is a `view` named after `X`, id
//!   `<app>:<state>=<value>`; file and line are `X`'s declaration. A state
//!   counts only with at least two values showing at least two different
//!   components declared in the project: a chip shown for every value, or
//!   built-in elements (`Rectangle`, layouts), are no view switch.
//! - `if <bool state>: X { }` with `X` named `…Dialog` or `inherits Dialog`
//!   -> `dialog`, id `<app>:<state>`.
//!
//! Start screens (`UiFlow.start_screens`): per view state the screen of its
//! default value: the literal after `property <T> view:`, without one Slint's
//! default (`0`, `""`, the first enum variant); for a model field the field's
//! value in the model's literal default (`[{ view: "graph" }]`). Plus the
//! window that Rust code runs: `let ui = X::new()…; ui.run()` or chained.
//!
//! Transitions, in text order per file:
//! - Slint: `root.view = 3;`, `view = Page.home;`, `Nav.page = …;`,
//!   `prompt-open = true;` in callbacks; `id.show()` of a popup of the
//!   enclosing window. Code inside a view's branch, a popup or a component
//!   shown as exactly one screen starts there; other code (the window's
//!   header, shared widgets) at `any_screen` of the application. The trigger
//!   is the literal `accessible-label`, else `text`, else `title` (also in
//!   `@tr("…")`) of the element whose callback holds the code.
//! - Rust: generated setters with exactly one argument,
//!   `ui.set_view(3)`, `ui.set_tab("help".into())`,
//!   `ui.global::<Nav>().set_page(Page::OrderDetails)` (enum variants compare
//!   without case, `-` and `_`), `set_prompt_open(true)`; `X::new()` of a
//!   window or dialog component with `.show()` chained or on its variable.
//!   They start at `any_screen`; no trigger.
//! - A literal `false` closes a dialog: no transition. A value that is no
//!   literal, or a literal without a view, is `Unresolved` with a gap. A model
//!   is always replaced as a whole (`set_panes(rows.into())`): unresolved.
//!
//! Notes name what is seen but not read: a model-field state (only
//! `set_<model>` is read) and a window callback called with a literal that
//! is a view value (`root.navigate("a")`): its Rust handler is not followed.
//!
//! Not recognized: view switches outside a window component (modes inside a
//! view), conditions with `&&`, `||`, `!=` or a ternary, the pre-1.0
//! `X := Base { }` component syntax, values passed through callbacks or
//! component properties (`ViewBarTab { view: "start"; navigate(v) => … }`),
//! Rust code that names the property by another path (`invoke_…`, models
//! changed in place), `slint!` macros inside Rust, and actions (no HTTP).
use crate::project_files::read_repo_file;
use crate::services::support::{Discovery, relative, test_path};
use archguard_core::{
    DiagnosticSeverity, Origin, ServiceTopology, Target, Transition, UiFlow, UiFlowDiagnostic,
};
use model::Ui;
use std::collections::BTreeMap;
use std::path::Path;
use text::Src;

const SOURCE: &str = "slint";
const MAX_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 5_000;
/// Element properties that name a trigger, in order of preference.
const LABELS: [&str; 3] = ["accessible-label", "text", "title"];

/// Screens and transitions of every Slint application below `root`.
pub fn extract(root: &Path) -> UiFlow {
    let mut flow = UiFlow::default();
    let (apps, rust) = files::discover(root, &mut flow);
    let uis: Vec<Ui> = (apps.into_iter())
        .map(|(app, files)| Ui::new(app, files))
        .collect();
    for ui in &uis {
        slint_side::emit(ui, &mut flow);
    }
    for src in &rust {
        rust_side::emit(&uis, src, &mut flow);
    }
    flow.start_screens.sort();
    flow.start_screens.dedup();
    flow.diagnostics.sort();
    flow.diagnostics.dedup();
    flow
}

/// Diagnostic at `file:line`; a gap also marks the flow incomplete.
fn diagnostic(flow: &mut UiFlow, file: &str, line: u32, message: String, gap: bool) {
    flow.complete &= !gap;
    let severity = if gap {
        DiagnosticSeverity::Gap
    } else {
        DiagnosticSeverity::Note
    };
    let file = file.to_string();
    (flow.diagnostics).push(UiFlowDiagnostic {
        file,
        line,
        message,
        severity,
    });
}

/// Transitions of one file, pushed in text order.
fn push(flow: &mut UiFlow, mut found: Vec<(usize, Transition)>) {
    found.sort_by_key(|(at, _)| *at);
    flow.transitions.extend(found.into_iter().map(|(_, t)| t));
}

/// A transition to `target` at `at` in `src`; an unresolved one adds a gap.
fn transition(flow: &mut UiFlow, src: &Src, at: usize, from: Origin, target: Target) -> Transition {
    let line = src.line(at);
    if let Target::Unresolved(text) = &target {
        let message = format!("`{text}` is no literal view value; the target is unknown.");
        diagnostic(flow, &src.rel, line, message, true);
    }
    Transition {
        from,
        target,
        trigger: None,
        file: src.rel.clone(),
        line,
        condition: None,
        source: SOURCE.into(),
    }
}

/// Enum variants compare without case, `-` and `_` (`order-details`,
/// `OrderDetails`).
fn variant(text: &str) -> String {
    (text.chars())
        .filter(|c| *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// A string, integer or boolean literal: its value.
fn plain(text: &str) -> Option<&str> {
    let text = text.trim();
    let int = text.strip_prefix('-').unwrap_or(text);
    if matches!(text, "true" | "false")
        || (!int.is_empty() && int.bytes().all(|b| b.is_ascii_digit()))
    {
        return Some(text);
    }
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('"') && !inner.contains('\\')).then_some(inner)
}

mod files {
    use super::*;

    type Apps = Vec<(String, Vec<Src>)>;

    /// The Slint applications (name, `.slint` files) and the Rust files of
    /// the packages that use Slint.
    pub(super) fn discover(root: &Path, flow: &mut UiFlow) -> (Apps, Vec<Src>) {
        let mut scratch = ServiceTopology::default();
        let discovery = match Discovery::new(root, &mut scratch) {
            Ok(discovery) => discovery,
            Err(error) => {
                let message = format!("Slint files not scanned: {error}");
                diagnostic(flow, "", 0, message, true);
                return (vec![], vec![]);
            }
        };
        let (mut packages, mut paths) = (BTreeMap::new(), Vec::new());
        for entry in discovery.walk(root).flatten() {
            let rel = relative(root, entry.path());
            if !entry.file_type().is_some_and(|t| t.is_file()) || test_path(&rel) {
                continue;
            }
            if rel == "Cargo.toml" || rel.ends_with("/Cargo.toml") {
                let text = read(root, &rel, flow).unwrap_or_default();
                packages.insert(parent(&rel).to_string(), text.contains("slint"));
            } else if rel.ends_with(".slint") || rel.ends_with(".rs") {
                paths.push(rel);
            }
        }
        paths.sort();
        paths.retain(|rel| rel.ends_with(".slint") || owner(rel, &packages).1);
        if paths.len() > MAX_FILES {
            let message =
                format!("More than {MAX_FILES} Slint and Rust files; the rest was not read.");
            diagnostic(flow, &paths[MAX_FILES], 0, message, true);
            paths.truncate(MAX_FILES);
        }
        let (mut apps, mut rust) = (BTreeMap::<String, Vec<Src>>::new(), Vec::new());
        for rel in paths {
            let Some(text) = read(root, &rel, flow) else {
                continue;
            };
            let app = app_name(root, owner(&rel, &packages).0);
            let src = Src::new(rel, &text);
            if src.rel.ends_with(".slint") {
                apps.entry(app).or_default().push(src);
            } else {
                rust.push(src);
            }
        }
        (apps.into_iter().collect(), rust)
    }

    fn parent(rel: &str) -> &str {
        rel.rsplit_once('/').map_or("", |(dir, _)| dir)
    }

    /// The nearest package folder of `rel` and whether it uses Slint; `""`
    /// without a package.
    fn owner<'a>(rel: &str, packages: &'a BTreeMap<String, bool>) -> (&'a str, bool) {
        let mut dir = rel;
        loop {
            dir = parent(dir);
            if let Some((key, slint)) = packages.get_key_value(dir) {
                return (key, *slint);
            }
            if dir.is_empty() {
                return ("", false);
            }
        }
    }

    fn app_name(root: &Path, dir: &str) -> String {
        match dir.rsplit('/').next() {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => (root.file_name()).map_or(String::new(), |n| n.to_string_lossy().into_owned()),
        }
    }

    fn read(root: &Path, rel: &str, flow: &mut UiFlow) -> Option<String> {
        match read_repo_file(root, Path::new(rel), MAX_BYTES) {
            Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            Err(_) => {
                let message = "File not read: larger than 1 MiB, not a regular file or not readable; its screens and navigation are unknown.";
                diagnostic(flow, rel, 0, message.into(), true);
                None
            }
        }
    }
}

mod model {
    use super::text::{after_ident, find_all, ident_before, is_path, offset, type_end, words};
    use super::*;
    use archguard_core::{Screen, ScreenKind};
    use std::collections::BTreeSet;

    /// A `component` or `global` declaration.
    pub(super) struct Comp {
        pub name: String,
        /// `Window`, `Dialog`, `Rectangle`, …; `global` for a global.
        pub base: String,
        pub file: usize,
        pub at: usize,
        /// Offsets of the body's braces.
        pub body: (usize, usize),
    }

    /// A state that chooses screens.
    #[derive(Clone, PartialEq, Eq)]
    pub(super) struct Key {
        /// `view`, `Nav.page` or `panes.view`: the id part.
        pub text: String,
        /// The generated Rust setter is `set_<setter>`.
        pub setter: String,
        /// The window of a window property or model; none for a global.
        pub window: Option<usize>,
        pub model: bool,
        /// Shows a dialog while `true`.
        pub boolean: bool,
    }

    /// One `if <state> …: X { }` inside a window.
    struct Branch {
        key: Key,
        /// Display and match form; none for a boolean state.
        value: Option<(String, String)>,
        comp: String,
        file: usize,
        at: usize,
        block: (usize, usize),
    }

    /// What the rules know about one application.
    pub(super) struct Ui {
        pub app: String,
        pub files: Vec<Src>,
        pub comps: Vec<Comp>,
        enums: BTreeMap<String, Vec<String>>,
        pub screens: Vec<Screen>,
        /// States with screens.
        pub keys: Vec<Key>,
        /// (state, match form of the value) -> screen id; dialogs under `true`.
        pub targets: BTreeMap<(String, String), String>,
        /// (file, braces, screen id): code inside starts at that screen.
        spans: Vec<(usize, (usize, usize), String)>,
        /// (window, popup element id) -> screen id.
        pub popups: BTreeMap<(usize, String), String>,
        pub starts: Vec<String>,
        /// (file, offset, message).
        pub notes: Vec<(usize, usize, String)>,
    }

    impl Ui {
        pub(super) fn new(app: String, files: Vec<Src>) -> Self {
            let mut ui = Ui {
                app,
                files,
                comps: vec![],
                enums: BTreeMap::new(),
                screens: vec![],
                keys: vec![],
                targets: BTreeMap::new(),
                spans: vec![],
                popups: BTreeMap::new(),
                starts: vec![],
                notes: vec![],
            };
            for (file, src) in ui.files.iter().enumerate() {
                let (comps, enums) = declarations(src, file);
                ui.comps.extend(comps);
                ui.enums.extend(enums);
            }
            let tops: Vec<Screen> = (ui.comps.iter())
                .filter_map(|c| {
                    let kind = match c.base.as_str() {
                        "Window" => ScreenKind::Window,
                        "Dialog" => ScreenKind::Dialog,
                        _ => return None,
                    };
                    Some(ui.screen(&c.name, &c.name, kind, c.file, c.at))
                })
                .collect();
            ui.screens.extend(tops);
            let windows: Vec<usize> = (0..ui.comps.len())
                .filter(|&c| ui.comps[c].base == "Window")
                .collect();
            let mut branches = vec![];
            for &w in &windows {
                ui.popups_of(w);
                branches.extend(ui.branches(w));
            }
            ui.views(&branches);
            comp_spans(&mut ui);
            for &w in &windows {
                callback_notes(&mut ui, w);
            }
            ui
        }

        fn screen(&self, id: &str, name: &str, kind: ScreenKind, file: usize, at: usize) -> Screen {
            Screen {
                id: format!("{}:{id}", self.app),
                name: name.into(),
                kind,
                route: None,
                application: Some(self.app.clone()),
                file: self.files[file].rel.clone(),
                line: self.files[file].line(at),
                access: None,
                source: SOURCE.into(),
            }
        }

        fn comp(&self, name: &str) -> Option<&Comp> {
            (self.comps.iter()).find(|c| c.name == name && c.base != "global")
        }

        /// The component of `base` whose body holds `at`.
        fn inside(&self, base: &str, file: usize, at: usize) -> Option<usize> {
            (0..self.comps.len()).find(|&c| {
                let comp = &self.comps[c];
                comp.base == base && comp.file == file && comp.body.0 < at && at < comp.body.1
            })
        }

        pub(super) fn window_at(&self, file: usize, at: usize) -> Option<usize> {
            self.inside("Window", file, at)
        }

        pub(super) fn global_at(&self, file: usize, at: usize) -> Option<&str> {
            (self.inside("global", file, at)).map(|c| self.comps[c].name.as_str())
        }

        /// Where code at `at` starts: the innermost screen block, else any screen.
        pub(super) fn origin(&self, file: usize, at: usize) -> Origin {
            (self.spans.iter())
                .filter(|(f, (o, c), _)| *f == file && *o < at && at < *c)
                .max_by_key(|(_, (o, _), _)| *o)
                .map_or_else(
                    || Origin::AnyScreen {
                        application: self.app.clone(),
                    },
                    |(_, _, id)| Origin::Screen(id.clone()),
                )
        }

        /// A literal value in Slint, enum variants included: display and
        /// match form.
        pub(super) fn value(&self, text: &str) -> Option<(String, String)> {
            let text = text.trim();
            if let Some(v) = plain(text) {
                return Some((v.to_string(), v.to_string()));
            }
            let (e, v) = text.split_once('.')?;
            let variants = self.enums.get(e)?;
            (variants.iter().any(|x| variant(x) == variant(v))).then(|| (v.to_string(), variant(v)))
        }

        /// The state a condition path names inside window `w`.
        fn key(&self, w: usize, path: &str, at: usize) -> Option<Key> {
            let key = |text: String, setter: &str, window, model| Key {
                text,
                setter: setter.replace('-', "_"),
                window,
                model,
                boolean: false,
            };
            match path.split('.').collect::<Vec<_>>().as_slice() {
                [p] | ["root", p] if self.declared(w, p).is_some() => {
                    Some(key(p.to_string(), p, Some(w), false))
                }
                [g, p]
                    if self
                        .comps
                        .iter()
                        .any(|c| c.base == "global" && c.name == *g) =>
                {
                    Some(key(path.to_string(), p, None, false))
                }
                [v, f] => {
                    let model = self.loop_model(w, v, at)?;
                    Some(key(format!("{model}.{f}"), &model, Some(w), true))
                }
                _ => None,
            }
        }

        /// Type and default text of property `name` declared in component `c`.
        fn declared(&self, c: usize, name: &str) -> Option<(String, Option<String>)> {
            let comp = &self.comps[c];
            let body = &self.files[comp.file].text[comp.body.0..comp.body.1];
            words(body, "property").find_map(|at| {
                let rest = body[at + "property".len()..]
                    .trim_start()
                    .strip_prefix('<')?;
                let close = type_end(rest)?;
                let ty = rest[..close].trim().to_string();
                let (found, rest) = after_ident(rest[close + 1..].trim_start());
                if found != name {
                    return None;
                }
                let init = rest
                    .trim_start()
                    .strip_prefix(':')
                    .filter(|r| !r.starts_with('='));
                let init = init.map(|i| i[..i.find(';').unwrap_or(i.len())].trim().to_string());
                Some((ty, init))
            })
        }

        /// The model a `for <var>` around `at` in window `w` repeats over.
        fn loop_model(&self, w: usize, var: &str, at: usize) -> Option<String> {
            let comp = &self.comps[w];
            let src = &self.files[comp.file];
            let start = comp.body.0;
            let found = words(&src.text[start..at], "for").filter_map(|f| {
                let f = start + f;
                let (v, rest) = after_ident(src.text[f + 3..].trim_start());
                let rest = rest.trim_start();
                let rest = match rest.strip_prefix('[') {
                    Some(r) => &r[r.find(']')? + 1..],
                    None => rest,
                };
                let rest = rest.trim_start().strip_prefix("in ")?;
                let model = rest[..rest.find(':')?].trim();
                let model = model.strip_prefix("root.").unwrap_or(model);
                let open = f + src.text[f..].find('{')?;
                let encloses = src.close(open).is_some_and(|close| close > at);
                (v == var && encloses && self.declared(w, model).is_some())
                    .then(|| model.to_string())
            });
            found.last()
        }

        /// `id := PopupWindow { }` and `id := X { }` (`X inherits PopupWindow`)
        /// in window `w`.
        fn popups_of(&mut self, w: usize) {
            let comp = &self.comps[w];
            let src = &self.files[comp.file];
            let mut found = vec![];
            for at in find_all(&src.text[comp.body.0..comp.body.1], ":=").map(|a| a + comp.body.0) {
                let id = ident_before(&src.text, at);
                let (name, rest) = after_ident(src.text[at + 2..].trim_start());
                let popup = name == "PopupWindow"
                    || self.comp(name).is_some_and(|c| c.base == "PopupWindow");
                let open = offset(&src.text, rest.trim_start());
                let Some(close) = src.close(open).filter(|_| popup && !id.is_empty()) else {
                    continue;
                };
                let shown = if name == "PopupWindow" { id } else { name };
                let id_part = format!("{}.{id}", comp.name);
                let screen = self.screen(&id_part, shown, ScreenKind::Dialog, comp.file, at);
                found.push((id.to_string(), screen, (open, close)));
            }
            let file = self.comps[w].file;
            for (id, screen, block) in found {
                self.popups.insert((w, id), screen.id.clone());
                self.spans.push((file, block, screen.id.clone()));
                self.screens.push(screen);
            }
        }

        /// Every `if <state> …: X { }` in window `w` whose state is readable.
        fn branches(&self, w: usize) -> Vec<Branch> {
            let comp = &self.comps[w];
            let src = &self.files[comp.file];
            let body = &src.text[comp.body.0..comp.body.1];
            let mut out = vec![];
            for at in words(body, "if").map(|a| a + comp.body.0) {
                let Some(open) = src.text[at..].find('{').map(|o| at + o) else {
                    continue;
                };
                let Some((cond, name)) = element_if(&src.text[at + 2..open]) else {
                    continue;
                };
                let Some((path, value)) = self.condition(cond) else {
                    continue;
                };
                let (Some(mut key), Some(close)) = (self.key(w, path, at), src.close(open)) else {
                    continue;
                };
                key.boolean = value.is_none();
                let (comp, file, block) = (name.to_string(), comp.file, (open, close));
                out.push(Branch {
                    key,
                    value,
                    comp,
                    file,
                    at,
                    block,
                });
            }
            out
        }

        /// `path == value`, `value == path` or a bare boolean `path`.
        fn condition<'a>(&self, cond: &'a str) -> Option<(&'a str, Option<(String, String)>)> {
            let cond = cond.trim();
            let cond = (cond.strip_prefix('(').and_then(|c| c.strip_suffix(')')))
                .unwrap_or(cond)
                .trim();
            let Some((a, b)) = cond.split_once("==") else {
                return is_path(cond).then_some((cond, None));
            };
            let (a, b) = (a.trim(), b.trim());
            match (self.value(a), self.value(b)) {
                (None, Some(v)) if is_path(a) => Some((a, Some(v))),
                (Some(v), None) if is_path(b) => Some((b, Some(v))),
                _ => None,
            }
        }

        fn views(&mut self, branches: &[Branch]) {
            let mut keys: Vec<&Key> = vec![];
            for b in branches {
                if !keys.contains(&&b.key) {
                    keys.push(&b.key);
                }
            }
            for key in keys {
                let mine: Vec<&Branch> = branches.iter().filter(|b| b.key == *key).collect();
                let shown = if key.boolean {
                    self.dialog(key, &mine)
                } else {
                    self.switch(key, &mine)
                };
                if shown {
                    self.keys.push(key.clone());
                }
            }
        }

        /// The views of one state, when it switches between at least two
        /// project components; false otherwise.
        fn switch(&mut self, key: &Key, mine: &[&Branch]) -> bool {
            let shown: Vec<(&Branch, usize, usize)> = (mine.iter())
                .filter_map(|b| self.comp(&b.comp).map(|c| (*b, c.file, c.at)))
                .collect();
            let values: BTreeSet<&str> = shown
                .iter()
                .filter_map(|(b, ..)| b.value.as_ref())
                .map(|v| v.1.as_str())
                .collect();
            let comps: BTreeSet<&str> = shown.iter().map(|(b, ..)| b.comp.as_str()).collect();
            if values.len() < 2 || comps.len() < 2 {
                return false;
            }
            for (b, file, at) in shown {
                let Some((display, matched)) = b.value.clone() else {
                    continue;
                };
                let target = (key.text.clone(), matched);
                if self.targets.contains_key(&target) {
                    continue;
                }
                let id_part = format!("{}={display}", key.text);
                let screen = self.screen(&id_part, &b.comp, ScreenKind::View, file, at);
                self.targets.insert(target, screen.id.clone());
                self.spans.push((b.file, b.block, screen.id.clone()));
                self.screens.push(screen);
            }
            if key.model {
                let message = format!(
                    "Views of `{}` are chosen by a field of the model `{}`, which the Rust side replaces as a whole; only `set_{}(…)` calls are read, and their argument names no view.",
                    key.text,
                    key.text.split('.').next().unwrap_or_default(),
                    key.setter
                );
                self.notes.push((mine[0].file, mine[0].at, message));
            }
            self.start(key, mine[0]);
            true
        }

        /// The dialog a boolean state shows (`…Dialog` or `inherits Dialog`).
        fn dialog(&mut self, key: &Key, mine: &[&Branch]) -> bool {
            let is_dialog = |b: &&&Branch| {
                b.comp.ends_with("Dialog") || self.comp(&b.comp).is_some_and(|c| c.base == "Dialog")
            };
            let Some(b) = mine.iter().find(is_dialog) else {
                return false;
            };
            let (file, at) = self
                .comp(&b.comp)
                .map_or((b.file, b.at), |c| (c.file, c.at));
            let screen = self.screen(&key.text, &b.comp, ScreenKind::Dialog, file, at);
            (self.targets).insert((key.text.clone(), "true".into()), screen.id.clone());
            self.spans.push((b.file, b.block, screen.id.clone()));
            self.screens.push(screen);
            true
        }

        /// The start screen of a view state: the view of its default value.
        fn start(&mut self, key: &Key, first: &Branch) {
            match self.default(key) {
                Some(Some((_, matched))) => {
                    if let Some(id) = self.targets.get(&(key.text.clone(), matched)) {
                        self.starts.push(id.clone());
                    }
                }
                Some(None) => {
                    let message = format!(
                        "The default of `{}` is no literal value; its start view is unknown.",
                        key.text
                    );
                    self.notes.push((first.file, first.at, message));
                }
                None => {}
            }
        }

        /// The default value of a state: `Some(None)` when it is no literal,
        /// `None` without a declaration or for an empty model.
        fn default(&self, key: &Key) -> Option<Option<(String, String)>> {
            let (owner, property) = match (key.window, key.text.split_once('.')) {
                (Some(w), Some((model, _))) if key.model => (w, model),
                (Some(w), _) => (w, key.text.as_str()),
                (None, Some((g, p))) => {
                    let g = (0..self.comps.len())
                        .find(|&c| self.comps[c].base == "global" && self.comps[c].name == g)?;
                    (g, p)
                }
                (None, None) => return None,
            };
            let (ty, init) = self.declared(owner, property)?;
            if key.model {
                let field = key.text.split_once('.')?.1;
                return Some(model_field(&init?, field).and_then(|v| self.value(v)));
            }
            let Some(init) = init else {
                return Some(type_default(&self.enums, &ty));
            };
            Some(self.value(&init))
        }
    }

    /// Slint's default of a type without an initializer.
    fn type_default(enums: &BTreeMap<String, Vec<String>>, ty: &str) -> Option<(String, String)> {
        let value = match ty {
            "int" | "float" => "0",
            "string" => "",
            "bool" => "false",
            _ => {
                let first = enums.get(ty)?.first()?;
                return Some((first.clone(), variant(first)));
            }
        };
        Some((value.into(), value.into()))
    }

    /// Components shown as exactly one screen: their code starts there.
    fn comp_spans(ui: &mut Ui) {
        let mut spans = vec![];
        for comp in ui
            .comps
            .iter()
            .filter(|c| c.base != "Window" && c.base != "global")
        {
            let ids: Vec<&Screen> = ui.screens.iter().filter(|s| s.name == comp.name).collect();
            if let [screen] = ids.as_slice() {
                spans.push((comp.file, comp.body, screen.id.clone()));
            }
        }
        ui.spans.extend(spans);
    }

    /// Notes for `root.<callback>(<literal>)` in window `w` whose literal
    /// is a value of one of its view states: the Rust handler is unread.
    fn callback_notes(ui: &mut Ui, w: usize) {
        let comp = &ui.comps[w];
        let src = &ui.files[comp.file];
        let body = &src.text[comp.body.0..comp.body.1];
        let mine: BTreeSet<&str> = (ui.keys.iter())
            .filter(|k| k.window == Some(w) && !k.boolean)
            .map(|k| k.text.as_str())
            .collect();
        let values: BTreeSet<&str> = (ui.targets.keys())
            .filter(|(k, _)| mine.contains(k.as_str()))
            .map(|(_, v)| v.as_str())
            .collect();
        let mut notes = vec![];
        for at in words(body, "callback") {
            let (name, _) = after_ident(body[at + "callback".len()..].trim_start());
            let call = format!("root.{name}(");
            for c in find_all(body, &call) {
                let arg = &body[c + call.len()..];
                let arg = arg[..arg.find(')').unwrap_or(0)].trim();
                if ui
                    .value(arg)
                    .is_some_and(|(_, m)| values.contains(m.as_str()))
                {
                    let message = format!(
                        "The callback `{name}` passes the view value {arg} to Rust; its handler is not followed, so this is no transition."
                    );
                    notes.push((comp.file, comp.body.0 + c, message));
                }
            }
        }
        ui.notes.extend(notes);
    }

    /// Components, globals and enums declared in one file.
    fn declarations(src: &Src, file: usize) -> (Vec<Comp>, Vec<(String, Vec<String>)>) {
        let text = &src.text;
        let mut comps = vec![];
        for word in ["component", "global"] {
            for at in words(text, word) {
                let (name, rest) = after_ident(text[at + word.len()..].trim_start());
                let rest = rest.trim_start();
                let (base, rest) = match rest.strip_prefix("inherits") {
                    Some(r) => after_ident(r.trim_start()),
                    None => ("", rest),
                };
                let open = offset(text, rest.trim_start());
                let Some(close) = src.close(open).filter(|_| !name.is_empty()) else {
                    continue;
                };
                let base = if word == "global" { word } else { base };
                let (name, base, body) = (name.to_string(), base.to_string(), (open, close));
                comps.push(Comp {
                    name,
                    base,
                    file,
                    at,
                    body,
                });
            }
        }
        let enums = words(text, "enum").filter_map(|at| {
            let (name, rest) = after_ident(text[at + 4..].trim_start());
            let body = rest.trim_start().strip_prefix('{')?;
            let body = &body[..body.find('}')?];
            let variants = body.split(',').map(str::trim).filter(|v| !v.is_empty());
            Some((name.to_string(), variants.map(str::to_string).collect()))
        });
        (comps, enums.collect())
    }

    /// `cond: X` or `cond: id := X` of an element `if`: condition and `X`.
    fn element_if(header: &str) -> Option<(&str, &str)> {
        let header = header.trim_end();
        let rest = header.trim_end_matches(text::is_ident);
        let name = &header[rest.len()..];
        let rest = rest.trim_end();
        let rest = match rest.strip_suffix(":=") {
            Some(r) => r.trim_end().trim_end_matches(text::is_ident).trim_end(),
            None => rest,
        };
        let cond = rest.strip_suffix(':')?;
        (!name.is_empty()).then_some((cond, name))
    }

    /// The literal text of `field: …` in a model literal `[{ field: "x" }]`.
    fn model_field<'a>(init: &'a str, field: &str) -> Option<&'a str> {
        words(init, field).find_map(|at| {
            let rest = init[at + field.len()..].trim_start().strip_prefix(':')?;
            Some(rest[..rest.find([',', '}']).unwrap_or(rest.len())].trim())
        })
    }
}

mod slint_side {
    use super::model::Key;
    use super::text::{find_all, ident_before, path_before, string_at, words};
    use super::*;

    /// Screens, start screens, notes and Slint-side transitions of one app.
    pub(super) fn emit(ui: &Ui, flow: &mut UiFlow) {
        flow.screens.extend(ui.screens.iter().cloned());
        flow.start_screens.extend(ui.starts.iter().cloned());
        for (file, at, message) in &ui.notes {
            let src = &ui.files[*file];
            diagnostic(flow, &src.rel, src.line(*at), message.clone(), false);
        }
        for (file, src) in ui.files.iter().enumerate() {
            let mut found = vec![];
            for at in assignments(&src.text) {
                if let Some(t) = assignment(ui, flow, file, at) {
                    found.push((at, t));
                }
            }
            for at in find_all(&src.text, ".show()") {
                let id = ident_before(&src.text, at);
                let Some(w) = ui.window_at(file, at) else {
                    continue;
                };
                if let Some(target) = ui.popups.get(&(w, id.to_string())) {
                    let target = Target::Screen(target.clone());
                    let mut t = transition(flow, src, at, ui.origin(file, at), target);
                    t.trigger = label(src, at);
                    found.push((at, t));
                }
            }
            push(flow, found);
        }
    }

    /// Offsets of plain `=`: not `==`, `!=`, `<=`, `>=`, `=>`, `:=`, `+=`, ….
    fn assignments(text: &str) -> Vec<usize> {
        let b = text.as_bytes();
        (0..b.len())
            .filter(|&i| {
                b[i] == b'='
                    && !matches!(b.get(i + 1), Some(b'=' | b'>'))
                    && !(i > 0 && b"=!<>+-*/:".contains(&b[i - 1]))
            })
            .collect()
    }

    /// `state = value;` as a transition, when `state` is a view or dialog state.
    fn assignment(ui: &Ui, flow: &mut UiFlow, file: usize, at: usize) -> Option<Transition> {
        let src = &ui.files[file];
        let key = assigned(ui, file, at, path_before(&src.text, at))?;
        let rest = &src.text[at + 1..];
        let expr = rest[..rest.find([';', '}']).unwrap_or(rest.len())].trim();
        let target = match ui.value(expr) {
            Some((_, m)) if key.boolean && m == "false" => return None,
            Some((_, m)) => (ui.targets.get(&(key.text.clone(), m))).map_or_else(
                || Target::Unresolved(expr.into()),
                |id| Target::Screen(id.clone()),
            ),
            None => Target::Unresolved(expr.into()),
        };
        let mut t = transition(flow, src, at, ui.origin(file, at), target);
        t.trigger = label(src, at);
        Some(t)
    }

    /// The state an assigned path names: `view`/`root.view` inside its
    /// window, `Nav.page` anywhere, `page` inside `global Nav`.
    fn assigned<'a>(ui: &'a Ui, file: usize, at: usize, path: &str) -> Option<&'a Key> {
        let segments: Vec<&str> = path.split('.').collect();
        let (window, global) = (ui.window_at(file, at), ui.global_at(file, at));
        ui.keys.iter().find(|k| {
            !k.model
                && match (segments.as_slice(), k.window) {
                    ([p] | ["root", p], Some(w)) => window == Some(w) && *p == k.text,
                    ([p], None) => global.is_some_and(|g| k.text == format!("{g}.{p}")),
                    (_, None) => k.text == path,
                    _ => false,
                }
        })
    }

    /// The literal `accessible-label`, `text` or `title` of the element whose
    /// callback holds `at`.
    pub(super) fn label(src: &Src, at: usize) -> Option<String> {
        let mut handler = src.around(at)?;
        while !src.text[..handler.0].trim_end().ends_with("=>") {
            handler = src.around(handler.0)?;
        }
        let element = src.around(handler.0)?;
        let body = &src.text[element.0..element.1];
        LABELS.iter().find_map(|name| {
            words(body, name)
                .map(|a| a + element.0)
                .filter(|&a| src.around(a) == Some(element))
                .find_map(|a| {
                    let rest = src.text[a + name.len()..].trim_start().strip_prefix(':')?;
                    string_at(rest.trim_start()).map(str::to_string)
                })
        })
    }
}

mod rust_side {
    use super::model::Key;
    use super::text::{after_ident, find_all, literal_end, words};
    use super::*;

    /// Transitions and run windows in one Rust file.
    pub(super) fn emit(uis: &[Ui], src: &Src, flow: &mut UiFlow) {
        let mut found = vec![];
        for at in find_all(&src.text, ".set_") {
            let (name, rest) = after_ident(&src.text[at + ".set_".len()..]);
            let Some(arg) = rest.strip_prefix('(').and_then(single_argument) else {
                continue;
            };
            for ui in uis {
                for key in ui.keys.iter().filter(|k| k.setter == name) {
                    if let Some(t) = setter(ui, key, src, at, arg, flow) {
                        found.push((at, t));
                    }
                }
            }
        }
        for ui in uis {
            windows(ui, src, flow, &mut found);
        }
        push(flow, found);
    }

    fn setter(
        ui: &Ui,
        key: &Key,
        src: &Src,
        at: usize,
        arg: &str,
        flow: &mut UiFlow,
    ) -> Option<Transition> {
        let unresolved = || Target::Unresolved(arg.into());
        let target = match rust_value(arg) {
            _ if key.model => unresolved(),
            Some(v) if key.boolean && v == "false" => return None,
            Some(v) => (ui.targets.get(&(key.text.clone(), v)))
                .map_or_else(unresolved, |id| Target::Screen(id.clone())),
            None => unresolved(),
        };
        let from = Origin::AnyScreen {
            application: ui.app.clone(),
        };
        Some(transition(flow, src, at, from, target))
    }

    /// The match form of a Rust literal: `"x"`, `"x".into()`,
    /// `SharedString::from("x")`, `3`, `true`, `Page::OrderDetails`.
    fn rust_value(arg: &str) -> Option<String> {
        let arg = arg.trim().trim_start_matches('&');
        let arg = arg.strip_suffix(".into()").unwrap_or(arg);
        let arg = (arg
            .strip_prefix("SharedString::from(")
            .and_then(|a| a.strip_suffix(')')))
        .unwrap_or(arg)
        .trim();
        if let Some(v) = plain(arg) {
            return Some(v.to_string());
        }
        let (_, v) = arg.rsplit_once("::")?;
        (!v.is_empty() && v.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| variant(v))
    }

    /// The text of a call's only argument, after its `(`.
    fn single_argument(text: &str) -> Option<&str> {
        let (b, mut depth, mut i, mut commas) = (text.as_bytes(), 0usize, 0, 0);
        while i < b.len() {
            if let Some(end) = literal_end(text, i) {
                i = end;
                continue;
            }
            match b[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' if depth == 0 => {
                    let arg = text[..i].trim();
                    return (commas == 0 && !arg.is_empty()).then_some(arg);
                }
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                b',' if depth == 0 => commas += 1,
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// `X::new()` of a window or dialog component: `.run()` marks a start
    /// screen, `.show()` is a transition.
    fn windows(ui: &Ui, src: &Src, flow: &mut UiFlow, found: &mut Vec<(usize, Transition)>) {
        let top = ui
            .comps
            .iter()
            .filter(|c| matches!(c.base.as_str(), "Window" | "Dialog"));
        for comp in top {
            let news = words(&src.text, &comp.name)
                .filter(|&a| src.text[a + comp.name.len()..].starts_with("::new("));
            for at in news.collect::<Vec<_>>() {
                let id = format!("{}:{}", ui.app, comp.name);
                match opened(&src.text, at) {
                    Some("run") => flow.start_screens.push(id),
                    Some(_) => {
                        let from = Origin::AnyScreen {
                            application: ui.app.clone(),
                        };
                        found.push((at, transition(flow, src, at, from, Target::Screen(id))));
                    }
                    None => {}
                }
            }
        }
    }

    /// `run` or `show`, chained on the `X::new()` statement or called on the
    /// variable it was assigned to.
    fn opened(text: &str, at: usize) -> Option<&'static str> {
        let start = text[..at].rfind([';', '{', '}']).map_or(0, |s| s + 1);
        let end = text[at..].find(';').map_or(text.len(), |e| at + e);
        let statement = &text[start..end];
        let verbs = ["run", "show"];
        if let Some(verb) = verbs
            .into_iter()
            .find(|v| statement.contains(&format!(".{v}()")))
        {
            return Some(verb);
        }
        let var = statement.trim_start().strip_prefix("let ")?.trim_start();
        let (var, _) = after_ident(var.strip_prefix("mut ").unwrap_or(var).trim_start());
        let rest = &text[end..];
        verbs.into_iter().find(|v| {
            let call = format!(".{v}()");
            words(rest, var).any(|a| rest[a + var.len()..].starts_with(&call))
        })
    }
}

mod text {
    /// A file with comments blanked (line breaks kept) and its brace pairs.
    pub(super) struct Src {
        pub rel: String,
        pub text: String,
        /// `{` and `}` offsets outside literals, sorted by `{`.
        pairs: Vec<(usize, usize)>,
    }

    impl Src {
        pub(super) fn new(rel: String, raw: &str) -> Self {
            let text = blank(raw);
            let pairs = pairs(&text);
            Src { rel, text, pairs }
        }

        pub(super) fn line(&self, at: usize) -> u32 {
            self.text[..at.min(self.text.len())].matches('\n').count() as u32 + 1
        }

        /// The `}` of the `{` at `open`.
        pub(super) fn close(&self, open: usize) -> Option<usize> {
            let i = self.pairs.binary_search_by_key(&open, |p| p.0).ok()?;
            Some(self.pairs[i].1)
        }

        /// The innermost brace pair strictly around `at`.
        pub(super) fn around(&self, at: usize) -> Option<(usize, usize)> {
            (self.pairs.iter())
                .filter(|(o, c)| *o < at && at < *c)
                .max_by_key(|(o, _)| *o)
                .copied()
        }
    }

    /// `//` and `/* */` comments replaced by spaces, keeping line breaks;
    /// literals stay.
    fn blank(text: &str) -> String {
        let b = text.as_bytes();
        let (mut out, mut i, mut kept) = (String::with_capacity(text.len()), 0, 0);
        while i < b.len() {
            if let Some(end) = literal_end(text, i) {
                i = end;
                continue;
            }
            let end = match (b[i], b.get(i + 1)) {
                (b'/', Some(b'/')) => text[i..].find('\n').map_or(b.len(), |e| i + e),
                (b'/', Some(b'*')) => text[i + 2..].find("*/").map_or(b.len(), |e| i + e + 4),
                _ => {
                    i += 1;
                    continue;
                }
            };
            out.push_str(&text[kept..i]);
            out.extend(
                text[i..end]
                    .chars()
                    .map(|c| if c == '\n' { '\n' } else { ' ' }),
            );
            (i, kept) = (end, end);
        }
        out.push_str(&text[kept..]);
        out
    }

    fn pairs(text: &str) -> Vec<(usize, usize)> {
        let (mut stack, mut out, mut i) = (vec![], vec![], 0);
        while i < text.len() {
            if let Some(end) = literal_end(text, i) {
                i = end;
                continue;
            }
            match text.as_bytes()[i] {
                b'{' => stack.push(i),
                b'}' => out.extend(stack.pop().map(|o| (o, i))),
                _ => {}
            }
            i += 1;
        }
        out.sort();
        out
    }

    /// The end of the string, raw string or char literal starting at `i`.
    pub(super) fn literal_end(text: &str, i: usize) -> Option<usize> {
        let b = text.as_bytes();
        match b[i] {
            b'"' => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                Some((j + 1).min(b.len()))
            }
            b'\'' => {
                let c = text[i + 1..].chars().next()?;
                let close = if c == '\\' {
                    i + 3 + text.get(i + 3..)?.find('\'')?
                } else {
                    i + 1 + c.len_utf8()
                };
                (b.get(close) == Some(&b'\'')).then_some(close + 1)
            }
            b'r' if i == 0 || !is_ident(char::from(b[i - 1])) => {
                let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
                if b.get(i + 1 + hashes) != Some(&b'"') {
                    return None;
                }
                let close = format!("\"{}", "#".repeat(hashes));
                let from = i + 2 + hashes;
                Some(
                    text[from..]
                        .find(&close)
                        .map_or(b.len(), |e| from + e + close.len()),
                )
            }
            _ => None,
        }
    }

    pub(super) fn is_ident(c: char) -> bool {
        c.is_alphanumeric() || c == '_' || c == '-'
    }

    /// Offsets of `word` with identifier boundaries, not after a `.`.
    pub(super) fn words<'a>(text: &'a str, word: &'a str) -> impl Iterator<Item = usize> + 'a {
        text.match_indices(word)
            .map(|(at, _)| at)
            .filter(move |&at| {
                let before = text[..at].chars().next_back();
                let after = text[at + word.len()..].chars().next();
                !before.is_some_and(|c| is_ident(c) || c == '.') && !after.is_some_and(is_ident)
            })
    }

    pub(super) fn find_all<'a>(
        text: &'a str,
        pattern: &'a str,
    ) -> impl Iterator<Item = usize> + 'a {
        text.match_indices(pattern).map(|(at, _)| at)
    }

    /// The identifier at the start of `text` and the rest.
    pub(super) fn after_ident(text: &str) -> (&str, &str) {
        let end = text.find(|c: char| !is_ident(c)).unwrap_or(text.len());
        text.split_at(end)
    }

    /// The identifier ending before `at`, whitespace skipped.
    pub(super) fn ident_before(text: &str, at: usize) -> &str {
        let head = text[..at].trim_end();
        &head[head.trim_end_matches(is_ident).len()..]
    }

    /// The dotted path ending before `at`, whitespace skipped.
    pub(super) fn path_before(text: &str, at: usize) -> &str {
        let head = text[..at].trim_end();
        &head[head.trim_end_matches(|c| is_ident(c) || c == '.').len()..]
    }

    pub(super) fn is_path(text: &str) -> bool {
        text.split('.').all(|s| {
            s.starts_with(|c: char| c.is_alphabetic() || c == '_') && s.chars().all(is_ident)
        })
    }

    /// The offset of `sub`, a slice of `text`.
    pub(super) fn offset(text: &str, sub: &str) -> usize {
        sub.as_ptr() as usize - text.as_ptr() as usize
    }

    /// The `>` closing a type that starts after `<`.
    pub(super) fn type_end(text: &str) -> Option<usize> {
        let mut depth = 0usize;
        for (i, b) in text.bytes().enumerate() {
            match b {
                b'<' => depth += 1,
                b'>' if depth == 0 => return Some(i),
                b'>' => depth -= 1,
                _ => {}
            }
        }
        None
    }

    /// The content of a string literal at the start of `text`, also inside
    /// `@tr(…)`.
    pub(super) fn string_at(text: &str) -> Option<&str> {
        let text = text.strip_prefix("@tr(").map_or(text, str::trim_start);
        let rest = text.strip_prefix('"')?;
        Some(&rest[..rest.find('"')?])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{DiagnosticSeverity, Origin, Screen, ScreenKind, Target};

    const CARGO: &str = "[package]\nname = \"app\"\n[dependencies]\nslint = \"1.18\"\n";

    fn extract_files(files: &[(&str, &str)]) -> UiFlow {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        extract(dir.path())
    }

    fn screen<'a>(flow: &'a UiFlow, id: &str) -> &'a Screen {
        let found = flow.screens.iter().find(|s| s.id == id);
        found.unwrap_or_else(|| panic!("no screen {id}: {:#?}", flow.screens))
    }

    fn to(id: &str) -> Target {
        Target::Screen(id.into())
    }

    /// (from, target, trigger, file, line) of every transition, in order.
    fn moves(flow: &UiFlow) -> Vec<(Origin, Target, Option<&str>, &str, u32)> {
        (flow.transitions.iter())
            .map(|t| {
                let trigger = t.trigger.as_deref();
                (
                    t.from.clone(),
                    t.target.clone(),
                    trigger,
                    t.file.as_str(),
                    t.line,
                )
            })
            .collect()
    }

    const INT_WINDOW: &str = r#"import { Button } from "std-widgets.slint";
component HomeView inherits Rectangle { }
component OrdersView inherits Rectangle { }
// A chip is no view: the same component for every value.
component Chip inherits Rectangle { }
component Modes inherits Rectangle {
    in property <int> mode;
    if mode == 0: HomeView { }
    if mode == 1: OrdersView { }
}
export component Main inherits Window {
    in-out property <int> view: 1;
    in property <string> focus;
    HorizontalLayout {
        Button { text: "Home"; clicked => { root.view = 0; } }
        Button { accessible-label: "show-orders"; text: @tr("Orders"); clicked => { view = 1; } }
        Button { text: "Next"; clicked => { root.view = root.view + 1; } }
        Button { text: "About"; clicked => { about.show(); } }
    }
    if focus == "a": Chip { }
    if focus == "b": Chip { }
    if view == 0: HomeView { }
    if root.view == 1: OrdersView {
        Button { text: "Back"; clicked => { root.view = 0; } }
    }
    about := PopupWindow { Text { text: "ArchGuard"; } }
}
"#;

    #[test]
    fn window_views_by_int_property_popup_and_slint_side_switches() {
        let flow = extract_files(&[("app/Cargo.toml", CARGO), ("app/ui/main.slint", INT_WINDOW)]);
        let file = "app/ui/main.slint";
        let window = screen(&flow, "app:Main");
        assert_eq!((window.kind, window.line), (ScreenKind::Window, 11));
        let home = screen(&flow, "app:view=0");
        assert_eq!(
            (home.name.as_str(), home.kind),
            ("HomeView", ScreenKind::View)
        );
        assert_eq!(
            (home.file.as_str(), home.line, home.route.clone()),
            (file, 2, None)
        );
        assert_eq!(screen(&flow, "app:view=1").name, "OrdersView");
        let about = screen(&flow, "app:Main.about");
        assert_eq!(
            (about.name.as_str(), about.kind),
            ("about", ScreenKind::Dialog)
        );
        // Neither the chip (one component for all values) nor the modes
        // outside a window are views.
        assert_eq!(flow.screens.len(), 4, "{:#?}", flow.screens);
        assert!(
            flow.screens
                .iter()
                .all(|s| s.application.as_deref() == Some("app"))
        );
        assert!(flow.screens.iter().all(|s| s.source == "slint"));
        let any = Origin::AnyScreen {
            application: "app".into(),
        };
        let next = Target::Unresolved("root.view + 1".into());
        assert_eq!(
            moves(&flow),
            [
                (any.clone(), to("app:view=0"), Some("Home"), file, 15),
                (any.clone(), to("app:view=1"), Some("show-orders"), file, 16),
                (any.clone(), next, Some("Next"), file, 17),
                (any, to("app:Main.about"), Some("About"), file, 18),
                (
                    Origin::Screen("app:view=1".into()),
                    to("app:view=0"),
                    Some("Back"),
                    file,
                    24
                ),
            ]
        );
        // Default value 1: the orders view starts.
        assert_eq!(flow.start_screens, ["app:view=1"]);
        assert!(!flow.complete);
        let gaps: Vec<(u32, DiagnosticSeverity)> = flow
            .diagnostics
            .iter()
            .map(|d| (d.line, d.severity))
            .collect();
        assert_eq!(
            gaps,
            [(17, DiagnosticSeverity::Gap)],
            "{:#?}",
            flow.diagnostics
        );
    }

    const ENUM_WINDOW: &str = r#"export enum Page { home, order-details }
export global Nav {
    in-out property <Page> page;
    in-out property <bool> confirm-open;
}
component HomePage inherits Rectangle { }
component DetailsPage inherits Rectangle {
    TouchArea { accessible-label: "back"; clicked => { Nav.page = Page.home; } }
}
component SettingsView inherits Rectangle { }
component HelpView inherits Rectangle { }
export component ConfirmDialog inherits Rectangle { }
export component Shell inherits Window {
    in property <string> tab: "settings";
    if Nav.page == Page.home: HomePage { }
    if Nav.page == Page.order-details: DetailsPage { }
    if tab == "settings": SettingsView { }
    if tab == "help": HelpView { }
    if Nav.confirm-open: ConfirmDialog { }
}
export component Extra inherits Dialog { }
"#;

    const MAIN_RS: &str = r#"slint::include_modules!();
fn main() {
    let ui = Shell::new().unwrap();
    ui.global::<Nav>().set_page(Page::OrderDetails);
    ui.set_tab("help".into());
    ui.set_tab(name);
    ui.global::<Nav>().set_confirm_open(true);
    ui.global::<Nav>().set_confirm_open(false);
    tree.set_tab(&a, &b);
    Extra::new().unwrap().show().unwrap();
    ui.run().unwrap();
}
"#;

    #[test]
    fn enum_global_string_property_dialogs_and_rust_side_switches() {
        let flow = extract_files(&[
            ("app/Cargo.toml", CARGO),
            ("app/ui/shell.slint", ENUM_WINDOW),
            ("app/src/main.rs", MAIN_RS),
            // Rust code of a package without slint is not read.
            ("other/Cargo.toml", "[package]\nname = \"other\"\n"),
            (
                "other/src/lib.rs",
                "fn f() { ui.set_tab(\"help\".into()); }\n",
            ),
        ]);
        let kind = |id: &str| screen(&flow, id).kind;
        assert_eq!(kind("app:Shell"), ScreenKind::Window);
        assert_eq!(kind("app:Extra"), ScreenKind::Dialog);
        assert_eq!(kind("app:Nav.confirm-open"), ScreenKind::Dialog);
        assert_eq!(screen(&flow, "app:Nav.confirm-open").name, "ConfirmDialog");
        assert_eq!(
            screen(&flow, "app:Nav.page=order-details").name,
            "DetailsPage"
        );
        assert_eq!(screen(&flow, "app:tab=help").kind, ScreenKind::View);
        assert_eq!(flow.screens.len(), 7, "{:#?}", flow.screens);
        let rs = "app/src/main.rs";
        let any = Origin::AnyScreen {
            application: "app".into(),
        };
        assert_eq!(
            moves(&flow),
            [
                (
                    Origin::Screen("app:Nav.page=order-details".into()),
                    to("app:Nav.page=home"),
                    Some("back"),
                    "app/ui/shell.slint",
                    8
                ),
                (any.clone(), to("app:Nav.page=order-details"), None, rs, 4),
                (any.clone(), to("app:tab=help"), None, rs, 5),
                (any.clone(), Target::Unresolved("name".into()), None, rs, 6),
                (any.clone(), to("app:Nav.confirm-open"), None, rs, 7),
                (any, to("app:Extra"), None, rs, 10),
            ]
        );
        // The run window, the first enum variant and the string default.
        assert_eq!(
            flow.start_screens,
            ["app:Nav.page=home", "app:Shell", "app:tab=settings"]
        );
        assert!(!flow.complete);
        assert_eq!(flow.diagnostics.len(), 1, "{:#?}", flow.diagnostics);
        assert_eq!(
            (flow.diagnostics[0].file.as_str(), flow.diagnostics[0].line),
            (rs, 6)
        );
    }

    const MODEL_WINDOW: &str = r#"export struct Pane { view: string, id: int }
component AView inherits Rectangle { }
component BView inherits Rectangle { }
export component Win inherits Window {
    in property <[Pane]> panes: [{ id: 1, view: "b" }];
    callback navigate(string);
    Button { text: "A"; clicked => { root.navigate("a"); } }
    for p[i] in root.panes: VerticalLayout {
        if p.view == "a": AView { }
        if p.view == "b": BView { }
    }
}
"#;

    #[test]
    fn model_field_views_start_at_the_model_default_and_name_what_is_not_read() {
        let flow = extract_files(&[
            ("app/Cargo.toml", CARGO),
            ("app/ui/win.slint", MODEL_WINDOW),
            (
                "app/src/panes.rs",
                "fn f(ui: &Win) {\n    ui.set_panes(rows.into());\n}\n",
            ),
        ]);
        assert_eq!(screen(&flow, "app:panes.view=a").name, "AView");
        assert_eq!(flow.start_screens, ["app:panes.view=b"]);
        let any = Origin::AnyScreen {
            application: "app".into(),
        };
        let rows = Target::Unresolved("rows.into()".into());
        assert_eq!(moves(&flow), [(any, rows, None, "app/src/panes.rs", 2)]);
        let found: Vec<(&str, u32, DiagnosticSeverity)> = (flow.diagnostics.iter())
            .map(|d| (d.file.as_str(), d.line, d.severity))
            .collect();
        let slint = "app/ui/win.slint";
        assert_eq!(
            found,
            [
                ("app/src/panes.rs", 2, DiagnosticSeverity::Gap),
                (slint, 7, DiagnosticSeverity::Note),
                (slint, 9, DiagnosticSeverity::Note),
            ],
            "{:#?}",
            flow.diagnostics
        );
        assert!(flow.diagnostics[1].message.contains("navigate"));
        assert!(flow.diagnostics[2].message.contains("set_panes"));
    }

    #[test]
    fn no_slint_no_flow() {
        let flow = extract_files(&[("app/Cargo.toml", CARGO), ("app/src/main.rs", MAIN_RS)]);
        assert_eq!(flow, UiFlow::default());
    }
}
