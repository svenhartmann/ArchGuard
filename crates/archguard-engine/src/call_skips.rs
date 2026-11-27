//! What a skipped input does to a call analysis; the data model's promise,
//! not a property of any one language.

use archguard_core::CallAnalysis;

/// Skipped files make a ready analysis partial and are named in the diagnostics.
pub(crate) fn with_skipped(mut analysis: CallAnalysis, skipped: &[String]) -> CallAnalysis {
    if skipped.is_empty() {
        return analysis;
    }
    if analysis.status == "ready" {
        analysis.status = "partial".into();
    }
    analysis.diagnostics.extend_from_slice(skipped);
    for analyzer in &mut analysis.analyzers {
        if analyzer.status == "ready" {
            analyzer.status = "partial".into();
        }
        analyzer.diagnostics.extend_from_slice(skipped);
    }
    analysis
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::CallAnalyzer;

    fn ready() -> CallAnalysis {
        CallAnalysis {
            status: "ready".into(),
            analyzers: vec![CallAnalyzer {
                language: "x".into(),
                backend: "y".into(),
                status: "ready".into(),
                files: 1,
                version: None,
                project: None,
                diagnostics: vec!["old".into()],
            }],
            ..CallAnalysis::default()
        }
    }

    #[test]
    fn skips_make_ready_partial_and_show_in_both_diagnostic_lists() {
        let skipped = vec!["a skipped: why".to_string()];
        let out = with_skipped(ready(), &skipped);
        assert_eq!(out.status, "partial");
        assert_eq!(out.diagnostics, skipped);
        assert_eq!(out.analyzers[0].status, "partial");
        assert_eq!(out.analyzers[0].diagnostics, ["old", "a skipped: why"]);
    }

    #[test]
    fn an_empty_skip_list_changes_nothing() {
        assert_eq!(with_skipped(ready(), &[]), ready());
    }

    #[test]
    fn a_non_ready_status_stays_but_still_names_the_skips() {
        let mut a = ready();
        a.status = "unavailable".into();
        a.analyzers[0].status = "unavailable".into();
        let out = with_skipped(a, &["s".to_string()]);
        assert_eq!(out.status, "unavailable");
        assert_eq!(out.analyzers[0].status, "unavailable");
        assert_eq!(out.diagnostics, ["s"]);
    }
}
