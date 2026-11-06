//! The one rating of class metrics, used by Quality's heatmap and the
//! Start world: `classMetricValue`, `classMetricLanguage` and
//! `classMetricAssessment` of the web viewer. A measured value is ranked
//! among all measured classes of the same source language in the original
//! project (the cohort); search and scope never change it.

use std::collections::BTreeMap;

use archguard_engine::{ClassMetrics, CountMetric};

/// Metric columns, in the web's order.
pub const METRICS: [&str; 6] = ["cbo", "rfc", "dit", "noc", "wmc", "lcom4"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rating {
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

impl Rating {
    /// The slot of the `rate-*` token scale (`RatingChip.rating`):
    /// 0 unknown, 1 low, 2 elevated, 3 high, 4 informational, 5 unrated.
    pub fn slot(self) -> i32 {
        match self {
            Rating::Unknown => 0,
            Rating::Low => 1,
            Rating::Medium => 2,
            Rating::High => 3,
            Rating::Info => 4,
            Rating::NotApplicable | Rating::Small | Rating::Equal => 5,
        }
    }

    /// Template key of the rating's name.
    pub fn label(self) -> &'static str {
        match self {
            Rating::NotApplicable => "Not applicable",
            Rating::Low => "Low",
            Rating::Medium => "Elevated",
            Rating::High => "High",
            Rating::Unknown => "Unknown",
            Rating::Small => "Few comparisons",
            Rating::Equal => "Equal values",
            Rating::Info => "Informational",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Assessment {
    pub value: Option<f64>,
    pub state: Rating,
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
pub fn language(file: &str) -> &'static str {
    let ext = file.rsplit('.').next().unwrap_or("").to_lowercase();
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
                    map.entry((language(&row.file), key)).or_default().push(v);
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
            .get(&(language(&row.file), key))
            .map_or(&[][..], Vec::as_slice);
        let value = value(row, key);
        let state = match value {
            None if not_applicable(row, key) => Rating::NotApplicable,
            None => Rating::Unknown,
            Some(v) => rate(key, v, language(&row.file), values),
        };
        Assessment {
            value,
            state,
            population: values.len(),
        }
    }
}

fn rate(key: &str, v: f64, language: &str, values: &[f64]) -> Rating {
    if key == "noc" {
        return Rating::Info;
    }
    if v == 0.0 || (key == "lcom4" && v == 1.0) {
        return Rating::Low;
    }
    if language == "unknown" || values.len() < 5 {
        return Rating::Small;
    }
    if values[0] == values[values.len() - 1] {
        return Rating::Equal;
    }
    // Strictly smaller values: equal measurements share a rank.
    let rank = values.partition_point(|&x| x < v) as f64 / (values.len() - 1) as f64;
    if rank >= 0.8 {
        Rating::High
    } else if rank >= 0.5 {
        Rating::Medium
    } else {
        Rating::Low
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tab_support::Texts;

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
        assert_eq!((a.value, a.state), (None, Rating::Unknown));
    }

    #[test]
    fn unknown_language_and_small_cohorts_stay_neutral() {
        let mut rows = vec![class("rust:a.rs::A", "a.rs", Some(11), None)];
        rows.extend(java(&[3, 4]));
        let cohorts = Cohorts::new(&rows);
        assert_eq!(cohorts.assess(&rows[0], "wmc").state, Rating::Small);
        assert_eq!(cohorts.assess(&rows[1], "cbo").state, Rating::Small);
        assert_eq!(cohorts.assess(&rows[0], "noc").state, Rating::Info);
    }

    #[test]
    fn rank_is_the_share_of_strictly_smaller_values() {
        // Cohort 1..=6: rank = smaller / (n - 1).
        let rows = java(&[1, 2, 3, 4, 5, 6]);
        let cohorts = Cohorts::new(&rows);
        let state = |i: usize| cohorts.assess(&rows[i], "cbo").state;
        assert_eq!(state(2), Rating::Low); // 2/5 = 0.4
        assert_eq!(state(3), Rating::Medium); // 3/5 = 0.6
        assert_eq!(state(4), Rating::High); // 4/5 = 0.8
        assert_eq!(cohorts.assess(&rows[0], "cbo").population, 6);
        // LCOM4 1 (one connected group) is low; equal cohorts stay neutral.
        assert_eq!(cohorts.assess(&rows[5], "lcom4").state, Rating::Low);
        let equal = java(&[3, 3, 3, 3, 3]);
        assert_eq!(
            Cohorts::new(&equal).assess(&equal[0], "cbo").state,
            Rating::Equal
        );
        let zero = java(&[0, 3, 3, 3, 3]);
        assert_eq!(
            Cohorts::new(&zero).assess(&zero[0], "cbo").state,
            Rating::Low
        );
    }

    #[test]
    fn not_applicable_keeps_its_own_state() {
        let mut rows = java(&[1, 2, 3, 4, 5]);
        rows[0].dit = count(None, "not_applicable");
        let cohorts = Cohorts::new(&rows);
        assert_eq!(cohorts.assess(&rows[0], "dit").state, Rating::NotApplicable);
    }

    /// An empty status counts as measured, everywhere (Start once read it
    /// as unknown while Quality rated it).
    #[test]
    fn an_empty_status_is_a_measurement() {
        let mut row = class("A", "src/A.java", Some(3), Some(1));
        row.cbo.status.clear();
        assert_eq!(value(&row, "cbo"), Some(3.0));
        assert_eq!(value(&row, "lcom4"), Some(1.0));
    }

    #[test]
    fn languages_come_from_the_extension() {
        assert_eq!(language("src/A.TSX"), "typescript");
        assert_eq!(language("lib.rs"), "unknown");
        assert_eq!(language("a/B.cs"), "csharp");
    }

    #[test]
    fn every_rating_has_a_slot_and_a_translated_name() {
        let de = Texts::german();
        let all = [
            Rating::NotApplicable,
            Rating::Low,
            Rating::Medium,
            Rating::High,
            Rating::Unknown,
            Rating::Small,
            Rating::Equal,
            Rating::Info,
        ];
        for r in all {
            assert!((0..=5).contains(&r.slot()));
            assert_ne!(de.t(r.label()), r.label(), "{r:?}");
        }
        assert_eq!(Rating::High.slot(), 3);
        assert_eq!(Rating::Equal.slot(), Rating::Small.slot());
    }
}
