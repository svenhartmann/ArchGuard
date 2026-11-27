//! Source bytes to text: one decoder for every reader of source files.

/// The text of a file, and whether decoding was faithful.
#[derive(Debug, PartialEq, Eq)]
pub enum Decoded {
    /// BOM dropped, every byte decoded as written.
    Clean(String),
    /// Invalid UTF-8 became U+FFFD: the text is usable, but not what is on disk.
    Lossy(String),
    /// A UTF-16 file with an odd byte count or unpaired surrogates: no text.
    Unreadable,
}

/// Decodes by byte order mark: UTF-8 BOM, UTF-16LE (`FF FE`), UTF-16BE
/// (`FE FF`); without a mark UTF-8. UTF-32 and legacy code pages are not
/// detected (shortcut: unmarked non-UTF-8 reads as `Lossy`, add a detector
/// when real inputs in UTF-32 or a code page need their text).
///
/// One case lies: UTF-32LE starts with `FF FE 00 00`, the UTF-16LE mark, so
/// such a file decodes as UTF-16LE into `Clean` text full of NUL characters —
/// `Clean` then claims a faithfulness it does not have. Harmless while no
/// caller reads `Clean` as an assurance; a caller that does (a diagnostic,
/// say) needs the detector above first. UTF-32BE (`00 00 FE FF`) carries no
/// mark this decoder knows and reads as `Lossy`.
pub fn decode_text(bytes: &[u8]) -> Decoded {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        if !rest.len().is_multiple_of(2) {
            return Decoded::Unreadable;
        }
        let units = rest.as_chunks::<2>().0.iter().map(|c| unit(*c));
        match char::decode_utf16(units).collect::<Result<String, _>>() {
            Ok(text) => Decoded::Clean(text),
            Err(_) => Decoded::Unreadable,
        }
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        _ => {
            let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
            match String::from_utf8_lossy(bytes) {
                std::borrow::Cow::Borrowed(text) => Decoded::Clean(text.to_owned()),
                std::borrow::Cow::Owned(text) => Decoded::Lossy(text),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_outcomes() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFa"), Decoded::Clean("a".into()));
        assert_eq!(decode_text(b"a\xE9"), Decoded::Lossy("a\u{FFFD}".into()));
        assert_eq!(decode_text(b"\xFF\xFEa"), Decoded::Unreadable);
        assert_eq!(decode_text(b"\xFF\xFE\x00\xD8"), Decoded::Unreadable);
    }
}
