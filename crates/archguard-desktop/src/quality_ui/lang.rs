//! Status words and number formats the tab builds in Rust, in the web
//! viewer's wording (`viewer_i18n.js`) and formats (`engineeringNumber`,
//! `engineeringPercent`, `engineeringStatus`).

use crate::format::known;
use crate::status::Status;
use crate::tab_support::Texts;

/// `engineeringStatus`, falling back to `coverageStatusText`: the shared
/// [`Status`] word, else the comparison and mapping words below.
pub fn status<'a>(lang: &'a Texts, status: &str) -> &'a str {
    if let Some(s) = Status::from_code(status) {
        return lang.t(s.label());
    }
    let (_, label) = STATUS
        .iter()
        .find(|(key, _)| *key == status)
        .unwrap_or(&("", "not available"));
    lang.t(label)
}

/// Status labels of `engineeringStatus` and `coverageStatusText`; the
/// labels are templates in `ui/texts.slint`.
const STATUS: [(&str, &str); 12] = [
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
];

/// `engineeringNumber`: two decimals at most, trailing zeros dropped; `—`
/// when unknown.
pub fn number(lang: &Texts, value: Option<f64>) -> String {
    known(value, |v| lang.locale().num(v, 2))
}

/// `engineeringPercent`: one decimal and `%`; `—` when unknown.
pub fn percent(lang: &Texts, value: Option<f64>) -> String {
    known(value, |v| lang.locale().pct(v, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_in_the_web_viewer() {
        let en = Texts::default();
        assert_eq!(number(&en, Some(11.0)), "11");
        assert_eq!(number(&en, Some(1.5)), "1.5");
        assert_eq!(number(&en, Some(2.0 / 3.0)), "0.67");
        assert_eq!(number(&en, Some(-0.001)), "0");
        assert_eq!(number(&en, None), "—");
        assert_eq!(number(&en, Some(f64::NAN)), "—");
        assert_eq!(percent(&en, Some(100.0)), "100.0%");
        assert_eq!(percent(&en, None), "—");
        let de = Texts::german();
        assert_eq!(number(&de, Some(1234.5)), "1.234,5");
        assert_eq!(percent(&de, Some(12.5)), "12,5\u{a0}%");
    }

    /// Every status label is a template, so the scan of call sites does
    /// not see them; check them here.
    #[test]
    fn status_words_follow_the_language() {
        let de = Texts::german();
        assert_eq!(status(&de, "weird"), "nicht verfügbar");
        assert_eq!(status(&de, "stale"), "veraltet");
        assert_eq!(status(&de, "ready"), "vollständig");
        assert_eq!(status(&Texts::default(), ""), "not imported");
        for (_, label) in STATUS {
            assert_ne!(de.t(label), label, "{label} has German");
        }
    }
}
