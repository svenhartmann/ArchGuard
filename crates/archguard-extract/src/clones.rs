//! Token clone detection (Type-1, optionally Type-2): identical token runs,
//! not semantic equivalence. Tokens come from the lightweight lexer that the
//! import scanners use, so comments and layout never count. Every punctuation
//! character is one token, so counts are finer than PMD CPD's.

use crate::walk::Lang;
use std::collections::{BTreeMap, HashMap, hash_map::Entry};
use std::time::Instant;

/// Default minimum clone length in tokens; PMD CPD uses the same number.
pub const DEFAULT_MIN_TOKENS: usize = 100;

/// Limits and normalization for [`find_clones`].
#[derive(Debug, Clone)]
pub struct CloneOptions {
    pub min_tokens: usize,
    /// Treat every identifier and every literal as equal (Type-2 clones).
    pub normalize: bool,
    /// Files beyond this total token budget are not scanned.
    pub max_tokens: usize,
    /// Stop when this instant passes and report the scan as truncated.
    pub deadline: Option<Instant>,
}

impl Default for CloneOptions {
    fn default() -> Self {
        Self {
            min_tokens: DEFAULT_MIN_TOKENS,
            normalize: false,
            max_tokens: 5_000_000,
            deadline: None,
        }
    }
}

/// One copy: `file` indexes the input slice; lines and columns are 1-based
/// and inclusive, columns count characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CloneLocation {
    pub file: usize,
    pub line: u32,
    pub end_line: u32,
    pub column: u32,
    pub end_column: u32,
}

/// Two or more copies of the same `tokens`-long token run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneGroup {
    pub tokens: usize,
    pub occurrences: Vec<CloneLocation>,
}

#[derive(Debug, Clone, Default)]
pub struct CloneScan {
    pub groups: Vec<CloneGroup>,
    pub files_scanned: usize,
    pub tokens_scanned: usize,
    /// Why the scan stopped early; `None` only for a complete scan.
    pub truncated: Option<String>,
}

/// Words that stay distinct in normalized mode, the union over all scanned
/// languages: `if` and `while` must not match each other.
#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "abstract", "and", "as", "async", "await", "break", "case", "catch", "class", "const",
    "continue", "def", "default", "defer", "del", "delete", "do", "elif", "else", "enum",
    "except", "export", "extends", "false", "final", "finally", "fn", "for", "foreach", "from",
    "fun", "func", "function", "go", "if", "impl", "implements", "import", "in", "instanceof",
    "interface", "is", "lambda", "let", "loop", "match", "mod", "move", "mut", "namespace",
    "new", "nil", "None", "not", "null", "object", "or", "override", "package", "pass",
    "private", "protected", "pub", "public", "raise", "range", "return", "select", "self",
    "Self", "static", "struct", "super", "switch", "this", "throw", "throws", "trait", "True",
    "False", "true", "try", "type", "typeof", "unsafe", "use", "using", "val", "var", "void",
    "when", "where", "while", "with", "yield",
];
const BASE: u64 = 1_000_003;

/// A lexed token: interned symbol, 1-based lines and byte range.
struct Tok {
    sym: u32,
    line: u32,
    end_line: u32,
    start: usize,
    end: usize,
}
/// Position of a token in the scanned streams: (file, token index).
type At = (usize, usize);

/// Every clone group among `files`, each given as language and source text.
pub fn find_clones(files: &[(Lang, &str)], options: &CloneOptions) -> CloneScan {
    let mut scan = CloneScan::default();
    let mut symbols = HashMap::new();
    let mut streams = Vec::new();
    for (lang, text) in files {
        if expired(options) {
            scan.truncated = Some(format!(
                "time limit reached after lexing {} of {} files",
                streams.len(),
                files.len()
            ));
            break;
        }
        let tokens = lex(*lang, text, options.normalize, &mut symbols);
        if scan.tokens_scanned + tokens.len() > options.max_tokens {
            scan.truncated = Some(format!(
                "token budget of {} reached; {} of {} files not scanned",
                options.max_tokens,
                files.len() - streams.len(),
                files.len()
            ));
            break;
        }
        scan.tokens_scanned += tokens.len();
        streams.push(tokens);
    }
    scan.files_scanned = streams.len();
    let found = match_windows(&streams, options, &mut scan.truncated);
    scan.groups = found
        .into_iter()
        .map(|(a, copies)| group(files, &streams, a, &copies))
        .collect();
    scan.groups
        .sort_by(|a, b| a.occurrences.cmp(&b.occurrences));
    scan
}

fn expired(options: &CloneOptions) -> bool {
    options.deadline.is_some_and(|d| Instant::now() >= d)
}

/// Tokens without layout; in normalized mode names and literals collapse.
fn lex(lang: Lang, text: &str, normalize: bool, symbols: &mut HashMap<String, u32>) -> Vec<Tok> {
    let mut out = Vec::new();
    for t in crate::lex::tokens(text, crate::lex::flavor(lang)) {
        if t.text == "\n" {
            continue;
        }
        let first = t.text.chars().next().unwrap_or(' ');
        let key = if !normalize {
            t.text.as_str()
        } else if t.literal || first.is_ascii_digit() {
            "\0lit"
        } else if (first.is_alphabetic() || matches!(first, '_' | '$'))
            && !KEYWORDS.contains(&t.text.as_str())
        {
            "\0id"
        } else {
            t.text.as_str()
        };
        let next = symbols.len() as u32;
        let sym = *symbols.entry(key.to_owned()).or_insert(next);
        out.push(Tok {
            sym,
            line: t.line,
            end_line: t.end_line,
            start: t.start,
            end: t.end,
        });
    }
    out
}

fn mix(sym: u32) -> u64 {
    (u64::from(sym) + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// Rabin-Karp over every `min_tokens` window. The first window with a hash
/// is remembered; a later equal window that is verified and not inside an
/// already reported copy in the same file becomes a copy of it.
fn match_windows(
    streams: &[Vec<Tok>],
    options: &CloneOptions,
    truncated: &mut Option<String>,
) -> BTreeMap<At, Vec<(At, usize)>> {
    let m = options.min_tokens.max(1);
    let power = (1..m).fold(1u64, |p, _| p.wrapping_mul(BASE));
    let mut first = HashMap::<u64, At>::new();
    let mut found = BTreeMap::<At, Vec<(At, usize)>>::new();
    for (f, tokens) in streams.iter().enumerate() {
        let mut covered = 0;
        let mut hash = 0u64;
        for (i, t) in tokens.iter().enumerate() {
            if i >= m {
                hash = hash.wrapping_sub(mix(tokens[i - m].sym).wrapping_mul(power));
            }
            hash = hash.wrapping_mul(BASE).wrapping_add(mix(t.sym));
            if i + 1 < m {
                continue;
            }
            let start = i + 1 - m;
            if start.is_multiple_of(4096) && expired(options) {
                *truncated = Some(format!(
                    "time limit reached while matching file {} of {}; groups are incomplete",
                    f + 1,
                    streams.len()
                ));
                return found;
            }
            match first.entry(hash) {
                Entry::Vacant(e) => {
                    e.insert((f, start));
                }
                Entry::Occupied(e) if start >= covered => {
                    if let Some(len) = extend(streams, *e.get(), (f, start), m) {
                        found.entry(*e.get()).or_default().push(((f, start), len));
                        covered = start + len;
                    }
                }
                Entry::Occupied(_) => {}
            }
        }
    }
    found
}

/// Length of the equal token run at `a` and `b`, if it reaches `m` and the
/// two copies do not overlap.
fn extend(streams: &[Vec<Tok>], a: At, b: At, m: usize) -> Option<usize> {
    let cap = if a.0 == b.0 { b.1 - a.1 } else { usize::MAX };
    if cap < m {
        return None;
    }
    let (x, y) = (&streams[a.0][a.1..], &streams[b.0][b.1..]);
    let len = x
        .iter()
        .zip(y)
        .take(cap)
        .take_while(|(p, q)| p.sym == q.sym)
        .count();
    (len >= m).then_some(len)
}

/// All copies of one first occurrence, cut to the shortest common length.
fn group(
    files: &[(Lang, &str)],
    streams: &[Vec<Tok>],
    a: At,
    copies: &[(At, usize)],
) -> CloneGroup {
    let tokens = copies.iter().map(|c| c.1).min().unwrap_or(0);
    let mut occurrences: Vec<_> = std::iter::once(a)
        .chain(copies.iter().map(|c| c.0))
        .map(|(f, i)| {
            let (first, last) = (&streams[f][i], &streams[f][i + tokens - 1]);
            let text = files[f].1;
            CloneLocation {
                file: f,
                line: first.line,
                end_line: last.end_line,
                column: column(text, first.start),
                end_column: column(text, last.end) - 1,
            }
        })
        .collect();
    occurrences.sort();
    CloneGroup {
        tokens,
        occurrences,
    }
}

/// 1-based character column of byte offset `at`.
fn column(text: &str, at: usize) -> u32 {
    let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    text[line_start..at].chars().count() as u32 + 1
}
