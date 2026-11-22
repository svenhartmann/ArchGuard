//! Django and Django REST framework extractor for ui-flow/v1 (ADR 0021): a
//! text-level reader of URL configurations, views and templates (no Python
//! interpreter), on [`crate::lang_python`].
//!
//! A project is a folder with `manage.py`; its name is the `application` of
//! every screen and endpoint. Routing starts at each literal `ROOT_URLCONF`
//! of the project's `settings.py` (or `settings/*.py`).
//!
//! Recognized, see the modules for details:
//! - [`urls`]: `urlpatterns` (assigned, `+=`, `.append`, `.extend`, lists of
//!   the same module, also inside `if` blocks) with `path` (`<int:pk>` becomes
//!   `:pk`), `re_path`/`url` with simple regexes (`(?P<pk>…)` becomes `:pk`;
//!   anything else is a gap), `include('module')` with the prefix and the
//!   namespace (`namespace=`, `include((x, 'ns'))` or the module's
//!   `app_name`), `include` of lists and `router.urls`, and DRF routers
//!   (`router.register(prefix, ViewSet, basename=…)`).
//! - [`views`]: a function or class view that renders a template
//!   (`render`, `TemplateResponse`, `render_to_response` with a literal or
//!   `self.<attribute>`) or derives from a Django generic display view
//!   (`TemplateView`, `ListView`, `DetailView`, `CreateView`, `UpdateView`,
//!   `DeleteView`, `FormView`, with `template_name` or the default
//!   `<app>/<model>_list.html` etc.) is a `page` screen with the route; every
//!   other view is an endpoint (methods from `require_*` decorators, the
//!   handler methods of the class, else `ANY`). DRF `APIView`s, generic API
//!   views, `@api_view` functions and ViewSets (standard actions by the
//!   viewset/mixin bases and the methods defined, `@action` with `detail`,
//!   `methods`, `url_path`, `permission_classes`) are endpoints with handler
//!   `module.View.method` or `module.function`.
//! - [`access`]: `@login_required`, `LoginRequiredMixin` = signed in;
//!   `@permission_required`, `PermissionRequiredMixin.permission_required` =
//!   `policy(perm)` (several: all); `@user_passes_test(f)` and
//!   `UserPassesTestMixin` = `custom(f)` / `custom(View.test_func)`;
//!   `staff_member_required` and DRF `IsAdminUser` = `custom(is_staff)`; DRF
//!   `permission_classes` (`AllowAny` open, `IsAuthenticated` signed in,
//!   `IsAuthenticatedOrReadOnly` open for GET/HEAD/OPTIONS, `|`, `&`, `~`),
//!   else `DEFAULT_PERMISSION_CLASSES` of `REST_FRAMEWORK`; an empty list is
//!   open. Any decorator, mixin or permission class named in `[views.access]`
//!   takes that meaning (decorators pass their literal arguments to `$arg`;
//!   a mixin's test is listed as `"View.test_func"`);
//!   an unlisted DRF permission class is `custom(name)`; a project's own
//!   `get_permissions` is `unknown`. Wrappers in `urls.py`
//!   (`login_required(view)`) count. Nothing found is `undeclared`.
//! - [`templates`]: links of a screen's templates and the templates they
//!   `{% include %}` start at the screen; links of the templates they
//!   `{% extends %}` (and their includes) start at every screen of the
//!   application. `{% url 'ns:name' … %}` resolves by URL name; a literal
//!   `<a href="/x/">` by route. A link to a view that is no screen is a `GET`
//!   action on that endpoint. `redirect(…)`, `reverse(…)`, `reverse_lazy(…)`
//!   and a literal `HttpResponseRedirect('/x/')` in a screen view (its body, or
//!   the bodies of its classes) are transitions from that screen; a
//!   `redirect` to an endpoint is a `GET` action, a `reverse` to an endpoint
//!   only builds a URL and adds nothing.
//!
//! Not recognized (gap or note, never guessed): routes from calls (route
//! registries such as NetBox's `get_model_urls`), a non-literal route prefix
//! (gap; the routes below are read relative to it), modules outside the scan
//! (`include('allauth.urls')`, `admin.site.urls`: notes), view classes of
//! installed packages other than the Django and DRF generics, non-literal
//! template names, URL names and redirect targets, `{% url %}` with a
//! variable name, relative `href`s, router subclasses that change routes,
//! form posts. The method resolution order is approximated depth first, left
//! to right.
use crate::extract_ctx::{
    ExtractContext, dir_name, finish, gap, read_text, walk, with_root_config,
};
use crate::lang_python::{self as py, Arg, Def, DefKind, Import, Stmt};
use crate::lang_text::{collapse, idents, line, matching, matching_back, next_nonws, split_top};
use crate::services::support::test_path;
use archguard_core::{
    ANY_METHOD, AccessMeaning, AccessState, Action, Call, Condition, DiagnosticSeverity,
    EndpointAccess, Origin, Screen, ScreenKind, Target, Transition, UiFlow, UiFlowDiagnostic,
    best_route, path_patterns_match,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

const SOURCE: &str = "django";
const MAX_BYTES: u64 = 1 << 20;

/// Screens, navigation, link calls and endpoints of every Django project below `root`.
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run; `[views.access]` gives project guards their meaning.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    match walk(ctx.root) {
        Ok(w) => {
            let files: Vec<String> = w.files.into_iter().filter(|f| !test_path(f)).collect();
            let dirs: BTreeSet<&str> = (files.iter())
                .filter(|f| *f == "manage.py" || f.ends_with("/manage.py"))
                .map(|f| f.rsplit_once('/').map_or("", |(d, _)| d))
                .collect();
            if let (false, Some(e)) = (dirs.is_empty(), &ctx.config_error) {
                let message = format!("Configuration not read; [views.access] not applied: {e}");
                note(&mut flow, ".", 0, message);
            }
            for dir in &dirs {
                let mine = (files.iter())
                    .filter(|f| owner(f, &dirs) == Some(*dir))
                    .map(String::as_str);
                let project =
                    Project::read(ctx.root, dir, mine, &ctx.config.views.access, &mut flow);
                project.extract(&mut flow);
            }
        }
        Err(e) => gap(
            &mut flow.diagnostics,
            "",
            0,
            format!("Discovery exclusions not read; no Django project analyzed: {e}"),
        ),
    }
    finish(&mut flow);
    flow
}

/// The nearest project folder holding `file`.
fn owner<'a>(file: &str, dirs: &BTreeSet<&'a str>) -> Option<&'a str> {
    crate::extract_ctx::ancestors(file).find_map(|a| dirs.get(a).copied())
}

fn note(flow: &mut UiFlow, file: &str, line: u32, message: impl Into<String>) {
    flow.diagnostics.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message: message.into(),
        severity: DiagnosticSeverity::Note,
    });
}

/// The project's sources and what the rules read across files.
mod project {
    use super::*;

    /// One Python file, lexed.
    pub(super) struct Py {
        pub(super) rel: String,
        pub(super) src: String,
        pub(super) masked: String,
        pub(super) stmts: Vec<Stmt>,
        pub(super) defs: Vec<Def>,
        pub(super) imports: Vec<Import>,
        pub(super) module: String,
    }

    impl Py {
        pub(super) fn line(&self, at: usize) -> u32 {
            line(&self.src, at)
        }

        /// The innermost `def`/`class` whose body holds `at`.
        pub(super) fn owner(&self, at: usize) -> Option<usize> {
            (self.defs.iter().enumerate())
                .filter(|(_, d)| d.body.0 <= at && at < d.body.1)
                .max_by_key(|(_, d)| d.body.0)
                .map(|(i, _)| i)
        }

        /// Statements of the module itself, also inside `if` and `try`.
        pub(super) fn module_stmts(&self) -> impl Iterator<Item = &Stmt> {
            self.stmts.iter().filter(|s| self.owner(s.at).is_none())
        }

        pub(super) fn top_def(&self, name: &str) -> Option<usize> {
            (self.defs.iter()).rposition(|d| d.parent.is_none() && d.name == name)
        }

        pub(super) fn member(&self, class: usize, name: &str) -> Option<usize> {
            (self.defs.iter()).rposition(|d| d.parent == Some(class) && d.name == name)
        }

        /// The last assignment to `name` in the body of `class` itself: (value, offset).
        pub(super) fn class_attr(&self, class: usize, name: &str) -> Option<(&str, usize)> {
            let body = self.defs[class].body;
            (self.stmts.iter())
                .filter(|s| body.0 <= s.at && s.at < body.1 && self.owner(s.at) == Some(class))
                .filter_map(|s| py::assignment(&self.src, &self.masked, s))
                .rfind(|a| a.target == name && !a.augmented)
                .map(|a| (a.value, a.value_at))
        }

        /// The last module-level `name = <literal>`.
        pub(super) fn module_literal(&self, name: &str) -> Option<String> {
            (self.module_stmts())
                .filter_map(|s| py::assignment(&self.src, &self.masked, s))
                .filter(|a| a.target == name)
                .last()
                .and_then(|a| py::string_value(a.value))
        }

        /// `(from, to)` of `text` (a slice of `src`) without surrounding blanks.
        /// Comments count as blanks (trimmed on the masked text).
        pub(super) fn span(&self, text: &str) -> (usize, usize) {
            let at = crate::lang_text::offset(&self.src, text);
            let m = &self.masked[at..at + text.len()];
            let lead = m.len() - m.trim_start().len();
            (at + lead, at + m.trim_end().len().max(lead))
        }

        /// The call spanning `src[at..to]` as a whole: (callee, offset of its last `(`).
        pub(super) fn call(&self, at: usize, to: usize) -> Option<(&str, usize)> {
            let (at, to) = self.span(&self.src[at..to]);
            if to <= at || self.masked.as_bytes()[to - 1] != b')' {
                return None;
            }
            let open = matching_back(self.masked.as_bytes(), to - 1)?;
            let callee = self.src[at..open].trim();
            (open > at && !callee.is_empty()).then_some((callee, open))
        }

        pub(super) fn args(&self, open: usize) -> Vec<Arg<'_>> {
            py::call_args(&self.src, &self.masked, open).map_or_else(Vec::new, |a| a.0)
        }

        /// The ranges of the elements of a list or tuple display spanning `src[at..to]`.
        pub(super) fn elements(&self, at: usize, to: usize) -> Option<Vec<(usize, usize)>> {
            let (at, to) = self.span(&self.src[at..to]);
            let b = self.masked.as_bytes();
            if to <= at || !matches!(b[at], b'[' | b'(') || matching(b, at) != Some(to - 1) {
                return None;
            }
            Some(
                (py::split_commas(&self.masked, at + 1, to - 1).into_iter())
                    .map(|(a, b)| self.span(&self.src[a..b]))
                    .filter(|(a, b)| a < b)
                    .collect(),
            )
        }

        /// A string literal, or a list or tuple of them.
        pub(super) fn literals(&self, at: usize, to: usize) -> Option<Vec<String>> {
            if let Some(s) = py::string_value(&self.src[at..to]) {
                return Some(vec![s]);
            }
            (self.elements(at, to)?.into_iter())
                .map(|(a, b)| py::string_value(&self.src[a..b]))
                .collect()
        }

        /// Calls of the simple names `names` in `src[from..to]`: (name, offset of `(`).
        pub(super) fn calls_in(
            &self,
            from: usize,
            to: usize,
            names: &[&str],
        ) -> Vec<(&str, usize)> {
            let b = self.masked.as_bytes();
            (idents(&self.masked, from, to).into_iter())
                .filter(|(_, n)| names.contains(n))
                .filter_map(|(at, n)| {
                    let open = next_nonws(b, at + n.len()).filter(|&k| b[k] == b'(')?;
                    Some((&self.src[at..at + n.len()], open))
                })
                .collect()
        }
    }

    /// `text` when it is a dotted name.
    pub(super) fn dotted(text: &str) -> Option<&str> {
        let t = text.trim();
        let ok = !t.is_empty()
            && !t.starts_with(|c: char| c.is_ascii_digit() || c == '.')
            && t.bytes().all(|c| crate::lang_text::is_id(c) || c == b'.');
        ok.then_some(t)
    }

    pub(super) fn simple(name: &str) -> &str {
        name.rsplit('.').next().unwrap_or(name)
    }

    /// What a name refers to.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) enum Sym {
        /// A `def` or `class` of the scan: (file, def).
        Def(usize, usize),
        Module(usize),
        /// Outside the scan, by its dotted import path (or as written).
        External(String),
    }

    pub(super) struct Project<'a> {
        pub(super) app: String,
        pub(super) access: &'a BTreeMap<String, AccessMeaning>,
        pub(super) pys: Vec<Py>,
        pub(super) by_rel: HashMap<String, usize>,
        pub(super) rels: BTreeSet<String>,
        /// Template name (path below `templates/`) to its files.
        pub(super) templates: BTreeMap<String, Vec<String>>,
        pub(super) parsed: HashMap<String, super::templates::Template>,
        /// `DEFAULT_PERMISSION_CLASSES` (simple names) when the settings declare them.
        pub(super) default_perms: Option<Vec<String>>,
        /// URL configurations to start at: (file, settings file, line).
        pub(super) roots: Vec<(usize, String, u32)>,
        /// Project decorators on views that no `[views.access]` entry explains: (file, line, name).
        pub(super) unresolved: std::cell::RefCell<BTreeSet<(usize, u32, String)>>,
        /// Findings made while reading access: (file, line, is a gap, message).
        pub(super) findings: std::cell::RefCell<BTreeSet<(usize, u32, bool, String)>>,
    }

    impl<'a> Project<'a> {
        pub(super) fn read<'f>(
            root: &Path,
            dir: &str,
            files: impl Iterator<Item = &'f str>,
            access: &'a BTreeMap<String, AccessMeaning>,
            flow: &mut UiFlow,
        ) -> Self {
            let (mut sources, mut templates, mut parsed) =
                (vec![], BTreeMap::new(), HashMap::new());
            for rel in files {
                let template = (rel.rsplit_once("templates/"))
                    .filter(|(head, _)| head.is_empty() || head.ends_with('/'))
                    .filter(|_| {
                        rel.ends_with(".html") || rel.ends_with(".htm") || rel.ends_with(".txt")
                    });
                if !rel.ends_with(".py") && template.is_none() {
                    continue;
                }
                let text = match read_text(root, rel, MAX_BYTES) {
                    Ok(text) => text,
                    Err(e) => {
                        let message = format!(
                            "File not read ({e}): larger than 1 MiB or not readable; its routes, views or links are unknown."
                        );
                        gap(&mut flow.diagnostics, rel, 0, message);
                        continue;
                    }
                };
                if let Some((_, name)) = template {
                    templates
                        .entry(name.to_string())
                        .or_insert_with(Vec::new)
                        .push(rel.to_string());
                    parsed.insert(rel.to_string(), super::templates::parse(&text));
                } else {
                    sources.push((rel.to_string(), text));
                }
            }
            let rels: BTreeSet<String> = sources.iter().map(|(r, _)| r.clone()).collect();
            let pys: Vec<Py> = (sources.into_iter())
                .map(|(rel, src)| {
                    let masked = py::mask(&src);
                    Py {
                        stmts: py::statements(&masked),
                        defs: py::defs(&masked),
                        imports: py::imports(&masked),
                        module: py::module_name(&rel, &rels),
                        rel,
                        src,
                        masked,
                    }
                })
                .collect();
            let by_rel = pys
                .iter()
                .enumerate()
                .map(|(i, p)| (p.rel.clone(), i))
                .collect();
            let mut project = Project {
                app: dir_name(root, dir),
                access,
                pys,
                by_rel,
                rels,
                templates,
                parsed,
                default_perms: None,
                roots: vec![],
                unresolved: Default::default(),
                findings: Default::default(),
            };
            project.read_settings(dir, flow);
            project
        }

        /// `ROOT_URLCONF` and `REST_FRAMEWORK['DEFAULT_PERMISSION_CLASSES']`.
        fn read_settings(&mut self, dir: &str, flow: &mut UiFlow) {
            let mut defaults: BTreeMap<Vec<String>, (String, u32)> = BTreeMap::new();
            let mut roots = vec![];
            for p in self.pys.iter().filter(|p| is_settings(&p.rel)) {
                let assignments =
                    (p.module_stmts()).filter_map(|s| py::assignment(&p.src, &p.masked, s));
                for a in assignments {
                    let line = p.line(a.value_at);
                    let to = a.value_at + a.value.len();
                    match a.target {
                        "ROOT_URLCONF" => roots.extend(self.root_urlconf(p, a.value, line, flow)),
                        "REST_FRAMEWORK" => {
                            let classes = default_permission_classes(p, a.value_at, to);
                            classes.map(|c| defaults.entry(c).or_insert((p.rel.clone(), line)));
                        }
                        _ => {}
                    }
                }
            }
            self.roots = roots;
            if defaults.len() == 1 {
                self.default_perms = defaults.into_keys().next();
            } else {
                for (file, line) in defaults.values() {
                    let message = "Settings files disagree on DEFAULT_PERMISSION_CLASSES; \
                                   DRF views without permission_classes stay undeclared.";
                    gap(&mut flow.diagnostics, file, *line, message);
                }
            }
            if self.roots.is_empty() {
                let file = format!("{dir}/manage.py");
                let message = "No literal ROOT_URLCONF in settings.py; no routes read for this Django project.";
                gap(
                    &mut flow.diagnostics,
                    file.trim_start_matches('/'),
                    0,
                    message,
                );
            }
            self.roots.sort();
            self.roots.dedup_by_key(|r| r.0);
        }

        /// The URL configuration a `ROOT_URLCONF = value` names; a gap when unreadable.
        fn root_urlconf(
            &self,
            p: &Py,
            value: &str,
            line: u32,
            flow: &mut UiFlow,
        ) -> Option<(usize, String, u32)> {
            let Some(module) = py::string_value(value) else {
                let message = format!(
                    "ROOT_URLCONF `{}` is not a literal; routes are unknown.",
                    collapse(value)
                );
                gap(&mut flow.diagnostics, &p.rel, line, message);
                return None;
            };
            let Some(f) = py::resolve_module(&p.rel, &module, &self.rels) else {
                let message =
                    format!("ROOT_URLCONF `{module}` is not in the scan; its routes are unknown.");
                gap(&mut flow.diagnostics, &p.rel, line, message);
                return None;
            };
            Some((self.by_rel[&f], p.rel.clone(), line))
        }

        /// What `dotted` names in file `f`; outside the scan an `External` by import path.
        pub(super) fn resolve(&self, f: usize, dotted: &str) -> Sym {
            (self.resolve_in(f, dotted, 0))
                .unwrap_or_else(|| Sym::External(self.external(f, dotted)))
        }

        /// The import path of `dotted` in file `f` as far as its imports tell.
        fn external(&self, f: usize, dotted: &str) -> String {
            let (head, rest) = dotted
                .split_once('.')
                .map_or((dotted, None), |(h, r)| (h, Some(r)));
            let Some(imp) = self.pys[f].imports.iter().rev().find(|i| i.local == head) else {
                return dotted.to_string();
            };
            let base = match &imp.name {
                Some(name) => format!("{}.{name}", imp.module.trim_end_matches('.')),
                None => imp.module.clone(),
            };
            rest.map_or(base.clone(), |r| format!("{base}.{r}"))
        }

        fn resolve_in(&self, f: usize, dotted: &str, depth: u8) -> Option<Sym> {
            if depth > 8 {
                return None;
            }
            let p = &self.pys[f];
            let (head, rest) = dotted
                .split_once('.')
                .map_or((dotted, None), |(h, r)| (h, Some(r)));
            if let Some(d) = p.top_def(head) {
                return rest.is_none().then_some(Sym::Def(f, d));
            }
            if let Some(imp) = p.imports.iter().rev().find(|i| i.local == head) {
                let (file, symbol) = py::resolve_import(&p.rel, imp, &self.rels)?;
                let g = self.by_rel[&file];
                return match (symbol, rest) {
                    (None, None) => Some(Sym::Module(g)),
                    (Some(s), None) => self.resolve_in(g, &s, depth + 1),
                    (None, Some(r)) => self.resolve_in(g, r, depth + 1),
                    (Some(s), Some(r)) => self.resolve_in(g, &format!("{s}.{r}"), depth + 1),
                };
            }
            // A submodule of this package (`import a.b` then `a.b.X`).
            let submodule = (p.rel.ends_with("__init__.py"))
                .then(|| py::resolve_module(&p.rel, &format!(".{head}"), &self.rels))
                .flatten();
            if let Some(file) = submodule {
                let g = self.by_rel[&file];
                return match rest {
                    None => Some(Sym::Module(g)),
                    Some(r) => self.resolve_in(g, r, depth + 1),
                };
            }
            (p.imports.iter().filter(|i| i.local == "*"))
                .filter_map(|i| py::resolve_module(&p.rel, &i.module, &self.rels))
                .find_map(|file| self.resolve_in(self.by_rel[&file], dotted, depth + 1))
        }

        pub(super) fn simple_name(&self, sym: &Sym) -> String {
            match sym {
                Sym::Def(f, d) => self.pys[*f].defs[*d].name.clone(),
                Sym::External(s) => simple(s).to_string(),
                Sym::Module(f) => self.pys[*f].module.clone(),
            }
        }

        /// The class and its bases, depth first and left to right, each once.
        pub(super) fn mro(&self, start: &Sym) -> Vec<Sym> {
            let mut out = vec![start.clone()];
            let mut i = 0;
            while i < out.len() {
                if let Sym::Def(f, d) = out[i] {
                    let bases: Vec<Sym> = (self.bases(f, d).into_iter())
                        .filter(|b| !out.contains(b))
                        .fold(vec![], |mut v, b| {
                            if !v.contains(&b) {
                                v.push(b);
                            }
                            v
                        });
                    // Depth first: the bases of `out[i]` come right after it.
                    let tail = out.split_off(i + 1);
                    out.extend(bases);
                    let rest: Vec<Sym> = tail.into_iter().filter(|s| !out.contains(s)).collect();
                    out.extend(rest);
                }
                i += 1;
            }
            out
        }

        /// The base classes written after class `d` of file `f` (generic arguments dropped).
        fn bases(&self, f: usize, d: usize) -> Vec<Sym> {
            let p = &self.pys[f];
            let Some((a, b)) = p.defs[d].head.filter(|_| p.defs[d].kind == DefKind::Class) else {
                return vec![];
            };
            (py::split_commas(&p.masked, a, b).into_iter())
                .filter_map(|(x, y)| {
                    let text = p.src[x..y].trim();
                    dotted(&text[..text.find('[').unwrap_or(text.len())])
                })
                .map(|name| self.resolve(f, name))
                .collect()
        }

        /// The first class of `mro` assigning `name`: (file, value, offset).
        pub(super) fn attr(&self, mro: &[Sym], name: &str) -> Option<(usize, &str, usize)> {
            mro.iter().find_map(|s| match s {
                Sym::Def(f, d) => (self.pys[*f].class_attr(*d, name)).map(|(v, at)| (*f, v, at)),
                _ => None,
            })
        }

        /// The first class of `mro` defining method `name`: (file, def).
        pub(super) fn method(&self, mro: &[Sym], name: &str) -> Option<(usize, usize)> {
            mro.iter().find_map(|s| match s {
                Sym::Def(f, d) => self.pys[*f].member(*d, name).map(|m| (*f, m)),
                _ => None,
            })
        }

        /// Methods of `mro`, each name by its first definer: (name, file, def).
        pub(super) fn methods(&self, mro: &[Sym]) -> Vec<(String, usize, usize)> {
            let mut out: Vec<(String, usize, usize)> = vec![];
            for s in mro {
                if let Sym::Def(f, d) = s {
                    for (m, def) in self.pys[*f].defs.iter().enumerate() {
                        if def.parent == Some(*d)
                            && def.kind == DefKind::Function
                            && !out.iter().any(|o| o.0 == def.name)
                        {
                            out.push((def.name.clone(), *f, m));
                        }
                    }
                }
            }
            out
        }

        pub(super) fn names(&self, mro: &[Sym]) -> Vec<String> {
            mro.iter().map(|s| self.simple_name(s)).collect()
        }
    }

    fn is_settings(rel: &str) -> bool {
        rel == "settings.py" || rel.ends_with("/settings.py") || rel.contains("/settings/")
    }

    /// The simple class names of `'DEFAULT_PERMISSION_CLASSES': (…)` in a
    /// `REST_FRAMEWORK` dict display at `src[at..to]`.
    fn default_permission_classes(p: &Py, at: usize, to: usize) -> Option<Vec<String>> {
        let key = p.src[at..to].find("DEFAULT_PERMISSION_CLASSES")? + at;
        let colon = p.masked[key..to].find(':')? + key;
        let b = p.masked.as_bytes();
        let open = next_nonws(b, colon + 1).filter(|&k| matches!(b[k], b'[' | b'('))?;
        let close = matching(b, open)?;
        let names = p.literals(open, close + 1)?;
        Some(names.iter().map(|n| simple(n).to_string()).collect())
    }
}
use project::{Project, Py, Sym, dotted, simple};

/// Access declarations: Django decorators and mixins, DRF permission classes.
mod access {
    use super::views::{ACTIONS, API_GENERICS, HTTP, VIEWSETS};
    use super::*;

    /// What was declared: nothing (`undeclared`), or conditions that must all
    /// hold; an empty list after a declaration is `open`.
    #[derive(Debug, Clone, Default)]
    pub(super) struct Decl {
        pub(super) declared: bool,
        pub(super) parts: Vec<Condition>,
        /// Project decorators that are not read; never lets a declaration be `open`.
        pub(super) unresolved: Vec<String>,
    }

    impl Decl {
        /// One declaration; `None` allows everyone.
        pub(super) fn add(&mut self, c: Option<Condition>) {
            self.declared = true;
            match c {
                // `all(a, b)` next to `c` is `all(a, b, c)`.
                Some(Condition::All(parts)) => parts.into_iter().for_each(|c| self.add(Some(c))),
                Some(c) if !self.parts.contains(&c) => self.parts.push(c),
                _ => {}
            }
        }

        pub(super) fn extend(&mut self, other: Decl) {
            self.unresolved.extend(other.unresolved);
            if other.declared {
                self.declared = true;
                other.parts.into_iter().for_each(|c| self.add(Some(c)));
            }
        }

        pub(super) fn state(&self) -> AccessState {
            match (self.declared, self.condition()) {
                (false, _) => AccessState::Undeclared,
                (true, None) if !self.unresolved.is_empty() => {
                    AccessState::Protected(Condition::Unknown(self.unresolved.join(", ")))
                }
                (true, None) => AccessState::Open,
                (true, Some(c)) => AccessState::Protected(c),
            }
        }

        pub(super) fn condition(&self) -> Option<Condition> {
            match self.parts.as_slice() {
                [] => None,
                [one] => Some(one.clone()),
                many => Some(Condition::All(many.to_vec())),
            }
        }
    }

    /// What the decorators of a view (or wrappers in `urls.py`) say.
    #[derive(Debug, Default)]
    pub(super) struct Effects {
        pub(super) decl: Decl,
        /// From `require_http_methods` and friends, or `@api_view`.
        pub(super) methods: Option<Vec<String>>,
        pub(super) api_view: bool,
        /// DRF `@permission_classes(…)`: (file, from, to) of the list.
        pub(super) perms: Option<(usize, usize, usize)>,
        /// A `@permission_classes` above `@api_view`, which DRF ignores: (file, offset).
        pub(super) perms_above: Option<(usize, usize)>,
        /// Declarations of one handler method (lower case): `method_decorator(name=…)`
        /// and the decorators of the handler itself.
        pub(super) by_method: BTreeMap<String, Decl>,
    }

    impl Effects {
        /// The view's declarations plus those of handler `method` (any case).
        pub(super) fn for_method(&self, method: &str) -> Decl {
            let mut decl = self.decl.clone();
            if let Some(d) = self.by_method.get(&method.to_ascii_lowercase()) {
                decl.extend(d.clone());
            }
            decl
        }
    }

    const STAFF: &str = "is_staff";

    /// Decorators that perform no access check; every other unread one is unknown (D1).
    /// Django: https://docs.djangoproject.com/en/stable/topics/http/decorators/ and
    /// https://docs.djangoproject.com/en/stable/ref/csrf/ ; DRF: https://www.django-rest-framework.org/api-guide/views/
    const NO_ACCESS_CHECK: [&str; 12] = [
        "csrf_exempt",
        "csrf_protect",
        "cache_page",
        "never_cache",
        "vary_on_headers",
        "vary_on_cookie",
        "renderer_classes",
        "parser_classes",
        "throttle_classes",
        "schema",
        "condition",
        "etag",
    ];
    /// DRF `IsAdminUser` and `staff_member_required`: a signed-in staff user.
    fn staff() -> Condition {
        Condition::All(vec![
            Condition::Authenticated,
            Condition::Custom(STAFF.into()),
        ])
    }
    const SAFE: [&str; 3] = ["GET", "HEAD", "OPTIONS"];

    fn any(mut c: Vec<Condition>) -> Condition {
        if c.len() == 1 {
            c.remove(0)
        } else {
            Condition::Any(c)
        }
    }

    /// The `[views.access]` meaning at a use site with literal `args`; `None` is open.
    pub(super) fn meaning(m: &AccessMeaning, name: &str, args: &[String]) -> Option<Condition> {
        let from_args = |make: fn(String) -> Condition| match args.is_empty() {
            true => Condition::Unknown(name.to_string()),
            false => any(args.iter().cloned().map(make).collect()),
        };
        match m.at_use_site(!args.is_empty()) {
            AccessMeaning::Open => None,
            AccessMeaning::Authenticated => Some(Condition::Authenticated),
            AccessMeaning::Never => Some(Condition::Never),
            AccessMeaning::Role(n) => Some(Condition::Role(n.clone())),
            AccessMeaning::Policy(n) => Some(Condition::Policy(n.clone())),
            AccessMeaning::RoleFromArguments => Some(from_args(Condition::Role)),
            AccessMeaning::PolicyFromArguments => Some(from_args(Condition::Policy)),
            AccessMeaning::FromArgumentsOr(..) => Some(Condition::Unknown(name.to_string())),
        }
    }

    impl Project<'_> {
        /// Applies the decorator expression `src[at..to]` of file `f`.
        pub(super) fn decorate(&self, f: usize, at: usize, to: usize, fx: &mut Effects) {
            let p = &self.pys[f];
            let (callee, args) = match p.call(at, to) {
                Some((callee, open)) => (callee, p.args(open)),
                None => (p.src[at..to].trim(), vec![]),
            };
            let positional: Vec<&Arg> = args.iter().filter(|a| a.name.is_none()).collect();
            let first = positional.first().map(|a| p.span(a.value));
            let name = simple(callee);
            let named = args.iter().find(|a| a.name == Some("name"));
            if let (Some(n), "method_decorator") = (named, name) {
                self.named_method_decorator(f, at, first, py::string_value(n.value), fx);
                return;
            }
            if self.decorate_request(f, name, first, fx) {
                return;
            }
            let literals = first
                .and_then(|(a, b)| p.literals(a, b))
                .filter(|l| !l.is_empty());
            let condition = match name {
                "login_required" => Some(Condition::Authenticated),
                "staff_member_required" => Some(staff()),
                "permission_required" => {
                    Some(literals.map_or_else(|| unknown_call(p, name, first), all_policies))
                }
                "user_passes_test" => match first.and_then(|r| dotted(&p.src[r.0..r.1])) {
                    Some(test) => self.test_condition(simple(test)),
                    None => Some(unknown_call(p, name, first)),
                },
                _ => {
                    let Some(m) = self.access.get(name) else {
                        if !self.checks_no_access(f, callee) {
                            fx.decl.unresolved.push(name.to_string());
                            let line = p.line(at);
                            (self.unresolved.borrow_mut()).insert((f, line, name.to_string()));
                        }
                        return;
                    };
                    let args: Vec<String> = positional
                        .iter()
                        .filter_map(|a| py::string_value(a.value))
                        .collect();
                    meaning(m, name, &args)
                }
            };
            fx.decl.add(condition);
        }

        /// `method_decorator(d, name="post")`: `d` guards that handler only.
        fn named_method_decorator(
            &self,
            f: usize,
            at: usize,
            first: Option<(usize, usize)>,
            target: Option<String>,
            fx: &mut Effects,
        ) {
            let mut sub = Effects::default();
            self.decorate_request(f, "method_decorator", first, &mut sub);
            let line = self.pys[f].line(at);
            match target.as_deref().map(str::to_ascii_lowercase) {
                Some(t) if t == "dispatch" => fx.decl.extend(sub.decl),
                Some(t) if HTTP.contains(&t.as_str()) || ACTIONS.iter().any(|a| a.0 == t) => {
                    fx.by_method.entry(t).or_default().extend(sub.decl)
                }
                other => {
                    let what = other.map_or("an unreadable name".to_string(), |t| format!("`{t}`"));
                    let message = format!(
                        "`method_decorator` targets {what}, not a known HTTP handler; the access it checks is unknown."
                    );
                    self.finding(f, line, true, message);
                    fx.decl
                        .add(Some(Condition::Unknown("method_decorator(name=…)".into())));
                }
            }
        }

        pub(super) fn finding(&self, f: usize, line: u32, gap: bool, message: String) {
            (self.findings.borrow_mut()).insert((f, line, gap, message));
        }

        /// A `NO_ACCESS_CHECK` name counts only when imported from its Django/DRF module.
        fn checks_no_access(&self, f: usize, callee: &str) -> bool {
            let from_framework = |path: &str| {
                [
                    "django.views.decorators.",
                    "django.utils.decorators.",
                    "rest_framework.decorators.",
                ]
                .iter()
                .any(|m| path.starts_with(m))
            };
            matches!(self.resolve(f, callee), Sym::External(path)
                if from_framework(&path) && NO_ACCESS_CHECK.contains(&simple(&path)))
        }

        /// Decorators about the request rather than the user: methods, DRF
        /// views, and `method_decorator` around others. False for any other.
        fn decorate_request(
            &self,
            f: usize,
            name: &str,
            first: Option<(usize, usize)>,
            fx: &mut Effects,
        ) -> bool {
            let p = &self.pys[f];
            let methods = |r: Option<(usize, usize)>| {
                let literals = r.and_then(|(a, b)| p.literals(a, b));
                literals.map(|m| m.iter().map(|x| x.to_ascii_uppercase()).collect::<Vec<_>>())
            };
            let fixed = |m: &[&str]| Some(m.iter().map(|x| x.to_string()).collect());
            match name {
                "method_decorator" => {
                    let Some((a, b)) = first else { return true };
                    let list = p.elements(a, b).unwrap_or_else(|| vec![(a, b)]);
                    list.into_iter()
                        .for_each(|(x, y)| self.decorate(f, x, y, fx));
                }
                "api_view" => {
                    fx.api_view = true;
                    fx.methods = match first {
                        Some(r) => Some(methods(Some(r)).unwrap_or_default()),
                        None => fixed(&["GET"]),
                    };
                }
                "permission_classes" if fx.api_view => fx.perms = first.map(|(a, b)| (f, a, b)),
                "permission_classes" => fx.perms_above = first.map(|(a, _)| (f, a)),
                "require_http_methods" => fx.methods = methods(first),
                "require_GET" => fx.methods = fixed(&["GET"]),
                "require_POST" => fx.methods = fixed(&["POST"]),
                "require_safe" => fx.methods = fixed(&["GET", "HEAD"]),
                _ => return false,
            }
            true
        }

        /// A test function's condition: its `[views.access]` meaning, else `custom(name)`.
        fn test_condition(&self, name: &str) -> Option<Condition> {
            match self.access.get(name) {
                Some(m) => meaning(m, name, &[]),
                None => Some(Condition::Custom(name.to_string())),
            }
        }

        /// Django mixins of a class view and the decorators on its classes, `dispatch`
        /// and handler methods.
        pub(super) fn class_decl(&self, mro: &[Sym], fx: &mut Effects) {
            self.mixin_decl(mro, fx);
            for sym in mro {
                if let Sym::Def(f, d) = sym {
                    for &(a, b) in &self.pys[*f].defs[*d].decorators {
                        self.decorate(*f, a, b, fx);
                    }
                }
            }
            if let Some((f, d)) = self.method(mro, "dispatch") {
                for &(a, b) in &self.pys[f].defs[d].decorators {
                    self.decorate(f, a, b, fx);
                }
            }
            for (name, f, d) in self.methods(mro) {
                if !HTTP.contains(&name.as_str()) {
                    continue;
                }
                let mut sub = Effects::default();
                for &(a, b) in &self.pys[f].defs[d].decorators {
                    self.decorate(f, a, b, &mut sub);
                }
                if sub.decl.declared || !sub.decl.unresolved.is_empty() {
                    fx.by_method.entry(name).or_default().extend(sub.decl);
                }
            }
        }

        /// Mixins that run: a mixin right of a view base never does, because the
        /// base's `dispatch` does not call on.
        fn mixin_decl(&self, mro: &[Sym], fx: &mut Effects) {
            let stop = (mro.iter())
                .position(|s| self.is_view_base(s))
                .unwrap_or(mro.len());
            for (i, sym) in mro.iter().enumerate() {
                let name = self.simple_name(sym);
                let Some(condition) = self.mixin_condition(mro, &name) else {
                    continue;
                };
                if i < stop {
                    fx.decl.add(condition);
                } else if let Sym::Def(f, d) = mro[0] {
                    let base = self.simple_name(&mro[stop]);
                    let message = format!(
                        "`{name}` is listed after `{base}`, whose `dispatch` never calls on; it gives no protection."
                    );
                    self.finding(f, self.pys[f].defs[d].line, false, message);
                }
            }
        }

        /// A framework view class: its `dispatch` runs before any mixin to its right.
        fn is_view_base(&self, sym: &Sym) -> bool {
            let Sym::External(path) = sym else {
                return false;
            };
            let views = [
                "django.views.",
                "django.contrib.auth.views.",
                "rest_framework.",
            ];
            let named = !simple(path).ends_with("Mixin");
            named && views.iter().any(|v| path.starts_with(v))
        }

        /// What base class `name` of a view declares; none when nothing.
        fn mixin_condition(&self, mro: &[Sym], name: &str) -> Option<Option<Condition>> {
            match name {
                "LoginRequiredMixin" => Some(Some(Condition::Authenticated)),
                "PermissionRequiredMixin" => {
                    let perms = (self.attr(mro, "permission_required"))
                        .and_then(|(f, v, at)| self.pys[f].literals(at, at + v.len()))
                        .filter(|p| !p.is_empty());
                    let unknown = || Condition::Unknown("permission_required".into());
                    Some(Some(perms.map_or_else(unknown, all_policies)))
                }
                "UserPassesTestMixin" => {
                    let owner = self.method(mro, "test_func").map(|(f, d)| {
                        let p = &self.pys[f];
                        p.defs[d]
                            .parent
                            .map_or(String::new(), |c| p.defs[c].name.clone())
                    });
                    let owner = owner.unwrap_or_else(|| self.simple_name(&mro[0]));
                    Some(self.test_condition(&format!("{owner}.test_func")))
                }
                other => self.access.get(other).map(|m| meaning(m, other, &[])),
            }
        }

        /// DRF access for one method: the action's or class's
        /// `permission_classes`, a project `get_permissions` (unknown), else
        /// the settings' default.
        pub(super) fn drf_decl(
            &self,
            mro: &[Sym],
            own: Option<(usize, usize, usize)>,
            method: &str,
        ) -> Decl {
            if let Some((f, d)) = self.method(mro, "get_permissions") {
                let p = &self.pys[f];
                let class = p.defs[d].parent.map_or("", |c| p.defs[c].name.as_str());
                let mut decl = Decl::default();
                decl.add(Some(Condition::Unknown(format!("{class}.get_permissions"))));
                return decl;
            }
            if let Some((f, a, b)) = own {
                return self.perm_list(f, a, b, (method, mro));
            }
            // The first class of the MRO that sets them; a base outside the scan may.
            for sym in mro {
                match sym {
                    Sym::Def(f, d) => {
                        if let Some((v, at)) = self.pys[*f].class_attr(*d, "permission_classes") {
                            return self.perm_list(*f, at, at + v.len(), (method, mro));
                        }
                    }
                    Sym::External(path) if !framework_base(path) => {
                        let name = simple(path);
                        if let Sym::Def(f, d) = mro[0] {
                            let message = format!(
                                "Base class `{name}` is outside the scan; its permission_classes are unknown."
                            );
                            self.finding(f, self.pys[f].defs[d].line, true, message);
                        }
                        let mut decl = Decl::default();
                        decl.add(Some(Condition::Unknown(format!(
                            "{name}.permission_classes"
                        ))));
                        return decl;
                    }
                    _ => {}
                }
            }
            let mut decl = Decl::default();
            for name in self.default_perms.iter().flatten() {
                decl.add(self.perm_expr(None, name, (method, mro)));
            }
            decl
        }

        fn perm_list(&self, f: usize, a: usize, b: usize, at: (&str, &[Sym])) -> Decl {
            let p = &self.pys[f];
            let mut decl = Decl::default();
            match p.elements(a, b) {
                Some(list) => {
                    decl.declared = list.is_empty();
                    for (x, y) in list {
                        if let Some(name) = self.object_only(f, &p.src[x..y]) {
                            let message = format!(
                                "`{name}` checks objects only; list and create requests are not guarded by it."
                            );
                            self.finding(f, p.line(x), false, message);
                            continue;
                        }
                        decl.add(self.perm_expr(Some(f), &p.src[x..y], at));
                    }
                }
                None => decl.add(Some(Condition::Unknown(collapse(&p.src[a..b])))),
            }
            decl
        }

        /// The name of a project permission class that defines `has_object_permission`
        /// but no `has_permission` (so it allows every request), else none.
        fn object_only(&self, f: usize, text: &str) -> Option<String> {
            let name = dotted(text.trim().trim_end_matches("()"))?;
            if self.access.get(simple(name)).is_some() {
                return None;
            }
            let sym @ Sym::Def(..) = self.resolve(f, name) else {
                return None;
            };
            let mro = self.mro(&sym);
            let bare = |s: &Sym| match s {
                Sym::External(path) => matches!(simple(path), "BasePermission" | "object"),
                _ => true,
            };
            (mro.iter().all(bare)
                && self.method(&mro, "has_object_permission").is_some()
                && self.method(&mro, "has_permission").is_none())
            .then(|| simple(name).to_string())
        }

        /// One permission class expression; `None` allows everyone.
        fn perm_expr(&self, f: Option<usize>, text: &str, at: (&str, &[Sym])) -> Option<Condition> {
            let text = crate::lang_text::unparen(text);
            let parts = split_top(text, "|");
            if parts.len() > 1 {
                let all: Option<Vec<Condition>> =
                    parts.iter().map(|t| self.perm_expr(f, t, at)).collect();
                return all.map(any);
            }
            let parts = split_top(text, "&");
            if parts.len() > 1 {
                let all: Vec<Condition> = parts
                    .iter()
                    .filter_map(|t| self.perm_expr(f, t, at))
                    .collect();
                return (!all.is_empty()).then(|| {
                    if all.len() == 1 {
                        all[0].clone()
                    } else {
                        Condition::All(all)
                    }
                });
            }
            if let Some(inner) = text.strip_prefix('~') {
                return Some(match self.perm_expr(f, inner, at) {
                    None => Condition::Never,
                    Some(c) => Condition::Not(Box::new(c)),
                });
            }
            if let Some(name) = f.and_then(|f| self.object_only(f, text)) {
                // Inside an operator it is no request guard either, but not a plain `allow`.
                return Some(Condition::Unknown(format!("{name} (object-level only)")));
            }
            self.perm_class(simple(text.trim().trim_end_matches("()")), at)
        }

        /// One permission class by name: `[views.access]`, the DRF built-ins, else custom.
        fn perm_class(&self, name: &str, (method, mro): (&str, &[Sym])) -> Option<Condition> {
            if let Some(m) = self.access.get(name) {
                return meaning(m, name, &[]);
            }
            match name {
                "AllowAny" => None,
                "IsAuthenticated" => Some(Condition::Authenticated),
                "IsAdminUser" => Some(staff()),
                "IsAuthenticatedOrReadOnly" if SAFE.contains(&method) => None,
                "IsAuthenticatedOrReadOnly" if method == ANY_METHOD => {
                    Some(Condition::Unknown(name.into()))
                }
                "IsAuthenticatedOrReadOnly" => Some(Condition::Authenticated),
                "DjangoModelPermissionsOrAnonReadOnly" if SAFE.contains(&method) => None,
                "DjangoModelPermissionsOrAnonReadOnly" if method == ANY_METHOD => {
                    Some(Condition::Unknown(name.into()))
                }
                "DjangoModelPermissions"
                | "DjangoModelPermissionsOrAnonReadOnly"
                | "DjangoObjectPermissions" => Some(self.model_permissions(name, method, mro)),
                _ => Some(Condition::Custom(name.to_string())),
            }
        }

        /// DRF's model permissions for one method: a signed-in user holding the model's
        /// add/change/delete permission; the model is the view's `queryset`.
        fn model_permissions(&self, name: &str, method: &str, mro: &[Sym]) -> Condition {
            let unknown = || {
                if let Some(&Sym::Def(f, d)) = mro.first() {
                    let message = format!(
                        "`{name}` needs the view's model; its `queryset` is not readable, so the permission is unknown."
                    );
                    self.finding(f, self.pys[f].defs[d].line, true, message);
                }
                Condition::All(vec![
                    Condition::Authenticated,
                    Condition::Unknown(name.into()),
                ])
            };
            let verb = match method {
                "GET" | "HEAD" | "OPTIONS" => return Condition::Authenticated,
                "POST" => "add",
                "PUT" | "PATCH" => "change",
                "DELETE" => "delete",
                _ => return unknown(),
            };
            let Some((app, model)) = self.queryset_model(mro) else {
                return unknown();
            };
            Condition::All(vec![
                Condition::Authenticated,
                Condition::Policy(format!("{app}.{verb}_{model}")),
            ])
        }

        /// (app label, lower-case model name) of the view's `queryset = Model.objects…`.
        /// The app label is the model's `Meta.app_label`, else as in
        /// `domain_python`: `AppConfig.label` of the app's `apps.py`, else the
        /// last part of `AppConfig.name`, else the package name (a duplicate of
        /// that logic: its index types are the domain extractor's own).
        fn queryset_model(&self, mro: &[Sym]) -> Option<(String, String)> {
            if self.method(mro, "get_queryset").is_some() {
                return None;
            }
            let (f, v, _) = self.attr(mro, "queryset")?;
            let head = v.split(".objects").next().filter(|h| h.len() < v.len())?;
            let Sym::Def(mf, md) = self.resolve(f, dotted(head.trim())?) else {
                return None;
            };
            let p = &self.pys[mf];
            if p.defs[md].kind != DefKind::Class {
                return None;
            }
            Some((self.app_label(mf, md)?, p.defs[md].name.to_lowercase()))
        }

        /// `None` when an `app_label` is written but not as a plain string
        /// in the model's own `Meta`.
        fn app_label(&self, mf: usize, md: usize) -> Option<String> {
            let p = &self.pys[mf];
            let (a, b) = p.defs[md].body;
            if p.src[a..b].contains("app_label") {
                let meta = (0..p.defs.len())
                    .find(|&d| p.defs[d].name == "Meta" && p.defs[d].parent == Some(md))?;
                return py::string_value(p.class_attr(meta, "app_label")?.0);
            }
            let mut dirs: Vec<&str> = p.rel.split('/').collect();
            dirs.pop();
            if dirs.last() == Some(&"models") {
                dirs.pop();
            }
            let package = (*dirs.last()?).to_string();
            let apps = (0..dirs.len()).rev().find_map(|n| {
                let rel = format!("{}/apps.py", dirs[..=n].join("/"));
                self.by_rel.get(&rel).copied()
            });
            Some(apps.and_then(|f| self.config_label(f)).unwrap_or(package))
        }

        /// `label`, else the last part of `name`, of the first `AppConfig` of `apps.py`.
        fn config_label(&self, f: usize) -> Option<String> {
            let p = &self.pys[f];
            let configs = (0..p.defs.len()).filter(|&d| {
                let def = &p.defs[d];
                let bases = def.head.map_or("", |(a, b)| &p.src[a..b]);
                def.kind == DefKind::Class
                    && bases
                        .split(',')
                        .any(|x| x.trim().rsplit('.').next() == Some("AppConfig"))
            });
            let attr = |d: usize, n: &str| py::string_value(p.class_attr(d, n)?.0);
            configs.into_iter().find_map(|d| {
                attr(d, "label").or_else(|| {
                    attr(d, "name").map(|n| n.rsplit('.').next().unwrap_or(&n).to_string())
                })
            })
        }
    }

    /// A base class that sets no `permission_classes` of its own: Django, DRF, builtins.
    fn framework_base(path: &str) -> bool {
        let known = (API_GENERICS.iter().map(|g| g.0))
            .chain(VIEWSETS)
            .chain(["View", "object"]);
        let bare = !path.contains('.') && known.clone().any(|k| k == path);
        bare || ["rest_framework.", "django.", "typing.", "abc."]
            .iter()
            .any(|m| path.starts_with(m))
    }

    /// `policy(p)` for one permission, all of them for several.
    fn all_policies(mut perms: Vec<String>) -> Condition {
        match perms.len() {
            1 => Condition::Policy(perms.remove(0)),
            _ => Condition::All(perms.into_iter().map(Condition::Policy).collect()),
        }
    }

    /// `unknown(name(first argument))`.
    fn unknown_call(p: &project::Py, name: &str, first: Option<(usize, usize)>) -> Condition {
        let arg = first
            .map(|(a, b)| collapse(&p.src[a..b]))
            .unwrap_or_default();
        Condition::Unknown(format!("{name}({arg})"))
    }
}
use access::{Decl, Effects};

/// URL configurations: routes with their views.
mod urls {
    use super::*;

    pub(super) enum Kind {
        /// A view; `as_view` is the `(` of `.as_view(…)`, `wrappers` the
        /// decorator calls around it in `urls.py` (`login_required(view)`).
        View {
            as_view: Option<usize>,
            wrappers: Vec<(usize, usize)>,
        },
        /// A ViewSet registered on a router.
        ViewSet {
            slash: &'static str,
            basename: Option<String>,
            namespace: Vec<String>,
        },
    }

    pub(super) struct Route {
        /// Without the leading `/`, parameters as `:name`.
        pub(super) path: String,
        pub(super) name: Option<String>,
        pub(super) target: Sym,
        pub(super) kind: Kind,
        /// The `urls.py` and line of the entry.
        pub(super) file: usize,
        pub(super) line: u32,
    }

    /// Reads URL configurations from the project's roots.
    pub(super) struct Reader<'p, 'a> {
        pub(super) p: &'p Project<'a>,
        pub(super) flow: &'p mut UiFlow,
        pub(super) routes: Vec<Route>,
        stack: Vec<(usize, String)>,
    }

    impl<'p, 'a> Reader<'p, 'a> {
        pub(super) fn read(p: &'p Project<'a>, flow: &'p mut UiFlow) -> Vec<Route> {
            let mut r = Reader {
                p,
                flow,
                routes: vec![],
                stack: vec![],
            };
            for (f, _, _) in &p.roots {
                r.conf(*f, "", &[]);
            }
            r.routes
        }

        fn gap(&mut self, f: usize, at: usize, message: String) {
            let p = &self.p.pys[f];
            gap(&mut self.flow.diagnostics, &p.rel, p.line(at), message);
        }

        fn conf(&mut self, f: usize, prefix: &str, ns: &[String]) {
            if !self.var(f, "urlpatterns", prefix, ns) {
                let rel = self.p.pys[f].rel.clone();
                gap(
                    &mut self.flow.diagnostics,
                    &rel,
                    0,
                    "No `urlpatterns` in this URL configuration; its routes are unknown.",
                );
            }
        }

        /// Evaluates every statement building module variable `name`; false when there is none.
        fn var(&mut self, f: usize, name: &str, prefix: &str, ns: &[String]) -> bool {
            let key = (f, name.to_string());
            if self.stack.contains(&key) {
                return true;
            }
            self.stack.push(key);
            let p = self.p.pys.get(f).expect("file");
            let mut found = false;
            let stmts: Vec<Stmt> = p.module_stmts().copied().collect();
            for s in stmts {
                let p = &self.p.pys[f];
                if let Some(a) = py::assignment(&p.src, &p.masked, &s).filter(|a| a.target == name)
                {
                    found = true;
                    self.expr(f, a.value_at, a.value_at + a.value.len(), prefix, ns);
                    continue;
                }
                let text = &p.masked[s.at..s.end];
                // An appended entry and an extending list both read as patterns.
                for method in [".append(", ".extend(", ".insert("] {
                    let call = format!("{name}{method}");
                    if !text.starts_with(&call) {
                        continue;
                    }
                    found = true;
                    let open = s.at + call.len() - 1;
                    let args: Vec<(usize, usize)> =
                        p.args(open).iter().map(|a| p.span(a.value)).collect();
                    let arg = if method == ".insert(" {
                        args.get(1)
                    } else {
                        args.first()
                    };
                    if let Some(&(a, b)) = arg {
                        self.expr(f, a, b, prefix, ns);
                    }
                }
            }
            self.stack.pop();
            found
        }

        /// A list of URL patterns spanning `src[at..to]`.
        fn expr(&mut self, f: usize, at: usize, to: usize, prefix: &str, ns: &[String]) {
            let p = &self.p.pys[f];
            let (at, to) = p.span(&p.src[at..to]);
            if at >= to {
                return;
            }
            let at = at + usize::from(p.src[at..].starts_with('*'));
            if let Some(list) = p.elements(at, to) {
                for (a, b) in list {
                    self.expr(f, a, b, prefix, ns);
                }
                return;
            }
            let parts = split_top(&p.masked[at..to], "+");
            if parts.len() > 1 {
                let mut from = at;
                for part in parts {
                    self.expr(f, from, from + part.len(), prefix, ns);
                    from += part.len() + 1;
                }
                return;
            }
            let text = p.src[at..to].to_string();
            if let Some(name) = dotted(&text) {
                let router = name.strip_suffix(".urls").filter(|r| !r.contains('.'));
                if router.is_some_and(|r| self.router(f, r, prefix, ns)) {
                    return;
                }
                if !name.contains('.') && self.var(f, name, prefix, ns) {
                    return;
                }
            }
            let entry = p.call(at, to).filter(|(callee, _)| {
                matches!(*callee, "path" | "re_path" | "url")
                    || callee.ends_with(".path")
                    || callee.ends_with(".re_path")
            });
            if let Some((callee, open)) = entry {
                let regex = simple(callee) != "path";
                return self.entry(f, open, regex, prefix, ns);
            }
            self.gap(
                f,
                at,
                format!(
                    "URL patterns from `{}` are not read; their routes are unknown.",
                    collapse(&text)
                ),
            );
        }

        /// One `path(…)`/`re_path(…)` entry.
        fn entry(&mut self, f: usize, open: usize, regex: bool, prefix: &str, ns: &[String]) {
            let pr = self.p;
            let p = &pr.pys[f];
            let args = p.args(open);
            let arg = |i: usize, kw: &[&str]| {
                (args
                    .iter()
                    .find(|a| a.name.is_some_and(|n| kw.contains(&n))))
                .or_else(|| args.iter().filter(|a| a.name.is_none()).nth(i))
                .map(|a| p.span(a.value))
            };
            let (Some(route), Some(view)) = (arg(0, &["route", "regex"]), arg(1, &["view"])) else {
                return self.gap(f, open, "URL entry without route or view; not read.".into());
            };
            let name = arg(3, &["name"]).and_then(|(a, b)| py::string_value(&p.src[a..b]));
            let line = p.line(route.0);
            let include = (p.call(view.0, view.1))
                .filter(|(c, _)| simple(c) == "include")
                .map(|(_, o)| o);
            let Some(piece) = self.piece(f, route, regex, include.is_some()) else {
                return;
            };
            let path = format!("{prefix}{piece}");
            if let Some(open) = include {
                return self.include(f, open, &path, ns);
            }
            let view_text = collapse(&p.src[view.0..view.1]);
            if view_text.ends_with("site.urls") {
                let message = format!("Routes of `{view_text}` (Django admin) are not read.");
                return note(self.flow, &p.rel, line, message);
            }
            match view_ref(pr, f, view) {
                Some((target, kind)) => self.routes.push(Route {
                    path,
                    name: qualify(ns, name),
                    target,
                    kind,
                    file: f,
                    line,
                }),
                None => {
                    let message = format!("View `{view_text}` is not read; this route is unknown.");
                    self.gap(f, view.0, message);
                }
            }
        }

        /// The route text at `route` as a path piece; none (and a gap) when
        /// unreadable. A non-literal prefix of an `include` reads as empty.
        fn piece(
            &mut self,
            f: usize,
            route: (usize, usize),
            regex: bool,
            include: bool,
        ) -> Option<String> {
            let text = collapse(&self.p.pys[f].src[route.0..route.1]);
            let piece = match py::string_value(&self.p.pys[f].src[route.0..route.1]) {
                Some(r) if regex => regex_route(&r).ok_or(format!(
                    "Route regex `{r}` is not simple (only `^`, `$`, literal text and `(?P<name>…)` are read); this route is unknown."
                )),
                Some(r) => Ok(path_route(&r)),
                None if include => {
                    let message = format!("Route prefix `{text}` is not a literal; the routes below are read without it.");
                    self.gap(f, route.0, message);
                    Ok(String::new())
                }
                None => Err(format!("Route `{text}` is not a literal; this route is unknown.")),
            };
            piece.map_err(|message| self.gap(f, route.0, message)).ok()
        }

        fn push(&mut self, route: Route) {
            self.routes.push(route);
        }

        /// `include(…)` below `path`.
        fn include(&mut self, f: usize, open: usize, path: &str, ns: &[String]) {
            let p = &self.p.pys[f];
            let args = p.args(open);
            let positional: Vec<(usize, usize)> = (args.iter())
                .filter(|a| a.name.is_none())
                .map(|a| p.span(a.value))
                .collect();
            let Some(&(mut a, mut b)) = positional.first() else {
                return;
            };
            let mut namespace = (args.iter())
                .find(|x| x.name == Some("namespace"))
                .and_then(|x| py::string_value(x.value));
            if let Some(tuple) = p
                .elements(a, b)
                .filter(|t| t.len() == 2 && p.src[a..].starts_with('('))
            {
                namespace = namespace.or_else(|| py::string_value(&p.src[tuple[1].0..tuple[1].1]));
                (a, b) = tuple[0];
            }
            let text = p.src[a..b].to_string();
            let Some(module) = py::string_value(&text) else {
                let ns: Vec<String> = ns.iter().cloned().chain(namespace).collect();
                return self.expr(f, a, b, path, &ns);
            };
            let rel = p.rel.clone();
            match py::resolve_module(&rel, &module, &self.p.rels) {
                Some(file) => {
                    let g = self.p.by_rel[&file];
                    let namespace = namespace.or_else(|| self.p.pys[g].module_literal("app_name"));
                    let ns: Vec<String> = ns.iter().cloned().chain(namespace).collect();
                    self.conf(g, path, &ns);
                }
                None => {
                    let line = p.line(a);
                    note(
                        self.flow,
                        &rel,
                        line,
                        format!(
                            "Routes of `{module}` are not in the scan (an installed package); not read."
                        ),
                    );
                }
            }
        }

        /// The registrations of router variable `var` of file `f`; false when there is none.
        fn router(&mut self, f: usize, var: &str, prefix: &str, ns: &[String]) -> bool {
            let p = self.p.pys.get(f).expect("file");
            let ctor = (p.module_stmts())
                .filter_map(|s| py::assignment(&p.src, &p.masked, s))
                .filter(|a| a.target == var)
                .last()
                .and_then(|a| p.call(a.value_at, a.value_at + a.value.len()));
            let Some((_, open)) = ctor else { return false };
            let ctor_args = p.args(open);
            if ctor_args.iter().any(|a| a.name.is_none()) {
                self.gap(f, open, format!("Router `{var}` is built from other routers (nested routes); its routes are not read."));
                return true;
            }
            let slash = match ctor_args.iter().find(|a| a.name == Some("trailing_slash")) {
                Some(a) if a.value == "False" => "",
                _ => "/",
            };
            let call = format!("{var}.register(");
            let regs: Vec<usize> = (p.module_stmts())
                .filter(|s| p.masked[s.at..s.end].starts_with(&call))
                .map(|s| s.at + call.len() - 1)
                .collect();
            for open in regs {
                let p = &self.p.pys[f];
                let args = p.args(open);
                let arg = |i: usize, kw: &str| {
                    (args.iter().find(|a| a.name == Some(kw)))
                        .or_else(|| args.iter().filter(|a| a.name.is_none()).nth(i))
                        .map(|a| a.value)
                };
                let (Some(route), Some(viewset)) = (arg(0, "prefix"), arg(1, "viewset")) else {
                    continue;
                };
                let line = p.line(open);
                let Some(route) = py::string_value(route) else {
                    let message = format!(
                        "Router prefix `{}` is not a literal; these routes are unknown.",
                        collapse(route)
                    );
                    self.gap(f, open, message);
                    continue;
                };
                let Some(viewset) = dotted(viewset) else {
                    let message = format!(
                        "ViewSet `{}` is not a name; these routes are unknown.",
                        collapse(viewset)
                    );
                    self.gap(f, open, message);
                    continue;
                };
                let basename = arg(2, "basename").and_then(py::string_value);
                let path = format!(
                    "{prefix}{}",
                    route.trim_start_matches('^').trim_matches('/')
                );
                let target = self.p.resolve(f, viewset);
                self.push(Route {
                    path,
                    name: qualify(ns, None),
                    target,
                    kind: Kind::ViewSet {
                        slash,
                        basename,
                        namespace: ns.to_vec(),
                    },
                    file: f,
                    line,
                });
            }
            true
        }
    }

    /// The view at `view` in file `f`: its target, the `(` of `.as_view(…)`
    /// and the wrapper decorators around it (`login_required(v)`,
    /// `cache_page(…)(v)`); none when it is not a name or such a call.
    fn view_ref(pr: &Project, f: usize, view: (usize, usize)) -> Option<(Sym, Kind)> {
        let p = &pr.pys[f];
        let (mut a, mut b) = view;
        let mut wrappers = vec![];
        while let Some((callee, open)) = p.call(a, b) {
            if let Some(class) = callee.strip_suffix(".as_view") {
                return Some((
                    pr.resolve(f, class),
                    Kind::View {
                        as_view: Some(open),
                        wrappers,
                    },
                ));
            }
            let args = p.args(open);
            let [arg] = args.as_slice() else { return None };
            wrappers.push(p.span(&p.src[a..open]));
            (a, b) = p.span(arg.value);
        }
        Some((
            pr.resolve(f, dotted(&p.src[a..b])?),
            Kind::View {
                as_view: None,
                wrappers,
            },
        ))
    }

    /// `ns1:ns2:name`.
    pub(super) fn qualify(ns: &[String], name: Option<String>) -> Option<String> {
        name.map(|n| {
            ns.iter()
                .map(String::as_str)
                .chain([n.as_str()])
                .collect::<Vec<_>>()
                .join(":")
        })
    }

    /// `x/<int:pk>/` as `x/:pk/`.
    pub(super) fn path_route(route: &str) -> String {
        let mut out = String::new();
        let mut rest = route;
        while let Some(open) = rest.find('<') {
            let Some(close) = rest[open..].find('>').map(|c| open + c) else {
                break;
            };
            out.push_str(&rest[..open]);
            let inner = &rest[open + 1..close];
            out.push(':');
            out.push_str(inner.rsplit(':').next().unwrap_or(inner));
            rest = &rest[close + 1..];
        }
        out.push_str(rest);
        out
    }

    /// A regex route of literal text and `(?P<name>…)` groups as a path; none otherwise.
    pub(super) fn regex_route(regex: &str) -> Option<String> {
        let r = regex.strip_prefix('^').unwrap_or(regex);
        let r = r
            .strip_suffix('$')
            .or_else(|| r.strip_suffix("\\Z"))
            .unwrap_or(r);
        let (b, mut out, mut i) = (r.as_bytes(), String::new(), 0);
        while i < b.len() {
            match b[i] {
                b'\\' if b.get(i + 1).is_some_and(|c| b"./-_~".contains(c)) => {
                    out.push(char::from(b[i + 1]));
                    i += 2;
                }
                b'(' if r[i..].starts_with("(?P<") => {
                    let close = matching(b, i)?;
                    let name_end = r[i..close].find('>')? + i;
                    if r[name_end..close].contains('(') {
                        return None;
                    }
                    out.push(':');
                    out.push_str(&r[i + 4..name_end]);
                    i = close + 1;
                }
                c if b".*+?[]{}()|\\^$".contains(&c) => return None,
                _ => {
                    let ch = r[i..].chars().next()?;
                    out.push(ch);
                    i += ch.len_utf8();
                }
            }
        }
        Some(out)
    }
}

/// Views: screens and endpoints of each route.
mod views {
    use super::*;
    use urls::{Kind, Route};

    /// A screen with what its links come from.
    pub(super) struct ScreenInfo {
        pub(super) screen: Screen,
        /// Template names with where they were named: (name, file, line).
        pub(super) templates: Vec<(String, String, u32)>,
        /// Code whose redirects leave the screen: (file, from, to).
        pub(super) scopes: Vec<(usize, usize, usize)>,
    }

    /// What a URL name or route leads to.
    #[derive(Clone, Debug)]
    pub(super) enum Named {
        Screen(String),
        Endpoint(String),
    }

    #[derive(Default)]
    pub(super) struct Built {
        pub(super) screens: BTreeMap<String, ScreenInfo>,
        pub(super) names: HashMap<String, Named>,
        /// Every screen route: (screen id, route).
        pub(super) routes: Vec<(String, String)>,
        pub(super) endpoint_paths: Vec<String>,
    }

    const GENERIC: [(&str, &str); 7] = [
        ("TemplateView", ""),
        ("ListView", "_list"),
        ("DetailView", "_detail"),
        ("CreateView", "_form"),
        ("UpdateView", "_form"),
        ("DeleteView", "_confirm_delete"),
        ("FormView", ""),
    ];
    pub(super) const VIEWSETS: [&str; 5] = [
        "ViewSet",
        "GenericViewSet",
        "ModelViewSet",
        "ReadOnlyModelViewSet",
        "ViewSetMixin",
    ];
    /// DRF generic API views and the methods they answer.
    pub(super) const API_GENERICS: [(&str, &[&str]); 11] = [
        ("APIView", &[]),
        ("GenericAPIView", &[]),
        ("ListAPIView", &["GET"]),
        ("CreateAPIView", &["POST"]),
        ("RetrieveAPIView", &["GET"]),
        ("DestroyAPIView", &["DELETE"]),
        ("UpdateAPIView", &["PUT", "PATCH"]),
        ("ListCreateAPIView", &["GET", "POST"]),
        ("RetrieveUpdateAPIView", &["GET", "PUT", "PATCH"]),
        ("RetrieveDestroyAPIView", &["GET", "DELETE"]),
        (
            "RetrieveUpdateDestroyAPIView",
            &["GET", "PUT", "PATCH", "DELETE"],
        ),
    ];
    /// ViewSet actions: (action, detail, method).
    pub(super) const ACTIONS: [(&str, bool, &str); 6] = [
        ("list", false, "GET"),
        ("create", false, "POST"),
        ("retrieve", true, "GET"),
        ("update", true, "PUT"),
        ("partial_update", true, "PATCH"),
        ("destroy", true, "DELETE"),
    ];
    /// Bases that bring actions: (base, actions).
    const ACTION_BASES: [(&str, &[&str]); 7] = [
        (
            "ModelViewSet",
            &[
                "list",
                "create",
                "retrieve",
                "update",
                "partial_update",
                "destroy",
            ],
        ),
        ("ReadOnlyModelViewSet", &["list", "retrieve"]),
        ("ListModelMixin", &["list"]),
        ("CreateModelMixin", &["create"]),
        ("RetrieveModelMixin", &["retrieve"]),
        ("UpdateModelMixin", &["update", "partial_update"]),
        ("DestroyModelMixin", &["destroy"]),
    ];
    pub(super) const HTTP: [&str; 5] = ["get", "post", "put", "patch", "delete"];

    /// One endpoint row before it is emitted.
    struct Ep {
        method: String,
        path: String,
        access: AccessState,
        handler: String,
        file: usize,
        line: u32,
        name: Option<String>,
    }

    impl Project<'_> {
        pub(super) fn build(&self, routes: &[Route], flow: &mut UiFlow) -> Built {
            let mut built = Built::default();
            for r in routes {
                let path = format!("/{}", r.path);
                match &r.kind {
                    Kind::ViewSet {
                        slash, basename, ..
                    } => {
                        for ep in self.viewset(
                            r,
                            slash,
                            basename.as_deref(),
                            (Decl::default(), None),
                            flow,
                        ) {
                            self.endpoint(ep, &mut built, flow);
                        }
                    }
                    Kind::View { as_view, wrappers } => {
                        let rf = &self.pys[r.file];
                        // Wrappers apply outside the view's own decorators.
                        let mut outer = Effects::default();
                        for &(a, b) in wrappers {
                            self.decorate(r.file, a, b, &mut outer);
                        }
                        let kwargs = as_view.map(|o| rf.args(o)).unwrap_or_default();
                        match self.view(r, &path, &kwargs, outer, flow) {
                            Outcome::Screen(mut info) => {
                                let id = info.screen.id.clone();
                                if let Some(n) = &r.name {
                                    built
                                        .names
                                        .entry(n.clone())
                                        .or_insert(Named::Screen(id.clone()));
                                }
                                built.routes.push((id.clone(), path.clone()));
                                info.screen.route.get_or_insert(path);
                                built.screens.entry(id).or_insert(*info);
                            }
                            Outcome::Endpoints(eps) => {
                                for ep in eps {
                                    self.endpoint(ep, &mut built, flow);
                                }
                            }
                        }
                    }
                }
            }
            built
        }

        fn endpoint(&self, ep: Ep, built: &mut Built, flow: &mut UiFlow) {
            if let Some(n) = ep.name {
                built
                    .names
                    .entry(n)
                    .or_insert(Named::Endpoint(ep.path.clone()));
            }
            built.endpoint_paths.push(ep.path.clone());
            flow.endpoints.push(EndpointAccess {
                method: ep.method,
                path: ep.path,
                access: ep.access,
                application: Some(self.app.clone()),
                file: self.pys[ep.file].rel.clone(),
                line: ep.line,
                source: SOURCE.into(),
                handler: Some(ep.handler),
                sends: vec![],
            });
        }

        /// The screen or endpoints of a routed view.
        fn view(
            &self,
            r: &Route,
            path: &str,
            kwargs: &[Arg],
            outer: Effects,
            flow: &mut UiFlow,
        ) -> Outcome {
            let as_view = matches!(
                r.kind,
                Kind::View {
                    as_view: Some(_),
                    ..
                }
            );
            let (handler, file, line, class) = match &r.target {
                Sym::Def(f, d) => {
                    let p = &self.pys[*f];
                    let def = &p.defs[*d];
                    (
                        format!("{}.{}", p.module, def.name),
                        *f,
                        def.line,
                        def.kind == DefKind::Class,
                    )
                }
                Sym::External(s) => (s.clone(), r.file, r.line, as_view),
                Sym::Module(_) => {
                    let message = "The view is a module; this route is unknown.";
                    gap(
                        &mut flow.diagnostics,
                        &self.pys[r.file].rel,
                        r.line,
                        message,
                    );
                    return Outcome::Endpoints(vec![]);
                }
            };
            let site = Site {
                r,
                path,
                handler,
                file,
                line,
            };
            if !class {
                return self.function_view(&site, outer);
            }
            let mro = self.mro(&r.target);
            let names = self.names(&mro);
            let has = |n: &str| names.iter().any(|x| x == n);
            // A ViewSet routed by `as_view({'get': 'list'})`.
            if VIEWSETS.iter().any(|v| has(v)) {
                let mapping = kwargs.iter().find(|a| a.name.is_none());
                let mapping = mapping.map(|a| (r.file, self.pys[r.file].span(a.value)));
                return Outcome::Endpoints(self.viewset(r, "", None, (outer.decl, mapping), flow));
            }
            if API_GENERICS.iter().any(|(n, _)| has(n)) {
                let mut fx = Effects::default();
                self.class_decl(&mro, &mut fx);
                fx.decl.extend(outer.decl);
                let eps = (self.api_methods(&mro, &names).into_iter())
                    .map(|(m, at)| {
                        let handler = format!("{}.{}", site.handler, m.to_ascii_lowercase());
                        let mut decl = fx.for_method(&m);
                        decl.extend(self.drf_decl(&mro, None, &m));
                        site.ep(&m, &decl, handler, at.unwrap_or(site.at()))
                    })
                    .collect();
                return Outcome::Endpoints(eps);
            }
            let mut fx = Effects::default();
            self.class_decl(&mro, &mut fx);
            fx.decl.extend(outer.decl);
            self.class_view(&site, &mro, &names, kwargs, &fx)
        }

        /// A Django class view: a screen when it renders a template, else endpoints.
        fn class_view(
            &self,
            site: &Site,
            mro: &[Sym],
            names: &[String],
            kwargs: &[Arg],
            fx: &Effects,
        ) -> Outcome {
            let rf = &self.pys[site.r.file];
            let generic = GENERIC.iter().find(|(n, _)| names.iter().any(|x| x == n));
            let kw = kwargs.iter().find(|a| a.name == Some("template_name"));
            let templates = match kw {
                Some(a) => {
                    let line = rf.line(rf.span(a.value).0);
                    vec![(
                        py::string_value(a.value).unwrap_or_default(),
                        rf.rel.clone(),
                        line,
                    )]
                }
                None => self.class_templates(site, mro, generic.map(|g| g.1)),
            };
            if templates.is_empty() && generic.is_none() {
                return Outcome::Endpoints(self.plain_endpoints(site, mro, fx));
            }
            let template = kw.and_then(|a| py::string_value(a.value));
            let suffix = template.map(|t| format!("({t})")).unwrap_or_default();
            let scopes = (mro.iter())
                .filter_map(|s| match s {
                    Sym::Def(f, d) => Some((*f, self.pys[*f].defs[*d].body)),
                    _ => None,
                })
                .map(|(f, (a, b))| (f, a, b))
                .collect();
            Outcome::Screen(Box::new(self.screen_info(
                site,
                &suffix,
                &fx.for_method("get"),
                templates,
                scopes,
            )))
        }

        /// Templates a class view renders; an empty name stands for an unreadable one.
        fn class_templates(
            &self,
            site: &Site,
            mro: &[Sym],
            generic: Option<&str>,
        ) -> Vec<(String, String, u32)> {
            let mut templates = vec![];
            for (_, f, d) in self.methods(mro) {
                self.render_templates(f, self.pys[f].defs[d].body, mro, &mut templates);
            }
            let Some(suffix) = generic.filter(|_| templates.is_empty()) else {
                return templates;
            };
            templates.push(match self.attr(mro, "template_name") {
                Some((f, v, at)) => {
                    let p = &self.pys[f];
                    (
                        py::string_value(v).unwrap_or_default(),
                        p.rel.clone(),
                        p.line(at),
                    )
                }
                None => {
                    let name = self.default_template(mro, suffix).unwrap_or_default();
                    (name, self.pys[site.file].rel.clone(), site.line)
                }
            });
            templates
        }

        /// A class view without template: one endpoint per handler method, else `ANY`.
        fn plain_endpoints(&self, site: &Site, mro: &[Sym], fx: &Effects) -> Vec<Ep> {
            let methods: Vec<Ep> = (self.methods(mro).into_iter())
                .filter(|(n, _, _)| HTTP.contains(&n.as_str()))
                .map(|(m, f, d)| {
                    let at = (f, self.pys[f].defs[d].line);
                    site.ep(
                        &m.to_ascii_uppercase(),
                        &fx.for_method(&m),
                        format!("{}.{m}", site.handler),
                        at,
                    )
                })
                .collect();
            if !methods.is_empty() {
                return methods;
            }
            vec![site.ep(ANY_METHOD, &fx.decl, site.handler.clone(), site.at())]
        }

        /// A function view: DRF `@api_view` endpoints, a screen when it renders, else endpoints.
        fn function_view(&self, site: &Site, outer: Effects) -> Outcome {
            let Sym::Def(f, d) = site.r.target else {
                // A function of an installed package: no declarations readable.
                return Outcome::Endpoints(vec![site.ep(
                    ANY_METHOD,
                    &outer.decl,
                    site.handler.clone(),
                    site.at(),
                )]);
            };
            let p = &self.pys[f];
            let mut fx = Effects::default();
            for &(a, b) in &p.defs[d].decorators {
                self.decorate(f, a, b, &mut fx);
            }
            fx.decl.extend(outer.decl);
            let methods = fx.methods.clone();
            if let (true, Some((pf, at))) = (fx.api_view, fx.perms_above) {
                let message =
                    "`@permission_classes` above `@api_view` is ignored by DRF; it must sit below.";
                self.finding(pf, self.pys[pf].line(at), false, message.into());
            }
            if fx.api_view {
                let eps = (methods.iter().flatten())
                    .map(|m| {
                        let mut decl = fx.decl.clone();
                        decl.extend(self.drf_decl(&[], fx.perms, m));
                        site.ep(m, &decl, site.handler.clone(), site.at())
                    })
                    .collect();
                return Outcome::Endpoints(eps);
            }
            let (mut templates, body) = (vec![], p.defs[d].body);
            self.render_templates(f, body, &[], &mut templates);
            if templates.is_empty() {
                let methods = methods.unwrap_or_else(|| vec![ANY_METHOD.into()]);
                let eps = methods
                    .iter()
                    .map(|m| site.ep(m, &fx.decl, site.handler.clone(), site.at()));
                return Outcome::Endpoints(eps.collect());
            }
            let info = self.screen_info(site, "", &fx.decl, templates, vec![(f, body.0, body.1)]);
            Outcome::Screen(Box::new(info))
        }

        fn screen_info(
            &self,
            site: &Site,
            suffix: &str,
            decl: &Decl,
            templates: Vec<(String, String, u32)>,
            scopes: Vec<(usize, usize, usize)>,
        ) -> ScreenInfo {
            let handler = &site.handler;
            ScreenInfo {
                screen: Screen {
                    weak: false,
                    id: format!("{}:{handler}{suffix}", self.app),
                    name: format!("{}{suffix}", simple(handler)),
                    kind: ScreenKind::Page,
                    route: None,
                    application: Some(self.app.clone()),
                    file: self.pys[site.file].rel.clone(),
                    line: site.line,
                    access: archguard_core::AccessState::from_parts(decl.condition(), false),
                    source: SOURCE.into(),
                },
                templates,
                scopes,
            }
        }

        /// Templates named by `render`-like calls in `body` of file `f`;
        /// `self.x` reads attribute `x` of `mro`.
        fn render_templates(
            &self,
            f: usize,
            body: (usize, usize),
            mro: &[Sym],
            templates: &mut Vec<(String, String, u32)>,
        ) {
            let p = &self.pys[f];
            for (name, open) in p.calls_in(
                body.0,
                body.1,
                &["render", "TemplateResponse", "render_to_response"],
            ) {
                let args = p.args(open);
                let (index, kw) = match name {
                    "render" => (1, "template_name"),
                    "TemplateResponse" => (1, "template"),
                    _ => (0, "template_name"),
                };
                let arg = (args.iter().find(|a| a.name == Some(kw)))
                    .or_else(|| args.iter().filter(|a| a.name.is_none()).nth(index));
                let Some(arg) = arg else { continue };
                let (a, b) = p.span(arg.value);
                let attr = (arg
                    .value
                    .strip_prefix("self.")
                    .or_else(|| arg.value.strip_prefix("cls.")))
                .filter(|n| dotted(n).is_some_and(|n| !n.contains('.')))
                .and_then(|n| self.attr(mro, n));
                let found = match attr {
                    Some((g, v, at)) => {
                        (self.pys[g].literals(at, at + v.len())).map(|t| (t, g, at))
                    }
                    None => p.literals(a, b).map(|t| (t, f, a)),
                };
                match found {
                    Some((names, g, at)) if !names.is_empty() => {
                        // `select_template` order: the first that exists, else the first.
                        let pick = (names.iter())
                            .find(|n| self.templates.contains_key(*n))
                            .unwrap_or(&names[0]);
                        templates.push((
                            pick.clone(),
                            self.pys[g].rel.clone(),
                            self.pys[g].line(at),
                        ));
                    }
                    // An unreadable template: an empty name.
                    _ => templates.push((String::new(), p.rel.clone(), p.line(a))),
                }
            }
        }

        /// `<app>/<model><suffix>.html` from `model` or `queryset = Model.objects…`.
        fn default_template(&self, mro: &[Sym], generic_suffix: &str) -> Option<String> {
            let suffix = match self.attr(mro, "template_name_suffix") {
                Some((_, v, _)) => py::string_value(v)?,
                None => generic_suffix.to_string(),
            };
            let (f, model) = match self.attr(mro, "model") {
                Some((f, v, _)) => (f, dotted(v)?.to_string()),
                None => {
                    let (f, v, _) = self.attr(mro, "queryset")?;
                    (f, v.split(".objects").next().and_then(dotted)?.to_string())
                }
            };
            let Sym::Def(g, d) = self.resolve(f, &model) else {
                return None;
            };
            let rel = &self.pys[g].rel;
            let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
            // `app/models.py` or `app/models/<x>.py`.
            let app_dir = if rel.ends_with("/models.py") {
                dir
            } else {
                dir.strip_suffix("/models")?
            };
            let app = app_dir.rsplit('/').next().unwrap_or(app_dir);
            Some(format!(
                "{app}/{}{suffix}.html",
                self.pys[g].defs[d].name.to_lowercase()
            ))
        }

        /// Methods of a DRF API view: (METHOD, line of the handler).
        fn api_methods(
            &self,
            mro: &[Sym],
            names: &[String],
        ) -> Vec<(String, Option<(usize, u32)>)> {
            let mut out: Vec<(String, Option<(usize, u32)>)> = (self.methods(mro).into_iter())
                .filter(|(n, _, _)| HTTP.contains(&n.as_str()))
                .map(|(n, f, d)| (n.to_ascii_uppercase(), Some((f, self.pys[f].defs[d].line))))
                .collect();
            for (base, methods) in API_GENERICS {
                if names.iter().any(|n| n == base) {
                    for m in methods {
                        if !out.iter().any(|o| o.0 == *m) {
                            out.push((m.to_string(), None));
                        }
                    }
                }
            }
            if out.is_empty() {
                out.push((ANY_METHOD.into(), None));
            }
            out
        }

        /// Endpoints of a ViewSet: router routes, or the `as_view` mapping at `mapping`.
        fn viewset(
            &self,
            r: &Route,
            slash: &str,
            basename: Option<&str>,
            (outer, mapping): (Decl, Option<FileSpan>),
            flow: &mut UiFlow,
        ) -> Vec<Ep> {
            let Some(vs) = self.viewset_context(r, slash, basename, outer) else {
                let name = self.simple_name(&r.target);
                let message =
                    format!("ViewSet `{name}` is not in the scan; its routes are unknown.");
                gap(
                    &mut flow.diagnostics,
                    &self.pys[r.file].rel,
                    r.line,
                    message,
                );
                return vec![];
            };
            match mapping {
                Some((mf, span)) => self.mapped_actions(&vs, mf, span, flow),
                None => {
                    let mut out = self.standard_actions(&vs);
                    out.extend(self.extra_actions(&vs, flow));
                    out
                }
            }
        }

        fn viewset_context<'r>(
            &self,
            r: &'r Route,
            slash: &'r str,
            basename: Option<&str>,
            outer: Decl,
        ) -> Option<Vs<'r>> {
            let Sym::Def(vf, vd) = r.target else {
                return None;
            };
            let vp = &self.pys[vf];
            let mro = self.mro(&r.target);
            // Django mixins, decorators and `urls.py` wrappers run outside DRF's checks.
            let mut fx = Effects::default();
            self.class_decl(&mro, &mut fx);
            fx.decl.extend(outer);
            let lookup = (self.attr(&mro, "lookup_url_kwarg"))
                .or_else(|| self.attr(&mro, "lookup_field"))
                .and_then(|(_, v, _)| py::string_value(v))
                .unwrap_or_else(|| "pk".into());
            let basename = basename.map(String::from).or_else(|| {
                let (_, v, _) = self.attr(&mro, "queryset")?;
                Some(simple(v.split(".objects").next()?.trim()).to_lowercase())
            });
            let ns: &[String] = match &r.kind {
                Kind::ViewSet { namespace, .. } => namespace,
                Kind::View { .. } => &[],
            };
            Some(Vs {
                names: self.names(&mro),
                methods: self.methods(&mro),
                mro,
                handler: format!("{}.{}", vp.module, vp.defs[vd].name),
                home: (vf, vp.defs[vd].line),
                base: format!("/{}", r.path),
                lookup,
                slash,
                ns,
                basename,
                route_name: r.name.clone(),
                layer: fx.decl,
                by_action: fx.by_method,
            })
        }

        /// One ViewSet endpoint row; `name` is set by the caller.
        fn vs_ep(
            &self,
            vs: &Vs,
            (method, action): (&str, &str),
            path: &str,
            perms: Option<(usize, usize, usize)>,
            at: (usize, u32),
        ) -> Ep {
            let mut decl = vs.layer.clone();
            if let Some(d) = vs.by_action.get(action) {
                decl.extend(d.clone());
            }
            decl.extend(self.drf_decl(&vs.mro, perms, method));
            Ep {
                method: method.into(),
                path: path.into(),
                access: decl.state(),
                handler: format!("{}.{action}", vs.handler),
                file: at.0,
                line: at.1,
                name: None,
            }
        }

        /// `as_view({'get': 'list', …})`.
        fn mapped_actions(
            &self,
            vs: &Vs,
            mf: usize,
            (a, b): (usize, usize),
            flow: &mut UiFlow,
        ) -> Vec<Ep> {
            let mp = &self.pys[mf];
            let dict = mp.src[a..b].starts_with('{') && mp.src[a..b].ends_with('}');
            let pairs = match dict {
                true => py::split_commas(&mp.masked, a + 1, b - 1),
                false => vec![],
            };
            let out: Vec<Ep> = (pairs.into_iter())
                .filter_map(|(x, y)| {
                    let colon = mp.masked[x..y].find(':')? + x;
                    let m = py::string_value(&mp.src[x..colon])?;
                    let act = py::string_value(&mp.src[colon + 1..y])?;
                    let ep = self.vs_ep(
                        vs,
                        (&m.to_ascii_uppercase(), &act),
                        &vs.base,
                        None,
                        vs.at(self, &act),
                    );
                    Some(Ep {
                        name: vs.route_name.clone(),
                        ..ep
                    })
                })
                .collect();
            if out.is_empty() {
                let message =
                    "ViewSet `as_view` mapping is not a literal dict; this route is unknown.";
                gap(&mut flow.diagnostics, &mp.rel, mp.line(a), message);
            }
            out
        }

        /// `list`, `create`, `retrieve`, `update`, `partial_update`, `destroy`
        /// as far as the class defines them or its bases bring them.
        fn standard_actions(&self, vs: &Vs) -> Vec<Ep> {
            let has = |n: &str| vs.names.iter().any(|x| x == n);
            let list_path = format!("{}{}", vs.base, vs.slash);
            let detail_path = format!("{}/:{}{}", vs.base, vs.lookup, vs.slash);
            (ACTIONS.into_iter())
                .filter(|(action, _, _)| {
                    vs.defined(self, action).is_some()
                        || ACTION_BASES
                            .iter()
                            .any(|(b, acts)| has(b) && acts.contains(action))
                })
                .map(|(action, detail, method)| {
                    let (path, suffix) = if detail {
                        (&detail_path, "detail")
                    } else {
                        (&list_path, "list")
                    };
                    let ep = self.vs_ep(vs, (method, action), path, None, vs.at(self, action));
                    Ep {
                        name: vs.name(suffix),
                        ..ep
                    }
                })
                .collect()
        }

        /// Methods decorated with `@action(detail=…, methods=[…], url_path=…)`.
        fn extra_actions(&self, vs: &Vs, flow: &mut UiFlow) -> Vec<Ep> {
            let mut out = vec![];
            for (fname, f, d) in &vs.methods {
                let p = &self.pys[*f];
                for &(a, b) in &p.defs[*d].decorators {
                    let Some((_, open)) = p.call(a, b).filter(|(c, _)| simple(c) == "action")
                    else {
                        continue;
                    };
                    let spec = match action_spec(p, open, fname) {
                        Ok(spec) => spec,
                        Err(message) => {
                            gap(&mut flow.diagnostics, &p.rel, p.line(a), message);
                            continue;
                        }
                    };
                    let path = match spec.detail {
                        true => format!("{}/:{}/{}{}", vs.base, vs.lookup, spec.url_path, vs.slash),
                        false => format!("{}/{}{}", vs.base, spec.url_path, vs.slash),
                    };
                    let perms = spec.perms.map(|(x, y)| (*f, x, y));
                    for m in &spec.methods {
                        let ep = self.vs_ep(
                            vs,
                            (&m.to_ascii_uppercase(), fname),
                            &path,
                            perms,
                            (*f, p.defs[*d].line),
                        );
                        out.push(Ep {
                            name: vs.name(&spec.url_name),
                            ..ep
                        });
                    }
                }
            }
            out
        }
    }

    /// A file and the byte range in it.
    type FileSpan = (usize, (usize, usize));

    /// What one ViewSet's routes share.
    struct Vs<'r> {
        mro: Vec<Sym>,
        names: Vec<String>,
        methods: Vec<(String, usize, usize)>,
        handler: String,
        /// The class's own location.
        home: (usize, u32),
        base: String,
        lookup: String,
        slash: &'r str,
        ns: &'r [String],
        basename: Option<String>,
        route_name: Option<String>,
        /// Django-side access around DRF's own.
        layer: Decl,
        /// `method_decorator(name=…)` per handler or action.
        by_action: BTreeMap<String, Decl>,
    }

    impl Vs<'_> {
        /// Where action `a` is defined, if the class or a project base defines it.
        fn defined(&self, pr: &Project, a: &str) -> Option<(usize, u32)> {
            (self.methods.iter())
                .find(|m| m.0 == a)
                .map(|m| (m.1, pr.pys[m.1].defs[m.2].line))
        }

        fn at(&self, pr: &Project, a: &str) -> (usize, u32) {
            self.defined(pr, a).unwrap_or(self.home)
        }

        /// `ns:basename-suffix`.
        fn name(&self, suffix: &str) -> Option<String> {
            let name = self.basename.as_ref().map(|b| format!("{b}-{suffix}"));
            urls::qualify(self.ns, name)
        }
    }

    /// What an `@action(…)` declares.
    struct ActionSpec {
        detail: bool,
        methods: Vec<String>,
        url_path: String,
        url_name: String,
        perms: Option<(usize, usize)>,
    }

    /// The `@action` call whose `(` is at `open`; `Err` with the gap message when unreadable.
    fn action_spec(p: &project::Py, open: usize, fname: &str) -> Result<ActionSpec, String> {
        let args = p.args(open);
        let kw = |k: &str| {
            args.iter()
                .find(|x| x.name == Some(k))
                .map(|x| p.span(x.value))
        };
        let literal = |k: &str| kw(k).and_then(|(x, y)| py::string_value(&p.src[x..y]));
        let detail = match kw("detail").map(|(x, y)| &p.src[x..y]) {
            Some("True") => true,
            Some("False") => false,
            other => {
                let found = other.unwrap_or("missing");
                return Err(format!(
                    "@action `{fname}` has no literal `detail` ({found}); its route is unknown."
                ));
            }
        };
        let methods = match kw("methods") {
            Some((x, y)) => p.literals(x, y).ok_or(format!(
                "@action `{fname}` methods are not literal; its route is unknown."
            ))?,
            None => vec!["get".into()],
        };
        Ok(ActionSpec {
            detail,
            methods,
            url_path: literal("url_path").unwrap_or_else(|| fname.to_string()),
            url_name: literal("url_name").unwrap_or_else(|| fname.replace('_', "-")),
            perms: kw("permission_classes"),
        })
    }

    /// The routed view while its rows are built.
    struct Site<'r> {
        r: &'r Route,
        path: &'r str,
        handler: String,
        file: usize,
        line: u32,
    }

    impl Site<'_> {
        fn ep(&self, method: &str, decl: &Decl, handler: String, (file, line): (usize, u32)) -> Ep {
            Ep {
                method: method.into(),
                path: self.path.into(),
                access: decl.state(),
                handler,
                file,
                line,
                name: self.r.name.clone(),
            }
        }

        fn at(&self) -> (usize, u32) {
            (self.file, self.line)
        }
    }

    enum Outcome {
        Screen(Box<ScreenInfo>),
        Endpoints(Vec<Ep>),
    }
}

/// Django templates: inheritance, includes and links.
mod templates {
    use super::*;
    use crate::lang_markup::{attr, blank, tags};

    /// A template reference (`{% extends %}`, `{% include %}`): the literal name, or the text.
    #[derive(Debug, Clone)]
    pub(super) struct Ref {
        pub(super) name: Result<String, String>,
        pub(super) line: u32,
    }

    #[derive(Debug, Clone)]
    pub(super) enum Link {
        /// `{% url 'name' %}`; `Err` holds a tag with a variable name.
        Url(Result<String, String>, String),
        /// A literal rooted `href`.
        Path(String),
        /// A relative `href`.
        Relative(String),
    }

    #[derive(Debug, Clone, Default)]
    pub(super) struct Template {
        pub(super) extends: Option<Ref>,
        pub(super) includes: Vec<Ref>,
        pub(super) links: Vec<(Link, u32)>,
    }

    /// Words of a template tag; quoted strings stay one word.
    fn words(s: &str) -> Vec<&str> {
        let (b, mut out, mut i) = (s.as_bytes(), vec![], 0);
        while i < b.len() {
            if b[i].is_ascii_whitespace() {
                i += 1;
                continue;
            }
            let start = i;
            if matches!(b[i], b'\'' | b'"') {
                let q = b[i];
                i = (i + 1..b.len())
                    .find(|&k| b[k] == q)
                    .map_or(b.len(), |k| k + 1);
            } else {
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
            }
            out.push(&s[start..i]);
        }
        out
    }

    fn quoted(word: &str) -> Option<String> {
        let w = word.trim();
        let q = w.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
        (w.len() >= 2 && w.ends_with(q)).then(|| w[1..w.len() - 1].to_string())
    }

    pub(super) fn parse(text: &str) -> Template {
        let text = blank(text, "{#", "#}");
        let text = blank(&text, "{% comment", "{% endcomment %}");
        let text = blank(&text, "<!--", "-->");
        let mut t = Template::default();
        let mut from = 0;
        while let Some(open) = text[from..].find("{%").map(|o| from + o) {
            let Some(close) = text[open..].find("%}").map(|c| open + c) else {
                break;
            };
            from = close + 2;
            tag(&text, open, close, &mut t);
        }
        for a in tags(&text).filter(|t| t.name.eq_ignore_ascii_case("a") && !t.closing) {
            if let Some(link) = attr(a.attrs, "href").and_then(href_link) {
                t.links.push((link, line(&text, a.at)));
            }
        }
        t
    }

    /// The template tag `text[open..close + 2]`: `extends`, `include` or `url`.
    fn tag(text: &str, open: usize, close: usize, t: &mut Template) {
        let line = line(text, open);
        let w = words(&text[open + 2..close]);
        let first = w.get(1).copied().unwrap_or("");
        let whole = collapse(&text[open..close + 2]);
        let reference = || Ref {
            name: quoted(first).ok_or_else(|| whole.clone()),
            line,
        };
        match w.first().copied() {
            Some("extends") => t.extends = Some(reference()),
            Some("include") => t.includes.push(reference()),
            Some("url") => {
                let name = quoted(first).ok_or_else(|| whole.clone());
                t.links.push((Link::Url(name, whole.clone()), line));
            }
            _ => {}
        }
    }

    /// A literal `href` as a link; none for template output, fragments,
    /// queries, other schemes and other hosts.
    fn href_link(href: &str) -> Option<Link> {
        let href = href.trim();
        let skip = href.is_empty()
            || href.contains("{{")
            || href.contains("{%")
            || href.starts_with(['#', '?'])
            || href.starts_with("//")
            || href.contains(':');
        if skip {
            return None;
        }
        Some(match href.starts_with('/') {
            true => Link::Path(href.split(['?', '#']).next().unwrap_or(href).to_string()),
            false => Link::Relative(href.to_string()),
        })
    }

    impl Project<'_> {
        /// The one file of template `name`; a gap when there is none or several.
        pub(super) fn template_file(
            &self,
            name: &str,
            file: &str,
            line: u32,
            flow: &mut UiFlow,
        ) -> Option<&str> {
            match self.templates.get(name).map(Vec::as_slice) {
                Some([one]) => Some(one),
                Some(many) => {
                    gap(
                        &mut flow.diagnostics,
                        file,
                        line,
                        format!(
                            "Template `{name}` exists {} times ({}); which one renders depends on settings, so its links are unknown.",
                            many.len(),
                            many.join(", ")
                        ),
                    );
                    None
                }
                None => {
                    gap(
                        &mut flow.diagnostics,
                        file,
                        line,
                        format!("Template `{name}` is not in the scan; its links are unknown."),
                    );
                    None
                }
            }
        }

        /// Links of template file `rel` and the templates it includes, each file once.
        pub(super) fn links_of<'s>(
            &'s self,
            rel: &'s str,
            seen: &mut BTreeSet<String>,
            out: &mut Vec<(&'s str, &'s Link, u32)>,
            flow: &mut UiFlow,
        ) {
            if !seen.insert(rel.to_string()) {
                return;
            }
            let Some(t) = self.parsed.get(rel) else {
                return;
            };
            out.extend(t.links.iter().map(|(l, line)| (rel, l, *line)));
            for inc in &t.includes {
                match &inc.name {
                    Ok(name) => {
                        if let Some(file) = self.template_file(name, rel, inc.line, flow) {
                            self.links_of(file, seen, out, flow);
                        }
                    }
                    Err(text) => gap(
                        &mut flow.diagnostics,
                        rel,
                        inc.line,
                        format!(
                            "`{text}` names no literal template; the links it adds are unknown."
                        ),
                    ),
                }
            }
        }

        /// The templates `rel` extends, nearest first.
        pub(super) fn ancestors(&self, rel: &str, flow: &mut UiFlow) -> Vec<&str> {
            let mut out: Vec<&str> = vec![];
            let mut current = rel.to_string();
            while let Some(ext) = self.parsed.get(&current).and_then(|t| t.extends.clone()) {
                let next = match &ext.name {
                    Ok(name) => self.template_file(name, &current, ext.line, flow),
                    Err(text) => {
                        gap(
                            &mut flow.diagnostics,
                            &current,
                            ext.line,
                            format!(
                                "`{text}` names no literal template; the links of the base template are unknown."
                            ),
                        );
                        None
                    }
                };
                match next {
                    Some(n) if n != rel && !out.contains(&n) => {
                        out.push(n);
                        current = n.to_string();
                    }
                    _ => break,
                }
            }
            out
        }
    }
}

/// Builds screens, endpoints, transitions and link calls of one project.
impl Project<'_> {
    fn extract(&self, flow: &mut UiFlow) {
        let routes = urls::Reader::read(self, flow);
        let built = self.build(&routes, flow);
        for (f, line, name) in self.unresolved.take() {
            let message = format!(
                "Decorator `{name}` is not read; any access check it makes is unknown (declare it in [views.access])."
            );
            gap(&mut flow.diagnostics, &self.pys[f].rel, line, message);
        }
        for (f, line, is_gap, message) in self.findings.take() {
            let rel = &self.pys[f].rel;
            match is_gap {
                true => gap(&mut flow.diagnostics, rel, line, message),
                false => note(flow, rel, line, message),
            }
        }
        let mut bases_done = BTreeSet::new();
        for info in built.screens.values() {
            flow.screens.push(info.screen.clone());
            self.screen_links(&built, info, &mut bases_done, flow);
            let from = Origin::Screen(info.screen.id.clone());
            for &scope in &info.scopes {
                self.redirects(&built, &from, scope, flow);
            }
        }
    }

    /// Links of one screen's templates; those of its base templates (each
    /// once per project) start at every screen of the application.
    fn screen_links<'s>(
        &'s self,
        built: &views::Built,
        info: &views::ScreenInfo,
        bases_done: &mut BTreeSet<&'s str>,
        flow: &mut UiFlow,
    ) {
        let from = Origin::Screen(info.screen.id.clone());
        let any = Origin::AnyScreen {
            application: self.app.clone(),
        };
        for (name, file, line) in &info.templates {
            if name.is_empty() {
                let screen = &info.screen.name;
                let message = format!(
                    "Template of `{screen}` is not a literal; links of this screen are unknown."
                );
                gap(&mut flow.diagnostics, file, *line, message);
                continue;
            }
            let Some(rel) = self.template_file(name, file, *line, flow) else {
                continue;
            };
            let bases: Vec<&str> = self
                .ancestors(rel, flow)
                .into_iter()
                .filter(|b| bases_done.insert(b))
                .collect();
            let starts = std::iter::once((&from, rel)).chain(bases.into_iter().map(|b| (&any, b)));
            for (origin, template) in starts {
                let mut links = vec![];
                self.links_of(template, &mut BTreeSet::new(), &mut links, flow);
                for (file, link, line) in links {
                    self.link(built, origin, link, (file, line), flow);
                }
            }
        }
    }

    fn link(
        &self,
        built: &views::Built,
        from: &Origin,
        link: &templates::Link,
        at: (&str, u32),
        flow: &mut UiFlow,
    ) {
        use templates::Link;
        let target = match link {
            Link::Url(Ok(name), tag) => self.by_name(built, name, tag),
            Link::Url(Err(tag), _) => Err((
                tag.clone(),
                format!("`{tag}` names no literal URL; the target is unknown."),
            )),
            Link::Path(path) => self.by_path(built, path),
            Link::Relative(href) => Err((
                href.clone(),
                format!("Relative link `{href}` is not resolved; the target is unknown."),
            )),
        };
        self.emit(from, target, at, flow);
    }

    fn by_name(
        &self,
        built: &views::Built,
        name: &str,
        text: &str,
    ) -> Result<views::Named, (String, String)> {
        built.names.get(name).cloned().ok_or_else(|| {
            (
                text.to_string(),
                format!("URL name `{name}` is not among the routes read; the target is unknown."),
            )
        })
    }

    fn by_path(&self, built: &views::Built, path: &str) -> Result<views::Named, (String, String)> {
        match best_route(
            path,
            built.routes.iter().map(|(id, r)| (id.clone(), r.as_str())),
        ) {
            Ok(id) => Ok(views::Named::Screen(id)),
            Err(tied) if tied.is_empty() => (built.endpoint_paths.iter())
                .find(|e| path_patterns_match(e, path))
                .map(|e| views::Named::Endpoint(e.clone()))
                .ok_or_else(|| {
                    (
                        path.to_string(),
                        format!("No route read matches `{path}`; the target is unknown."),
                    )
                }),
            Err(tied) => Err((
                path.to_string(),
                format!(
                    "`{path}` matches several routes ({}); the target is unknown.",
                    tied.join(", ")
                ),
            )),
        }
    }

    fn emit(
        &self,
        from: &Origin,
        target: Result<views::Named, (String, String)>,
        (file, line): (&str, u32),
        flow: &mut UiFlow,
    ) {
        let transition = |target| Transition {
            weak: false,
            from: from.clone(),
            target,
            trigger: None,
            file: file.into(),
            line,
            condition: archguard_core::AccessState::Undeclared,
            source: SOURCE.into(),
        };
        match target {
            Ok(views::Named::Screen(id)) => flow.transitions.push(transition(Target::Screen(id))),
            Ok(views::Named::Endpoint(path)) => flow.actions.push(Action {
                weak: false,
                screen: from.clone(),
                trigger: None,
                call: Call::Http {
                    method: "GET".into(),
                    path,
                },
                file: file.into(),
                line,
                condition: archguard_core::AccessState::Undeclared,
                source: SOURCE.into(),
            }),
            Err((text, message)) => {
                flow.transitions.push(transition(Target::Unresolved(text)));
                gap(&mut flow.diagnostics, file, line, message);
            }
        }
    }

    /// `redirect`, `reverse`, `reverse_lazy` and literal `HttpResponseRedirect`
    /// in `src[a..b]` of file `f`.
    fn redirects(
        &self,
        built: &views::Built,
        from: &Origin,
        (f, a, b): (usize, usize, usize),
        flow: &mut UiFlow,
    ) {
        let p = &self.pys[f];
        let names = [
            "redirect",
            "reverse",
            "reverse_lazy",
            "HttpResponseRedirect",
        ];
        for (name, open) in p.calls_in(a, b, &names) {
            let Some(target) = self.redirect_target(built, p, name, open) else {
                continue;
            };
            // `reverse` only builds a URL: to a screen it reads as navigation, to an endpoint it claims no request.
            if name.starts_with("reverse") && matches!(target, Ok(views::Named::Endpoint(_))) {
                continue;
            }
            self.emit(from, target, (&p.rel, p.line(open)), flow);
        }
    }

    /// The target of the redirect call `name(` at `open`; none when it is
    /// left to another rule (a nested `reverse`) or no navigation.
    fn redirect_target(
        &self,
        built: &views::Built,
        p: &Py,
        name: &str,
        open: usize,
    ) -> Option<Result<views::Named, (String, String)>> {
        let args = p.args(open);
        let first = (args.iter()).find(|x| x.name.is_none() || x.name == Some("viewname"))?;
        let call_end = matching(p.masked.as_bytes(), open).map_or(open, |c| c + 1);
        let text = collapse(&p.src[open - name.len()..call_end]);
        let (x, y) = p.span(first.value);
        let nested = p
            .call(x, y)
            .is_some_and(|(c, _)| matches!(simple(c), "reverse" | "reverse_lazy"));
        match py::string_value(first.value) {
            Some(v) if v.starts_with('/') => {
                Some(self.by_path(built, v.split(['?', '#']).next().unwrap_or(&v)))
            }
            Some(v) if name != "HttpResponseRedirect" && !v.contains('/') => {
                Some(self.by_name(built, &v, &text))
            }
            Some(_) => None,
            // `redirect(reverse('x'))`: the inner call is read on its own.
            None if nested => None,
            None => {
                let message = format!("`{text}` has no literal target; the target is unknown.");
                Some(Err((text, message)))
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{AccessState, Call, Condition, DiagnosticSeverity, Origin, Target};

    fn write(dir: &Path, files: &[(&str, &str)]) {
        for (path, text) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    const SETTINGS: &str = "ROOT_URLCONF = 'shop.urls'\nREST_FRAMEWORK = {\n    'DEFAULT_PERMISSION_CLASSES': [\n        'rest_framework.permissions.IsAuthenticated',\n    ],\n}\n";

    const ROOT_URLS: &str = r#"from django.urls import include, path, re_path
from django.contrib.auth.decorators import login_required
from orders import views as order_views

urlpatterns = [
    # Base views: a comment before an entry
    path('', order_views.home, name='home'),
    path('orders/', include('orders.urls')),
    path('api/', include('orders.api_urls')),
    re_path(r'^archive/(?P<year>[0-9]{4})/$', order_views.archive, name='archive'),
    re_path(r'^x/(a|b)/$', order_views.archive),
    path('secret/', login_required(order_views.secret), name='secret'),
    path('dyn/', include(make_urls())),
    path('accounts/', include('allauth.urls')),
]
"#;

    const ORDER_URLS: &str = r#"from django.urls import path
from django.views.generic import TemplateView
from . import views

app_name = 'orders'
urlpatterns = [
    path('', views.OrderListView.as_view(), name='list'),
    path('<int:pk>/', views.OrderDetailView.as_view(), name='detail'),
    path('about/', TemplateView.as_view(template_name='orders/about.html'), name='about'),
    path('<int:pk>/cancel/', views.cancel, name='cancel'),
]
"#;

    const VIEWS: &str = r#"from django.contrib.auth.decorators import login_required, permission_required, user_passes_test
from django.contrib.auth.mixins import LoginRequiredMixin, PermissionRequiredMixin
from django.shortcuts import render, redirect
from django.views.generic import ListView, DetailView
from .models import Order


def home(request):
    return render(request, 'home.html')


@login_required
def archive(request, year):
    if not year:
        return redirect('home')
    return render(request, "orders/archive.html", {})


@user_passes_test(is_manager)
def secret(request):
    if request.GET:
        return redirect(reverse('orders:about'))
    return render(request, 'secret.html')


@permission_required('orders.change_order')
def cancel(request, pk):
    return redirect('orders:list')


class OrderListView(LoginRequiredMixin, ListView):
    model = Order


class OrderDetailView(PermissionRequiredMixin, DetailView):
    permission_required = ('orders.view_order', 'orders.change_order')
    template_name = 'orders/detail.html'

    def get_context_data(self, **kwargs):
        return {'cancel_url': reverse('orders:cancel', args=[self.object.pk])}
"#;

    const API_URLS: &str = r#"from django.urls import include, path
from rest_framework.routers import DefaultRouter
from .api import HealthView, ItemViewSet, OrderViewSet, PingView, stats

router = DefaultRouter()
router.register(r'orders', OrderViewSet, basename='order')
router.register('items', ItemViewSet)

urlpatterns = [
    path('ping/', PingView.as_view()),
    path('health/', HealthView.as_view()),
    path('stats/', stats),
    path('', include(router.urls)),
]
"#;

    const API: &str = r#"from rest_framework import viewsets
from rest_framework.decorators import action, api_view, permission_classes
from rest_framework.views import APIView
from rest_framework.permissions import AllowAny, IsAdminUser, IsAuthenticatedOrReadOnly
from .base import AuditedViewSet, HealthBase


class OrderViewSet(AuditedViewSet):
    permission_classes = [IsAuthenticatedOrReadOnly]

    @action(detail=True, methods=['post'], permission_classes=[IsAdminUser])
    def refund(self, request, pk=None):
        pass

    @action(detail=False, url_path='recent-items')
    def recent(self, request):
        pass


class ItemViewSet(viewsets.ReadOnlyModelViewSet):
    lookup_field = 'slug'


class HealthView(HealthBase):
    pass


class PingView(APIView):
    permission_classes = [AllowAny]

    def get(self, request):
        pass


@api_view(['GET', 'POST'])
@permission_classes([HasShopAccess])
def stats(request):
    pass
"#;

    const CONFIG: &str =
        "[views.access]\nis_manager = \"role:Manager\"\nHasShopAccess = \"policy:shop\"\n";

    fn shop() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &[
                (".archguard/archguard.toml", CONFIG),
                ("shop/manage.py", "import django\n"),
                ("shop/shop/__init__.py", ""),
                ("shop/shop/settings.py", SETTINGS),
                ("shop/shop/urls.py", ROOT_URLS),
                ("shop/orders/__init__.py", ""),
                ("shop/orders/urls.py", ORDER_URLS),
                ("shop/orders/views.py", VIEWS),
                (
                    "shop/orders/models.py",
                    "class Order(models.Model):\n    pass\n",
                ),
                ("shop/orders/api_urls.py", API_URLS),
                ("shop/orders/api.py", API),
                (
                    "shop/orders/base.py",
                    "from rest_framework import viewsets\nfrom rest_framework.views import APIView\n\n\nclass AuditedViewSet(viewsets.ModelViewSet):\n    def list(self, request):\n        pass\n\n\nclass HealthBase(APIView):\n    permission_classes = []\n\n    def get(self, request):\n        pass\n",
                ),
                (
                    "shop/templates/base.html",
                    "<a href=\"{% url 'home' %}\">Home</a>\n<a href=\"/orders/about/\">About</a>\n<link href=\"/static/x.css\">\n",
                ),
                (
                    "shop/templates/home.html",
                    "{% extends \"base.html\" %}\n{% include 'nav.html' with x=1 %}\n<a href=\"{% url 'orders:detail' 7 %}\">x</a>\n{% url 'missing' %}\n<a href=\"{% url 'orders:cancel' 3 %}\">Cancel</a>\n",
                ),
                (
                    "shop/templates/nav.html",
                    "<a href=\"{% url 'secret' %}\">S</a>\n",
                ),
                ("shop/templates/secret.html", "{% extends 'base.html' %}\n"),
                (
                    "shop/orders/templates/orders/order_list.html",
                    "{# {% url 'gone' %} #}\n{% comment %}{% url 'gone2' %}{% endcomment %}\n<a href=\"{% url 'orders:about' %}\">About</a>\n",
                ),
                ("shop/orders/templates/orders/detail.html", "<p></p>\n"),
                ("shop/orders/templates/orders/about.html", "<p></p>\n"),
                ("shop/orders/templates/orders/archive.html", "<p></p>\n"),
            ],
        );
        dir
    }

    fn screen<'a>(flow: &'a UiFlow, id: &str) -> &'a archguard_core::Screen {
        (flow.screens.iter())
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no screen {id}: {:#?}", flow.screens))
    }

    fn endpoint(flow: &UiFlow, method: &str, path: &str) -> AccessState {
        (flow.endpoints.iter())
            .find(|e| e.method == method && e.path == path)
            .unwrap_or_else(|| panic!("no {method} {path}: {:#?}", flow.endpoints))
            .access
            .clone()
    }

    #[test]
    fn urls_and_views_give_screens_with_routes_and_access() {
        let dir = shop();
        let flow = extract(dir.path());
        let routes: Vec<_> = (flow.screens.iter())
            .map(|s| (s.id.as_str(), s.route.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(
            routes,
            [
                (
                    "shop:django.views.generic.TemplateView(orders/about.html)",
                    "/orders/about/"
                ),
                ("shop:orders.views.OrderDetailView", "/orders/:pk/"),
                ("shop:orders.views.OrderListView", "/orders/"),
                ("shop:orders.views.archive", "/archive/:year/"),
                ("shop:orders.views.home", "/"),
                ("shop:orders.views.secret", "/secret/"),
            ]
        );
        let home = screen(&flow, "shop:orders.views.home");
        assert_eq!(
            (
                home.name.as_str(),
                home.application.as_deref(),
                home.file.as_str(),
                home.line
            ),
            ("home", Some("shop"), "shop/orders/views.py", 8)
        );
        assert_eq!(home.access, archguard_core::AccessState::Undeclared);
        let access = |id: &str| screen(&flow, id).access.clone();
        assert_eq!(
            access("shop:orders.views.archive"),
            archguard_core::AccessState::Protected(Condition::Authenticated)
        );
        assert_eq!(
            access("shop:orders.views.secret"),
            archguard_core::AccessState::Protected(Condition::All(vec![
                Condition::Role("Manager".into()),
                Condition::Authenticated
            ]))
        );
        assert_eq!(
            access("shop:orders.views.OrderListView"),
            archguard_core::AccessState::Protected(Condition::Authenticated)
        );
        assert_eq!(
            access("shop:orders.views.OrderDetailView"),
            archguard_core::AccessState::Protected(Condition::All(vec![
                Condition::Policy("orders.view_order".into()),
                Condition::Policy("orders.change_order".into())
            ]))
        );
        // A view without a template is an endpoint for every method.
        let cancel = (flow.endpoints.iter())
            .find(|e| e.path == "/orders/:pk/cancel/")
            .unwrap();
        assert_eq!(cancel.method, "ANY");
        assert_eq!(
            cancel.access,
            AccessState::Protected(Condition::Policy("orders.change_order".into()))
        );
        assert_eq!(cancel.handler.as_deref(), Some("orders.views.cancel"));
        assert_eq!(cancel.application.as_deref(), Some("shop"));
        assert!(flow.screens.iter().all(|s| s.source == "django"));
    }

    #[test]
    fn drf_routers_viewsets_and_api_views_give_endpoints() {
        let dir = shop();
        let flow = extract(dir.path());
        let open = AccessState::Open;
        let signed_in = AccessState::Protected(Condition::Authenticated);
        assert_eq!(endpoint(&flow, "GET", "/api/orders/"), open);
        assert_eq!(endpoint(&flow, "POST", "/api/orders/"), signed_in);
        assert_eq!(endpoint(&flow, "GET", "/api/orders/:pk/"), open);
        for method in ["PUT", "PATCH", "DELETE"] {
            assert_eq!(endpoint(&flow, method, "/api/orders/:pk/"), signed_in);
        }
        assert_eq!(
            endpoint(&flow, "POST", "/api/orders/:pk/refund/"),
            AccessState::Protected(Condition::All(vec![
                Condition::Authenticated,
                Condition::Custom("is_staff".into())
            ]))
        );
        assert_eq!(endpoint(&flow, "GET", "/api/orders/recent-items/"), open);
        // Settings' DEFAULT_PERMISSION_CLASSES; a read-only viewset; lookup_field.
        assert_eq!(endpoint(&flow, "GET", "/api/items/"), signed_in);
        assert_eq!(endpoint(&flow, "GET", "/api/items/:slug/"), signed_in);
        assert!(
            !flow
                .endpoints
                .iter()
                .any(|e| e.path.starts_with("/api/items") && e.method != "GET")
        );
        assert_eq!(endpoint(&flow, "GET", "/api/ping/"), open);
        let policy = AccessState::Protected(Condition::Policy("shop".into()));
        assert_eq!(endpoint(&flow, "GET", "/api/stats/"), policy);
        assert_eq!(endpoint(&flow, "POST", "/api/stats/"), policy);
        let handler = |m: &str, p: &str| {
            (flow.endpoints.iter())
                .find(|e| e.method == m && e.path == p)
                .and_then(|e| e.handler.clone())
        };
        assert_eq!(
            handler("PATCH", "/api/orders/:pk/").as_deref(),
            Some("orders.api.OrderViewSet.partial_update")
        );
        assert_eq!(
            handler("POST", "/api/orders/:pk/refund/").as_deref(),
            Some("orders.api.OrderViewSet.refund")
        );
        assert_eq!(
            handler("GET", "/api/ping/").as_deref(),
            Some("orders.api.PingView.get")
        );
        assert_eq!(
            handler("POST", "/api/stats/").as_deref(),
            Some("orders.api.stats")
        );
        // The location is the defining method's, also in a base class of another file.
        let at = |m: &str, p: &str| {
            (flow.endpoints.iter())
                .find(|e| e.method == m && e.path == p)
                .map(|e| (e.handler.clone().unwrap(), e.file.as_str(), e.line))
        };
        let list = "orders.api.OrderViewSet.list".to_string();
        assert_eq!(
            at("GET", "/api/orders/"),
            Some((list, "shop/orders/base.py", 6))
        );
        // `permission_classes = []` is open; the handler is inherited from another file.
        assert_eq!(endpoint(&flow, "GET", "/api/health/"), open);
        let get = "orders.api.HealthView.get".to_string();
        assert_eq!(
            at("GET", "/api/health/"),
            Some((get, "shop/orders/base.py", 13))
        );
        let update = "orders.api.OrderViewSet.update".to_string();
        assert_eq!(
            at("PUT", "/api/orders/:pk/"),
            Some((update, "shop/orders/api.py", 8))
        );
    }

    #[test]
    fn templates_and_redirects_give_transitions() {
        let dir = shop();
        let flow = extract(dir.path());
        let any = || Origin::AnyScreen {
            application: "shop".into(),
        };
        let from = |id: &str| Origin::Screen(format!("shop:orders.views.{id}"));
        let to = |id: &str| Target::Screen(format!("shop:{id}"));
        let mut edges: Vec<_> = (flow.transitions.iter())
            .map(|t| (t.from.clone(), t.target.clone(), t.file.as_str(), t.line))
            .collect();
        let about = to("django.views.generic.TemplateView(orders/about.html)");
        let mut expected = [
            (
                any(),
                to("orders.views.home"),
                "shop/templates/base.html",
                1,
            ),
            (any(), about.clone(), "shop/templates/base.html", 2),
            (
                from("OrderListView"),
                about.clone(),
                "shop/orders/templates/orders/order_list.html",
                3,
            ),
            (
                from("archive"),
                to("orders.views.home"),
                "shop/orders/views.py",
                15,
            ),
            (
                from("home"),
                to("orders.views.OrderDetailView"),
                "shop/templates/home.html",
                3,
            ),
            (
                from("home"),
                Target::Unresolved("{% url 'missing' %}".into()),
                "shop/templates/home.html",
                4,
            ),
            (
                from("home"),
                to("orders.views.secret"),
                "shop/templates/nav.html",
                1,
            ),
            // `redirect(reverse('…'))`: the inner `reverse` names the target.
            (from("secret"), about.clone(), "shop/orders/views.py", 22),
        ];
        edges.sort();
        expected.sort();
        assert_eq!(edges, expected);
        // A link to a view without a template is a GET call of that endpoint.
        let calls: Vec<_> = (flow.actions.iter())
            .map(|a| (&a.screen, &a.call))
            .collect();
        assert_eq!(
            calls,
            [(
                &from("home"),
                &Call::Http {
                    method: "GET".into(),
                    path: "/orders/:pk/cancel/".into()
                }
            )]
        );
    }

    #[test]
    fn unreadable_parts_are_gaps_and_external_includes_notes() {
        let dir = shop();
        let flow = extract(dir.path());
        let has = |severity: DiagnosticSeverity, file: &str, line: u32, text: &str| {
            flow.diagnostics.iter().any(|d| {
                d.severity == severity
                    && d.file == file
                    && d.line == line
                    && d.message.contains(text)
            })
        };
        assert!(
            has(DiagnosticSeverity::Gap, "shop/shop/urls.py", 11, "(a|b)"),
            "{:#?}",
            flow.diagnostics
        );
        assert!(
            has(
                DiagnosticSeverity::Gap,
                "shop/shop/urls.py",
                13,
                "make_urls()"
            ),
            "{:#?}",
            flow.diagnostics
        );
        assert!(
            has(
                DiagnosticSeverity::Note,
                "shop/shop/urls.py",
                14,
                "allauth.urls"
            ),
            "{:#?}",
            flow.diagnostics
        );
        // Readable redirects leave no gap, also around a `reverse`.
        assert!(
            !flow
                .diagnostics
                .iter()
                .any(|d| d.file == "shop/orders/views.py"),
            "{:#?}",
            flow.diagnostics
        );
        assert!(
            has(
                DiagnosticSeverity::Gap,
                "shop/templates/home.html",
                4,
                "missing"
            ),
            "{:#?}",
            flow.diagnostics
        );
        assert!(!flow.complete);
    }

    const CLUB_URLS: &str = r#"from django.urls import include, path
from django.contrib.admin.views.decorators import staff_member_required
from members import views
from members.api import router

urlpatterns = [
    path('m/', include(('members.urls', 'mem'))),
    path('staff/', staff_member_required(views.staff)),
    path('api/', include('members.api')),
]
urlpatterns += [path('x/', include('members.urls', namespace='x'))]
urlpatterns.append(path('nested/', include(router.urls)))
"#;

    const MEMBER_URLS: &str = r#"from django.urls import path
from . import views

urlpatterns = [
    path('rq/', views.QueueView.as_view(), name='queue'),
    path('edit/<int:pk>/', views.EditView.as_view(), name='edit'),
    path('ping/', views.ping, name='ping'),
    path('hook/', views.HookView.as_view(), name='hook'),
    path('club/<slug:slug>/', views.ClubDetail.as_view(), name='club'),
]
"#;

    const MEMBER_VIEWS: &str = r#"from django.contrib.auth.mixins import UserPassesTestMixin
from django.contrib.auth.decorators import login_required
from django.utils.decorators import method_decorator
from django.views.decorators.http import require_http_methods
from django.views.generic import DetailView, UpdateView, View
from .models import Club


def staff(request):
    return render(request, 'staff.html')


class BaseQueue(UserPassesTestMixin, View):
    def test_func(self):
        return self.request.user.is_superuser


class QueueView(BaseQueue):
    template_name = 'queue.html'

    def get(self, request):
        return render(request, self.template_name)


@method_decorator(login_required, name='dispatch')
class EditView(UpdateView):
    model = Club


@require_http_methods(['POST'])
def ping(request):
    return JsonResponse({})


class HookView(View):
    def post(self, request):
        pass

    def delete(self, request):
        pass


class ClubDetail(DetailView):
    queryset = Club.objects.all()
"#;

    const MEMBER_API: &str = r#"from rest_framework import routers, viewsets
from rest_framework.decorators import action
from rest_framework.permissions import IsAdminUser, IsAuthenticated


class ClubViewSet(viewsets.GenericViewSet):
    permission_classes = [IsAdminUser | ~IsAuthenticated]

    def list(self, request):
        pass

    @action(methods=['get'])
    def broken(self, request):
        pass


router = routers.SimpleRouter(trailing_slash=False)
router.register('clubs', ClubViewSet, basename='club')
urlpatterns = router.urls
"#;

    #[test]
    fn mixins_method_decorators_namespaces_and_default_templates() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &[
                (
                    ".archguard/archguard.toml",
                    "[views.access]\n\"BaseQueue.test_func\" = \"role:superuser\"\n",
                ),
                ("club/manage.py", ""),
                ("club/club/__init__.py", ""),
                ("club/club/settings.py", "ROOT_URLCONF = \"club.urls\"\n"),
                ("club/club/urls.py", CLUB_URLS),
                ("club/members/__init__.py", ""),
                ("club/members/urls.py", MEMBER_URLS),
                ("club/members/views.py", MEMBER_VIEWS),
                ("club/members/api.py", MEMBER_API),
                (
                    "club/members/models/__init__.py",
                    "from .club import Club\n",
                ),
                (
                    "club/members/models/club.py",
                    "class Club(models.Model):\n    pass\n",
                ),
                ("club/members/templates/queue.html", ""),
                ("club/members/templates/staff.html", ""),
                (
                    "club/members/templates/members/club_form.html",
                    "<a href=\"{% url 'mem:queue' %}\">Q</a>\n<a href=\"edit/\">E</a>\n",
                ),
                (
                    "club/members/templates/members/club_detail.html",
                    "<a href=\"{% url 'x:club' 'a' %}\">C</a>\n",
                ),
            ],
        );
        let flow = extract(dir.path());
        let screens: Vec<_> = (flow.screens.iter())
            .map(|s| {
                (
                    s.id.as_str(),
                    s.route.as_deref().unwrap_or(""),
                    s.access.condition().cloned(),
                )
            })
            .collect();
        assert_eq!(
            screens,
            [
                ("club:members.views.ClubDetail", "/m/club/:slug/", None),
                (
                    "club:members.views.EditView",
                    "/m/edit/:pk/",
                    Some(Condition::Authenticated)
                ),
                (
                    "club:members.views.QueueView",
                    "/m/rq/",
                    Some(Condition::Role("superuser".into()))
                ),
                (
                    "club:members.views.staff",
                    "/staff/",
                    Some(Condition::All(vec![
                        Condition::Authenticated,
                        Condition::Custom("is_staff".into())
                    ]))
                ),
            ]
        );
        let endpoints: Vec<_> = (flow.endpoints.iter())
            .map(|e| {
                (
                    e.method.as_str(),
                    e.path.as_str(),
                    e.access.clone(),
                    e.handler.as_deref().unwrap_or(""),
                )
            })
            .collect();
        let either = AccessState::Protected(Condition::Any(vec![
            Condition::All(vec![
                Condition::Authenticated,
                Condition::Custom("is_staff".into()),
            ]),
            Condition::Not(Box::new(Condition::Authenticated)),
        ]));
        let undeclared = AccessState::Undeclared;
        assert_eq!(
            endpoints,
            [
                (
                    "DELETE",
                    "/m/hook/",
                    undeclared.clone(),
                    "members.views.HookView.delete"
                ),
                (
                    "DELETE",
                    "/x/hook/",
                    undeclared.clone(),
                    "members.views.HookView.delete"
                ),
                ("GET", "/api/clubs", either, "members.api.ClubViewSet.list"),
                (
                    "POST",
                    "/m/hook/",
                    undeclared.clone(),
                    "members.views.HookView.post"
                ),
                ("POST", "/m/ping/", undeclared.clone(), "members.views.ping"),
                (
                    "POST",
                    "/x/hook/",
                    undeclared.clone(),
                    "members.views.HookView.post"
                ),
                ("POST", "/x/ping/", undeclared, "members.views.ping"),
            ]
        );
        let edges: Vec<_> = (flow.transitions.iter())
            .map(|t| (t.from.clone(), t.target.clone(), t.line))
            .collect();
        let screen = |id: &str| format!("club:members.views.{id}");
        assert_eq!(
            edges,
            [
                (
                    Origin::Screen(screen("ClubDetail")),
                    Target::Screen(screen("ClubDetail")),
                    1
                ),
                (
                    Origin::Screen(screen("EditView")),
                    Target::Screen(screen("QueueView")),
                    1
                ),
                (
                    Origin::Screen(screen("EditView")),
                    Target::Unresolved("edit/".into()),
                    2
                ),
            ]
        );
        let gaps: Vec<_> = (flow.diagnostics.iter())
            .map(|d| {
                (
                    d.file.as_str(),
                    d.line,
                    d.message.split(';').next().unwrap_or(""),
                )
            })
            .collect();
        assert_eq!(
            gaps,
            [
                (
                    "club/club/urls.py",
                    12,
                    "URL patterns from `router.urls` are not read"
                ),
                (
                    "club/members/api.py",
                    12,
                    "@action `broken` has no literal `detail` (missing)"
                ),
                (
                    "club/members/templates/members/club_form.html",
                    2,
                    "Relative link `edit/` is not resolved"
                ),
            ]
        );
    }

    const W1_VIEWS: &str = r#"
from django.contrib.auth.decorators import login_required
from django.contrib.auth.mixins import LoginRequiredMixin
from rest_framework.permissions import AllowAny
from rest_framework.views import APIView
from rest_framework import viewsets
from rest_framework.decorators import action
from .deco import some_project_decorator
from vendor.auth import api_key_required
from django.views.decorators.csrf import csrf_exempt
from rest_framework.decorators import api_view, permission_classes


@some_project_decorator
@login_required
def guarded(request):
    pass


@some_project_decorator
@permission_classes([AllowAny])
def unknown_only(request):
    pass


@api_key_required
@api_view(["GET"])
@permission_classes([AllowAny])
def ext(request):
    pass


@csrf_exempt
@api_view(["GET"])
@permission_classes([AllowAny])
def plain_open(request):
    pass


class Pub(APIView):
    permission_classes = [AllowAny]

    def get(self, request):
        pass


class Mixed(LoginRequiredMixin, APIView):
    permission_classes = [AllowAny]

    def get(self, request):
        pass


class VS(viewsets.ViewSet):
    def get_permissions(self):
        return []

    @action(detail=True, permission_classes=[AllowAny])
    def ping(self, request, pk=None):
        pass

    def list(self, request):
        pass
"#;

    fn w1() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &[
                ("w/manage.py", ""),
                ("w/w/__init__.py", ""),
                ("w/w/settings.py", "ROOT_URLCONF = \"w.urls\"\n"),
                (
                    "w/w/urls.py",
                    "from django.urls import path\nfrom django.contrib.auth.decorators import login_required\nfrom rest_framework.routers import DefaultRouter\nfrom app import views\n\nrouter = DefaultRouter()\nrouter.register('vs', views.VS, basename='vs')\nurlpatterns = [\n    path('g/', views.guarded),\n    path('u/', views.unknown_only),\n    path('pub/', views.Pub.as_view()),\n    path('wrapped/', login_required(views.Pub.as_view())),\n    path('mixed/', views.Mixed.as_view()),\n    path('ext/', views.ext),\n    path('po/', views.plain_open),\n] + router.urls\n",
                ),
                ("w/app/__init__.py", ""),
                (
                    "w/app/deco.py",
                    "def some_project_decorator(f):\n    return f\n",
                ),
                ("w/app/views.py", W1_VIEWS),
            ],
        );
        dir
    }

    #[test]
    fn w1_unknown_project_decorator_is_a_gap_and_never_open() {
        let flow = extract(w1().path());
        assert_eq!(
            endpoint(&flow, "ANY", "/g/"),
            AccessState::Protected(Condition::Authenticated)
        );
        let gaps = (flow.diagnostics.iter())
            .filter(|d| {
                d.severity == DiagnosticSeverity::Gap
                    && d.message.contains("some_project_decorator")
            })
            .count();
        assert!(gaps >= 1, "{:#?}", flow.diagnostics);
        assert!(!matches!(endpoint(&flow, "ANY", "/u/"), AccessState::Open));
    }

    #[test]
    fn w1_external_decorator_is_unknown_never_open() {
        let flow = extract(w1().path());
        let unknown = AccessState::Protected(Condition::Unknown("api_key_required".into()));
        assert_eq!(endpoint(&flow, "GET", "/ext/"), unknown);
        let gap = (flow.diagnostics.iter()).any(|d| {
            d.severity == DiagnosticSeverity::Gap && d.message.contains("api_key_required")
        });
        assert!(gap, "{:#?}", flow.diagnostics);
        // A decorator that checks no access leaves the explicit marker alone.
        assert_eq!(endpoint(&flow, "GET", "/po/"), AccessState::Open);
    }

    #[test]
    fn w1_wrapper_around_drf_view_applies() {
        let flow = extract(w1().path());
        assert_eq!(endpoint(&flow, "GET", "/pub/"), AccessState::Open);
        let wrapped = endpoint(&flow, "GET", "/wrapped/");
        assert_eq!(wrapped, AccessState::Protected(Condition::Authenticated));
    }

    #[test]
    fn w1_django_mixin_outside_drf_applies() {
        let flow = extract(w1().path());
        let mixed = endpoint(&flow, "GET", "/mixed/");
        assert_eq!(mixed, AccessState::Protected(Condition::Authenticated));
    }

    #[test]
    fn w1_get_permissions_beats_action_permission_classes() {
        let flow = extract(w1().path());
        let unknown = AccessState::Protected(Condition::Unknown("VS.get_permissions".into()));
        assert_eq!(endpoint(&flow, "GET", "/vs/:pk/ping/"), unknown);
    }

    /// A minimal Django project: `views.py` plus the routes in `urls.py`.
    fn mini(views: &str, routes: &str) -> UiFlow {
        mini_with(views, routes, "", &[])
    }

    fn mini_with(views: &str, routes: &str, settings: &str, extra: &[(&str, &str)]) -> UiFlow {
        let dir = tempfile::tempdir().unwrap();
        let urls = format!(
            "from django.urls import path\nfrom django.contrib.auth.decorators import login_required\nfrom . import views\nurlpatterns = [\n{routes}\n]\n"
        );
        write(
            dir.path(),
            &[
                ("m/manage.py", ""),
                ("m/m/__init__.py", ""),
                (
                    "m/m/settings.py",
                    &format!("ROOT_URLCONF = \"m.urls\"\n{settings}"),
                ),
                ("m/m/urls.py", &urls),
                ("m/m/views.py", views),
            ],
        );
        write(dir.path(), extra);
        extract(dir.path())
    }

    #[test]
    fn project_decorator_named_like_an_allowlisted_one_is_unknown() {
        let views = "from rest_framework.decorators import api_view, permission_classes\nfrom rest_framework.permissions import AllowAny\nfrom rest_framework.decorators import schema as drf_schema\n\ndef schema(f):\n    return f\n\n@schema\n@api_view(['GET'])\n@permission_classes([AllowAny])\ndef proj(request):\n    pass\n\n@drf_schema(None)\n@api_view(['GET'])\n@permission_classes([AllowAny])\ndef real(request):\n    pass\n\n@condition\n@api_view(['GET'])\n@permission_classes([AllowAny])\ndef unimported(request):\n    pass\n";
        let flow = mini(
            views,
            "    path('p/', views.proj),\n    path('r/', views.real),\n    path('u/', views.unimported),",
        );
        let unknown = |n: &str| AccessState::Protected(Condition::Unknown(n.into()));
        assert_eq!(endpoint(&flow, "GET", "/p/"), unknown("schema"));
        assert_eq!(endpoint(&flow, "GET", "/u/"), unknown("condition"));
        assert_eq!(endpoint(&flow, "GET", "/r/"), AccessState::Open);
    }

    fn has_diag(flow: &UiFlow, severity: DiagnosticSeverity, needle: &str) -> bool {
        (flow.diagnostics.iter()).any(|d| d.severity == severity && d.message.contains(needle))
    }

    fn protected(c: Condition) -> AccessState {
        AccessState::Protected(c)
    }

    #[test]
    fn mixin_right_of_the_view_base_never_runs() {
        let views = "from django.views import View\nfrom django.contrib.auth.mixins import LoginRequiredMixin, PermissionRequiredMixin, UserPassesTestMixin\n\nclass Late(View, LoginRequiredMixin):\n    def get(self, request):\n        pass\n\nclass Both(LoginRequiredMixin, PermissionRequiredMixin, View):\n    permission_required = 'a.b'\n    def get(self, request):\n        pass\n\nclass T1(UserPassesTestMixin):\n    def test_func(self):\n        return True\n\nclass T2(UserPassesTestMixin):\n    def test_func(self):\n        return False\n\nclass Two(T1, T2, View):\n    def get(self, request):\n        pass\n";
        let flow = mini(
            views,
            "    path('late/', views.Late.as_view()),\n    path('both/', views.Both.as_view()),\n    path('two/', views.Two.as_view()),",
        );
        assert_eq!(endpoint(&flow, "GET", "/late/"), AccessState::Undeclared);
        assert!(has_diag(
            &flow,
            DiagnosticSeverity::Note,
            "LoginRequiredMixin"
        ));
        let both = Condition::All(vec![
            Condition::Authenticated,
            Condition::Policy("a.b".into()),
        ]);
        assert_eq!(endpoint(&flow, "GET", "/both/"), protected(both));
        let two = Condition::Custom("T1.test_func".into());
        assert_eq!(endpoint(&flow, "GET", "/two/"), protected(two));
    }

    #[test]
    fn method_decorator_applies_to_the_named_or_decorated_method_only() {
        let views = "from django.views import View\nfrom django.utils.decorators import method_decorator\nfrom django.contrib.auth.decorators import login_required, permission_required\n\n@method_decorator(login_required, name='post')\nclass Named(View):\n    def get(self, request):\n        pass\n    def post(self, request):\n        pass\n\nclass OnHandler(View):\n    def get(self, request):\n        pass\n    @method_decorator(permission_required('a.b'))\n    def post(self, request):\n        pass\n\n@method_decorator(login_required, name=WHICH)\nclass Dyn(View):\n    def get(self, request):\n        pass\n";
        let flow = mini(
            views,
            "    path('n/', views.Named.as_view()),\n    path('h/', views.OnHandler.as_view()),\n    path('d/', views.Dyn.as_view()),",
        );
        assert_eq!(endpoint(&flow, "GET", "/n/"), AccessState::Undeclared);
        assert_eq!(
            endpoint(&flow, "POST", "/n/"),
            protected(Condition::Authenticated)
        );
        assert_eq!(endpoint(&flow, "GET", "/h/"), AccessState::Undeclared);
        let policy = Condition::Policy("a.b".into());
        assert_eq!(endpoint(&flow, "POST", "/h/"), protected(policy));
        // An unreadable `name=` is unknown for the whole view, never dropped.
        assert!(matches!(
            endpoint(&flow, "GET", "/d/"),
            AccessState::Protected(Condition::Unknown(_))
        ));
        assert!(has_diag(&flow, DiagnosticSeverity::Gap, "method_decorator"));
    }

    #[test]
    fn staff_decorator_and_admin_permission_need_authentication_too() {
        let staff = Condition::All(vec![
            Condition::Authenticated,
            Condition::Custom("is_staff".into()),
        ]);
        let views = "from django.contrib.admin.views.decorators import staff_member_required\nfrom rest_framework.views import APIView\nfrom rest_framework.permissions import AllowAny, IsAdminUser, IsAuthenticated\nfrom .perms import IsOwner\n\n@staff_member_required\ndef dj(request):\n    pass\n\nclass Adm(APIView):\n    permission_classes = [IsAdminUser]\n    def get(self, request):\n        pass\n\nclass Mix(APIView):\n    permission_classes = [IsAuthenticated & (IsAdminUser | IsOwner)]\n    def get(self, request):\n        pass\n\nclass AnyOpen(APIView):\n    permission_classes = [IsAuthenticated | AllowAny]\n    def get(self, request):\n        pass\n\nclass Nope(APIView):\n    permission_classes = [~AllowAny]\n    def get(self, request):\n        pass\n\nclass AndOpen(APIView):\n    permission_classes = [IsAuthenticated & AllowAny]\n    def get(self, request):\n        pass\n";
        let perms = "from rest_framework.permissions import BasePermission\n\nclass IsOwner(BasePermission):\n    def has_permission(self, request, view):\n        return True\n";
        let flow = mini_with(
            views,
            "    path('dj/', views.dj),\n    path('adm/', views.Adm.as_view()),\n    path('mix/', views.Mix.as_view()),\n    path('ao/', views.AnyOpen.as_view()),\n    path('no/', views.Nope.as_view()),\n    path('an/', views.AndOpen.as_view()),",
            "",
            &[("m/m/perms.py", perms)],
        );
        assert_eq!(endpoint(&flow, "ANY", "/dj/"), protected(staff.clone()));
        assert_eq!(endpoint(&flow, "GET", "/adm/"), protected(staff.clone()));
        let inner = Condition::Any(vec![staff, Condition::Custom("IsOwner".into())]);
        let mix = Condition::All(vec![Condition::Authenticated, inner]);
        assert_eq!(endpoint(&flow, "GET", "/mix/"), protected(mix));
        assert_eq!(endpoint(&flow, "GET", "/ao/"), AccessState::Open);
        assert_eq!(endpoint(&flow, "GET", "/no/"), protected(Condition::Never));
        assert_eq!(
            endpoint(&flow, "GET", "/an/"),
            protected(Condition::Authenticated)
        );
    }

    #[test]
    fn django_model_permissions_read_the_queryset_model() {
        let views = "from rest_framework.views import APIView\nfrom rest_framework.permissions import DjangoModelPermissions\nfrom blog.models import Post\n\nclass Known(APIView):\n    permission_classes = [DjangoModelPermissions]\n    queryset = Post.objects.all()\n    def get(self, request):\n        pass\n    def post(self, request):\n        pass\n    def patch(self, request):\n        pass\n    def put(self, request):\n        pass\n    def delete(self, request):\n        pass\n\nclass Dyn(APIView):\n    permission_classes = [DjangoModelPermissions]\n    queryset = get_qs()\n    def post(self, request):\n        pass\n";
        let flow = mini_with(
            views,
            "    path('k/', views.Known.as_view()),\n    path('d/', views.Dyn.as_view()),",
            "",
            &[
                ("m/blog/__init__.py", ""),
                ("m/blog/models.py", "class Post:\n    pass\n"),
            ],
        );
        let with = |p: &str| {
            let all = vec![Condition::Authenticated, Condition::Policy(p.into())];
            protected(Condition::All(all))
        };
        assert_eq!(
            endpoint(&flow, "GET", "/k/"),
            protected(Condition::Authenticated)
        );
        assert_eq!(endpoint(&flow, "POST", "/k/"), with("blog.add_post"));
        assert_eq!(endpoint(&flow, "PATCH", "/k/"), with("blog.change_post"));
        assert_eq!(endpoint(&flow, "PUT", "/k/"), with("blog.change_post"));
        assert_eq!(endpoint(&flow, "DELETE", "/k/"), with("blog.delete_post"));
        let unknown = Condition::Unknown("DjangoModelPermissions".into());
        let all = Condition::All(vec![Condition::Authenticated, unknown]);
        assert_eq!(endpoint(&flow, "POST", "/d/"), protected(all));
        assert!(has_diag(
            &flow,
            DiagnosticSeverity::Gap,
            "DjangoModelPermissions"
        ));
    }

    /// POST on a `DjangoModelPermissions` view whose queryset is `Post` of the
    /// given extra files: the condition it needs.
    fn post_policy(extra: &[(&str, &str)]) -> Condition {
        let views = "from rest_framework.views import APIView\nfrom rest_framework.permissions import DjangoModelPermissions\nfrom blog.models import Post\n\nclass Known(APIView):\n    permission_classes = [DjangoModelPermissions]\n    queryset = Post.objects.all()\n    def post(self, request):\n        pass\n";
        let flow = mini_with(views, "    path('k/', views.Known.as_view()),", "", extra);
        match endpoint(&flow, "POST", "/k/") {
            AccessState::Protected(c) => c,
            other => panic!("{other:?}"),
        }
    }

    fn needs(p: &str) -> Condition {
        Condition::All(vec![Condition::Authenticated, Condition::Policy(p.into())])
    }

    #[test]
    fn model_permission_app_label_follows_appconfig_and_meta() {
        let model = "class Post:\n    pass\n";
        let with_apps = |body: &str| {
            let apps = format!("from django.apps import AppConfig\nclass C(AppConfig):\n{body}");
            let extra = [
                ("m/blog/__init__.py", ""),
                ("m/blog/models.py", model),
                ("m/blog/apps.py", apps.as_str()),
            ];
            post_policy(&extra)
        };
        assert_eq!(
            with_apps("    label = 'journal'\n"),
            needs("journal.add_post")
        );
        assert_eq!(
            with_apps("    name = 'store.shop'\n"),
            needs("shop.add_post")
        );
        assert_eq!(
            with_apps("    name = 'store.shop'\n    label = 'journal'\n"),
            needs("journal.add_post")
        );
        let meta = "class Post:\n    class Meta:\n        app_label = 'news'\n";
        let apps =
            "from django.apps import AppConfig\nclass C(AppConfig):\n    label = 'journal'\n";
        let extra = [
            ("m/blog/__init__.py", ""),
            ("m/blog/models.py", meta),
            ("m/blog/apps.py", apps),
        ];
        assert_eq!(post_policy(&extra), needs("news.add_post"));
        let dynamic = "class Post:\n    class Meta:\n        app_label = LABEL\n";
        let extra = [("m/blog/__init__.py", ""), ("m/blog/models.py", dynamic)];
        let unknown = Condition::Unknown("DjangoModelPermissions".into());
        assert_eq!(
            post_policy(&extra),
            Condition::All(vec![Condition::Authenticated, unknown])
        );
    }

    #[test]
    fn permission_classes_above_api_view_is_not_applied() {
        let views = "from rest_framework.decorators import api_view, permission_classes\nfrom rest_framework.permissions import IsAuthenticated\n\n@permission_classes([IsAuthenticated])\n@api_view(['GET'])\ndef above(request):\n    pass\n\n@api_view(['GET'])\n@permission_classes([IsAuthenticated])\ndef below(request):\n    pass\n";
        let flow = mini(
            views,
            "    path('a/', views.above),\n    path('b/', views.below),",
        );
        assert_eq!(endpoint(&flow, "GET", "/a/"), AccessState::Undeclared);
        assert!(has_diag(
            &flow,
            DiagnosticSeverity::Note,
            "permission_classes"
        ));
        assert_eq!(
            endpoint(&flow, "GET", "/b/"),
            protected(Condition::Authenticated)
        );
    }

    #[test]
    fn object_only_permission_class_is_no_request_guard() {
        let views = "from rest_framework.views import APIView\nfrom rest_framework.permissions import BasePermission, IsAuthenticated\n\nclass IsOwner(BasePermission):\n    def has_object_permission(self, request, view, obj):\n        return True\n\nclass Only(APIView):\n    permission_classes = [IsOwner]\n    def get(self, request):\n        pass\n\nclass Plus(APIView):\n    permission_classes = [IsAuthenticated, IsOwner]\n    def get(self, request):\n        pass\n";
        let flow = mini(
            views,
            "    path('o/', views.Only.as_view()),\n    path('p/', views.Plus.as_view()),",
        );
        assert_eq!(endpoint(&flow, "GET", "/o/"), AccessState::Undeclared);
        assert!(has_diag(&flow, DiagnosticSeverity::Note, "IsOwner"));
        assert_eq!(
            endpoint(&flow, "GET", "/p/"),
            protected(Condition::Authenticated)
        );
    }

    #[test]
    fn base_class_outside_the_scan_hides_its_permission_classes() {
        let views = "from vendor.base import ThirdPartyBaseView\nfrom rest_framework.views import APIView\n\nclass V(ThirdPartyBaseView, APIView):\n    def get(self, request):\n        pass\n\nclass Plain(APIView):\n    def get(self, request):\n        pass\n";
        let settings = "REST_FRAMEWORK = {'DEFAULT_PERMISSION_CLASSES': ['rest_framework.permissions.IsAuthenticated']}\n";
        let flow = mini_with(
            views,
            "    path('v/', views.V.as_view()),\n    path('p/', views.Plain.as_view()),",
            settings,
            &[],
        );
        let unknown = Condition::Unknown("ThirdPartyBaseView.permission_classes".into());
        assert_eq!(endpoint(&flow, "GET", "/v/"), protected(unknown));
        assert!(has_diag(
            &flow,
            DiagnosticSeverity::Gap,
            "ThirdPartyBaseView"
        ));
        assert_eq!(
            endpoint(&flow, "GET", "/p/"),
            protected(Condition::Authenticated)
        );
    }
}
