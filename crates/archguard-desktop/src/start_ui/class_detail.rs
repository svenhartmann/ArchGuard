//! The Explore details of one class: the class-metric part of
//! `renderWorldContext` (viewer_world.js) with `classMetricInterpretation`
//! (viewer_class_heatmap.js), the method rows of `worldClassMethods` and
//! "Other class metrics". Method rows are entries that open and close
//! (`toggle-section`), as the web's `<details data-world-method>`. Ratings come from [`crate::rating`], the one
//! rating the Quality heatmap uses.

use std::collections::BTreeMap;

use super::evidence::{KPIS, Method, Rating};
use super::lines::{kind, line};
use super::state::StartState;
use crate::format::{Locale, UNKNOWN, known};
use crate::rating;
use crate::ui::StartLine;

/// The rating's template in `start_details.slint` (`StartLine.entity`).
fn state_code(r: Rating) -> i32 {
    match r {
        Rating::NotApplicable => 0,
        Rating::Low => 1,
        Rating::Medium => 2,
        Rating::High => 3,
        Rating::Unknown => 4,
        Rating::Small => 5,
        Rating::Equal => 6,
        Rating::Info => 7,
    }
}

fn number(l: Locale, v: Option<f64>) -> String {
    known(v, |v| l.num(v, 2))
}

/// The interpretation of the shown class metric (`kpi` 1..=6), its
/// observed lower bound when incomplete, and its evidence.
pub fn kpi(l: Locale, out: &mut Vec<StartLine>, s: &StartState, class_id: &str) {
    let Some(row) = s.ev.classes.get(class_id) else {
        out.push(line(kind::NO_KPI, "", "", ""));
        return;
    };
    let key = KPIS[s.kpi - 1];
    let a = s.ev.cohorts.assess(row, key);
    out.push(StartLine {
        entity: state_code(a.state),
        ..line(
            kind::KPI_NOTE,
            number(l, a.value),
            l.count(a.population),
            "",
        )
    });
    let Some(m) = measurement(row, key) else {
        return;
    };
    if a.value.is_none() && !rating::not_applicable(row, key) {
        out.push(line(kind::KPI_OBSERVED, l.int(m.observed as i64), "", ""));
    }
    for e in m.evidence.iter().take(s.limit) {
        out.push(line(kind::PATH, e.as_str(), "", ""));
    }
    if m.evidence.len() > s.limit {
        out.push(line(kind::MORE, "", "", ""));
    }
}

/// The count metric behind `key`; LCOM4 has no observed count.
fn measurement<'a>(
    row: &'a archguard_engine::ClassMetrics,
    key: &str,
) -> Option<&'a archguard_engine::CountMetric> {
    match key {
        "cbo" => Some(&row.cbo),
        "rfc" => Some(&row.rfc),
        "dit" => Some(&row.dit),
        "noc" => Some(&row.noc),
        "wmc" => Some(&row.wmc),
        _ => None,
    }
}

/// "Other class metrics": every metric of the class, unknown as `—`.
pub fn others(l: Locale, out: &mut Vec<StartLine>, s: &StartState, class_id: &str) {
    let Some(row) = s.ev.classes.get(class_id) else {
        return;
    };
    out.push(line(kind::OTHER_METRICS, "", "", ""));
    for key in KPIS {
        let value = number(l, rating::value(row, key));
        out.push(line(kind::KPI_ROW, key.to_uppercase(), value, ""));
    }
}

/// `worldClassMethods`, highest complexity first, each with its
/// measurements and source range.
pub fn methods(l: Locale, out: &mut Vec<StartLine>, s: &StartState, e: usize) {
    let ent = s.ev.entity(e);
    let key = ent
        .class_id
        .as_deref()
        .unwrap_or(ent.file.as_deref().unwrap_or(&ent.node));
    let mut rows: Vec<&Method> =
        s.ev.methods
            .get(key)
            .map(|m| m.iter().collect())
            .unwrap_or_default();
    rows.sort_by(|a, b| {
        let cc = |m: &Method| m.cyclomatic.map_or(-1, i64::from);
        cc(b).cmp(&cc(a)).then_with(|| a.name.cmp(&b.name))
    });
    out.push(line(kind::METHODS, "", "", ""));
    out.push(line(
        kind::METHOD_RECORDS,
        l.count(s.limit.min(rows.len())),
        l.count(rows.len()),
        "",
    ));
    for (i, m) in rows.iter().take(s.limit).enumerate() {
        let open = opened(&s.sections, &method_key(m), i == 0, s.kpi_active());
        method(l, out, m, open);
    }
    if rows.len() > s.limit {
        out.push(line(kind::MORE, l.count(rows.len() - s.limit), "", ""));
    }
}

/// The key of a method entry for `toggle-section`.
fn method_key(m: &Method) -> String {
    format!("method:{}:{}#{}", m.file, m.line, m.name)
}

/// Whether a method entry is open: toggled by the user, else the web's
/// `item.open=!worldKpiActive()&&method===methods[0]`.
fn opened(sections: &BTreeMap<String, bool>, key: &str, first: bool, kpi: bool) -> bool {
    sections.get(key).copied().unwrap_or(first && !kpi)
}

/// One method entry: name and CC; open, also nesting, range, Cognitive
/// and MI.
fn method(l: Locale, out: &mut Vec<StartLine>, m: &Method, open: bool) {
    let int = |v: Option<u32>| known(v, |v| l.int(v.into()));
    out.push(StartLine {
        open,
        ..line(
            kind::METHOD,
            m.name.as_str(),
            int(m.cyclomatic),
            method_key(m),
        )
    });
    if !open {
        return;
    }
    if m.cyclomatic.is_some() {
        out.push(line(kind::METHOD_CC, int(m.cyclomatic), int(m.nesting), ""));
    }
    let end = m.end_line.map_or(String::new(), |e| format!("–{e}"));
    // `flag`: indented under its entry.
    out.push(StartLine {
        flag: true,
        ..line(kind::PATH, format!("{}:{}{end}", m.file, m.line), "", "")
    });
    if m.cognitive.is_some() || m.maintainability.is_some() {
        let mi = m.maintainability.map_or(UNKNOWN.into(), |v| l.num(v, 2));
        out.push(line(kind::METHOD_MAINT, int(m.cognitive), mi, ""));
    }
    if m.cyclomatic.is_none() {
        out.push(line(kind::METHOD_NO_CC, "", "", ""));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(m: Method) -> Vec<(i32, String, String)> {
        let mut out = Vec::new();
        method(Locale::En, &mut out, &m, true);
        out.iter()
            .map(|l| (l.kind, l.a.to_string(), l.b.to_string()))
            .collect()
    }

    fn base() -> Method {
        Method {
            name: "run()".into(),
            cyclomatic: Some(4),
            nesting: Some(2),
            file: "src/a.ts".into(),
            line: 10,
            end_line: Some(20),
            cognitive: Some(3),
            maintainability: Some(61.234),
        }
    }

    /// A measured method lists CC and nesting, its range, Cognitive and MI.
    #[test]
    fn a_method_row_lists_its_measurements_and_range() {
        let s = |v: &str| v.to_string();
        assert_eq!(
            row(base()),
            [
                (kind::METHOD, s("run()"), s("4")),
                (kind::METHOD_CC, s("4"), s("2")),
                (kind::PATH, s("src/a.ts:10–20"), s("")),
                (kind::METHOD_MAINT, s("3"), s("61.23")),
            ]
        );
    }

    /// Without measurements the row says so and keeps the line only.
    #[test]
    fn an_unmeasured_method_says_so() {
        let m = Method {
            cyclomatic: None,
            nesting: None,
            end_line: None,
            cognitive: None,
            maintainability: None,
            ..base()
        };
        let kinds: Vec<i32> = row(m).iter().map(|r| r.0).collect();
        assert_eq!(kinds, [kind::METHOD, kind::PATH, kind::METHOD_NO_CC]);
        assert_eq!(state_code(Rating::Info), 7);
    }

    /// A closed method entry is its summary only; it carries its key and
    /// open state for the toggle.
    #[test]
    fn a_closed_method_is_its_summary_only() {
        let mut out = Vec::new();
        method(Locale::En, &mut out, &base(), false);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].kind, out[0].open), (kind::METHOD, false));
        assert_eq!(out[0].c, method_key(&base()));
        let mut open = Vec::new();
        method(Locale::En, &mut open, &base(), true);
        assert!(open[0].open);
    }

    /// As the web: the top method starts open in the complexity lens,
    /// every method closed under a class metric; a toggle wins.
    #[test]
    fn the_top_method_starts_open_unless_a_metric_is_shown() {
        let none = BTreeMap::new();
        assert!(opened(&none, "k", true, false));
        assert!(!opened(&none, "k", false, false));
        assert!(!opened(&none, "k", true, true));
        let toggled = BTreeMap::from([("k".to_string(), false)]);
        assert!(!opened(&toggled, "k", true, false));
    }
}
