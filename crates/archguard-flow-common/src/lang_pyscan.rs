//! Byte scanning over masked Python text (`lang_python::mask`), for readers
//! that walk statements by offset (the FastAPI extractor).
use crate::lang_text::is_id;

/// Spaces and tabs only: outside brackets a newline ends the statement.
pub fn skip_sp(b: &[u8], mut i: usize) -> usize {
    while b.get(i).is_some_and(|c| *c == b' ' || *c == b'\t') {
        i += 1;
    }
    i
}

/// Any white space and line continuations, for inside brackets.
pub fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while (b.get(i)).is_some_and(|c| c.is_ascii_whitespace() || *c == b'\\') {
        i += 1;
    }
    i
}

/// Keyword or name `w` as a whole word at `i`.
pub fn word(b: &[u8], i: usize, w: &str) -> bool {
    b[i..].starts_with(w.as_bytes())
        && !b.get(i + w.len()).copied().is_some_and(is_id)
        && (i == 0 || !is_id(b[i - 1]))
}

/// The dotted name at `i` (`router.get`) and where it ends; empty and `i`
/// when no name starts there.
pub fn dotted(b: &[u8], i: usize) -> (String, usize) {
    let end = (i..b.len())
        .find(|&k| !is_id(b[k]) && b[k] != b'.')
        .unwrap_or(b.len());
    if !b.get(i).is_some_and(|c| is_id(*c) && !c.is_ascii_digit()) {
        return (String::new(), i);
    }
    (String::from_utf8_lossy(&b[i..end]).into_owned(), end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanning_helpers_read_names_and_blanks() {
        let b = b"  \\\n router.get(x) is_a";
        assert_eq!(skip_sp(b, 0), 2);
        assert_eq!(skip_ws(b, 0), 5);
        assert_eq!(dotted(b, 5), ("router.get".to_string(), 15));
        assert_eq!(dotted(b"1a.b", 0), (String::new(), 0));
        assert!(word(b, 5, "router.get") && !word(b, 5, "rout"));
        assert!(!word(b, 20, "s_a") && word(b, 19, "is_a"));
    }
}
