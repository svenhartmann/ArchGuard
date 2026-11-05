//! Class heatmap: `viewer_class_heatmap.js` in Rust. Colors compare a
//! measured value with all measured classes of the same source language
//! in the original project (the cohort); search and scope never change it.

use std::collections::BTreeMap;

use archguard_engine::{ClassMetrics, CountMetric};

/// Metric columns, in the web's order.
pub const METRICS: [&str; 6] = ["cbo", "rfc", "dit", "noc", "wmc", "lcom4"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    NotApplicable,
    Low,
    Medium,
    High,
    Unknown,
    /// Fewer than five measurements or an unknown language.
    Small,
    /// All measurements of the cohort are equal.
    Equal,
    Info,
}

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

#[derive(Clone, Debug, PartialEq)]
pub struct Assessment {
    pub value: Option<f64>,
    pub state: State,
    pub population: usize,
}

fn metric<'a>(row: &'a ClassMetrics, key: &str) -> Option<&'a CountMetric> {
    match key {
        "cbo" => Some(&row.cbo),
        "rfc" => Some(&row.rfc),
        "dit" => Some(&row.dit),
        "noc" => Some(&row.noc),
        "wmc" => Some(&row.wmc),
        _ => None,
    }
}

/// `classMetricValue`: a complete measurement, else `None`.
pub fn value(row: &ClassMetrics, key: &str) -> Option<f64> {
    if key == "lcom4" {
        return row.lcom4.map(|v| v as f64);
    }
    let m = metric(row, key)?;
    m.value
        .filter(|_| m.status.is_empty() || m.status == "ready")
        .map(|v| v as f64)
}

/// `true` when the metric does not apply to the class (Go embedding).
pub fn not_applicable(row: &ClassMetrics, key: &str) -> bool {
    metric(row, key).is_some_and(|m| m.status == "not_applicable")
}

/// `classMetricLanguage`: the cohort language from the file extension.
pub fn language(row: &ClassMetrics) -> &'static str {
    let ext = row.file.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "go" => "go",
        "java" => "java",
        "cs" => "csharp",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        _ => "unknown",
    }
}

/// Sorted measured values per `(language, metric)`.
#[derive(Debug, Default)]
pub struct Cohorts(BTreeMap<(&'static str, &'static str), Vec<f64>>);

impl Cohorts {
    pub fn new(classes: &[ClassMetrics]) -> Self {
        let mut map: BTreeMap<_, Vec<f64>> = BTreeMap::new();
        for row in classes {
            for key in METRICS {
                if let Some(v) = value(row, key) {
                    map.entry((language(row), key)).or_default().push(v);
                }
            }
        }
        for values in map.values_mut() {
            values.sort_by(f64::total_cmp);
        }
        Cohorts(map)
    }

    /// `classMetricAssessment`.
    pub fn assess(&self, row: &ClassMetrics, key: &'static str) -> Assessment {
        let values = self
            .0
            .get(&(language(row), key))
            .map_or(&[][..], Vec::as_slice);
        let value = value(row, key);
        let state = match value {
            None if not_applicable(row, key) => State::NotApplicable,
            None => State::Unknown,
            Some(v) => rate(key, v, language(row), values),
        };
        Assessment {
            value,
            state,
            population: values.len(),
        }
    }
}

fn rate(key: &str, v: f64, language: &str, values: &[f64]) -> State {
    if key == "noc" {
        return State::Info;
    }
    if v == 0.0 || (key == "lcom4" && v == 1.0) {
        return State::Low;
    }
    if language == "unknown" || values.len() < 5 {
        return State::Small;
    }
    if values[0] == values[values.len() - 1] {
        return State::Equal;
    }
    // Strictly smaller values: equal measurements share a rank.
    let rank = values.partition_point(|&x| x < v) as f64 / (values.len() - 1) as f64;
    if rank >= 0.8 {
        State::High
    } else if rank >= 0.5 {
        State::Medium
    } else {
        State::Low
    }
}

/// `classHeatmapPriority`: high counts 2, elevated 1.
pub fn priority(cohorts: &Cohorts, row: &ClassMetrics) -> u32 {
    METRICS
        .iter()
        .map(|k| match cohorts.assess(row, k).state {
            State::High => 2,
            State::Medium => 1,
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
        Filter::High => rated().any(|s| s == State::High),
        Filter::Low => rated().all(|s| s == State::Low),
        Filter::Unknown => METRICS
            .iter()
            .any(|k| cohorts.assess(row, k).state == State::Unknown),
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
    use super::*;

    pub fn count(value: Option<u64>, status: &str) -> CountMetric {
        CountMetric {
            value,
            observed: value.unwrap_or(0),
            status: status.into(),
            evidence: Vec::new(),
        }
    }

    /// A class with all five count metrics at `v` (ready) and LCOM4 `lcom4`.
    pub fn class(id: &str, file: &str, v: Option<u64>, lcom4: Option<usize>) -> ClassMetrics {
        let m = || count(v, if v.is_some() { "ready" } else { "partial" });
        ClassMetrics {
            class_id: id.into(),
            node: file.into(),
            file: file.into(),
            cbo: m(),
            rfc: m(),
            dit: m(),
            noc: m(),
            wmc: m(),
            lcom4,
        }
    }

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
    fn partial_measurements_stay_unknown_and_count_nothing() {
        let rows = vec![class("rust:a.rs::A", "a.rs", None, None)];
        let cohorts = Cohorts::new(&rows);
        let a = cohorts.assess(&rows[0], "cbo");
        assert_eq!((a.value, a.state), (None, State::Unknown));
        assert_eq!(priority(&cohorts, &rows[0]), 0);
    }

    #[test]
    fn unknown_language_and_small_cohorts_stay_neutral() {
        let mut rows = vec![class("rust:a.rs::A", "a.rs", Some(11), None)];
        rows.extend(java(&[3, 4]));
        let cohorts = Cohorts::new(&rows);
        assert_eq!(cohorts.assess(&rows[0], "wmc").state, State::Small);
        assert_eq!(cohorts.assess(&rows[1], "cbo").state, State::Small);
        assert_eq!(cohorts.assess(&rows[0], "noc").state, State::Info);
    }

    #[test]
    fn rank_is_the_share_of_strictly_smaller_values() {
        // Cohort 1..=6: rank = smaller / (n - 1).
        let rows = java(&[1, 2, 3, 4, 5, 6]);
        let cohorts = Cohorts::new(&rows);
        let state = |i: usize| cohorts.assess(&rows[i], "cbo").state;
        assert_eq!(state(2), State::Low); // 2/5 = 0.4
        assert_eq!(state(3), State::Medium); // 3/5 = 0.6
        assert_eq!(state(4), State::High); // 4/5 = 0.8
        assert_eq!(cohorts.assess(&rows[0], "cbo").population, 6);
        // LCOM4 1 (one connected group) is low; equal cohorts stay neutral.
        assert_eq!(cohorts.assess(&rows[5], "lcom4").state, State::Low);
        let equal = java(&[3, 3, 3, 3, 3]);
        assert_eq!(
            Cohorts::new(&equal).assess(&equal[0], "cbo").state,
            State::Equal
        );
        let zero = java(&[0, 3, 3, 3, 3]);
        assert_eq!(
            Cohorts::new(&zero).assess(&zero[0], "cbo").state,
            State::Low
        );
    }

    #[test]
    fn not_applicable_keeps_its_own_state() {
        let mut rows = java(&[1, 2, 3, 4, 5]);
        rows[0].dit = count(None, "not_applicable");
        let cohorts = Cohorts::new(&rows);
        assert_eq!(cohorts.assess(&rows[0], "dit").state, State::NotApplicable);
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
