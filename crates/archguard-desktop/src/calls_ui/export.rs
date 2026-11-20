//! The Calls tab as the export's [`CallsInput`]: what the tab shows in
//! its mode, with the toolbar filters under the web's names.

use serde_json::{Value, json};

use super::cycles::Group;
use super::model::{ElementKind, Resolution};
use super::rankings::{self, Sort};
use super::state::{Mode, State};
use crate::export::{CallCycle, CallEdge, CallRank, CallsInput};

fn kind_name(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Class => "class",
        ElementKind::Method => "method",
    }
}

fn filters(s: &State) -> Value {
    let f = &s.filter;
    let resolution = match f.resolution {
        Resolution::Project => "project",
        Resolution::External => "external",
        Resolution::Unresolved => "unresolved",
        Resolution::All => "all",
    };
    let element = s
        .element
        .as_ref()
        .map(|e| json!({"kind": kind_name(e.kind), "id": e.id}));
    json!({"language": f.language, "package": f.package, "query": f.query,
        "resolution": resolution, "include_internal": f.internal, "element": element})
}

fn cycle(g: &Group) -> CallCycle {
    CallCycle {
        id: g.id.clone(),
        kind: kind_name(g.kind).into(),
        members: g.members.clone(),
        witness: g.witness.clone(),
        edges: (g.edges.iter())
            .map(|e| CallEdge {
                from: e.from.clone(),
                to: e.to.clone(),
                resolution: "project".into(),
                calls: e.calls.clone(),
            })
            .collect(),
        call_sites: g.call_sites,
    }
}

fn sort_name(sort: Sort) -> &'static str {
    match sort {
        Sort::Incoming => "incoming",
        Sort::Outgoing => "outgoing",
        Sort::Total => "total",
        Sort::Cyclomatic => "cyclomatic",
        Sort::MaxNesting => "max_nesting",
    }
}

fn rankings(s: &State) -> (String, String, Vec<CallRank>) {
    let rows = (s.view.rank_rows.iter())
        .map(|r| CallRank {
            name: rankings::label(&s.idx, s.rank_method, &r.id, &s.labels),
            incoming: r.incoming,
            outgoing: r.outgoing,
            total: r.total,
            cyclomatic: r.cyclomatic,
            max_nesting: r.max_nesting,
        })
        .collect();
    let level = if s.rank_method { "method" } else { "class" };
    (level.into(), sort_name(s.rank_sort).into(), rows)
}

/// The export input of the tab state `s`.
pub fn input<'a>(s: &'a State, project: &str) -> CallsInput<'a> {
    let (idx, l) = (&s.idx, &s.labels);
    let mut input = CallsInput {
        project: project.into(),
        analysis: &s.idx.a,
        filters: filters(s),
        nodes: Vec::new(),
        edges: Vec::new(),
        cycles: None,
        rankings: None,
    };
    if s.mode == Mode::Cycles {
        let groups = s.cycles.groups(s.cycle_kind);
        let matching: Vec<CallCycle> = s.view.groups.iter().map(|&g| cycle(&groups[g])).collect();
        let at = s
            .view
            .group
            .and_then(|g| s.view.groups.iter().position(|&m| m == g));
        let members = s.group().map(|g| g.members.clone()).unwrap_or_default();
        input.nodes = (members.into_iter())
            .map(|id| (idx.qualified(s.cycle_kind, &id, l), id))
            .map(|(name, id)| (id, name))
            .collect();
        input.cycles = Some((matching, at));
        return input;
    }
    input.edges = (s.view.relations.iter())
        .map(|r| CallEdge {
            from: r.from.clone(),
            to: r.to.clone().unwrap_or_else(|| r.label.clone()),
            resolution: r.resolution.clone(),
            calls: r.calls.clone(),
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    for e in &input.edges {
        for id in [&e.from, &e.to] {
            if seen.insert(id.clone()) {
                input.nodes.push((id.clone(), idx.class_label(Some(id), l)));
            }
        }
    }
    if s.mode == Mode::Rankings {
        input.rankings = Some(rankings(s));
    }
    input
}

#[cfg(test)]
mod tests {
    use super::super::model::fixture;
    use super::*;
    use crate::export::calls_snapshot;

    /// Each mode exports what the tab shows: connections with their call
    /// sites, the selected cycle group, or the ranked rows.
    #[test]
    fn each_mode_exports_what_the_tab_shows() {
        let mut s = State::new(fixture::analysis());
        let v = calls_snapshot(&input(&s, "p")).0;
        assert_eq!(v["filters"]["package"], "blue");
        assert_eq!(v["metrics"]["shown_dependencies"], s.view.relations.len());
        let sites: usize = s.view.relations.iter().map(|r| r.calls.len()).sum();
        assert_eq!(v["metrics"]["call_sites"], sites);
        s.mode = Mode::Cycles;
        s.refresh();
        let g = s.group().expect("the fixture has a cycle").clone();
        let v = calls_snapshot(&input(&s, "p")).0;
        assert_eq!(v["selected_cycle"]["witness"], json!(g.witness));
        assert_eq!(v["edges"].as_array().unwrap().len(), g.edges.len());
        assert_eq!(v["metrics"]["call_sites"], g.call_sites);
        assert_eq!(v["nodes"].as_array().unwrap().len(), g.members.len());
        s.mode = Mode::Rankings;
        s.refresh();
        let v = calls_snapshot(&input(&s, "p")).0;
        assert_eq!(
            v["rankings"]["rows"].as_array().unwrap().len(),
            s.view.rank_rows.len()
        );
        assert!(!s.view.rank_rows.is_empty());
    }
}
