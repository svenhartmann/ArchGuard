//! Sentences of the generated arc42 text in the document's language (ADR
//! 0008). A leaf module: every arc42 module uses it, it uses none of them.

use archguard_core::DocLang;
use serde_json::Value;
use std::fmt::Write;

/// Entries per list before an excerpt note.
pub(crate) const LIMIT: usize = 30;

/// `template` with each `{}` replaced by the next value, in order: the
/// sentence of the document's language is chosen at run time (ADR 0008).
pub(crate) fn fill(template: &str, values: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::new();
    let mut rest = template;
    let mut values = values.iter();
    while let Some(at) = rest.find("{}") {
        out.push_str(&rest[..at]);
        if let Some(v) = values.next() {
            let _ = write!(out, "{v}");
        }
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

/// Escapes only what changes Markdown structure inside a line, so paths and
/// sentences stay readable in the hand-edited file.
pub(crate) fn cell(text: &str) -> String {
    let short: String = text.chars().take(512).collect();
    let text = if text.chars().count() > 512 {
        // Language-neutral: `cell` is shared by every section.
        short + " […]"
    } else {
        short
    };
    let mut out = String::new();
    for (i, ch) in text.chars().enumerate() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' | '\n' | '\t' => out.push(' '),
            '\\' | '`' | '*' | '_' | '[' | ']' | '|' | '~' | '!' => {
                out.push('\\');
                out.push(ch);
            }
            '#' | '+' | '-' | '=' if i == 0 => {
                out.push('\\');
                out.push(ch);
            }
            ch => out.push(ch),
        }
    }
    out
}
pub(crate) fn val(l: DocLang, value: &Value) -> String {
    match value {
        Value::Null => l.pick("unknown", "unbekannt").into(),
        Value::Bool(b) => yes(l, *b).into(),
        Value::String(s) => cell(s),
        other => cell(&other.to_string()),
    }
}
pub(crate) fn yes(l: DocLang, value: bool) -> &'static str {
    match (value, l) {
        (true, DocLang::En) => "yes",
        (false, DocLang::En) => "no",
        (true, DocLang::De) => "ja",
        (false, DocLang::De) => "nein",
    }
}
/// Producer status codes with their German words; in English the code
/// itself reads as words ("not_scanned" → "not scanned").
const STATUS_DE: [(&str, &str); 19] = [
    ("ready", "verfügbar"),
    ("partial", "teilweise"),
    ("unavailable", "nicht verfügbar"),
    ("not_imported", "nicht importiert"),
    ("not_scanned", "nicht gescannt"),
    ("not_applicable", "nicht anwendbar"),
    ("not_recorded", "nicht erfasst"),
    ("error", "Fehler"),
    ("stale", "veraltet"),
    ("current", "aktuell"),
    ("changed_since_import", "seit Import geändert"),
    ("unchanged_since_import", "seit Import unverändert"),
    ("changed_since_report", "seit Bericht geändert"),
    ("analyzed", "analysiert"),
    ("measured", "gemessen"),
    ("incomplete", "unvollständig"),
    ("unmapped", "nicht zugeordnet"),
    ("mixed", "gemischt"),
    ("unverified", "ungeprüft"),
];

/// A producer status code in words; unknown codes stay visible as code.
pub(crate) fn status(l: DocLang, value: &Value) -> String {
    let Some(code) = value.as_str() else {
        return val(l, value);
    };
    match STATUS_DE.iter().find(|(c, _)| *c == code) {
        None => format!("`{}`", code.replace('`', "'")),
        Some((_, de)) if l == DocLang::De => (*de).into(),
        Some(_) if code == "ready" => "available".into(),
        Some(_) => code.replace('_', " "),
    }
}
/// Writes header and alignment lines plus rows, or one sentence when there
/// is nothing to show (no output for an empty sentence).
pub(crate) fn table(out: &mut String, head: &str, rows: Vec<String>, empty: &str) {
    if rows.is_empty() {
        if !empty.is_empty() {
            let _ = writeln!(out, "{empty}\n");
        }
        return;
    }
    let _ = writeln!(out, "{head}");
    for row in rows {
        let _ = writeln!(out, "{row}");
    }
    out.push('\n');
}
pub(crate) fn rows(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}
pub(crate) fn note(l: DocLang, out: &mut String, count: usize) {
    if count > LIMIT {
        let text = l.pick(
            "Excerpt: {} of {} entries. Further evidence is in the respective ArchGuard views.",
            "Auszug: {} von {} Einträgen. Weitere Belege sind in den jeweiligen ArchGuard-Ansichten verfügbar.",
        );
        let _ = writeln!(out, "\n{}\n", fill(text, &[&LIMIT, &count]));
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_puts_values_in_order_and_keeps_the_rest() {
        assert_eq!(fill("{} of {}", &[&3, &"ten"]), "3 of ten");
        assert_eq!(
            fill("{} and {}", &[&1]),
            "1 and ",
            "a missing value is empty"
        );
        assert_eq!(fill("none", &[&1]), "none");
    }
}
