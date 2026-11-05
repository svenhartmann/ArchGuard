//! What the tabs share: callbacks on the active document's state, Slint
//! list models, picker indices, snapshot mode and background loading.
//!
//! A tab's state type implements [`TabState`]; [`Actions`] turns closures
//! on that state into Slint callbacks, [`show_async`] builds the state once
//! per document on a worker thread.

use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use archguard_engine::core::{CallAnalysis, Config, ServiceTopology};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::context::Loaded;
use crate::documents::{self, DocId};
use crate::tree::Tree;
use crate::ui::AppWindow;

/// One tab's state, kept per document in [`documents`].
pub trait TabState: Sized + 'static {
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
    documents::with_active_state(|s: &mut T| {
        f(s, ui);
        s.changed();
        s.push(ui);
    });
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
/// tabs.
pub fn tree(data: &Loaded) -> Rc<Tree> {
    let build = || Rc::new(Tree::from_graph(&data.run.graph));
    documents::with_active_state_or(build, |t: &mut Rc<Tree>| t.clone()).unwrap_or_else(build)
}

/// Show `node` (an index into the analysed graph's nodes) in the Graph tab.
pub fn reveal(ui: &AppWindow, node: usize) {
    ui.set_tab("graph".into());
    ui.invoke_reveal(node as i32);
}

/// Marks a document whose `T` is built on a worker thread, or whose build
/// failed (`Some(message)`).
struct Pending<T> {
    error: Option<String>,
    state: PhantomData<fn() -> T>,
}

/// What [`show_async`] does for the active document.
#[derive(Debug, PartialEq, Eq)]
enum Start {
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
fn start<T: 'static>() -> Start {
    if documents::with_active_state(|_: &mut T| ()).is_some() {
        return Start::Ready;
    }
    if let Some(error) = documents::with_active_state(|p: &mut Pending<T>| p.error.clone()) {
        return Start::Pending(error);
    }
    let Some(doc) = documents::active_id() else {
        return Start::None;
    };
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
/// its pending marker. Returns whether `doc` is active.
fn finish<T: 'static>(doc: DocId, result: Result<T, String>) -> bool {
    match result {
        Ok(state) => documents::set_state(doc, state),
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
fn catch<T>(work: impl FnOnce() -> T) -> Result<T, String> {
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
            documents::with_active_state(|s: &mut T| s.push(ui));
            return;
        }
        Start::Pending(error) => return T::pending(ui, error.as_deref()),
        Start::None => return,
        Start::Build(doc) => doc,
    };
    T::pending(ui, None);
    let work = prepare();
    if snapshot_mode() {
        return install(ui, doc, catch(work));
    }
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = catch(work);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                install(&ui, doc, result);
            }
        });
    });
}

fn install<T: TabState>(ui: &AppWindow, doc: DocId, result: Result<T, String>) {
    let result = result.map(|mut s| {
        s.arrive(ui);
        s
    });
    let error = result.as_ref().err().cloned();
    if !finish(doc, result) {
        return;
    }
    match error {
        None => {
            documents::with_active_state(|s: &mut T| s.push(ui));
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
        documents::with_active_state_or(|| Shared(Arc::default()), |s: &mut Shared<T>| s.clone())
            .unwrap_or_else(|| Shared(Arc::default()))
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
        self.slot
            .get_or_init(|| Arc::new(archguard_engine::analyze_calls(&self.root, &self.config)))
            .clone()
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn two_documents() -> (DocId, DocId) {
        documents::with_store(|s| {
            *s = documents::Store::default();
            let (a, _) = s.open(Path::new("/a"));
            let (b, _) = s.open(Path::new("/b"));
            (a, b)
        })
    }

    struct Demo(u32);

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
        assert!(!finish(b, Ok(Demo(2))), "b is not active");
        assert!(finish(a, Ok(Demo(1))));
        assert_eq!(start::<Demo>(), Start::Ready);
        assert_eq!(documents::with_active_state(|d: &mut Demo| d.0), Some(1));
        documents::with_store(|s| s.activate(1));
        assert_eq!(documents::with_active_state(|d: &mut Demo| d.0), Some(2));
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
        assert!(finish(b, result));
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
}
