//! The source model of the C-like readers: one file with comments blanked.
use std::collections::HashMap;

use crate::lex::{self, literal_end};

/// A source file with comments blanked (offsets and lines unchanged).
#[derive(Clone)]
pub struct Source {
    pub rel: String,
    pub text: String,
    pairs: HashMap<usize, usize>,
    lines: Vec<usize>,
}
impl Source {
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
