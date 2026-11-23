//! The source model of the C-like readers: one file with comments blanked.
use std::collections::HashMap;

use crate::lex::{self, literal_end};

/// A source file with comments blanked (offsets and lines unchanged).
pub struct Source {
    pub rel: String,
    pub text: String,
    pairs: HashMap<usize, usize>,
    lines: Vec<usize>,
}
impl Source {
    /// Whether the header starting at `i` and ending at line end `end` goes on
    /// below: the line holds only modifier keywords (`final`, `public`), or
    /// only a return type that a name and `(` follow.
    pub(crate) fn header_goes_on(&self, i: usize, end: usize) -> bool {
        const MODS: [&str; 17] = [
            "public",
            "private",
            "protected",
            "internal",
            "static",
            "final",
            "abstract",
            "open",
            "override",
            "synchronized",
            "native",
            "default",
            "sealed",
            "virtual",
            "async",
            "partial",
            "suspend",
        ];
        let line = &self.text[i..end];
        let rest = (line.split_whitespace().filter(|_| true)).collect::<Vec<_>>();
        let rest = rest.iter().skip_while(|w| MODS.contains(w)).count();
        let type_only = (rest == 1 || (rest > 1 && line.contains('<')))
            && line
                .chars()
                .all(|c| c.is_alphanumeric() || " _.<>,[]?".contains(c));
        let name_next = (self.text[end..].trim_start().split_once('(')).is_some_and(|(n, _)| {
            !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_')
        });
        self.bytes().get(end) == Some(&b'\n') && (rest == 0 || (type_only && name_next))
    }

    pub fn new(rel: String, raw: &str) -> Self {
        let text = lex::strip_comments(raw);
        let b = text.as_bytes();
        let lines = std::iter::once(0)
            .chain(
                b.iter()
                    .enumerate()
                    .filter(|(_, c)| **c == b'\n')
                    .map(|(i, _)| i + 1),
            )
            .collect();
        Self {
            rel,
            pairs: bracket_pairs(b),
            text,
            lines,
        }
    }
    pub fn line(&self, at: usize) -> u32 {
        self.lines.partition_point(|&s| s <= at) as u32
    }
    pub fn pair(&self, at: usize) -> Option<usize> {
        self.pairs.get(&at).copied()
    }
    pub fn bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }
}

/// Matching bracket positions, both ways, outside literals.
fn bracket_pairs(b: &[u8]) -> HashMap<usize, usize> {
    let (mut pairs, mut stack, mut i) = (HashMap::new(), Vec::new(), 0);
    while i < b.len() {
        if let Some(end) = literal_end(b, i) {
            i = end;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => stack.push(i),
            b')' | b']' | b'}' => {
                if let Some(open) = stack.pop() {
                    pairs.insert(open, i);
                    pairs.insert(i, open);
                }
            }
            _ => {}
        }
        i += 1;
    }
    pairs
}

pub fn statement_end(src: &Source, mut j: usize) -> usize {
    let b = src.bytes();
    while j < b.len() {
        if let Some(end) = literal_end(b, j) {
            j = end;
            continue;
        }
        match b[j] {
            b'(' | b'[' | b'{' => j = src.pair(j).map_or(j + 1, |c| c + 1),
            b';' => return j,
            _ => j += 1,
        }
    }
    b.len()
}
