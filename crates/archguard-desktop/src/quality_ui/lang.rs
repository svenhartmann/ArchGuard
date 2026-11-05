//! Texts the tab builds in Rust from data, in the web viewer's wording
//! (`t(en, de)` in `viewer_i18n.js`) and number formats
//! (`engineeringNumber`, `engineeringPercent`, `engineeringStatus`).

/// The window's language, read from `QualityData.lang`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lang {
    pub de: bool,
}

impl Lang {
    pub fn t(self, en: &'static str, de: &'static str) -> &'static str {
        if self.de { de } else { en }
    }

    /// `t` with `{name}` placeholders filled.
    pub fn f(self, en: &'static str, de: &'static str, args: &[(&str, String)]) -> String {
        fill(self.t(en, de), args)
    }

    /// The web's `plural(count, en, enMany, de, deMany)`.
    pub fn plural(self, count: usize, forms: [&'static str; 4]) -> String {
        let [en, en_many, de, de_many] = forms;
        let text = if count == 1 {
            self.t(en, de)
        } else {
            self.t(en_many, de_many)
        };
        fill(text, &[("count", count.to_string())])
    }

    /// `engineeringStatus`, falling back to `coverageStatusText`.
    pub fn status(self, status: &str) -> &'static str {
        let status = if status.is_empty() {
            "not_imported"
        } else {
            status
        };
        let (_, en, de) = STATUS
            .iter()
            .find(|(key, _, _)| *key == status)
            .unwrap_or(&("", "not available", "nicht verfügbar"));
        self.t(en, de)
    }
}

/// Status labels of `engineeringStatus` and `coverageStatusText`.
const STATUS: [(&str, &str, &str); 17] = [
    (
        "unchanged_since_import",
        "unchanged since import",
        "seit Import unverändert",
    ),
    (
        "changed_since_import",
        "changed since import",
        "seit Import geändert",
    ),
    ("unmapped", "unmapped", "nicht zugeordnet"),
    ("not_recorded", "not recorded", "nicht aufgezeichnet"),
    ("compared", "compared", "verglichen"),
    (
        "no_baseline",
        "no previous import",
        "kein vorheriger Import",
    ),
    ("unverified", "unverified", "nicht verifiziert"),
    ("comparable", "comparable", "vergleichbar"),
    ("incomparable", "not comparable", "nicht vergleichbar"),
    ("present", "present", "vorhanden"),
    ("added", "added", "hinzugefügt"),
    ("removed", "removed", "entfernt"),
    ("not_imported", "not imported", "nicht importiert"),
    ("ready", "imported", "importiert"),
    ("partial", "incomplete report", "unvollständiger Bericht"),
    (
        "stale",
        "stale: sources changed",
        "veraltet: Quellen geändert",
    ),
    (
        "error",
        "unavailable: import error",
        "nicht verfügbar: Importfehler",
    ),
];

pub fn fill(text: &str, args: &[(&str, String)]) -> String {
    args.iter().fold(text.to_string(), |s, (name, value)| {
        s.replace(&format!("{{{name}}}"), value)
    })
}

/// `engineeringNumber`: two decimals at most, trailing zeros dropped; `—`
/// when unknown.
pub fn number(value: Option<f64>) -> String {
    match value.filter(|v| v.is_finite()) {
        None => "—".into(),
        Some(v) => {
            let s = format!("{v:.2}");
            let s = if s.contains('.') {
                s.trim_end_matches('0').trim_end_matches('.')
            } else {
                &s
            };
            if s == "-0" { "0".into() } else { s.into() }
        }
    }
}

/// `engineeringPercent`: one decimal and `%`; `—` when unknown.
pub fn percent(value: Option<f64>) -> String {
    match value.filter(|v| v.is_finite()) {
        None => "—".into(),
        Some(v) => format!("{v:.1}%"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_in_the_web_viewer() {
        assert_eq!(number(Some(11.0)), "11");
        assert_eq!(number(Some(1.5)), "1.5");
        assert_eq!(number(Some(2.0 / 3.0)), "0.67");
        assert_eq!(number(Some(-0.001)), "0");
        assert_eq!(number(None), "—");
        assert_eq!(number(Some(f64::NAN)), "—");
        assert_eq!(percent(Some(100.0)), "100.0%");
        assert_eq!(percent(None), "—");
    }

    #[test]
    fn plural_and_placeholders_follow_the_language() {
        let forms = [
            "{count} file",
            "{count} files",
            "{count} Datei",
            "{count} Dateien",
        ];
        assert_eq!(Lang { de: false }.plural(1, forms), "1 file");
        assert_eq!(Lang { de: true }.plural(3, forms), "3 Dateien");
        let de = Lang { de: true };
        assert_eq!(
            de.f(
                "{a} of {b}",
                "{a} von {b}",
                &[("a", "1".into()), ("b", "2".into())]
            ),
            "1 von 2"
        );
        assert_eq!(de.status("weird"), "nicht verfügbar");
        assert_eq!(Lang::default().status(""), "not imported");
    }
}
