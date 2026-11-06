//! The one number formatter of the app. Rust formats every number the UI
//! shows; Slint calls back into it through the `Fmt` global
//! (`ui/format.slint`) instead of turning ints into strings itself.
//!
//! The locale follows the window language: German writes `4,11` and
//! `2.374`, English `4.11` and `2,374`. Unknown values read `—`, observed
//! lower bounds `≥ n`.

use slint::ComponentHandle;

use crate::ui::{AppWindow, Fmt};

/// Number conventions of the window language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Locale {
    #[default]
    En,
    De,
}

/// What an unknown value reads as.
pub const UNKNOWN: &str = "—";

impl Locale {
    /// `"de"` for German, English for everything else.
    pub fn from_code(code: &str) -> Self {
        if code.to_lowercase().starts_with("de") {
            Locale::De
        } else {
            Locale::En
        }
    }

    /// The code `Locale::from_code` reads.
    pub fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::De => "de",
        }
    }

    fn separators(self) -> (char, char) {
        match self {
            Locale::En => ('.', ','),
            Locale::De => (',', '.'),
        }
    }

    /// An integer with thousands separators.
    pub fn int(self, n: i64) -> String {
        let digits = n.unsigned_abs().to_string();
        let sign = if n < 0 { "-" } else { "" };
        format!("{sign}{}", self.group(&digits))
    }

    /// `n` as [`Locale::int`]; for counts held as `usize`.
    pub fn count(self, n: usize) -> String {
        self.int(i64::try_from(n).unwrap_or(i64::MAX))
    }

    /// `v` with exactly `places` decimals; `—` when not finite.
    pub fn dec(self, v: f64, places: usize) -> String {
        if !v.is_finite() {
            return UNKNOWN.into();
        }
        let s = format!("{:.*}", places, v.abs());
        let negative = v < 0.0 && s.chars().any(|c| c != '0' && c != '.');
        self.localise(&s, negative)
    }

    /// `v` with at most `places` decimals, trailing zeros dropped
    /// (`1.5`, `11`); `—` when not finite.
    pub fn num(self, v: f64, places: usize) -> String {
        if !v.is_finite() {
            return UNKNOWN.into();
        }
        let s = format!("{:.*}", places, v.abs());
        let s = if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.')
        } else {
            &s
        };
        let negative = v < 0.0 && s != "0";
        self.localise(s, negative)
    }

    /// `v` percent with `places` decimals: `12.5%`, German `12,5 %`.
    pub fn pct(self, v: f64, places: usize) -> String {
        if !v.is_finite() {
            return UNKNOWN.into();
        }
        let n = self.dec(v, places);
        match self {
            Locale::En => format!("{n}%"),
            Locale::De => format!("{n}\u{a0}%"),
        }
    }

    /// An observed lower bound: `≥1,234`.
    pub fn lower_bound(self, n: i64) -> String {
        format!("≥{}", self.int(n))
    }

    /// `part / whole`.
    pub fn ratio(self, part: i64, whole: i64) -> String {
        format!("{} / {}", self.int(part), self.int(whole))
    }

    /// `s` (ASCII digits and one `.`) with this locale's separators.
    fn localise(self, s: &str, negative: bool) -> String {
        let (decimal, _) = self.separators();
        let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
        let sign = if negative { "-" } else { "" };
        let whole = self.group(whole);
        if frac.is_empty() {
            format!("{sign}{whole}")
        } else {
            format!("{sign}{whole}{decimal}{frac}")
        }
    }

    fn group(self, digits: &str) -> String {
        let (_, thousands) = self.separators();
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(thousands);
            }
            out.push(c);
        }
        out
    }
}

/// Let Slint's `Fmt` global format through this module.
pub fn wire(ui: &AppWindow) {
    let fmt = ui.global::<Fmt>();
    fmt.on_format_int(|n, code| Locale::from_code(&code).int(n.into()).into());
    fmt.set_unknown(UNKNOWN.into());
}

/// Switch the numbers Slint shows to `locale`.
pub fn set_locale(ui: &AppWindow, locale: Locale) {
    ui.global::<Fmt>().set_locale(locale.code().into());
}

/// `Some` value formatted with `f`, else `—`.
pub fn known<T>(value: Option<T>, f: impl FnOnce(T) -> String) -> String {
    value.map_or_else(|| UNKNOWN.into(), f)
}

#[cfg(test)]
mod tests {
    use super::Locale::{De, En};
    use super::*;

    #[test]
    fn integers_group_thousands_by_locale() {
        assert_eq!(En.int(2374), "2,374");
        assert_eq!(De.int(2374), "2.374");
        assert_eq!(En.int(0), "0");
        assert_eq!(De.int(999), "999");
        assert_eq!(En.int(-1_234_567), "-1,234,567");
        assert_eq!(De.int(1_000_000), "1.000.000");
        assert_eq!(En.int(i64::MIN), "-9,223,372,036,854,775,808");
        assert_eq!(De.count(12_000), "12.000");
    }

    #[test]
    fn decimals_use_the_locale_separator() {
        assert_eq!(En.dec(4.111, 2), "4.11");
        assert_eq!(De.dec(4.111, 2), "4,11");
        assert_eq!(De.dec(3.546, 2), "3,55");
        assert_eq!(En.dec(1234.5, 1), "1,234.5");
        assert_eq!(De.dec(1234.5, 1), "1.234,5");
        assert_eq!(En.dec(0.0, 2), "0.00");
        assert_eq!(En.dec(-0.001, 2), "0.00", "no negative zero");
        assert_eq!(De.dec(-2.5, 1), "-2,5");
        assert_eq!(En.dec(7.0, 0), "7");
    }

    #[test]
    fn unknown_values_read_as_a_dash() {
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(En.dec(v, 2), UNKNOWN);
            assert_eq!(De.num(v, 2), UNKNOWN);
            assert_eq!(De.pct(v, 1), UNKNOWN);
        }
        assert_eq!(known(None::<f64>, |v| En.num(v, 2)), "—");
        assert_eq!(known(Some(2.0), |v| En.num(v, 2)), "2");
    }

    #[test]
    fn short_numbers_drop_trailing_zeros() {
        assert_eq!(En.num(11.0, 2), "11");
        assert_eq!(En.num(1.5, 2), "1.5");
        assert_eq!(De.num(1.5, 2), "1,5");
        assert_eq!(En.num(2.0 / 3.0, 2), "0.67");
        assert_eq!(En.num(-0.001, 2), "0");
        assert_eq!(De.num(12_345.0, 2), "12.345");
        assert_eq!(En.num(-1.25, 2), "-1.25");
    }

    #[test]
    fn percents_bounds_and_ratios() {
        assert_eq!(En.pct(12.5, 1), "12.5%");
        assert_eq!(De.pct(12.5, 1), "12,5\u{a0}%");
        assert_eq!(En.pct(100.0, 1), "100.0%");
        assert_eq!(De.lower_bound(1234), "≥1.234");
        assert_eq!(En.lower_bound(0), "≥0");
        assert_eq!(En.ratio(3, 2000), "3 / 2,000");
        assert_eq!(De.ratio(1500, 2000), "1.500 / 2.000");
    }

    #[test]
    fn locale_codes_round_trip() {
        assert_eq!(Locale::from_code("de"), De);
        assert_eq!(Locale::from_code("de_DE.UTF-8"), De);
        assert_eq!(Locale::from_code("en"), En);
        assert_eq!(Locale::from_code(""), En);
        assert_eq!(Locale::from_code(De.code()), De);
    }
}
