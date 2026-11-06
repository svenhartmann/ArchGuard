//! Section notes behind the status line's "?" (`#engineering-note` in
//! `renderEngineeringQuality`): description, coverage notes, import command.

use super::lang;
use crate::status::Status;
use crate::tab_support::Texts;

/// The web's sections, in the order of its "View" select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Heatmap,
    Classes,
    Methods,
    Knowledge,
    Measurements,
    Cohesion,
    History,
    Tests,
    Mutations,
    Duplicates,
}

impl Section {
    pub const ALL: [Section; 10] = [
        Section::Heatmap,
        Section::Classes,
        Section::Methods,
        Section::Knowledge,
        Section::Measurements,
        Section::Cohesion,
        Section::History,
        Section::Tests,
        Section::Mutations,
        Section::Duplicates,
    ];

    /// The web's section key (`#tab=quality&section=<key>`); snapshot mode
    /// opens a section by it.
    #[cfg_attr(not(feature = "snapshot"), allow(dead_code))]
    pub fn key(self) -> &'static str {
        match self {
            Section::Heatmap => "heatmap",
            Section::Classes => "classes",
            Section::Methods => "methods",
            Section::Knowledge => "knowledge",
            Section::Measurements => "measurements",
            Section::Cohesion => "cohesion",
            Section::History => "history",
            Section::Tests => "tests",
            Section::Mutations => "mutations",
            Section::Duplicates => "duplicates",
        }
    }

    pub fn index(self) -> usize {
        Section::ALL.iter().position(|&s| s == self).unwrap_or(0)
    }

    /// Code metrics and Git: computed by the analysis, not imported.
    pub fn computed(self) -> bool {
        matches!(
            self,
            Section::Heatmap | Section::Classes | Section::Methods | Section::Knowledge
        )
    }

    /// The CLI command that measures or imports the section's data.
    pub fn command(self) -> &'static str {
        match self {
            Section::Cohesion => "archguard cohesion . --format json",
            Section::History => "archguard quality-history . --record --tag after-refactoring",
            Section::Tests => {
                "archguard test-evidence . --report junit.xml --mapping test-map.json"
            }
            Section::Mutations => "archguard mutations . --report mutation-report.json",
            Section::Duplicates => "archguard duplicates . --report cpd.xml",
            Section::Measurements => "archguard metrics . --extended --report runs.json",
            _ => "archguard metrics . --extended --format json",
        }
    }

    pub fn description(self, lang: &Texts) -> &str {
        match self {
            Section::Heatmap => lang.t("Classes × metrics. Colors compare measured values; gray retains unknowns."),
            Section::Classes => lang.t("Class coupling, response set, inheritance and complexity. Unknown measurements remain —."),
            Section::Methods => lang.t("Method maintainability, ordered by Cognitive source complexity."),
            Section::Knowledge => lang.t("Contribution concentration in the configured Git history window."),
            Section::Measurements => lang.t("Recorded build, runtime and repeated-test runs. Project-wide observations are independent of the source selection."),
            Section::Cohesion => lang.t("Immediate files in each package. Internal share is structural, not proof of good domain boundaries; disconnected groups are review candidates."),
            Section::History => lang.t("Latest two recorded snapshots. Incompatible measurements have no delta; imported producer revisions remain unverified."),
            Section::Tests => lang.t("Matching source associations first, then measured duration. Selecting a file includes tests mapped to its transitive dependents. Unknown or incomplete mappings mean test omission is not safe."),
            Section::Mutations => lang.t("Survived and uncovered mutations first, then incoming-file count. Killed ratio = killed / (killed + survived + no coverage); incomplete evidence has no score. This is not ordinary line coverage."),
            Section::Duplicates => lang.t("CPD token matches, largest first. Inspect all locations before extracting shared code. Source hashes detect changes after import; producer revision, scan scope and token normalization remain unverified. No project-wide duplication percentage is inferred."),
        }
    }

    /// The status line of the section's source: `(Status::slot, "Analysis:
    /// partial")` for computed sections, "Import: …" for imported ones.
    /// Codes outside the shared vocabulary keep their own word.
    pub fn status(self, lang: &Texts, code: Option<&str>) -> (i32, String) {
        let code = code.unwrap_or(if self.computed() { "unavailable" } else { "" });
        let subject = if self.computed() {
            "Analysis"
        } else {
            "Import"
        };
        match Status::from_code(code) {
            Some(s) => (s.slot(), s.line(lang, subject)),
            None => (
                Status::Unavailable.slot(),
                lang.f(
                    "{subject}: {status}",
                    &[
                        ("subject", lang.t(subject).to_string()),
                        ("status", lang::status(lang, code).to_string()),
                    ],
                ),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computed_and_imported_sections_name_their_status_like_the_web() {
        let en = &Texts::default();
        let line = |s: Section, code| s.status(en, code).1;
        assert_eq!(line(Section::Heatmap, Some("partial")), "Analysis: partial");
        assert_eq!(line(Section::Heatmap, Some("ready")), "Analysis: ready");
        assert_eq!(line(Section::Knowledge, None), "Analysis: unavailable");
        assert_eq!(line(Section::Tests, Some("ready")), "Import: ready");
        assert_eq!(
            line(Section::Tests, Some("not_imported")),
            "Import: not imported"
        );
        assert_eq!(
            line(Section::History, Some("not_recorded")),
            "Import: not recorded"
        );
        assert_eq!(
            Section::Cohesion.status(&Texts::german(), Some("partial")),
            (1, "Import: teilweise".to_string())
        );
        assert_eq!(Section::ALL[5].index(), 5);
    }
}
