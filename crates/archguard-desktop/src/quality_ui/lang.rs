//! Status words and number formats the tab builds in Rust, in the web
//! viewer's wording (`viewer_i18n.js`) and formats (`engineeringNumber`,
//! `engineeringPercent`, `engineeringStatus`).

use crate::tab_support::Texts;

/// `engineeringStatus`, falling back to `coverageStatusText`.
pub fn status<'a>(lang: &'a Texts, status: &str) -> &'a str {
    let status = if status.is_empty() {
        "not_imported"
    } else {
        status
    };
    let (_, label) = STATUS
        .iter()
        .find(|(key, _)| *key == status)
        .unwrap_or(&("", "not available"));
    lang.t(label)
}

/// Status labels of `engineeringStatus` and `coverageStatusText`; the
/// labels are templates in `ui/texts.slint`.
const STATUS: [(&str, &str); 17] = [
    ("unchanged_since_import", "unchanged since import"),
    ("changed_since_import", "changed since import"),
    ("unmapped", "unmapped"),
    ("not_recorded", "not recorded"),
    ("compared", "compared"),
    ("no_baseline", "no previous import"),
    ("unverified", "unverified"),
    ("comparable", "comparable"),
    ("incomparable", "not comparable"),
    ("present", "present"),
    ("added", "added"),
    ("removed", "removed"),
    ("not_imported", "not imported"),
    ("ready", "imported"),
    ("partial", "incomplete report"),
    ("stale", "stale: sources changed"),
    ("error", "unavailable: import error"),
];

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

    /// Every status label is a template, so the scan of call sites does
    /// not see them; check them here.
    #[test]
    fn status_words_follow_the_language() {
        let de = Texts::german();
        assert_eq!(status(&de, "weird"), "nicht verfügbar");
        assert_eq!(status(&de, "stale"), "veraltet: Quellen geändert");
        assert_eq!(status(&Texts::default(), ""), "not imported");
        for (_, label) in STATUS {
            assert_ne!(de.t(label), label, "{label} has German");
        }
    }
}
