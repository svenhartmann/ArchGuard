//! Section notes behind the status line's "?" (`#engineering-note` in
//! `renderEngineeringQuality`): description, coverage notes, import command.

use super::lang::Lang;

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

    pub fn description(self, lang: Lang) -> &'static str {
        match self {
            Section::Heatmap => lang.t("Classes × metrics. Colors compare measured values; gray retains unknowns.", "Klassen × Kennzahlen. Farben vergleichen gemessene Werte; Grau erhält unbekannte Werte."),
            Section::Classes => lang.t("Class coupling, response set, inheritance and complexity. Unknown measurements remain —.", "Klassenkopplung, Aufrufumfang, Vererbung und Komplexität. Unbekannte Messungen bleiben —."),
            Section::Methods => lang.t("Method maintainability, ordered by Cognitive source complexity.", "Wartbarkeit pro Methode, sortiert nach Cognitive-Quellcodekomplexität."),
            Section::Knowledge => lang.t("Contribution concentration in the configured Git history window.", "Beitragsverteilung im konfigurierten Git-Zeitfenster."),
            Section::Measurements => lang.t("Recorded build, runtime and repeated-test runs. Project-wide observations are independent of the source selection.", "Aufgezeichnete Build-, Laufzeit- und wiederholte Testläufe. Projektweite Messungen sind unabhängig von der Quellauswahl."),
            Section::Cohesion => lang.t("Immediate files in each package. Internal share is structural, not proof of good domain boundaries; disconnected groups are review candidates.", "Direkte Dateien je Paket. Der interne Anteil ist strukturell und belegt keine guten fachlichen Grenzen; getrennte Gruppen sind Prüfkandidaten."),
            Section::History => lang.t("Latest two recorded snapshots. Incompatible measurements have no delta; imported producer revisions remain unverified.", "Die letzten zwei aufgezeichneten Snapshots. Unvergleichbare Messungen erhalten kein Delta; importierte Producer-Revisionen bleiben unverifiziert."),
            Section::Tests => lang.t("Matching source associations first, then measured duration. Selecting a file includes tests mapped to its transitive dependents. Unknown or incomplete mappings mean test omission is not safe.", "Passende Quellzuordnungen zuerst, danach gemessene Dauer. Eine Dateiauswahl enthält Tests ihrer transitiven Aufrufer. Unbekannte oder unvollständige Zuordnungen erlauben kein sicheres Weglassen von Tests."),
            Section::Mutations => lang.t("Survived and uncovered mutations first, then incoming-file count. Killed ratio = killed / (killed + survived + no coverage); incomplete evidence has no score. This is not ordinary line coverage.", "Überlebende und unabgedeckte Mutationen zuerst, danach eingehende Dateien. Erkennungsquote = erkannt / (erkannt + überlebt + keine Abdeckung); unvollständige Belege erhalten keinen Wert. Dies ist keine normale Zeilenabdeckung."),
            Section::Duplicates => lang.t("CPD token matches, largest first. Inspect all locations before extracting shared code. Source hashes detect changes after import; producer revision, scan scope and token normalization remain unverified. No project-wide duplication percentage is inferred.", "CPD-Tokenübereinstimmungen, größte zuerst. Vor einer gemeinsamen Abstraktion alle Fundstellen prüfen. Quellhashes erkennen Änderungen nach dem Import; Producer-Revision, Scanumfang und Tokennormalisierung bleiben unverifiziert. Es wird kein projektweiter Duplikatanteil abgeleitet."),
        }
    }

    /// The status word before "· shown / total matching records".
    pub fn status(self, lang: Lang, status: Option<&str>) -> &'static str {
        if !self.computed() {
            return lang.status(status.unwrap_or(""));
        }
        match status {
            Some("ready") => lang.t("measured", "gemessen"),
            Some("partial") => lang.t("partial analysis", "teilweise analysiert"),
            Some("not_applicable") => lang.t("not applicable", "nicht anwendbar"),
            _ => lang.t("unavailable", "nicht verfügbar"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computed_and_imported_sections_name_their_status_like_the_web() {
        let en = Lang::default();
        assert_eq!(
            Section::Heatmap.status(en, Some("partial")),
            "partial analysis"
        );
        assert_eq!(Section::Knowledge.status(en, None), "unavailable");
        assert_eq!(
            Section::Tests.status(en, Some("not_imported")),
            "not imported"
        );
        assert_eq!(
            Section::History.status(en, Some("not_recorded")),
            "not recorded"
        );
        assert_eq!(
            Section::Cohesion.status(Lang { de: true }, Some("partial")),
            "unvollständiger Bericht"
        );
        assert_eq!(Section::ALL[5].index(), 5);
    }
}
