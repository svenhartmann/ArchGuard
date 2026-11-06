//! The detail panel of a selection: the port of `renderWorldContext` and
//! the evidence part of `worldDetails`. Each line has a kind; the view
//! holds the translated template per kind (see `start_details.slint`).

use std::collections::BTreeMap;

use slint::{ModelRc, VecModel};

use super::state::{Lens, Selection, StartState};
use crate::format::{Locale, known};
use crate::ui::{StartLine, StartModel};

/// Line kinds shared with `start_details.slint`.
pub mod kind {
    pub const TEXT: i32 = 0;
    pub const MUTED: i32 = 1;
    pub const PATH: i32 = 2;
    pub const CLASS: i32 = 10;
    pub const FILE_ONLY: i32 = 11;
    pub const MAX_CC: i32 = 12;
    pub const MEASURED_METHODS: i32 = 13;
    pub const KPI_VALUE: i32 = 14;
    pub const ENTRIES_MEASURED: i32 = 15;
    pub const PICK_CLASS: i32 = 16;
    pub const FILE_COVERAGE: i32 = 17;
    pub const COVERAGE_UNKNOWN: i32 = 18;
    pub const FILES_MEASURED: i32 = 19;
    pub const IN_CYCLE: i32 = 20;
    pub const NO_CYCLE: i32 = 21;
    pub const FINDINGS: i32 = 22;
    pub const METHODS: i32 = 23;
    pub const METHOD: i32 = 24;
    pub const NO_KPI: i32 = 25;
    pub const METHOD_RECORDS: i32 = 26;
    pub const MEMBER: i32 = 30;
    pub const MORE: i32 = 31;
    pub const EVIDENCE: i32 = 40;
    pub const CONNECTION: i32 = 41;
    pub const NO_EVIDENCE: i32 = 42;
    /// An open connection's dependency count (`a`, "" unknown).
    pub const REFERENCES: i32 = 43;
    /// "Select neighbor" in the architecture World: `entity` the area.
    pub const NEIGHBOR: i32 = 44;
    // The services World (`services/view.rs`).
    pub const SVC_NOTE: i32 = 50;
    pub const SVC_EXTERNAL: i32 = 51;
    /// `a` → `b`, `entity` the kind (`services::KINDS`), `flag` cyclic.
    pub const SVC_LINK: i32 = 52;
    pub const SVC_AT: i32 = 53;
    /// "Select neighbor": `entity` the island.
    pub const SVC_NEIGHBOR: i32 = 54;
    pub const SVC_NONE: i32 = 55;
}

pub(super) fn line(
    kind: i32,
    a: impl Into<String>,
    b: impl Into<String>,
    c: impl Into<String>,
) -> StartLine {
    StartLine {
        kind,
        a: a.into().into(),
        b: b.into().into(),
        c: c.into().into(),
        entity: -1,
        flag: false,
        id: -1,
        open: false,
    }
}

fn opt(l: Locale, v: Option<f64>) -> String {
    known(v, |v| l.num(v, 2))
}

/// A count in the window's number format.
fn n(l: Locale, v: impl TryInto<i64>) -> String {
    l.int(v.try_into().unwrap_or(i64::MAX))
}

/// Fill the panel fields of the model for the current selection.
pub fn fill(m: &mut StartModel, s: &StartState, l: Locale) {
    let Some(sel) = &s.selection else { return };
    let Some(area) = s.scope.areas.get(sel.area) else {
        return;
    };
    m.has_selection = true;
    m.details_open = s.details_open;
    m.can_explore = sel.entity.is_none() && area.module;
    m.detail_title = sel
        .entity
        .map(|e| {
            let e = s.ev.entity(e);
            e.class_id.clone().unwrap_or_else(|| e.node.clone())
        })
        .unwrap_or_else(|| area.name.clone())
        .into();
    let mut lines = Vec::new();
    match s.lens {
        Lens::Explore => explore(l, &mut lines, s, sel),
        Lens::Coverage => coverage(l, &mut lines, s, sel),
        Lens::Problems => problems(l, &mut lines, s, sel),
    }
    if sel.entity.is_none() {
        members(l, &mut lines, s, sel);
    }
    evidence(&mut lines, s, sel);
    m.detail_lines = ModelRc::new(VecModel::from(lines));
}

fn members_of<'a>(s: &'a StartState, sel: &'a Selection) -> Vec<usize> {
    match (sel.entity, &sel.members) {
        (Some(e), _) => vec![e],
        (None, Some(m)) => m.clone(),
        (None, None) => s.scope.areas[sel.area].entities.clone(),
    }
}

fn explore(l: Locale, out: &mut Vec<StartLine>, s: &StartState, sel: &Selection) {
    let Some(e) = sel.entity else {
        let values: Vec<f64> = members_of(s, sel)
            .into_iter()
            .filter_map(|e| s.value(e))
            .collect();
        let max = values.iter().copied().reduce(f64::max);
        out.push(line(
            kind::ENTRIES_MEASURED,
            n(l, values.len()),
            n(l, members_of(s, sel).len()),
            opt(l, max),
        ));
        out.push(line(kind::PICK_CLASS, "", "", ""));
        return;
    };
    let ent = s.ev.entity(e);
    out.push(match &ent.class_id {
        Some(id) => line(kind::CLASS, id.as_str(), "", ""),
        None => line(kind::FILE_ONLY, "", "", ""),
    });
    if s.kpi_active() {
        let known = ent
            .class_id
            .as_ref()
            .is_some_and(|id| s.ev.kpis.contains_key(id));
        out.push(line(kind::KPI_VALUE, opt(l, s.value(e)), "", ""));
        if !known {
            out.push(line(kind::NO_KPI, "", "", ""));
        }
    } else {
        out.push(line(
            kind::MAX_CC,
            opt(l, ent.max_complexity.map(f64::from)),
            "",
            "",
        ));
        out.push(line(
            kind::MEASURED_METHODS,
            n(l, ent.measured_methods),
            n(l, ent.declared_methods),
            "",
        ));
    }
    methods(l, out, s, e);
}

/// `worldClassMethods`, highest complexity first.
fn methods(l: Locale, out: &mut Vec<StartLine>, s: &StartState, e: usize) {
    let ent = s.ev.entity(e);
    let key = ent
        .class_id
        .as_deref()
        .unwrap_or(ent.file.as_deref().unwrap_or(&ent.node));
    let mut rows: Vec<_> =
        s.ev.methods
            .get(key)
            .map(|m| m.iter().collect())
            .unwrap_or_default();
    rows.sort_by(|a, b| {
        let cc = |m: &super::evidence::Method| m.cyclomatic.map_or(-1, i64::from);
        cc(b).cmp(&cc(a)).then_with(|| a.name.cmp(&b.name))
    });
    out.push(line(kind::METHODS, "", "", ""));
    out.push(line(
        kind::METHOD_RECORDS,
        n(l, s.limit.min(rows.len())),
        n(l, rows.len()),
        "",
    ));
    for m in rows.iter().take(s.limit) {
        let cc = known(m.cyclomatic, |c| n(l, c));
        out.push(line(kind::METHOD, m.name.as_str(), cc, ""));
        out.push(line(kind::PATH, format!("{}:{}", m.file, m.line), "", ""));
    }
}

fn coverage(l: Locale, out: &mut Vec<StartLine>, s: &StartState, sel: &Selection) {
    let files: BTreeMap<&str, usize> = members_of(s, sel)
        .into_iter()
        .map(|e| (s.ev.entity(e).node.as_str(), e))
        .collect();
    let measured: Vec<_> = files
        .values()
        .filter_map(|&e| s.ev.entity(e).coverage.as_ref()?.lines.clone())
        .collect();
    if measured.is_empty() {
        out.push(line(kind::COVERAGE_UNKNOWN, "", "", ""));
    } else {
        let covered: u64 = measured.iter().map(|l| l.covered).sum();
        let total: u64 = measured.iter().map(|l| l.total).sum();
        out.push(line(
            kind::FILE_COVERAGE,
            n(l, covered),
            n(l, total),
            n(l, total.saturating_sub(covered)),
        ));
    }
    out.push(line(
        kind::FILES_MEASURED,
        n(l, measured.len()),
        n(l, files.len()),
        "",
    ));
}

fn problems(l: Locale, out: &mut Vec<StartLine>, s: &StartState, sel: &Selection) {
    let area = &s.scope.areas[sel.area];
    out.push(line(
        if area.cyclic {
            kind::IN_CYCLE
        } else {
            kind::NO_CYCLE
        },
        "",
        "",
        "",
    ));
    out.push(line(kind::FINDINGS, n(l, area.findings.len()), "", ""));
    for &f in area.findings.iter().take(s.limit) {
        out.push(line(kind::TEXT, finding_text(s, f), "", ""));
    }
}

fn finding_text(s: &StartState, f: usize) -> String {
    let f = &s.ev.findings[f];
    f.message.clone().unwrap_or_else(|| format!("{:?}", f.kind))
}

/// The member list of an area or a grouped surface.
fn members(l: Locale, out: &mut Vec<StartLine>, s: &StartState, sel: &Selection) {
    let label = |e: usize| {
        let e = s.ev.entity(e);
        e.class_id.clone().unwrap_or_else(|| e.node.clone())
    };
    let mut rows = members_of(s, sel);
    if s.lens == Lens::Explore {
        rows.sort_by(|&a, &b| {
            let v = |e: usize| s.value(e).unwrap_or(-1.0);
            v(b).total_cmp(&v(a)).then_with(|| label(a).cmp(&label(b)))
        });
    } else {
        rows.sort_by_key(|&e| label(e));
    }
    for &e in rows.iter().take(s.limit) {
        let value = if s.lens == Lens::Explore {
            opt(l, s.value(e))
        } else {
            String::new()
        };
        out.push(StartLine {
            entity: e as i32,
            ..line(kind::MEMBER, label(e), value, "")
        });
    }
    if rows.len() > s.limit {
        out.push(line(kind::MORE, n(l, rows.len() - s.limit), "", ""));
    }
}

/// The entry of connection `i` of the scope; open, its dependency count
/// and the way to the neighbour of `area`.
fn connection(out: &mut Vec<StartLine>, s: &StartState, area: usize, i: usize) {
    let c = &s.scope.connections[i];
    let areas = &s.scope.areas;
    let open = s.connection_open(i);
    out.push(StartLine {
        id: i as i32,
        open,
        ..line(
            kind::CONNECTION,
            areas[c.from].name.as_str(),
            areas[c.to].name.as_str(),
            if c.cyclic { "1" } else { "" },
        )
    });
    if open {
        let count = c.references.map_or(String::new(), |n| n.to_string());
        out.push(line(kind::REFERENCES, count, "", ""));
        let other = if c.from == area { c.to } else { c.from };
        out.push(StartLine {
            entity: other as i32,
            ..line(kind::NEIGHBOR, "", "", "")
        });
    }
}

/// Connections of the area at this level and its findings with evidence.
fn evidence(out: &mut Vec<StartLine>, s: &StartState, sel: &Selection) {
    out.push(line(kind::EVIDENCE, "", "", ""));
    let areas = &s.scope.areas;
    let edges: Vec<_> = (0..s.scope.connections.len())
        .filter(|&i| {
            let c = &s.scope.connections[i];
            c.from == sel.area || c.to == sel.area
        })
        .collect();
    for &i in edges.iter().take(s.limit) {
        connection(out, s, sel.area, i);
    }
    let findings = &areas[sel.area].findings;
    for &f in findings.iter().take(s.limit) {
        out.push(line(kind::TEXT, finding_text(s, f), "", ""));
        for r in s.ev.findings[f].evidence.iter().take(5) {
            out.push(line(
                kind::MUTED,
                format!("{}:{} {}", r.file, r.line, r.raw),
                "",
                "",
            ));
        }
    }
    if edges.is_empty() && findings.is_empty() {
        out.push(line(kind::NO_EVIDENCE, "", "", ""));
    }
    if edges.len() > s.limit || findings.len() > s.limit {
        out.push(line(kind::MORE, "", "", ""));
    }
}
