//! Technical-debt marker scan (R11): TODO / FIXME / HACK / XXX.
//!
//! Markers are counted where a comment starts in the language's syntax;
//! the scan is a documented line heuristic, not a parser — a marker
//! inside a string literal can match (see the tests).
//!
//! [`DebtMarker`] and [`MarkerKind`] are shared vocabulary and live in
//! `archguard-core` (the engine aggregates them into `DebtDensity`);
//! they are re-exported here as the producing module's interface.

use crate::walk::Lang;

pub use archguard_core::{DebtMarker, MarkerKind};

/// Every marker occurrence in `content` of file `rel_path`, line by line.
///
/// Documented heuristic (deliberately not a parser):
/// * A line is treated as comment text from the first comment token
///   (`//` or `/*` for C-like languages, `#` for Python) or, for
///   C-like languages, when it starts (whitespace ignored) with `*` —
///   a block-comment continuation line.
/// * Markers match case-insensitively on word boundaries, so `TODOS`
///   is not a `TODO`.
/// * Because the comment detection is textual, a marker inside a
///   *string literal* that also opens a comment token can match. That
///   is accepted and pinned by a test; a full lexer is out of scope.
pub fn scan_debt(rel_path: &str, content: &str, lang: Lang) -> Vec<DebtMarker> {
    let c_like = !matches!(lang, Lang::Py);
    let mut markers = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let Some(comment) = comment_text(line, c_like) else {
            continue;
        };
        let upper = comment.to_ascii_uppercase();
        for kind in [
            MarkerKind::Todo,
            MarkerKind::Fixme,
            MarkerKind::Hack,
            MarkerKind::Xxx,
        ] {
            let keyword = kind.keyword();
            let mut from = 0usize;
            let mut found = false;
            while let Some(offset) = upper[from..].find(keyword) {
                let start = from + offset;
                let end = start + keyword.len();
                let boundary_before = start == 0 || !is_word_char(upper.as_bytes()[start - 1]);
                let boundary_after = end >= upper.len() || !is_word_char(upper.as_bytes()[end]);
                if boundary_before && boundary_after {
                    found = true;
                    break;
                }
                from = end;
            }
            if found {
                markers.push(DebtMarker {
                    kind,
                    line: index as u32 + 1,
                    file: rel_path.to_string(),
                });
            }
        }
    }
    markers
}

/// The comment text of a line, `None` when the line carries no comment.
fn comment_text(line: &str, c_like: bool) -> Option<&str> {
    let trimmed = line.trim_start();
    if c_like {
        if let Some(rest) = trimmed.strip_prefix("/*") {
            return Some(rest.trim_end_matches("*/"));
        }
        if trimmed.starts_with('*') {
            return Some(trimmed.trim_start_matches('*').trim());
        }
        if let Some((_, rest)) = trimmed.split_once("//") {
            return Some(rest);
        }
        None
    } else {
        // Python and PHP comment lines start with `#` (PHP also knows
        // `//` and `/* */`; both spellings are accepted for PHP below).
        trimmed.split_once('#').map(|(_, rest)| rest)
    }
}

fn is_word_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::Lang;

    fn kinds(markers: &[DebtMarker]) -> Vec<(MarkerKind, u32)> {
        markers.iter().map(|m| (m.kind, m.line)).collect()
    }

    #[test]
    fn markers_are_found_in_three_languages() {
        let ts = "// TODO: split this\nconst a = 1; // fixme later\n/* HACK tight loop */\n/**\n * xxx unclear\n */\n";
        let found = scan_debt("a.ts", ts, Lang::Ts);
        assert_eq!(
            kinds(&found),
            vec![
                (MarkerKind::Todo, 1),
                (MarkerKind::Fixme, 2),
                (MarkerKind::Hack, 3),
                (MarkerKind::Xxx, 5),
            ]
        );
        for marker in &found {
            assert_eq!(marker.file, "a.ts");
        }

        let py = "# TODO rewrite\nx = 1  # FIXME: slow\n";
        assert_eq!(
            kinds(&scan_debt("m.py", py, Lang::Py)),
            vec![(MarkerKind::Todo, 1), (MarkerKind::Fixme, 2)]
        );

        let java = "class A { // Xxx revisit\n}\n";
        assert_eq!(
            kinds(&scan_debt("A.java", java, Lang::Java)),
            vec![(MarkerKind::Xxx, 1)]
        );
    }

    #[test]
    fn markers_in_code_not_comment_do_not_match() {
        // No comment token on the line: identifiers/strings alone are
        // never markers.
        let code = "const TODO = 1;\nfixme();\nlet hack = computeXXX();\n";
        assert!(scan_debt("a.ts", code, Lang::Ts).is_empty());
    }

    #[test]
    fn word_boundaries_reject_longer_identifiers() {
        assert!(scan_debt("a.ts", "// TODOS later\n", Lang::Ts).is_empty());
        // A hyphen is a word boundary, so hyphenated suffixes still
        // carry the marker itself.
        assert_eq!(
            kinds(&scan_debt("a.ts", "// FIXME-DINGENS\n", Lang::Ts)),
            vec![(MarkerKind::Fixme, 1)]
        );
        assert_eq!(
            kinds(&scan_debt("a.ts", "// TODO-dingens\n", Lang::Ts)),
            vec![(MarkerKind::Todo, 1)]
        );
    }

    #[test]
    fn marker_inside_a_string_with_comment_token_is_a_documented_heuristic() {
        // The detection is textual: a string containing `// TODO` opens a
        // pseudo-comment and matches. Accepted and pinned, see the doc.
        assert_eq!(
            kinds(&scan_debt(
                "a.ts",
                "const url = \"http://x// TODO fake\";\n",
                Lang::Ts
            )),
            vec![(MarkerKind::Todo, 1)]
        );
    }

    #[test]
    fn python_hash_never_matches_without_comment_token() {
        assert!(scan_debt("m.py", "TODO = '# not a comment'\n", Lang::Py).is_empty());
    }

    #[test]
    fn signature_is_stable() {
        let f: fn(&str, &str, Lang) -> Vec<DebtMarker> = scan_debt;
        let _ = f;
    }
}
