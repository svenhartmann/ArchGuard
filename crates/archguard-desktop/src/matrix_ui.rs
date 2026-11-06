//! Matrix tab state (one per document) and its push into the window.

use std::collections::BTreeSet;

use slint::SharedString;

use crate::context::Loaded;
use crate::documents;
use crate::format::Locale;
use crate::kpi::{self, Value};
use crate::matrix::{Matrix, MatrixRow, Target};
use crate::tab_support::{self, Actions, TabState, Texts, index, model, with_ui};
use crate::tree::Tree;
use crate::ui::{AppWindow, MatrixColumn, MatrixRowItem};
use std::rc::Rc;

/// Rows per page, as the web viewer's matrix.
const PAGE: usize = 120;
/// Evidence edges listed at most for one cell.
const EVIDENCE: usize = 40;

pub struct MatrixState {
    /// Shared with the Graph tab of the same document.
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

    /// Show `node` (a tree node, as the Graph tab's): leave a scope that
    /// does not hold it, drop filters, open its packages, page to its row
    /// and mark it.
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
}

/// Show the active document's matrix, building it on first use.
pub fn show(ui: &AppWindow, data: &Loaded) {
    let project = ui.get_project().to_string();
    documents::with_active_state_or(
        || MatrixState::new(tab_support::tree(data), project),
        |m: &mut MatrixState| m.push(ui),
    );
}

impl TabState for MatrixState {
    fn push(&mut self, ui: &AppWindow) {
        let rows = self.visible_rows();
        push(ui, self, &rows);
    }
}

pub fn wire(ui: &AppWindow) {
    // Linked selection from the Graph pane.
    crate::panes_ui::on_reveal("dsm", |ui, node| {
        tab_support::update(ui, |m: &mut MatrixState, _| m.reveal(node));
    });
    let a = Actions::<MatrixState>::new(ui);
    ui.on_m_toggle(a.on(|m, _, n: i32| {
        let n = index(n).unwrap_or(0);
        if !m.expanded.remove(&n) {
            m.expanded.insert(n);
        }
    }));
    ui.on_m_expand_all(a.on0(|m, _| {
        m.expanded = (0..m.tree.nodes.len())
            .filter(|&n| !m.tree.nodes[n].file)
            .collect();
        m.page = 0;
    }));
    ui.on_m_collapse_all(a.on0(|m, _| {
        m.expanded.clear();
        m.page = 0;
    }));
    ui.on_m_search(a.on(|m, _, q: SharedString| {
        m.query = q.to_string();
        m.page = 0;
    }));
    ui.on_m_scope(a.on(|m, _, i: i32| {
        // Entry 0 is the project overview.
        let scope = index(i - 1).and_then(|i| m.scopes.get(i).copied());
        m.set_scope(scope);
    }));
    // Entry 0 is "All dependencies".
    ui.on_m_cycle(a.on(|m, _, i: i32| choose_cycle(m, index(i - 1))));
    ui.on_m_toggle_only(a.on0(|m, _| {
        m.only = !m.only && m.cycle.is_some();
        m.page = 0;
    }));
    ui.on_m_page(a.on(|m, _, d: i32| {
        m.page = (m.page as i64 + d as i64).max(0) as usize;
    }));
    let cell = a.on2(|m, _, row: i32, col: i32| {
        m.evidence = Some((index(row).unwrap_or(0), index(col).unwrap_or(0)));
    });
    let link = with_ui(ui, move |ui, (row, col): (i32, i32)| {
        cell(row, col);
        if let Some(node) = index(row) {
            crate::panes_ui::link_from(ui, "dsm", node);
        }
    });
    ui.on_m_cell(move |row, col| link((row, col)));
    ui.on_m_close_evidence(a.on0(|m, _| m.evidence = None));
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

fn push(ui: &AppWindow, m: &MatrixState, rows: &[MatrixRow]) {
    let texts = Texts::of(ui);
    let locale = texts.locale();
    let mx = &m.matrix;
    let page = m.page;
    let path: BTreeSet<usize> = m
        .cycle
        .map(|c| mx.cycles[c].path.iter().copied().collect())
        .unwrap_or_default();
    ui.set_m_columns(model(mx.columns.iter().map(|c| column(&c.target, &c.name))));
    ui.set_m_rows(model(rows.iter().skip(page * PAGE).take(PAGE).map(|r| {
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
    })));
    let evidence = if mx.total_exact {
        Value::Count(mx.total as usize)
    } else {
        Value::AtLeast(mx.total as usize)
    };
    ui.set_m_kpis(model([
        kpi::MATRIX_FILES.item(&texts, mx.files),
        kpi::DEPENDENCY_EVIDENCE.item(&texts, evidence),
        kpi::FILE_CYCLE_GROUPS.item(&texts, mx.cycles.len()),
    ]));
    push_marked(ui, m, rows);
    ui.set_m_row_start((page * PAGE) as i32);
    ui.set_m_row_total(rows.len() as i32);
    ui.set_m_only(m.only);
    let mut scopes = vec![SharedString::from(texts.t("Project overview"))];
    scopes.extend(
        m.scopes
            .iter()
            .map(|&s| SharedString::from(m.tree.nodes[s].name.as_str())),
    );
    ui.set_m_scopes(model(scopes));
    let mut cycles = vec![SharedString::from(texts.t("All dependencies"))];
    cycles.extend(
        mx.cycles
            .iter()
            .map(|c| SharedString::from(format!("{} · {}", c.id, c.members.len()))),
    );
    ui.set_m_cycle_options(model(cycles));
    ui.set_m_cycle_path(cycle_path(m).into());
    // Position 0 of both lists is "Project overview" / "All dependencies".
    let scope = m
        .scope
        .and_then(|s| m.scopes.iter().position(|&n| n == s))
        .map_or(0, |i| i + 1);
    ui.set_m_scope_index(scope as i32);
    ui.set_m_cycle_index(m.cycle.map_or(0, |c| c + 1) as i32);
    ui.set_m_query(m.query.as_str().into());
    push_evidence(ui, m);
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
fn push_marked(ui: &AppWindow, m: &MatrixState, rows: &[MatrixRow]) {
    let shown = rows.iter().skip(m.page * PAGE).take(PAGE);
    let marked = shown.into_iter().position(|r| m.marked == Some(r.node));
    ui.set_m_marked_row(marked.map_or(-1, |i| i as i32));
}

/// Title and list of the open cell; both empty when no cell with
/// dependencies is open.
fn push_evidence(ui: &AppWindow, m: &MatrixState) {
    let (title, lines) = evidence(m).unwrap_or_default();
    ui.set_m_evidence_title(title.into());
    ui.set_m_evidence(model(lines));
}

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
}
