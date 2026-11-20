//! "Target architecture": the declared component model
//! (`architecture.model`) against the source dependencies, as the web
//! viewer's renderArchitecture and architectureRelations. The report is
//! `archguard_engine::analyze_target_architecture`, as `archguard
//! architecture` prints it; [`super::target_view`] makes the window model.

use std::collections::BTreeSet;

use archguard_engine::core::{ArchitectureDependency, Config, Graph, TargetArchitectureReport};

/// Directions per page.
pub(super) const PAGE: usize = 100;

/// The page's report and what the reader chose on it.
pub struct TargetView {
    /// `None`: no `architecture.model` declared.
    pub(super) report: Result<Option<TargetArchitectureReport>, String>,
    pub(super) only_violations: bool,
    pub(super) query: String,
    pub(super) limit: usize,
    pub(super) selected: Option<(String, String)>,
    /// Components whose details (and file list) are open, by name.
    pub(super) open: BTreeSet<String>,
}

/// The direction's state as the view shows it (`ArcTargetState.label`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum State {
    Allowed = 0,
    NotObserved = 1,
    Forbidden = 2,
    Private = 3,
}

impl State {
    pub(super) fn violates(self) -> bool {
        self >= State::Forbidden
    }
}

/// One component direction: observed (`dep`) or only declared allowed.
pub(super) struct Row<'a> {
    pub(super) from: &'a str,
    pub(super) to: &'a str,
    pub(super) state: State,
    pub(super) dep: Option<&'a ArchitectureDependency>,
}

/// Evaluate the project's target model on the full source graph.
pub(super) fn build(graph: &Graph, config: &Config) -> TargetView {
    TargetView {
        report: archguard_engine::analyze_target_architecture(graph, config)
            .map_err(|e| e.to_string()),
        only_violations: false,
        query: String::new(),
        limit: PAGE,
        selected: None,
        open: BTreeSet::new(),
    }
}

fn state(dep: &ArchitectureDependency) -> State {
    if !dep.allowed {
        State::Forbidden
    } else if !dep.private_accesses.is_empty() {
        State::Private
    } else if dep.source_edges == 0 {
        State::NotObserved
    } else {
        State::Allowed
    }
}

/// Every direction matching the filter, violations first, then by name.
pub(super) fn rows<'a>(v: &TargetView, r: &'a TargetArchitectureReport) -> Vec<Row<'a>> {
    let mut rows: Vec<Row<'a>> = r
        .dependencies
        .iter()
        .map(|d| Row {
            from: &d.from,
            to: &d.to,
            state: state(d),
            dep: Some(d),
        })
        .collect();
    for a in &r.allowed_dependencies {
        if !r
            .dependencies
            .iter()
            .any(|d| d.from == a.from && d.to == a.to)
        {
            rows.push(Row {
                from: &a.from,
                to: &a.to,
                state: State::NotObserved,
                dep: None,
            });
        }
    }
    let query = v.query.trim().to_lowercase();
    rows.retain(|row| {
        (!v.only_violations || row.state.violates())
            && format!("{} {}", row.from, row.to)
                .to_lowercase()
                .contains(&query)
    });
    rows.sort_by(|a, b| {
        (!a.state.violates(), a.from, a.to).cmp(&(!b.state.violates(), b.from, b.to))
    });
    rows
}

/// Select the `i`th shown direction, or unselect it when it is selected.
pub(super) fn select(v: &mut TargetView, i: usize) {
    let Ok(Some(r)) = &v.report else { return };
    let Some(row) = rows(v, r).into_iter().take(v.limit).nth(i) else {
        return;
    };
    let key = (row.from.to_string(), row.to.to_string());
    v.selected = (v.selected.as_ref() != Some(&key)).then_some(key);
}

/// Open or close the `i`th component's details.
pub(super) fn toggle_component(v: &mut TargetView, i: usize) {
    let Ok(Some(r)) = &v.report else { return };
    if let Some(c) = r.components.get(i)
        && !v.open.remove(&c.name)
    {
        v.open.insert(c.name.clone());
    }
}
