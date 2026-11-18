//! Identifier and keyword helpers shared by the line-based language scanners.

/// `true` for a plain identifier: a letter or `_`, then letters, digits or
/// `_` (digits up front, punctuation and stray characters never match).
pub(crate) fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// First identifier token (letters, digits, `_`) of `text`, `None` when
/// there is none.
pub(crate) fn first_identifier(text: &str) -> Option<String> {
    first_token(text, |c| c.is_alphanumeric() || c == '_')
}

/// [`first_identifier`] that also counts `$` as an identifier character.
pub(crate) fn first_identifier_with_dollar(text: &str) -> Option<String> {
    first_token(text, |c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// The first run of `is_part` characters in `text`, `None` when there is none.
fn first_token(text: &str, is_part: impl Fn(char) -> bool) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    while index < chars.len() && !is_part(chars[index]) {
        index += 1;
    }
    let start = index;
    while index < chars.len() && is_part(chars[index]) {
        index += 1;
    }
    (index > start).then(|| chars[start..index].iter().collect())
}

/// The remainder of `code` after the whole word `word` (followed by
/// whitespace) and its trailing whitespace, `None` when `code` does not
/// open with that word plus a non-empty remainder.
pub(crate) fn strip_word<'a>(code: &'a str, word: &str) -> Option<&'a str> {
    let rest = code.strip_prefix(word)?;
    let trimmed = rest.trim_start();
    (!trimmed.is_empty() && trimmed.len() != rest.len()).then_some(trimmed)
}
