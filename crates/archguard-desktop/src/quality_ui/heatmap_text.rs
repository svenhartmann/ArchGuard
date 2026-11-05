//! Heatmap cell texts: `metricHelp` (viewer.js `METRIC_HELP`) and
//! `classMetricInterpretation` (viewer_class_heatmap.js).

use archguard_engine::ClassMetrics;

use super::heatmap::{Cohorts, State};
use super::lang::{Lang, number};

/// The web's metric definitions (en, de), in [`super::heatmap::METRICS`] order.
const HELP: [(&str, &str); 6] = [
    (
        "CBO — Coupling Between Objects. Distinct resolved target types used by declaration types and calls, excluding self and inheritance. Higher values mean broader coupling to inspect. Missing compiler evidence leaves the value unknown (—); observed counts are lower bounds.",
        "CBO — Coupling Between Objects (Klassenkopplung). Verschiedene aufgelöste Zieltypen aus Deklarationstypen und Aufrufen; eigener Typ und Vererbung sind ausgenommen. Höhere Werte bedeuten breitere Kopplung und geben Anlass zur Prüfung. Fehlende Compiler-Belege lassen den Wert unbekannt (—); beobachtete Zahlen sind Untergrenzen.",
    ),
    (
        "RFC — Response For a Class. Distinct own non-constructor methods and their direct call targets, counting each member once. A larger response set increases the behavior to understand and test. This uses static resolution, not all possible runtime dispatch targets. Incomplete evidence remains —.",
        "RFC — Response For a Class (Antwortumfang). Menge eigener Methoden ohne Konstruktoren und ihrer direkten Aufrufziele; jedes Mitglied zählt einmal. Ein größerer Umfang bedeutet mehr Verhalten zum Verstehen und Testen. Statische Auflösung, nicht alle möglichen Laufzeitziele. Unvollständige Belege bleiben —.",
    ),
    (
        "DIT — Depth of Inheritance Tree. Superclass edges up to the compiler-known root, including known external bases; interfaces are excluded. Deeper inheritance means more inherited behavior to trace, not an automatic design error. Unknown hierarchy remains —.",
        "DIT — Depth of Inheritance Tree (Vererbungstiefe). Basisklassenkanten bis zur compilerbekannten Wurzel, einschließlich bekannter externer Basen; Interfaces zählen nicht. Tiefere Vererbung bedeutet mehr nachzuverfolgendes Verhalten, aber keinen automatischen Entwurfsfehler. Unbekannte Hierarchie bleibt —.",
    ),
    (
        "NOC — Number of Children. Direct subclasses inside the analyzed source scope. External subclasses are not counted. High values can mean intended reuse or a broad change impact; this is informational, not a good/bad score. Incomplete hierarchy remains —.",
        "NOC — Number of Children (direkte Unterklassen). Zählt Unterklassen im analysierten Quellumfang; externe Unterklassen fehlen. Hohe Werte können gewollte Wiederverwendung oder eine breite Änderungswirkung zeigen. Eine Information, keine Gut-/Schlecht-Note. Unvollständige Hierarchie bleibt —.",
    ),
    (
        "WMC — Weighted Methods per Class. Sum of source cyclomatic complexity of declared non-constructor methods. Static methods count; local functions and constructors are excluded. High values suggest a large or complex responsibility. Missing method measurements leave the sum unknown (—).",
        "WMC — Weighted Methods per Class (gewichtete Methoden). Summe der zyklomatischen Quellcodekomplexität deklarierter Methoden. Statische Methoden zählen; Konstruktoren und lokale Funktionen sind ausgenommen. Hohe Werte geben Anlass, Umfang und Verantwortung zu prüfen. Fehlende Methodenmessungen lassen die Summe unbekannt (—).",
    ),
    (
        "LCOM4 — Lack of Cohesion in Methods, variant 4. Connected groups of instance methods sharing a non-static field/property or calling each other. One group is connected; multiple groups invite a responsibility review. Constructors, static members and local functions are excluded. Incomplete compiler evidence remains —.",
        "LCOM4 — Lack of Cohesion in Methods, Variante 4 (Zusammenhalt). Gruppen von Instanzmethoden, die dasselbe nichtstatische Feld bzw. dieselbe Property nutzen oder einander aufrufen. Eine Gruppe ist zusammenhängend; mehrere Gruppen geben Anlass zur Verantwortungsprüfung. Konstruktoren, statische Mitglieder und lokale Funktionen zählen nicht. Unvollständige Compiler-Belege bleiben —.",
    ),
];

pub fn help(lang: Lang, metric: usize) -> &'static str {
    let (en, de) = HELP[metric];
    lang.t(en, de)
}

/// `classMetricStateLabel`.
pub fn state_label(lang: Lang, state: State) -> &'static str {
    match state {
        State::NotApplicable => lang.t("Not applicable", "Nicht anwendbar"),
        State::Low => lang.t("Low", "Niedrig"),
        State::Medium => lang.t("Elevated", "Erhöht"),
        State::High => lang.t("High", "Hoch"),
        State::Unknown => lang.t("Unknown", "Unbekannt"),
        State::Small => lang.t("Few comparisons", "Wenig Vergleichsdaten"),
        State::Equal => lang.t("Equal values", "Gleiche Werte"),
        State::Info => lang.t("Informational", "Information"),
    }
}

/// `classMetricInterpretation`.
pub fn interpretation(lang: Lang, cohorts: &Cohorts, row: &ClassMetrics, metric: usize) -> String {
    let key = super::heatmap::METRICS[metric];
    if super::heatmap::not_applicable(row, key) {
        return lang
            .t(
                "Not applicable · Go embedding is composition, not inheritance.",
                "Nicht anwendbar · Go-Embedding ist Komposition, keine Vererbung.",
            )
            .into();
    }
    let a = cohorts.assess(row, key);
    let head = format!("{} · {}. ", state_label(lang, a.state), number(a.value));
    let tail = match a.state {
        State::Unknown => lang.t(
            "Incomplete measurement. Observed lower bounds are not rated.",
            "Unvollständige Messung. Beobachtete Untergrenzen werden nicht bewertet.",
        ).to_string(),
        State::Info => lang.t(
            "More subclasses can reflect intended reuse; no good/bad rating.",
            "Mehr Unterklassen können gewollte Wiederverwendung bedeuten; keine Gut-/Schlecht-Bewertung.",
        ).to_string(),
        _ => lang.f(
            "Comparison: {count} measured classes of the same source language. Search and package filters do not change the colors.",
            "Vergleich: {count} gemessene Klassen derselben Quellsprache. Suche und Paketfilter ändern die Farben nicht.",
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
            interpretation(Lang::default(), &cohorts, &rows[0], 4),
            "Few comparisons · 11. Comparison: 1 measured classes of the same source language. Search and package filters do not change the colors."
        );
        assert!(
            interpretation(Lang::default(), &cohorts, &rows[0], 5)
                .starts_with("Unknown · —. Incomplete")
        );
        assert!(
            help(Lang { de: true }, 0)
                .starts_with("CBO — Coupling Between Objects (Klassenkopplung)")
        );
    }
}
