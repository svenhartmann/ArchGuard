//! Matrix tab state (one per document) and its push into the window.

use std::collections::BTreeSet;

use slint::{ModelRc, SharedString};

use crate::context::Loaded;
use crate::documents;
use crate::graph_ui::tab_support::{Actions, TabState, index, model};
use crate::matrix::{Matrix, Target};
use crate::tree::Tree;
use crate::ui::{AppWindow, MatrixColumn, MatrixRowItem};

/// Rows per page, as the web viewer's matrix.
const PAGE: usize = 120;
/// Evidence edges listed at most for one cell.
const EVIDENCE: usize = 40;

pub struct MatrixState {
    tree: Tree,
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
}

impl MatrixState {
    fn new(tree: Tree, project: String) -> Self {
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
    }
}

/// Show the active document's matrix, building it on first use.
pub fn show(ui: &AppWindow, data: &Loaded) {
    let project = ui.get_project().to_string();
    documents::with_active_state_or(
        || MatrixState::new(Tree::from_graph(&data.run.graph), project),
        |m: &mut MatrixState| push(ui, m),
    );
}

impl TabState for MatrixState {
    fn push(&mut self, ui: &AppWindow) {
        push(ui, self);
    }
}

pub fn wire(ui: &AppWindow) {
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
    ui.on_m_cell(a.on2(|m, _, row: i32, col: i32| {
        m.evidence = Some((index(row).unwrap_or(0), index(col).unwrap_or(0)));
    }));
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

fn push(ui: &AppWindow, m: &MatrixState) {
    let mx = &m.matrix;
    let rows = mx.rows(&m.tree, &m.expanded, &m.query, m.cycle, m.only);
    let pages = rows.len().div_ceil(PAGE).max(1);
    let page = m.page.min(pages - 1);
    let path: BTreeSet<usize> = m
        .cycle
        .map(|c| mx.cycles[c].path.iter().copied().collect())
        .unwrap_or_default();
    ui.set_m_columns(model(
        mx.columns.iter().map(|c| column(m, &c.target, &c.name)),
    ));
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
            badges: badges(mx, r.node).into(),
            cells: model((0..mx.columns.len()).map(|c| {
                cells
                    .and_then(|row| row.get(&c))
                    .map(|cell| format!("{}{}", if cell.exact { "" } else { "≥" }, cell.count))
                    .unwrap_or_default()
                    .into()
            })),
        }
    })));
    ui.set_m_files(mx.files as i32);
    ui.set_m_total(format!("{}{}", if mx.total_exact { "" } else { "≥" }, mx.total).into());
    ui.set_m_cycle_count(mx.cycles.len() as i32);
    ui.set_m_row_start((page * PAGE) as i32);
    ui.set_m_row_total(rows.len() as i32);
    ui.set_m_only(m.only);
    let mut scopes = vec![ui.get_m_overview_label()];
    scopes.extend(
        m.scopes
            .iter()
            .map(|&s| SharedString::from(m.tree.nodes[s].name.as_str())),
    );
    ui.set_m_scopes(model(scopes));
    let mut cycles = vec![ui.get_m_all_label()];
    cycles.extend(
        mx.cycles
            .iter()
            .map(|c| SharedString::from(format!("{} · {}", c.id, c.members.len()))),
    );
    ui.set_m_cycle_options(model(cycles));
    ui.set_m_cycle_path(cycle_path(m).into());
    push_evidence(ui, m);
}

fn column(m: &MatrixState, target: &Target, name: &str) -> MatrixColumn {
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
        Target::Outside => {
            let _ = m;
            MatrixColumn {
                label: "".into(),
                context: "".into(),
                kind: 2,
            }
        }
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

fn push_evidence(ui: &AppWindow, m: &MatrixState) {
    let Some((row, col)) = m.evidence else {
        ui.set_m_evidence_title("".into());
        ui.set_m_evidence(ModelRc::default());
        return;
    };
    let Some(cell) = m.matrix.cells.get(&row).and_then(|r| r.get(&col)) else {
        ui.set_m_evidence_title("".into());
        return;
    };
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
    ui.set_m_evidence_title(format!("{} · {}", name(row), cell.count).into());
    ui.set_m_evidence(model(lines));
}
