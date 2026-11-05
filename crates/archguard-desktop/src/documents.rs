//! Open projects ("documents"), each with its own analysis and per-tab
//! state. Independent of Slint; `documents_ui.rs` shows them.
//!
//! # Per-document tab state
//!
//! A tab keeps its state in the active document instead of a
//! `thread_local!`. `show(ui, &Loaded)` runs whenever a document with an
//! analysis becomes active (after its analysis and on every switch), so a
//! tab builds its state lazily and only pushes it afterwards:
//!
//! ```ignore
//! pub fn show(ui: &AppWindow, data: &Loaded) {
//!     documents::with_active_state_or(|| build(data), |s: &mut XState| push(ui, s));
//! }
//! // In a callback:
//! documents::with_active_state(|s: &mut XState| { change(s); push(&ui, s) });
//! ```
//!
//! Work finished on another thread is stored with [`set_state`] for the
//! document that started it ([`active_id`] at start); it returns whether that
//! document is still the active one, i.e. whether to push.
//!
//! State is held per type in an `Rc<RefCell<T>>`; the store itself is not
//! borrowed while `f` runs, so `f` may trigger callbacks of other tabs.
//! Re-entering the same type panics, as a `RefCell` thread-local would.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use archguard_engine::core::ServiceTopology;

use crate::context::Analysis;

pub type DocId = u64;

/// A project inside a monorepo that can be opened as its own document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubProject {
    pub name: String,
    /// `application`, `library`, … from the manifest, or `service` for a
    /// Compose build context.
    pub kind: String,
    /// Relative to the document root.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Loading,
    Ready,
    Failed(String),
}

pub struct Document {
    pub id: DocId,
    pub root: PathBuf,
    pub name: String,
    pub status: Status,
    pub analysis: Option<Rc<Analysis>>,
    pub projects: Vec<SubProject>,
    states: HashMap<TypeId, Rc<dyn Any>>,
}

impl Document {
    fn new(id: DocId, root: PathBuf) -> Self {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        Document {
            id,
            root,
            name,
            status: Status::Loading,
            analysis: None,
            projects: Vec::new(),
            states: HashMap::new(),
        }
    }

    fn cell<T: 'static>(&self) -> Option<Rc<dyn Any>> {
        self.states.get(&TypeId::of::<T>()).cloned()
    }

    fn insert<T: 'static>(&mut self, state: T) -> Rc<dyn Any> {
        let cell: Rc<dyn Any> = Rc::new(RefCell::new(state));
        self.states.insert(TypeId::of::<T>(), cell.clone());
        cell
    }

    pub fn has_state<T: 'static>(&self) -> bool {
        self.states.contains_key(&TypeId::of::<T>())
    }
}

/// The open documents in tab order and the active one.
#[derive(Default)]
pub struct Store {
    docs: Vec<Document>,
    active: Option<usize>,
    next: DocId,
}

impl Store {
    /// Open `root` as a new active document, or activate the document that
    /// already shows it. Returns its id and whether it is new.
    pub fn open(&mut self, root: &Path) -> (DocId, bool) {
        if let Some(i) = self.docs.iter().position(|d| d.root == root) {
            self.active = Some(i);
            return (self.docs[i].id, false);
        }
        self.next += 1;
        self.docs.push(Document::new(self.next, root.to_path_buf()));
        self.active = Some(self.docs.len() - 1);
        (self.next, true)
    }

    /// Activate the document at tab `index`; false if there is none.
    pub fn activate(&mut self, index: usize) -> bool {
        if index >= self.docs.len() {
            return false;
        }
        self.active = Some(index);
        true
    }

    /// Close the document at tab `index` and drop its state. The tab to its
    /// right (else left) becomes active if the closed one was.
    pub fn close(&mut self, index: usize) -> Option<Document> {
        if index >= self.docs.len() {
            return None;
        }
        let doc = self.docs.remove(index);
        self.active = match self.active {
            _ if self.docs.is_empty() => None,
            Some(a) if a > index => Some(a - 1),
            Some(a) if a == index => Some(index.min(self.docs.len() - 1)),
            other => other,
        };
        Some(doc)
    }

    pub fn docs(&self) -> &[Document] {
        &self.docs
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub fn active(&self) -> Option<&Document> {
        self.active.map(|i| &self.docs[i])
    }

    pub fn get_mut(&mut self, id: DocId) -> Option<&mut Document> {
        self.docs.iter_mut().find(|d| d.id == id)
    }

    /// Store `state` for document `id`; false if the document is closed.
    pub fn set_state<T: 'static>(&mut self, id: DocId, state: T) -> bool {
        self.get_mut(id).map(|d| d.insert(state)).is_some()
    }

    fn active_cell<T: 'static>(&self) -> Option<Rc<dyn Any>> {
        self.active()?.cell::<T>()
    }

    fn active_cell_or<T: 'static>(&mut self, build: impl FnOnce() -> T) -> Option<Rc<dyn Any>> {
        let doc = &mut self.docs[self.active?];
        Some(match doc.cell::<T>() {
            Some(cell) => cell,
            None => doc.insert(build()),
        })
    }
}

fn run<T: 'static, R>(cell: Rc<dyn Any>, f: impl FnOnce(&mut T) -> R) -> R {
    let cell = cell
        .downcast_ref::<RefCell<T>>()
        .expect("state is stored under its own type");
    f(&mut cell.borrow_mut())
}

/// The sub-projects of a monorepo worth opening: manifest projects and
/// local Compose build contexts, one per path, without the root itself.
/// Empty unless the topology shows more than one project or a monorepo.
pub fn sub_projects(topology: &ServiceTopology) -> Vec<SubProject> {
    let mut out: Vec<SubProject> = Vec::new();
    let mut add = |name: &str, kind: &str, path: &str| {
        let path = path.trim_start_matches("./").trim_end_matches('/');
        if path.is_empty() || path == "." || out.iter().any(|p| p.path == path) {
            return;
        }
        out.push(SubProject {
            name: name.to_string(),
            kind: kind.to_string(),
            path: path.to_string(),
        });
    };
    for p in &topology.projects {
        add(&p.name, &p.kind, &p.path);
    }
    for s in &topology.services {
        if let Some(ctx) = &s.build_context {
            add(&s.name, "service", ctx);
        }
    }
    if topology.projects.len() > 1 || topology.monorepo {
        out
    } else {
        Vec::new()
    }
}

/// Where a sub-project lives on disk.
pub fn project_root(doc_root: &Path, project: &SubProject) -> PathBuf {
    let joined = doc_root.join(&project.path);
    joined.canonicalize().unwrap_or(joined)
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store::default());
}

/// Read or change the store. Do not call other functions of this module
/// from `f`.
pub fn with_store<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    STORE.with_borrow_mut(f)
}

/// The active document's id, if any.
pub fn active_id() -> Option<DocId> {
    with_store(|s| s.active().map(|d| d.id))
}

/// Run `f` on the active document's state of type `T`; `None` if there is
/// no active document or it has no such state yet.
pub fn with_active_state<T: 'static, R>(f: impl FnOnce(&mut T) -> R) -> Option<R> {
    let cell = with_store(|s| s.active_cell::<T>())?;
    Some(run(cell, f))
}

/// As [`with_active_state`], building the state with `build` first if the
/// active document has none. `None` only without an active document.
pub fn with_active_state_or<T: 'static, R>(
    build: impl FnOnce() -> T,
    f: impl FnOnce(&mut T) -> R,
) -> Option<R> {
    // Build outside the store borrow: `build` may be slow, never re-entrant.
    let missing = with_store(|s| s.active().is_some_and(|d| !d.has_state::<T>()));
    let built = missing.then(build);
    let cell = with_store(|s| match built {
        Some(state) => s.active_cell_or(|| state),
        None => s.active_cell::<T>(),
    })?;
    Some(run(cell, f))
}

/// Store `state` for document `doc` (e.g. from a worker thread's result).
/// Returns true if `doc` is the active document, so the caller pushes it.
pub fn set_state<T: 'static>(doc: DocId, state: T) -> bool {
    with_store(|s| s.set_state(doc, state) && s.active().is_some_and(|d| d.id == doc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Service, ServiceProject};

    fn store(roots: &[&str]) -> Store {
        let mut s = Store::default();
        for r in roots {
            s.open(Path::new(r));
        }
        s
    }

    fn names(s: &Store) -> Vec<&str> {
        s.docs().iter().map(|d| d.name.as_str()).collect()
    }

    #[test]
    fn open_adds_an_active_loading_document_named_after_its_folder() {
        let s = store(&["/a/shop", "/b/api"]);
        assert_eq!(names(&s), ["shop", "api"]);
        assert_eq!(s.active_index(), Some(1));
        assert_eq!(s.active().map(|d| &d.status), Some(&Status::Loading));
    }

    #[test]
    fn opening_an_open_root_activates_it() {
        let mut s = store(&["/a/shop", "/b/api"]);
        let first = s.docs()[0].id;
        assert_eq!(s.open(Path::new("/a/shop")), (first, false));
        assert_eq!(s.docs().len(), 2);
        assert_eq!(s.active_index(), Some(0));
    }

    #[test]
    fn activate_switches_and_rejects_unknown_tabs() {
        let mut s = store(&["/a", "/b", "/c"]);
        assert!(s.activate(0));
        assert_eq!(s.active().map(|d| d.name.as_str()), Some("a"));
        assert!(!s.activate(3));
        assert_eq!(s.active_index(), Some(0));
    }

    #[test]
    fn close_keeps_a_neighbour_active_and_the_last_close_leaves_none() {
        let mut s = store(&["/x", "/a", "/b", "/c"]);
        s.close(0);
        assert_eq!(
            s.active().map(|d| d.name.as_str()),
            Some("c"),
            "left close keeps c"
        );
        s.activate(1);
        assert_eq!(s.close(1).map(|d| d.name), Some("b".into()));
        assert_eq!(s.active().map(|d| d.name.as_str()), Some("c"));
        s.close(1);
        assert_eq!(s.active().map(|d| d.name.as_str()), Some("a"));
        s.open(Path::new("/d"));
        s.activate(0);
        s.close(1);
        assert_eq!(s.active().map(|d| d.name.as_str()), Some("a"));
        s.close(0);
        assert!(s.active().is_none());
        assert!(s.close(0).is_none());
    }

    #[test]
    fn state_is_per_document_and_freed_on_close() {
        let mut s = store(&["/a", "/b"]);
        let (a, b) = (s.docs()[0].id, s.docs()[1].id);
        assert!(s.set_state(a, 1_u32));
        assert!(s.set_state(b, 2_u32));
        assert!(s.set_state(b, "b-only"));
        s.activate(0);
        assert_eq!(
            s.active_cell::<u32>().map(|c| run(c, |v: &mut u32| *v)),
            Some(1)
        );
        assert!(s.active_cell::<&str>().is_none());
        s.activate(1);
        let cell = s.active_cell::<u32>().expect("state of b");
        run(cell.clone(), |v: &mut u32| *v += 40);
        assert_eq!(run(cell.clone(), |v: &mut u32| *v), 42);
        s.close(1);
        assert_eq!(
            Rc::strong_count(&cell),
            1,
            "closing drops the store's handle"
        );
        assert!(!s.set_state(b, 3_u32));
    }

    #[test]
    fn thread_local_api_builds_lazily_once_per_document() {
        with_store(|s| *s = store(&["/a", "/b"]));
        let mut builds = 0;
        for _ in 0..2 {
            with_active_state_or(
                || {
                    builds += 1;
                    10_i64
                },
                |v: &mut i64| *v += 1,
            );
        }
        assert_eq!(builds, 1);
        assert_eq!(with_active_state(|v: &mut i64| *v), Some(12));
        with_store(|s| s.activate(0));
        assert_eq!(with_active_state(|v: &mut i64| *v), None);
        let b = with_store(|s| s.docs()[1].id);
        assert!(!set_state(b, 5_i64), "b is not active");
        with_store(|s| s.activate(1));
        assert_eq!(with_active_state(|v: &mut i64| *v), Some(5));
        // Another type's state may be used while one is borrowed.
        with_active_state_or(|| 1_u8, |_: &mut u8| ());
        let nested = with_active_state(|_: &mut i64| with_active_state(|v: &mut u8| *v));
        assert_eq!(nested, Some(Some(1)));
    }

    fn project(name: &str, path: &str, kind: &str) -> ServiceProject {
        ServiceProject {
            id: format!("project:{path}"),
            name: name.into(),
            path: path.into(),
            manifest: "Cargo.toml".into(),
            ecosystem: "rust".into(),
            kind: kind.into(),
            runtime: None,
            evidence: Vec::new(),
        }
    }

    fn service(name: &str, ctx: Option<&str>) -> Service {
        Service {
            id: name.into(),
            name: name.into(),
            compose_file: "compose.yaml".into(),
            build_context: ctx.map(Into::into),
            networks: Vec::new(),
            profiles: Vec::new(),
            project: None,
        }
    }

    fn topology(
        projects: Vec<ServiceProject>,
        services: Vec<Service>,
        mono: bool,
    ) -> ServiceTopology {
        ServiceTopology {
            schema_version: 1,
            root: "/r".into(),
            monorepo: mono,
            monorepo_reason: String::new(),
            compose_files: Vec::new(),
            services,
            projects,
            dependencies: Vec::new(),
            cycles: Vec::new(),
            diagnostics: Vec::new(),
            complete: true,
        }
    }

    #[test]
    fn sub_projects_list_projects_and_build_contexts_without_the_root() {
        let t = topology(
            vec![
                project("ws", ".", "workspace"),
                project("core", "crates/core", "library"),
            ],
            vec![
                service("orders", Some("services/orders/")),
                service("orders-worker", Some("./services/orders")),
                service("db", None),
                service("again", Some("crates/core")),
            ],
            true,
        );
        let subs = sub_projects(&t);
        let rows: Vec<(&str, &str, &str)> = subs
            .iter()
            .map(|p| (p.name.as_str(), p.kind.as_str(), p.path.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("core", "library", "crates/core"),
                ("orders", "service", "services/orders")
            ]
        );
    }

    #[test]
    fn a_single_project_offers_no_sub_projects() {
        let t = topology(vec![project("one", ".", "application")], Vec::new(), false);
        assert!(sub_projects(&t).is_empty());
    }

    #[test]
    fn project_root_resolves_below_the_document_root() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repository root");
        let p = SubProject {
            name: "orders".into(),
            kind: "service".into(),
            path: "examples/service-shop/services/orders".into(),
        };
        assert_eq!(
            project_root(&repo, &p),
            repo.join("examples/service-shop/services/orders")
        );
        assert!(project_root(&repo, &p).is_dir());
    }

    /// The monorepo example of this repository: its ten local service
    /// build contexts, as `archguard services examples/service-shop` lists.
    #[test]
    fn service_shop_offers_its_services() {
        let shop = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/service-shop");
        let t = archguard_engine::analyze_services(&shop).expect("service topology");
        let paths: Vec<String> = sub_projects(&t).into_iter().map(|p| p.path).collect();
        assert_eq!(paths.len(), 10, "{paths:?}");
        assert!(paths.contains(&"services/gateway".to_string()));
    }
}
