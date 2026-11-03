//! Composite architecture health index (R09).
//!
//! A weighted mean of normalized part KPIs (each `0.0..=1.0`, higher is
//! healthier), mapped to a 0–100 score with an ampel level and the delta
//! against a previous snapshot. Weights are parameters; the `[review]`
//! config section that feeds them lands in R30. Parts without a positive
//! finite weight (or with a non-finite value) are not measurable and drop
//! out — the score is only as complete as its parts.

use std::collections::BTreeMap;

use archguard_core::{HealthIndex, HealthLevel, HealthWeights};

/// Score at or above which the ampel is green.
const GREEN_AT: f64 = 80.0;
/// Score at or above which the ampel is yellow.
const YELLOW_AT: f64 = 50.0;

/// Weighted composite of `parts`; `previous` is the last score, if any.
///
/// Deterministic: parts are summed in sorted key order, so floating
/// point summation does not depend on map iteration order.
pub fn health_index(
    parts: &BTreeMap<String, f64>,
    weights: &HealthWeights,
    previous: Option<f64>,
) -> HealthIndex {
    let mut keys: Vec<&String> = parts.keys().collect();
    keys.sort();
    let mut weighted = 0.0f64;
    let mut total_weight = 0.0f64;
    let mut used: BTreeMap<String, f64> = BTreeMap::new();
    for key in keys {
        let weight = weights.weight(key);
        let value = parts[key].clamp(0.0, 1.0);
        if !weight.is_finite() || weight <= 0.0 || !parts[key].is_finite() {
            continue;
        }
        weighted += weight * value;
        total_weight += weight;
        used.insert(key.clone(), value);
    }
    let (score, level) = if total_weight > 0.0 {
        let score = weighted / total_weight * 100.0;
        let level = if score >= GREEN_AT {
            HealthLevel::Green
        } else if score >= YELLOW_AT {
            HealthLevel::Yellow
        } else {
            HealthLevel::Red
        };
        (Some(score), Some(level))
    } else {
        (None, None)
    };
    let trend = match (score, previous) {
        (Some(score), Some(previous)) => Some(score - previous),
        _ => None,
    };
    HealthIndex {
        score,
        level,
        trend,
        parts: used,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(entries: &[(&str, f64)]) -> BTreeMap<String, f64> {
        entries.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn all_parts_perfect_score_hundred_green() {
        let index = health_index(
            &parts(&[("sdp", 1.0), ("tangledness", 1.0), ("pc", 1.0)]),
            &HealthWeights::default(),
            None,
        );
        assert_eq!(index.score, Some(100.0));
        assert_eq!(index.level, Some(HealthLevel::Green));
        assert_eq!(index.trend, None, "no previous snapshot, no trend");
    }

    #[test]
    fn a_dead_part_drops_the_score_by_its_weight() {
        // Two equally weighted parts, one at 0 → mean 0.5 → 50 yellow.
        let index = health_index(
            &parts(&[("sdp", 1.0), ("cycles", 0.0)]),
            &HealthWeights::default(),
            None,
        );
        assert_eq!(index.score, Some(50.0));
        assert_eq!(index.level, Some(HealthLevel::Yellow));

        // Weighting cycles 3× pulls the same input down to 1/(1+3)·100.
        let weights = HealthWeights {
            weights: [("cycles".to_string(), 3.0)].into_iter().collect(),
        };
        let index = health_index(&parts(&[("sdp", 1.0), ("cycles", 0.0)]), &weights, None);
        assert!((index.score.unwrap() - 25.0).abs() < 1e-9);
        assert_eq!(index.level, Some(HealthLevel::Red));
    }

    #[test]
    fn trend_carries_the_delta_against_the_previous_score() {
        let perfect = parts(&[("sdp", 1.0)]);
        let index = health_index(&perfect, &HealthWeights::default(), Some(90.0));
        assert_eq!(index.trend, Some(10.0));
        let index = health_index(
            &parts(&[("sdp", 0.8)]),
            &HealthWeights::default(),
            Some(90.0),
        );
        assert_eq!(index.trend, Some(-10.0));
    }

    #[test]
    fn unmeasurable_parts_never_fake_a_score() {
        // Non-positive weights drop out; with only such parts there is
        // no measurable index at all — null, not zero.
        let weights = HealthWeights {
            weights: [("off".to_string(), 0.0)].into_iter().collect(),
        };
        let index = health_index(&parts(&[("off", 1.0)]), &weights, Some(50.0));
        assert_eq!(index.score, None);
        assert_eq!(index.level, None);
        assert_eq!(index.trend, None);
        assert!(index.parts.is_empty());

        // A non-finite part value is skipped, the rest still measures.
        let mut mixed = parts(&[("ok", 1.0)]);
        mixed.insert("nan".to_string(), f64::NAN);
        let index = health_index(&mixed, &HealthWeights::default(), None);
        assert_eq!(index.score, Some(100.0));
        assert!(!index.parts.contains_key("nan"));
    }

    #[test]
    fn part_values_are_clamped_into_the_normalized_range() {
        let index = health_index(
            &parts(&[("high", 5.0), ("low", -2.0)]),
            &HealthWeights::default(),
            None,
        );
        assert_eq!(index.score, Some(50.0), "clamped to 1.0 and 0.0");
        assert_eq!(index.parts["high"], 1.0);
        assert_eq!(index.parts["low"], 0.0);
    }

    #[test]
    fn no_parts_at_all_is_null_not_hundred() {
        let index = health_index(&BTreeMap::new(), &HealthWeights::default(), None);
        assert_eq!(index.score, None);
        assert_eq!(index.level, None);
    }

    #[test]
    fn signature_is_stable() {
        let f: fn(&BTreeMap<String, f64>, &HealthWeights, Option<f64>) -> HealthIndex =
            health_index;
        let _ = f;
    }
}
