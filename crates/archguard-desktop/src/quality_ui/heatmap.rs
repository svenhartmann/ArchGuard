//! Class heatmap: `viewer_class_heatmap.js` in Rust. Colors compare a
//! measured value with all measured classes of the same source language
//! in the original project (the cohort); search and scope never change it.

use archguard_engine::ClassMetrics;

pub use crate::rating::{Cohorts, METRICS, Rating, not_applicable, value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    High,
    Low,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Priority,
    Name,
    /// One of [`METRICS`], highest first.
    Metric(usize),
}

/// `classHeatmapPriority`: high counts 2, elevated 1.
pub fn priority(cohorts: &Cohorts, row: &ClassMetrics) -> u32 {
    METRICS
        .iter()
        .map(|k| match cohorts.assess(row, k).state {
            Rating::High => 2,
            Rating::Medium => 1,
            _ => 0,
        })
        .sum()
}

fn keep(cohorts: &Cohorts, row: &ClassMetrics, filter: Filter) -> bool {
    let rated = || {
        METRICS
            .iter()
            .filter(|&&k| k != "noc")
            .map(|k| cohorts.assess(row, k).state)
    };
    match filter {
        Filter::All => true,
        Filter::High => rated().any(|s| s == Rating::High),
        Filter::Low => rated().all(|s| s == Rating::Low),
        Filter::Unknown => METRICS
            .iter()
            .any(|k| cohorts.assess(row, k).state == Rating::Unknown),
    }
}

/// `classHeatmapRows`: filter, then sort (ties by class id).
pub fn rows<'a>(
    cohorts: &Cohorts,
    rows: Vec<&'a ClassMetrics>,
    filter: Filter,
    sort: Sort,
) -> Vec<&'a ClassMetrics> {
    let mut keyed: Vec<(&ClassMetrics, f64)> = rows
        .into_iter()
        .filter(|r| keep(cohorts, r, filter))
        .map(|r| {
            let key = match sort {
                Sort::Name => 0.0,
                Sort::Priority => f64::from(priority(cohorts, r)),
                Sort::Metric(i) => value(r, METRICS[i]).unwrap_or(-1.0),
            };
            (r, key)
        })
        .collect();
    keyed.sort_by(|(a, ka), (b, kb)| kb.total_cmp(ka).then_with(|| a.class_id.cmp(&b.class_id)));
    keyed.into_iter().map(|(r, _)| r).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    pub use crate::rating::tests::{class, count};

    use super::*;

    fn java(values: &[u64]) -> Vec<ClassMetrics> {
        values
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                class(
                    &format!("C{i}"),
                    &format!("src/C{i}.java"),
                    Some(v),
                    Some(1),
                )
            })
            .collect()
    }

    #[test]
    fn review_first_sorts_by_priority_then_name() {
        let rows = java(&[1, 2, 3, 4, 5, 6]);
        let cohorts = Cohorts::new(&rows);
        // C5: cbo rfc dit wmc high (2 each) = 8, noc info, lcom4 low.
        assert_eq!(priority(&cohorts, &rows[5]), 8);
        assert_eq!(priority(&cohorts, &rows[3]), 4);
        let refs: Vec<_> = rows.iter().collect();
        let sorted = super::rows(&cohorts, refs.clone(), Filter::All, Sort::Priority);
        let ids: Vec<_> = sorted.iter().map(|r| r.class_id.as_str()).collect();
        // C4 and C5 tie at 8 and keep name order.
        assert_eq!(ids, ["C4", "C5", "C3", "C0", "C1", "C2"]);
        let by_wmc = super::rows(&cohorts, refs.clone(), Filter::All, Sort::Metric(4));
        assert_eq!(by_wmc[0].class_id, "C5");
        let by_name = super::rows(&cohorts, refs.clone(), Filter::All, Sort::Name);
        assert_eq!(by_name[0].class_id, "C0");
    }

    #[test]
    fn filters_select_high_low_and_missing() {
        let mut rows = java(&[1, 2, 3, 4, 5, 6]);
        rows.push(class("C9", "src/C9.java", None, None));
        let cohorts = Cohorts::new(&rows);
        let refs: Vec<_> = rows.iter().collect();
        let ids = |f| {
            super::rows(&cohorts, refs.clone(), f, Sort::Name)
                .iter()
                .map(|r| r.class_id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(Filter::High), ["C4", "C5"]);
        assert_eq!(ids(Filter::Low), ["C0", "C1", "C2"]);
        assert_eq!(ids(Filter::Unknown), ["C9"]);
        assert_eq!(ids(Filter::All).len(), 7);
    }
}
