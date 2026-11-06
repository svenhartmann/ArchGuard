//! Heatmap cell texts: `metricHelp` (viewer.js `METRIC_HELP`) and
//! `classMetricInterpretation` (viewer_class_heatmap.js).

use archguard_engine::ClassMetrics;

use super::heatmap::{Cohorts, Rating};
use super::lang::number;
use crate::tab_support::Texts;

/// The web's metric definitions (templates), in [`super::heatmap::METRICS`] order.
const HELP: [&str; 6] = [
    "CBO — Coupling Between Objects. Distinct resolved target types used by declaration types and calls, excluding self and inheritance. Higher values mean broader coupling to inspect. Missing compiler evidence leaves the value unknown (—); observed counts are lower bounds.",
    "RFC — Response For a Class. Distinct own non-constructor methods and their direct call targets, counting each member once. A larger response set increases the behavior to understand and test. This uses static resolution, not all possible runtime dispatch targets. Incomplete evidence remains —.",
    "DIT — Depth of Inheritance Tree. Superclass edges up to the compiler-known root, including known external bases; interfaces are excluded. Deeper inheritance means more inherited behavior to trace, not an automatic design error. Unknown hierarchy remains —.",
    "NOC — Number of Children. Direct subclasses inside the analyzed source scope. External subclasses are not counted. High values can mean intended reuse or a broad change impact; this is informational, not a good/bad score. Incomplete hierarchy remains —.",
    "WMC — Weighted Methods per Class. Sum of source cyclomatic complexity of declared non-constructor methods. Static methods count; local functions and constructors are excluded. High values suggest a large or complex responsibility. Missing method measurements leave the sum unknown (—).",
    "LCOM4 — Lack of Cohesion in Methods, variant 4. Connected groups of instance methods sharing a non-static field/property or calling each other. One group is connected; multiple groups invite a responsibility review. Constructors, static members and local functions are excluded. Incomplete compiler evidence remains —.",
];

pub fn help(lang: &Texts, metric: usize) -> &str {
    lang.t(HELP[metric])
}

/// `classMetricInterpretation`.
pub fn interpretation(
    lang: &Texts,
    cohorts: &Cohorts,
    row: &ClassMetrics,
    metric: usize,
) -> String {
    let key = super::heatmap::METRICS[metric];
    if super::heatmap::not_applicable(row, key) {
        return lang
            .t("Not applicable · Go embedding is composition, not inheritance.")
            .into();
    }
    let a = cohorts.assess(row, key);
    let head = format!("{} · {}. ", lang.t(a.state.label()), number(lang, a.value));
    let tail = match a.state {
        Rating::Unknown => lang.t("Incomplete measurement. Observed lower bounds are not rated.",
        ).to_string(),
        Rating::Info => lang.t("More subclasses can reflect intended reuse; no good/bad rating.",
        ).to_string(),
        _ => lang.f("Comparison: {count} measured classes of the same source language. Search and package filters do not change the colors.",
            &[("count", a.population.to_string())],
        ),
    };
    head + &tail
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality_ui::heatmap::tests::class;

    #[test]
    fn interpretation_names_state_value_and_cohort() {
        let rows = vec![class("rust:a.rs::A", "a.rs", Some(11), None)];
        let cohorts = Cohorts::new(&rows);
        assert_eq!(
            interpretation(&Texts::default(), &cohorts, &rows[0], 4),
            "Few comparisons · 11. Comparison: 1 measured classes of the same source language. Search and package filters do not change the colors."
        );
        assert!(
            interpretation(&Texts::default(), &cohorts, &rows[0], 5)
                .starts_with("Unknown · —. Incomplete")
        );
        assert!(
            help(&Texts::german(), 0)
                .starts_with("CBO — Coupling Between Objects (Klassenkopplung)")
        );
    }
}
