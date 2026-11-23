//! Java and Kotlin lexing of the domain extractor (ADR 0022): comments and
//! literal contents blanked.
use crate::lang_blank::{blank_regions, find, line_end};

/// `src` with comments and the contents of string, text-block (`"""`) and
/// char literals blanked, offsets and lines unchanged.
pub fn mask(src: &str) -> String {
    blank_regions(src, |b, i| {
        let next = b.get(i + 1).copied();
        match b[i] {
            b'/' if next == Some(b'/') => Some((i, line_end(b, i), line_end(b, i))),
            b'/' if next == Some(b'*') => {
                let end = find(b, i + 2, b"*/").map_or(b.len(), |p| p + 2);
                Some((i, end, end))
            }
            b'"' if b[i..].starts_with(b"\"\"\"") => {
                let end = find(b, i + 3, b"\"\"\"").unwrap_or(b.len());
                Some((i + 3, end, end + 3))
            }
            b'\'' | b'"' => Some(quoted(b, i)),
            _ => None,
        }
    })
}

/// A `"…"` or `'…'` literal on one line, with `\` escapes.
fn quoted(b: &[u8], i: usize) -> (usize, usize, usize) {
    let q = b[i];
    let mut j = i + 1;
    while j < b.len() && b[j] != q && b[j] != b'\n' {
        j += if b[j] == b'\\' { 2 } else { 1 };
    }
    let j = j.min(b.len());
    (i + 1, j, j + 1)
}
