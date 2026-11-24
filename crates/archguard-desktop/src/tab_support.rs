//! What the tabs share: callbacks on the active document's state, Slint
//! list models, picker indices, snapshot mode and background loading.
//!
//! A tab's state type implements [`TabState`]; [`Actions`] turns closures
//! on that state into Slint callbacks, [`show_async`] builds the state once
//! per document on a worker thread.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use archguard_engine::core::{CallAnalysis, Config, ServiceTopology, UiFlow};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::context::Loaded;
use crate::documents::{self, DocId};
use crate::format::Locale;
use crate::tree::Tree;
use crate::ui::{AppWindow, Templates};

mod trace;
pub(crate) use trace::traced;

/// One tab's state, kept per document in [`documents`].
pub trait TabState: Sized + 'static {
    /// Whether the state is built from the analysis, so a new analysis
    /// drops it ([`documents::derived`]); false for one that reads
    /// something else and keeps itself current.
    const DERIVED: bool = true;

    /// Show the state in the window.
    fn push(&mut self, ui: &AppWindow);

    /// After a callback changed the state, before [`TabState::push`].
    fn changed(&mut self) {}

    /// While the state is built on a worker thread (`error` is `None`), or
    /// after that build failed.
    fn pending(_ui: &AppWindow, _error: Option<&str>) {}

    /// On the UI thread, before a state built on a worker is stored.
    fn arrive(&mut self, _ui: &AppWindow) {}
}

/// Run `f` on the active document's `T` and push it.
pub fn update<T: TabState>(ui: &AppWindow, f: impl FnOnce(&mut T, &AppWindow)) {
    let name = format!("desktop: {} action", trace::tab_name::<T>());
    trace::request(&name, || {
        documents::with_active_state(|s: &mut T| {
            f(s, ui);
            s.changed();
            crate::timing::measure(std::any::type_name::<T>(), || s.push(ui));
        })
    });
}

/// Changes to a `T` still building, done once it arrives ([`finish`]).
struct Queued<T>(Vec<Change<T>>);

type Change<T> = Box<dyn FnOnce(&mut T)>;

/// Run `f` on the active document's `T`; while `T` is still building,
/// keep `f` for when it arrives. Returns whether `f` ran.
pub fn run_or_queue<T: 'static>(f: impl FnOnce(&mut T) + 'static) -> bool {
    let mut f = Some(f);
    let ran = documents::with_active_state(|s: &mut T| f.take().map(|f| f(s))).is_some();
    if let Some(f) = f {
        documents::with_active_state_or(|| Queued::<T>(Vec::new()), |q| q.0.push(Box::new(f)));
    }
    ran
}

/// As [`update`], but a change to a tab that is still building is done
/// once its state arrives instead of being dropped ("Follow left").
pub fn update_or_queue<T: TabState>(ui: &AppWindow, f: impl FnOnce(&mut T) + 'static) {
    if run_or_queue(f) {
        update(ui, |_: &mut T, _| ());
    }
}

/// Builds Slint callbacks that change the active document's `T`.
pub struct Actions<T> {
    ui: slint::Weak<AppWindow>,
    state: PhantomData<fn() -> T>,
}

impl<T: TabState> Actions<T> {
    pub fn new(ui: &AppWindow) -> Self {
        Actions {
            ui: ui.as_weak(),
            state: PhantomData,
        }
    }

    /// A callback with one argument.
    pub fn on<A: 'static>(
        &self,
        f: impl Fn(&mut T, &AppWindow, A) + 'static,
    ) -> impl Fn(A) + 'static {
        let w = self.ui.clone();
        move |arg| {
            if let Some(ui) = w.upgrade() {
                update(&ui, |s: &mut T, ui| f(s, ui, arg));
            }
        }
    }

    /// A callback without argument.
    pub fn on0(&self, f: impl Fn(&mut T, &AppWindow) + 'static) -> impl Fn() + 'static {
        let g = self.on(move |s, ui, ()| f(s, ui));
        move || g(())
    }

    /// A callback with two arguments.
    pub fn on2<A: 'static, B: 'static>(
        &self,
        f: impl Fn(&mut T, &AppWindow, A, B) + 'static,
    ) -> impl Fn(A, B) + 'static {
        let g = self.on(move |s, ui, (a, b)| f(s, ui, a, b));
        move |a, b| g((a, b))
    }
}

/// The notice bar's "Saved {}" (notice 10, `ui/app.slint`) with what was saved.
pub fn saved_notice(ui: &AppWindow, what: &str) {
    ui.set_notice_kind(10);
    ui.set_notice(what.into());
}

/// A callback that runs `f` while the window exists.
pub fn with_ui<A: 'static>(
    ui: &AppWindow,
    f: impl Fn(&AppWindow, A) + 'static,
) -> impl Fn(A) + 'static {
    let w = ui.as_weak();
    move |a| {
        if let Some(ui) = w.upgrade() {
            f(&ui, a);
        }
    }
}

/// A Slint list model of `items`.
pub fn model<T: Clone + 'static>(items: impl IntoIterator<Item = T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items.into_iter().collect::<Vec<_>>()))
}

/// A row or picker index from Slint; `None` for negative values (`-1` is
/// "nothing"). Call sites choose their own fallback.
pub fn index(i: i32) -> Option<usize> {
    usize::try_from(i).ok()
}

/// Whether this run renders a PNG (`ARCHGUARD_DESKTOP_SNAPSHOT`); always
/// false without the `snapshot` feature. Snapshot runs build tab states in
/// place so the picture shows them.
pub fn snapshot_mode() -> bool {
    cfg!(feature = "snapshot") && std::env::var_os("ARCHGUARD_DESKTOP_SNAPSHOT").is_some()
}

/// The active document's package tree, built once for the Graph and Matrix
/// tabs, with the document's simulated changes applied.
pub fn tree(data: &Loaded) -> Rc<Tree> {
    use crate::tree::simulation::Simulation;
    documents::derived::<Rc<Tree>>();
    let (full, config) = (&data.run.graph, &data.run.config);
    let build = || {
        let simulated = documents::with_active_state(|s: &mut Simulation| s.apply(full, config));
        Rc::new(Tree::from_graph(
            simulated.flatten().as_ref().unwrap_or(full),
        ))
    };
    documents::with_active_state_or(build, |t: &mut Rc<Tree>| t.clone()).unwrap_or_else(build)
}

/// Marks a document whose `T` is built on a worker thread, or whose build
/// failed (`Some(message)`).
struct Pending<T> {
    error: Option<String>,
    state: PhantomData<fn() -> T>,
}

/// What [`show_async`] does for the active document.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Start {
    /// The state exists: show it.
    Ready,
    /// A worker builds it, or its build failed.
    Pending(Option<String>),
    /// Nothing yet: start building for this document.
    Build(DocId),
    /// No active document.
    None,
}

/// Decide for the active document, marking it pending when building starts.
pub(crate) fn start<T: TabState>() -> Start {
    if documents::with_active_state(|_: &mut T| ()).is_some() {
        return Start::Ready;
    }
    if let Some(error) = documents::with_active_state(|p: &mut Pending<T>| p.error.clone()) {
        return Start::Pending(error);
    }
    let Some(doc) = documents::active_id() else {
        return Start::None;
    };
    if T::DERIVED {
        documents::derived::<T>();
    }
    // Always: a new analysis drops a running build's result (generation),
    // so its marker must go too, or the tab would wait for it forever.
    documents::derived::<Pending<T>>();
    documents::derived::<Queued<T>>();
    documents::set_state(
        doc,
        Pending::<T> {
            error: None,
            state: PhantomData,
        },
    );
    Start::Build(doc)
}

/// Store a worker's result for `doc`: the state, or the failure message in
/// its pending marker. Returns whether `doc` is active. A result built
/// from an older analysis (`generation`) is dropped: a newer build runs.
pub(crate) fn finish<T: 'static>(doc: DocId, generation: u64, result: Result<T, String>) -> bool {
    if documents::generation(doc) != generation {
        return false;
    }
    match result {
        Ok(mut state) => {
            let queued = documents::with_state(doc, |q: &mut Queued<T>| std::mem::take(&mut q.0));
            for f in queued.into_iter().flatten() {
                f(&mut state);
            }
            documents::set_state(doc, state)
        }
        Err(error) => documents::set_state(
            doc,
            Pending::<T> {
                error: Some(error),
                state: PhantomData,
            },
        ),
    }
}

/// Run `work`, turning a panic into its message.
pub(crate) fn catch<T>(work: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).map_err(|e| {
        e.downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| e.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "the analysis failed".into())
    })
}

/// Show the active document's `T`, building it once per document.
///
/// `prepare` runs on the UI thread only when building starts; it copies
/// what the build needs and returns the build, which runs on a worker
/// thread (in place for snapshot runs). Showing the document again while it
/// runs starts no second worker; a panic in the build is kept as the tab's
/// error.
pub fn show_async<T, W>(ui: &AppWindow, prepare: impl FnOnce() -> W)
where
    T: TabState + Send,
    W: FnOnce() -> T + Send + 'static,
{
    let doc = match start::<T>() {
        Start::Ready => {
            let name = format!("desktop: show {}", trace::tab_name::<T>());
            trace::request(&name, || {
                documents::with_active_state(|s: &mut T| s.push(ui))
            });
            return;
        }
        Start::Pending(error) => return T::pending(ui, error.as_deref()),
        Start::None => return,
        Start::Build(doc) => doc,
    };
    T::pending(ui, None);
    let generation = documents::generation(doc);
    let root = documents::with_store(|s| s.get(doc).map(|d| d.root.clone()));
    let name = format!("desktop: show {}", trace::tab_name::<T>());
    let work = traced(&root.unwrap_or_default(), &name, prepare());
    if snapshot_mode() {
        return install(ui, doc, generation, catch(work));
    }
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = catch(work);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                install(&ui, doc, generation, result);
            }
        });
    });
}

fn install<T: TabState>(ui: &AppWindow, doc: DocId, generation: u64, result: Result<T, String>) {
    let result = result.map(|mut s| {
        s.arrive(ui);
        s
    });
    let error = result.as_ref().err().cloned();
    if !finish(doc, generation, result) {
        return;
    }
    match error {
        None => {
            // Changes queued while it was building are in it.
            documents::with_active_state(|s: &mut T| {
                s.changed();
                s.push(ui);
            });
        }
        Some(e) => T::pending(ui, Some(&e)),
    }
}

/// A value computed at most once per document, by whichever thread needs
/// it first; others wait for it.
pub struct Shared<T>(Arc<OnceLock<T>>);

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Shared(self.0.clone())
    }
}

impl<T: Send + Sync + 'static> Shared<T> {
    /// The active document's slot (UI thread); a fresh one without an
    /// active document.
    pub fn active() -> Self {
        documents::derived::<Shared<T>>();
        documents::with_active_state_or(|| Shared(Arc::default()), |s: &mut Shared<T>| s.clone())
            .unwrap_or_else(|| Shared(Arc::default()))
    }

    /// A new, empty slot (to store into a document before its worker runs).
    pub fn new() -> Self {
        Shared(Arc::default())
    }

    pub fn get_or_init(&self, f: impl FnOnce() -> T) -> &T {
        self.0.get_or_init(f)
    }
}

/// A document's call analysis. Start, Architecture, Calls and Quality all
/// need it; it runs once, in the first worker that asks.
#[derive(Clone)]
pub struct Calls {
    root: PathBuf,
    config: Config,
    slot: Shared<Arc<CallAnalysis>>,
}

impl Calls {
    /// The active document's analysis (UI thread).
    pub fn of(data: &Loaded) -> Self {
        Calls {
            root: data.root.to_path_buf(),
            config: data.run.config.clone(),
            slot: Shared::active(),
        }
    }

    /// An analysis of `root` shared with nothing (tests).
    #[cfg(test)]
    pub fn unshared(root: &Path, config: &Config) -> Self {
        Calls {
            root: root.to_path_buf(),
            config: config.clone(),
            slot: Shared(Arc::default()),
        }
    }

    /// Blocking: runs the analysis, or waits for the worker running it.
    pub fn get(&self) -> Arc<CallAnalysis> {
        let _span = archguard_engine::self_trace::tracing::info_span!("get").entered();
        self.slot
            .get_or_init(|| Arc::new(archguard_engine::analyze_calls(&self.root, &self.config)))
            .clone()
    }
}

/// A document's UI-flow analysis (runs the javac helper). Access layers and Calls both need it; it
/// runs once per analysis of the document, in the first worker that asks.
#[derive(Clone)]
pub struct Flow {
    root: PathBuf,
    slot: Shared<Arc<Result<UiFlow, String>>>,
}

impl Flow {
    /// The active document's flow (UI thread).
    pub fn of(data: &Loaded) -> Self {
        Flow {
            root: data.root.to_path_buf(),
            slot: Shared::active(),
        }
    }

    /// A flow of `root` shared with nothing (tests).
    #[cfg(test)]
    pub fn unshared(root: &Path) -> Self {
        Flow {
            root: root.to_path_buf(),
            slot: Shared::new(),
        }
    }

    /// Blocking: runs the analysis, or waits for the worker running it.
    pub fn get(&self) -> Arc<Result<UiFlow, String>> {
        self.get_with(|r| archguard_engine::ui_flow::analyze(r).map_err(|e| e.to_string()))
    }

    /// [`Flow::get`] with the analysis injected (tests count its runs).
    pub fn get_with(
        &self,
        analyze: impl FnOnce(&Path) -> Result<UiFlow, String>,
    ) -> Arc<Result<UiFlow, String>> {
        self.slot
            .get_or_init(|| Arc::new(analyze(&self.root)))
            .clone()
    }
}

/// Translated templates for text Rust composes from data, read from the
/// `Templates` global (`ui/texts.slint`, translated with `@tr` like all
/// UI text). Keys are the English texts; an unknown key reads as itself.
/// Cheap to clone and `Send`, so worker threads can build text with it.
#[derive(Clone, Debug, Default)]
pub struct Texts(Arc<HashMap<String, String>>);

impl Texts {
    /// The window's current language (UI thread).
    pub fn of(ui: &AppWindow) -> Self {
        let all = ui.global::<Templates>().get_all();
        Texts(Arc::new(
            all.iter()
                .map(|t| (t.key.to_string(), t.text.to_string()))
                .collect(),
        ))
    }

    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.0.get(key).map_or(key, String::as_str)
    }

    /// `t` with `{name}` placeholders filled.
    pub fn f(&self, key: &str, args: &[(&str, String)]) -> String {
        fill(self.t(key), args)
    }

    /// `one` for a count of 1, else `many`; `{count}` filled with the
    /// count in the window's number format.
    pub fn plural(&self, count: usize, one: &str, many: &str) -> String {
        let key = if count == 1 { one } else { many };
        self.f(key, &[("count", self.locale().count(count))])
    }

    /// The window language's number conventions: the `en` template reads
    /// `de` in the German translation.
    pub fn locale(&self) -> Locale {
        Locale::from_code(self.t("en"))
    }

    /// The arc42 document follows the window language (ADR 0008).
    pub fn doc_lang(&self) -> archguard_engine::core::DocLang {
        match self.locale() {
            Locale::De => archguard_engine::core::DocLang::De,
            _ => archguard_engine::core::DocLang::En,
        }
    }

    /// The German templates as the bundled translation has them: the
    /// keys of `ui/texts.slint` looked up in `lang/de/*.po` (tests, which
    /// have no window).
    #[cfg(test)]
    pub fn german() -> Self {
        let po = texts_check::po_de();
        let map = texts_check::template_keys()
            .into_iter()
            .filter_map(|key| {
                let text = po.get(&texts_check::braced(&key))?;
                Some((key, texts_check::unbraced(text)))
            })
            .collect();
        Texts(Arc::new(map))
    }
}

/// `text` with every `{name}` replaced by its value.
pub fn fill(text: &str, args: &[(&str, String)]) -> String {
    args.iter().fold(text.to_string(), |s, (name, value)| {
        s.replace(&format!("{{{name}}}"), value)
    })
}

/// A document's service topology, or why it could not be read.
pub type Topology = Arc<Result<ServiceTopology, String>>;

/// Blocking: the topology of `root`, read once per slot
/// (`Shared::<Topology>::active()` on the UI thread).
pub fn topology(slot: &Shared<Topology>, root: &Path) -> Topology {
    slot.get_or_init(|| {
        Arc::new(archguard_engine::analyze_services(root).map_err(|e| e.to_string()))
    })
    .clone()
}

/// Whether an action for document `doc` waiting for its `T` runs now
/// (`Some(true)`), waits (`Some(false)`) or is dropped (`None`).
fn ready<T: 'static>(doc: DocId) -> Option<bool> {
    if documents::active_id() != Some(doc) {
        return None;
    }
    Some(documents::with_state(doc, |_: &mut T| ()).is_some())
}

/// Run `f` once the active document has a `T` (a view's state is built
/// when the view is first shown); dropped when the user switches to
/// another document or closes it, and after a minute.
pub fn when_ready<T: 'static>(ui: &AppWindow, f: impl FnOnce(&AppWindow) + 'static) {
    if let Some(doc) = documents::active_id() {
        poll::<T>(ui.as_weak(), doc, 0, f);
    }
}

/// One check of [`when_ready`]; a single-shot timer per retry, so no
/// timer outlives its wait.
fn poll<T: 'static>(
    w: slint::Weak<AppWindow>,
    doc: DocId,
    tries: u32,
    f: impl FnOnce(&AppWindow) + 'static,
) {
    let Some(ui) = w.upgrade() else { return };
    match ready::<T>(doc) {
        Some(true) => f(&ui),
        Some(false) if tries < 600 => {
            slint::Timer::single_shot(std::time::Duration::from_millis(100), move || {
                poll::<T>(w, doc, tries + 1, f)
            });
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "texts_check.rs"]
pub(crate) mod texts_check;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Calls and Access layers hold clones of one document's `Flow`: opening either again
    /// (or both) runs the UI-flow analysis once.
    #[test]
    fn the_ui_flow_runs_once_per_document() {
        let runs = std::cell::Cell::new(0);
        let flow = Flow::unshared(Path::new("."));
        let count = |_: &Path| {
            runs.set(runs.get() + 1);
            Ok(UiFlow::default())
        };
        for f in [flow.clone(), flow.clone(), flow] {
            f.get_with(count);
        }
        assert_eq!(runs.get(), 1);
    }

    /// An action waits only for the document it was started in: switching
    /// to or closing that document drops it.
    #[test]
    fn a_waiting_action_belongs_to_its_document() {
        documents::with_store(|s| *s = Default::default());
        let a = documents::with_store(|s| s.open(Path::new("/a")).0);
        let b = documents::with_store(|s| s.open(Path::new("/b")).0);
        assert_eq!(ready::<u16>(b), Some(false), "b is active, no state yet");
        documents::set_state(a, 1_u16);
        assert_eq!(ready::<u16>(a), None, "a is not active");
        documents::set_state(b, 2_u16);
        assert_eq!(ready::<u16>(b), Some(true));
        documents::with_store(|s| s.activate(0));
        assert_eq!(ready::<u16>(b), None, "switched to a");
        assert_eq!(ready::<u16>(a), Some(true));
        documents::with_store(|s| s.close(0));
        assert_eq!(ready::<u16>(a), None, "a is closed");
    }

    fn two_documents() -> (DocId, DocId) {
        documents::with_store(|s| {
            *s = documents::Store::default();
            let (a, _) = s.open(Path::new("/a"));
            let (b, _) = s.open(Path::new("/b"));
            (a, b)
        })
    }

    struct Demo(u32);

    impl TabState for Demo {
        fn push(&mut self, _: &AppWindow) {}
    }

    /// Switching back to a document while its worker runs starts none.
    #[test]
    fn a_document_starts_one_build() {
        let (a, b) = two_documents();
        assert_eq!(start::<Demo>(), Start::Build(b));
        assert_eq!(start::<Demo>(), Start::Pending(None));
        documents::with_store(|s| s.activate(0));
        assert_eq!(start::<Demo>(), Start::Build(a), "per document");
        assert_eq!(start::<Demo>(), Start::Pending(None));
    }

    /// A finished build is shown; a result for an inactive document is
    /// stored without showing it.
    #[test]
    fn finish_stores_for_the_starting_document() {
        let (a, b) = two_documents();
        assert_eq!(start::<Demo>(), Start::Build(b));
        documents::with_store(|s| s.activate(0));
        assert_eq!(start::<Demo>(), Start::Build(a));
        assert!(
            !finish(b, documents::generation(b), Ok(Demo(2))),
            "b is not active"
        );
        assert!(finish(a, documents::generation(a), Ok(Demo(1))));
        assert_eq!(start::<Demo>(), Start::Ready);
        assert_eq!(documents::with_active_state(|d: &mut Demo| d.0), Some(1));
        documents::with_store(|s| s.activate(1));
        assert_eq!(documents::with_active_state(|d: &mut Demo| d.0), Some(2));
    }

    /// What reaches a tab while it builds ("Follow left") is kept and
    /// done once its state arrives, in order and for its own document.
    #[test]
    fn changes_wait_for_a_building_tab() {
        let (a, b) = two_documents();
        assert_eq!(start::<Demo>(), Start::Build(b));
        assert!(!run_or_queue(|d: &mut Demo| d.0 += 10), "kept");
        assert!(!run_or_queue(|d: &mut Demo| d.0 *= 2), "kept");
        documents::with_store(|s| s.activate(0));
        assert_eq!(start::<Demo>(), Start::Build(a));
        assert!(finish(a, documents::generation(a), Ok(Demo(1))));
        assert_eq!(documents::with_active_state(|d: &mut Demo| d.0), Some(1));
        documents::with_store(|s| s.activate(1));
        assert!(finish(b, documents::generation(b), Ok(Demo(1))));
        assert_eq!(documents::with_active_state(|d: &mut Demo| d.0), Some(22));
        assert!(run_or_queue(|d: &mut Demo| d.0 += 1), "the state is there");
        assert_eq!(documents::with_active_state(|d: &mut Demo| d.0), Some(23));
    }

    /// A new analysis starts a second build while the first still runs:
    /// the first one's result, finishing last, neither replaces the new
    /// state nor takes the changes queued for it.
    #[test]
    fn a_build_from_an_older_analysis_is_dropped() {
        let (_, b) = two_documents();
        assert_eq!(start::<Demo>(), Start::Build(b));
        let old = documents::generation(b);
        documents::with_store(|s| s.get_mut(b).unwrap().forget_derived());
        assert_eq!(start::<Demo>(), Start::Build(b), "built again");
        let new = documents::generation(b);
        assert!(!run_or_queue(|d: &mut Demo| d.0 += 10), "kept");
        assert!(!finish(b, old, Ok(Demo(1))), "old result dropped");
        assert_eq!(start::<Demo>(), Start::Pending(None), "still building");
        assert!(finish(b, new, Ok(Demo(2))));
        assert_eq!(documents::with_active_state(|d: &mut Demo| d.0), Some(12));
        assert!(!finish::<Demo>(b, old, Err("old failure".into())));
        assert_eq!(documents::with_active_state(|d: &mut Demo| d.0), Some(12));
    }

    /// A panicking build leaves an error, not a document pending forever,
    /// and starts no new worker on every switch.
    #[test]
    fn a_failed_build_keeps_its_error() {
        let (_, b) = two_documents();
        assert_eq!(start::<Demo>(), Start::Build(b));
        let result = catch(|| -> Demo { panic!("no call analysis") });
        assert_eq!(
            result.as_ref().err().map(String::as_str),
            Some("no call analysis")
        );
        assert!(finish(b, documents::generation(b), result));
        let failed = Start::Pending(Some("no call analysis".into()));
        assert_eq!(start::<Demo>(), failed);
        assert_eq!(start::<Demo>(), failed);
    }

    #[test]
    fn shared_values_are_per_document_and_computed_once() {
        two_documents();
        let b = Shared::<u32>::active();
        assert_eq!(*b.get_or_init(|| 2), 2);
        assert_eq!(*Shared::<u32>::active().get_or_init(|| 3), 2, "same slot");
        documents::with_store(|s| s.activate(0));
        assert_eq!(*Shared::<u32>::active().get_or_init(|| 1), 1);
    }

    #[test]
    fn negative_indices_are_none() {
        assert_eq!(index(-1), None);
        assert_eq!(index(3), Some(3));
    }

    /// Template keys of the kinds a build unit can have; unknown kinds show
    /// as they are.
    const KINDS: [&str; 7] = [
        "application",
        "library",
        "test",
        "workspace",
        "plugin",
        "module",
        "service",
    ];

    /// Build unit kinds (`documents::build_units`) are shown in the
    /// window's language.
    #[test]
    fn build_unit_kinds_are_translated() {
        let de = Texts::german();
        for k in KINDS {
            assert_ne!(de.t(k), k, "{k}");
        }
        assert_eq!(de.t("service"), "Dienst");
    }

    #[test]
    fn german_templates_fill_placeholders() {
        let de = Texts::german();
        assert_eq!(de.plural(3, "{count} file", "{count} files"), "3 Dateien");
        assert_eq!(de.plural(1, "{count} file", "{count} files"), "1 Datei");
        assert_eq!(de.t("Locate in graph"), "Im Graphen anzeigen");
        assert_eq!(
            de.plural(2374, "{count} file", "{count} files"),
            "2.374 Dateien"
        );
        assert_eq!(de.locale(), Locale::De);
        let en = Texts::default();
        assert_eq!(en.locale(), Locale::En);
        assert_eq!(
            en.plural(2374, "{count} file", "{count} files"),
            "2,374 files"
        );
        assert_eq!(en.plural(1, "{count} file", "{count} files"), "1 file");
        assert_eq!(en.t("unknown key"), "unknown key");
    }
}
