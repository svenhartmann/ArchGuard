//! Matrix view state, one per Matrix tab (`instances`), and its data for
//! the pane that shows it.

use std::collections::BTreeSet;
use std::rc::Rc;

use slint::{ComponentHandle, Model, SharedString, VecModel};

use crate::deferred::{Coalesce, TYPING};
use crate::documents::with_store;
use crate::export::GraphInput;
use crate::follow::{self, Nav};
use crate::follow_ui;
use crate::format::Locale;
use crate::instances::{self, Instance};
use crate::kpi::{self, Value};
use crate::matrix::{Matrix, MatrixRow, Target};
use crate::model_sync::{sync_rows, sync_rows_with};
use crate::panes::Layout;
use crate::panes_ui;
use crate::tab_support::{self, Texts, index, model};
use crate::tree::{Sort, Tree};
use crate::ui::{AppWindow, MatrixColumn, MatrixData, MatrixRowItem};
use crate::view_export;

/// Rows per page, as the web viewer's matrix.
const PAGE: usize = 120;
/// Evidence edges listed at most for one cell.
const EVIDENCE: usize = 40;

pub struct MatrixState {
    /// Shared with the other views of the same document.
    tree: Rc<Tree>,
    project: String,
    /// `None`: project overview; else a package node.
    scope: Option<usize>,
    /// Package nodes offered as scopes, sorted by name.
    scopes: Vec<usize>,
    matrix: Matrix,
    expanded: BTreeSet<usize>,
    query: String,
    cycle: Option<usize>,
    only: bool,
    page: usize,
    evidence: Option<(usize, usize)>,
    /// The node revealed from the pane beside the matrix, highlighted.
    marked: Option<usize>,
}

impl MatrixState {
    fn new(tree: Rc<Tree>, project: String) -> Self {
        let mut scopes: Vec<usize> = (0..tree.nodes.len())
            .filter(|&n| !tree.nodes[n].file)
            .collect();
        scopes.sort_by(|&a, &b| tree.nodes[a].name.cmp(&tree.nodes[b].name));
        let matrix = Matrix::build(&tree, None, &project);
        MatrixState {
            tree,
            project,
            scope: None,
            scopes,
            matrix,
            expanded: BTreeSet::new(),
            query: String::new(),
            cycle: None,
            only: false,
            page: 0,
            evidence: None,
            marked: None,
        }
    }

    /// The rows the filters leave; keeps `page` within them, so paging
    /// back works after a toggle, search or scope shrank the list.
    fn visible_rows(&mut self) -> Vec<MatrixRow> {
        let rows = self.matrix.rows(
            &self.tree,
            &self.expanded,
            &self.query,
            self.cycle,
            self.only,
        );
        self.page = self.page.min(rows.len().div_ceil(PAGE).max(1) - 1);
        rows
    }

    /// Show `node` (a tree node, as the Graph's): leave a scope that does
    /// not hold it, drop filters, open its packages, page to its row and
    /// mark it.
    pub fn reveal(&mut self, node: usize) {
        if node >= self.tree.nodes.len() {
            return;
        }
        if self.scope.is_some() && !self.matrix.included.contains(&node) {
            self.set_scope(None);
        }
        self.query.clear();
        self.only = false;
        let mut cur = self.tree.nodes[node].parent;
        while let Some(p) = cur {
            self.expanded.insert(p);
            cur = self.tree.nodes[p].parent;
        }
        self.marked = Some(node);
        let rows = self.visible_rows();
        if let Some(i) = rows.iter().position(|r| r.node == node) {
            self.page = i / PAGE;
        }
    }

    fn set_scope(&mut self, scope: Option<usize>) {
        self.scope = scope;
        self.matrix = Matrix::build(&self.tree, scope, &self.project);
        self.expanded.clear();
        self.cycle = None;
        self.only = false;
        self.page = 0;
        self.evidence = None;
        self.marked = None;
    }

    /// Where the matrix stands, for "Follow left": scope, open packages,
    /// the row of the open cell.
    fn marks(&self) -> follow::Marks {
        follow::Marks {
            scope: self.scope,
            open: self.expanded.clone(),
            selected: self.evidence.map(|(row, _)| row),
        }
    }

    /// "Follow left": do what the left pane did. A scope the matrix does
    /// not offer (a file) is revealed instead.
    pub fn follow(&mut self, nav: &Nav) {
        let node = |name: &str| instances::node_named(&self.tree, name);
        match nav {
            Nav::Scope(None) => self.set_scope(None),
            Nav::Scope(Some(name)) => match node(name) {
                Some(n) if self.scopes.contains(&n) => self.set_scope(Some(n)),
                Some(n) => self.reveal(n),
                None => {}
            },
            Nav::Expand { open, close } => {
                let (open, close): (Vec<_>, Vec<_>) = (
                    open.iter().filter_map(|n| node(n)).collect(),
                    close.iter().filter_map(|n| node(n)).collect(),
                );
                self.expanded.extend(open);
                for n in close {
                    self.expanded.remove(&n);
                }
            }
            Nav::Select(name) => {
                if let Some(n) = node(name) {
                    self.reveal(n);
                }
            }
        }
    }

    /// The export's input: every row of the scope at the open packages
    /// (all pages, no search) and every link between them, as the web
    /// exports the complete matrix scope.
    pub fn export_input(&self) -> GraphInput {
        let rows: BTreeSet<usize> = self
            .matrix
            .rows(&self.tree, &self.expanded, "", None, false)
            .iter()
            .map(|r| r.node)
            .collect();
        // The tree view shows a matrix row once its ancestors are open.
        let mut open = self.expanded.clone();
        for &row in &rows {
            let mut cur = self.tree.nodes[row].parent;
            while let Some(n) = cur {
                open.insert(n);
                cur = self.tree.nodes[n].parent;
            }
        }
        let view = self.tree.view_sorted(&open, Sort::Alphabetical);
        let links: Vec<usize> = (0..view.links.len()).collect();
        let keep = |r: usize| rows.contains(&view.rows[r].node);
        let mut input = view_export::tree_input(&self.tree, &view, &links, keep);
        input.project = self.project.clone();
        input.matrix = true;
        input.filters.reference = self.marked.map(|n| self.tree.nodes[n].name.clone());
        input
    }
}

thread_local! {
    /// Per pane: the data of the Matrix it shows.
    static PANES: Rc<VecModel<MatrixData>> = Rc::new(VecModel::default());
}

impl Instance for MatrixState {
    fn show(&mut self, ui: &AppWindow, pane: usize, id: u32) {
        let rows = self.visible_rows();
        let old = PANES.with(|m| m.row_data(pane)).unwrap_or_default();
        let row = data(ui, self, &rows, id, &old);
        PANES.with(|m| instances::set_row(m, pane, row));
    }
}

/// Build, drop and show the active document's Matrices for `layout`; a
/// tab's node is its scope.
fn sync(ui: &AppWindow, layout: &Layout) -> Vec<u32> {
    let Some(analysis) = with_store(|s| s.active()?.analysis.clone()) else {
        return Vec::new();
    };
    let tree = tab_support::tree(&analysis.loaded());
    let project = ui.get_project().to_string();
    instances::sync(ui, layout, "dsm", &tree, |_, node| {
        let mut m = MatrixState::new(tree.clone(), project.clone());
        if let Some(n) = node.filter(|n| m.scopes.contains(n)) {
            m.set_scope(Some(n));
        }
        Some(m)
    })
}

/// Change Matrix `id` with `f` and show it; its tab follows its scope.
fn act(ui: &AppWindow, id: i32, f: impl FnOnce(&mut MatrixState)) {
    let Some(id) = index(id).and_then(|i| u32::try_from(i).ok()) else {
        return;
    };
    let layout = panes_ui::layout();
    let result = instances::update(ui, &layout, id, |m: &mut MatrixState| {
        let was = m.marks();
        f(m);
        let navs = follow::navs(&m.tree, &was, &m.marks());
        let changed = (m.scope != was.scope).then(|| m.scope.map(|n| m.tree.nodes[n].name.clone()));
        (changed, navs)
    });
    let Some((changed, navs)) = result else {
        return;
    };
    if let Some(node) = changed {
        panes_ui::describe(ui, id, node, 0);
    }
    follow_ui::navigated(ui, id, navs);
}

/// A Matrix callback with the instance id and one argument.
fn on<A: 'static>(
    ui: &AppWindow,
    f: impl Fn(&mut MatrixState, A) + 'static,
) -> impl Fn(i32, A) + 'static {
    let w = ui.as_weak();
    move |id, a| {
        if let Some(ui) = w.upgrade() {
            act(&ui, id, |m| f(m, a));
        }
    }
}

/// A Matrix callback with the instance id only.
fn on0(ui: &AppWindow, f: impl Fn(&mut MatrixState) + 'static) -> impl Fn(i32) + 'static {
    let g = on(ui, move |m, ()| f(m));
    move |id| g(id, ())
}

pub fn wire(ui: &AppWindow) {
    ui.set_matrix_panes(PANES.with(|m| m.clone()).into());
    panes_ui::on_sync("dsm", sync);
    follow_ui::on_follow("dsm", |ui, id, nav| act(ui, id as i32, |m| m.follow(nav)));
    // Linked selection from the pane beside.
    panes_ui::on_reveal("dsm", |ui, id, node| act(ui, id as i32, |m| m.reveal(node)));
    ui.on_m_toggle(on(ui, |m, n: i32| {
        let n = index(n).unwrap_or(0);
        if !m.expanded.remove(&n) {
            m.expanded.insert(n);
        }
    }));
    ui.on_m_expand_all(on0(ui, |m| {
        m.expanded = (0..m.tree.nodes.len())
            .filter(|&n| !m.tree.nodes[n].file)
            .collect();
        m.page = 0;
    }));
    ui.on_m_collapse_all(on0(ui, |m| {
        m.expanded.clear();
        m.page = 0;
    }));
    let search = on(ui, |m, q: SharedString| {
        m.query = q.to_string();
        m.page = 0;
    });
    let search = Coalesce::debounce(TYPING, move |(id, q)| search(id, q));
    ui.on_m_search(move |id, q| search.call((id, q)));
    ui.on_m_scope(on(ui, |m, i: i32| {
        // Entry 0 is the project overview.
        let scope = index(i - 1).and_then(|i| m.scopes.get(i).copied());
        m.set_scope(scope);
    }));
    // Entry 0 is "All dependencies".
    ui.on_m_cycle(on(ui, |m, i: i32| choose_cycle(m, index(i - 1))));
    ui.on_m_toggle_only(on0(ui, |m| {
        m.only = !m.only && m.cycle.is_some();
        m.page = 0;
    }));
    ui.on_m_page(on(ui, |m, d: i32| {
        m.page = (m.page as i64 + d as i64).max(0) as usize;
    }));
    ui.on_m_close_evidence(on0(ui, |m| m.evidence = None));
    wire_cell(ui);
}

/// A cell opens its evidence; its row's node is revealed beside.
fn wire_cell(ui: &AppWindow) {
    let cell = on(ui, |m, (row, col): (i32, i32)| {
        m.evidence = Some((index(row).unwrap_or(0), index(col).unwrap_or(0)));
    });
    let w = ui.as_weak();
    ui.on_m_cell(move |id, row, col| {
        cell(id, (row, col));
        let (Some(ui), Ok(tab), Some(node)) = (w.upgrade(), u32::try_from(id), index(row)) else {
            return;
        };
        panes_ui::link_from(&ui, tab, node);
    });
}

/// Choosing a cycle opens its members' ancestors, as the web viewer does.
fn choose_cycle(m: &mut MatrixState, cycle: Option<usize>) {
    m.query.clear();
    m.cycle = cycle.filter(|&c| c < m.matrix.cycles.len());
    if m.cycle.is_none() {
        m.only = false;
    }
    m.evidence = None;
    if let Some(c) = m.cycle {
        for &member in &m.matrix.cycles[c].members {
            let mut cur = m.tree.nodes[member].parent;
            while let Some(p) = cur {
                m.expanded.insert(p);
                cur = m.tree.nodes[p].parent;
            }
        }
    }
}

/// A matrix count; `≥` marks one the analysis cut off.
fn count(locale: Locale, n: usize, exact: bool) -> String {
    let n = i64::try_from(n).unwrap_or(i64::MAX);
    if exact {
        locale.int(n)
    } else {
        locale.lower_bound(n)
    }
}

fn row_items(m: &MatrixState, rows: &[MatrixRow], locale: Locale) -> Vec<MatrixRowItem> {
    let mx = &m.matrix;
    let path: BTreeSet<usize> = m
        .cycle
        .map(|c| mx.cycles[c].path.iter().copied().collect())
        .unwrap_or_default();
    let shown = rows.iter().skip(m.page * PAGE).take(PAGE);
    shown
        .map(|r| {
            let n = &m.tree.nodes[r.node];
            let cells = mx.cells.get(&r.node);
            MatrixRowItem {
                node: r.node as i32,
                label: n.label.clone().into(),
                depth: r.depth as i32,
                package: !n.file,
                expanded: m.expanded.contains(&r.node) || !m.query.trim().is_empty() || m.only,
                in_cycle_path: path.contains(&r.node),
                marked: m.marked == Some(r.node),
                badges: badges(mx, r.node).into(),
                cells: model((0..mx.columns.len()).map(|c| {
                    cells
                        .and_then(|row| row.get(&c))
                        .map(|cell| count(locale, cell.count as usize, cell.exact))
                        .unwrap_or_default()
                        .into()
                })),
            }
        })
        .collect()
}

/// Everything the Matrix view of tab `id` shows of `m`, in the models of
/// `old` (what its pane showed before), changed in place.
fn data(
    ui: &AppWindow,
    m: &MatrixState,
    rows: &[MatrixRow],
    id: u32,
    old: &MatrixData,
) -> MatrixData {
    let texts = Texts::of(ui);
    let mx = &m.matrix;
    let records = if mx.total_exact {
        Value::Count(mx.total as usize)
    } else {
        Value::AtLeast(mx.total as usize)
    };
    let mut scopes = vec![SharedString::from(texts.t("Project overview"))];
    scopes.extend(
        m.scopes
            .iter()
            .map(|&s| m.tree.nodes[s].name.as_str().into()),
    );
    let mut cycles = vec![SharedString::from(texts.t("All dependencies"))];
    cycles.extend(
        mx.cycles
            .iter()
            .map(|c| SharedString::from(format!("{} · {}", c.id, c.members.len()))),
    );
    // Position 0 of both lists is "Project overview" / "All dependencies".
    let scope = m
        .scope
        .and_then(|s| m.scopes.iter().position(|&n| n == s))
        .map_or(0, |i| i + 1);
    let (title, lines) = evidence(m).unwrap_or_default();
    MatrixData {
        id: id as i32,
        columns: sync_rows(
            &old.columns,
            mx.columns.iter().map(|c| column(&c.target, &c.name)),
        ),
        rows: sync_rows_with(&old.rows, row_items(m, rows, texts.locale()), |o, n| {
            n.cells = sync_rows(&o.cells, n.cells.iter());
        }),
        kpis: sync_rows(
            &old.kpis,
            [
                kpi::MATRIX_FILES.item(&texts, mx.files),
                kpi::DEPENDENCY_EVIDENCE.item(&texts, records),
                kpi::FILE_CYCLE_GROUPS.item(&texts, mx.cycles.len()),
            ],
        ),
        row_start: (m.page * PAGE) as i32,
        row_total: rows.len() as i32,
        scopes: sync_rows(&old.scopes, scopes),
        cycle_options: sync_rows(&old.cycle_options, cycles),
        scope_index: scope as i32,
        cycle_index: m.cycle.map_or(0, |c| c + 1) as i32,
        query: m.query.as_str().into(),
        only: m.only,
        evidence: sync_rows(&old.evidence, lines),
        evidence_title: title.into(),
        cycle_path: cycle_path(m).into(),
        marked_row: marked_row(m, rows),
    }
}

fn column(target: &Target, name: &str) -> MatrixColumn {
    let (label, context) = match name.trim_end_matches('/').rsplit_once('/') {
        Some((ctx, last)) => (last.to_string(), ctx.to_string()),
        None => (name.to_string(), String::new()),
    };
    match target {
        Target::Package(_) => MatrixColumn {
            label: label.into(),
            context: context.into(),
            kind: 0,
        },
        Target::Direct => MatrixColumn {
            label: name.into(),
            context: "".into(),
            kind: 1,
        },
        Target::Outside => MatrixColumn {
            label: "".into(),
            context: "".into(),
            kind: 2,
        },
    }
}

fn badges(mx: &Matrix, node: usize) -> String {
    let Some(set) = mx.badges.get(&node) else {
        return String::new();
    };
    let ids: Vec<&str> = set
        .iter()
        .take(3)
        .map(|&c| mx.cycles[c].id.as_str())
        .collect();
    let more = if set.len() > 3 {
        format!(" +{}", set.len() - 3)
    } else {
        String::new()
    };
    format!("{}{more}", ids.join(" "))
}

fn cycle_path(m: &MatrixState) -> String {
    let Some(c) = m.cycle.map(|c| &m.matrix.cycles[c]) else {
        return String::new();
    };
    let steps: Vec<&str> = c
        .path
        .iter()
        .map(|&n| m.tree.nodes[n].label.as_str())
        .collect();
    format!("{} · {} · {}", c.id, c.path.len() - 1, steps.join(" → "))
}

/// The marked row's index on the shown page, for the view to scroll to.
fn marked_row(m: &MatrixState, rows: &[MatrixRow]) -> i32 {
    let shown = rows.iter().skip(m.page * PAGE).take(PAGE);
    let marked = shown.into_iter().position(|r| m.marked == Some(r.node));
    marked.map_or(-1, |i| i as i32)
}

/// Title and list of the open cell; `None` when no cell with
/// dependencies is open.
fn evidence(m: &MatrixState) -> Option<(String, Vec<SharedString>)> {
    let (row, col) = m.evidence?;
    let cell = m.matrix.cells.get(&row)?.get(&col)?;
    let name = |n: usize| m.tree.nodes[n].name.as_str();
    let full = &m.tree.full;
    let index: std::collections::BTreeMap<_, usize> = full
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    let mut lines = Vec::new();
    for &e in cell.edges.iter().take(EVIDENCE) {
        let edge = &full.edges[e];
        let (a, b) = (index[&edge.from], index[&edge.to]);
        lines.push(SharedString::from(format!("{} → {}", name(a), name(b))));
        for r in &edge.refs {
            lines.push(format!("    {}:{}  {}", r.file, r.line, r.raw.trim()).into());
        }
    }
    let more = cell.edges.len().saturating_sub(EVIDENCE);
    if more > 0 {
        lines.push(format!("… +{more}").into());
    }
    Some((format!("{} · {}", name(row), cell.count), lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> MatrixState {
        MatrixState::new(Rc::new(crate::matrix::tests::tree()), "p".into())
    }

    /// A page beyond the rows (after collapsing or a search) is pulled
    /// back, so "Previous" moves from the page that is shown.
    #[test]
    fn the_page_stays_within_the_rows() {
        let mut m = state();
        m.page = 3;
        let rows = m.visible_rows();
        assert!(rows.len() <= PAGE);
        assert_eq!(m.page, 0);
    }

    fn named(m: &MatrixState, name: &str) -> usize {
        m.tree
            .nodes
            .iter()
            .position(|n| n.name == name)
            .expect("node in the test tree")
    }

    /// Revealing a node from the pane beside it (linked selection) opens
    /// its packages, leaves a scope or search that hides it and marks it.
    #[test]
    fn reveal_shows_and_marks_a_hidden_node() {
        let mut m = state();
        let lib = named(&m, "lib");
        let app = named(&m, "app");
        let v = named(&m, "app/ui/v.rs");
        m.set_scope(Some(lib));
        m.query = "a.rs".into();
        m.reveal(v);
        let rows = m.visible_rows();
        assert!(rows.iter().any(|r| r.node == v), "v.rs is a row");
        assert_eq!(m.marked, Some(v));
        assert!(m.expanded.contains(&app));
        assert_eq!(m.scope, None);
        assert!(m.query.is_empty());
        m.reveal(usize::MAX);
        assert_eq!(m.marked, Some(v), "unknown nodes are ignored");
    }

    /// A cell without dependencies opens no evidence, not the last one's.
    #[test]
    fn an_empty_cell_has_no_evidence() {
        let mut m = state();
        let (&row, cols) = m.matrix.cells.iter().next().expect("a cell");
        let col = *cols.keys().next().expect("a column");
        m.evidence = Some((row, col));
        assert!(evidence(&m).is_some());
        m.evidence = Some((row, m.matrix.columns.len() + 1));
        assert_eq!(evidence(&m), None);
    }

    /// The export holds every row of the scope (not one page) and the
    /// links between them, without the rows outside the scope.
    #[test]
    fn the_export_is_the_complete_scope() {
        let mut m = state();
        let all = m.export_input();
        assert!(all.matrix);
        assert!(
            all.nodes.iter().any(|n| n.name == "top.rs"),
            "{:?}",
            all.nodes
        );
        assert!(all.nodes.len() > 1, "{:?}", all.nodes);
        let lib = named(&m, "lib");
        m.set_scope(Some(lib));
        m.expanded.insert(named(&m, "lib/src"));
        let scoped = m.export_input();
        let names: Vec<&str> = scoped.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"lib/src/a.rs"), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with("app")), "{names:?}");
        let pairs: Vec<_> = scoped
            .edges
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect();
        assert!(
            pairs.contains(&("lib/src/a.rs", "lib/src/b.rs")),
            "{pairs:?}"
        );
        assert!(
            pairs
                .iter()
                .all(|(f, t)| f.starts_with("lib") && t.starts_with("lib"))
        );
    }
}
