//! Read-only query guard for openCypher statements (T32, Kap. 8, hardened
//! by F04): plain string logic, available without the `db` feature.
//!
//! Security boundary (F04): a *structural allowlist* for exactly one
//! supported read-only statement shape —
//!
//! ```text
//! [ MATCH pattern ] [ WHERE expression ] RETURN projections
//!     [ ORDER BY items ] [ SKIP n ] [ LIMIT n ]
//! ```
//!
//! The lexer understands single- and double-quoted strings (with
//! backslash and doubled-quote escapes), backtick-quoted identifiers,
//! and — exactly matching kuzu's `CypherComment` grammar — `//` line
//! comments and `/* … */` block comments: their content is opaque, so
//! literal words such as `'delete me'` pass and no keyword can be
//! smuggled in or hidden through them. Crucially `--` is NOT a comment
//! (kuzu lexes it as two minus operators), so it is ordinary
//! punctuation here too and `;`/reserved words in its "tail" are still
//! seen; non-ASCII whitespace (NBSP & friends, which kuzu's `WHITESPACE`
//! admits) separates tokens instead of merging into words, so a clause
//! keyword can never hide inside one merged identifier. Outside that content
//! every word is classified: the clause keywords may appear only in
//! their single slot (`ORDER` and `BY` only adjacent), every mutation,
//! administration and composition keyword of openCypher/Kuzu is
//! rejected wherever it appears, and anything else is an identifier.
//! A second statement (a `;` that is not the single trailing one) is
//! rejected before execution, as are unsupported characters, parameter
//! markers, unterminated strings/comments and malformed SKIP/LIMIT
//! clauses. The former keyword denylist is subsumed by the reserved
//! set; keyword matching never acts inside quoted content.

use archguard_core::ArchguardError;

/// Words that structure the single allowed statement shape. They are
/// only legal at their slot position (see [`parse`]).
const CLAUSE_WORDS: [&str; 7] = ["MATCH", "RETURN", "WHERE", "ORDER", "BY", "SKIP", "LIMIT"];

/// Words that may never appear outside quoted content: every statement
/// keyword that mutates the database, administers it, or composes
/// additional query parts — including the ones the old lexical denylist
/// missed (F04: EXPORT, ATTACH, CHECKPOINT, IMPORT, USE, SHOW, …).
/// `WITH` is tolerated only as part of the `STARTS WITH` / `ENDS WITH`
/// operators, which the parser checks against the preceding word.
const RESERVED_WORDS: [&str; 30] = [
    "CREATE",
    "DELETE",
    "DETACH",
    "SET",
    "REMOVE",
    "MERGE",
    "DROP",
    "ALTER",
    "COPY",
    "LOAD",
    "CALL",
    "UNION",
    "EXCEPT",
    "INTERSECT",
    "WITH",
    "UNWIND",
    "OPTIONAL",
    "FOREACH",
    "IMPORT",
    "EXPORT",
    "ATTACH",
    "USE",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "CHECKPOINT",
    "SHOW",
    "DESCRIBE",
    "EXPLAIN",
    "PROFILE",
];

/// Punctuation the one allowed statement shape may contain outside
/// quoted content; `;` is handled separately (single trailing only),
/// anything else (e.g. `$`, `\`, `@`, `#`) rejects the statement.
const ALLOWED_PUNCTUATION: &str = "()[]{}.,:|*=?!<>=+-/%~^";

/// Maximum statement size in bytes (F05, review follow-up). The store
/// re-sanitizes every caller-supplied statement, so this bounds the
/// parser input itself: ANTLR parse/binding cost on an adversarially
/// long statement runs inside `conn.query` *before* kuzu's execution
/// timeout has any effect, and huge inline list/string literals are
/// materialized by the engine at once. Statements beyond this cap
/// reject outright.
pub const MAX_STATEMENT_BYTES: usize = 65_536;

/// Best-effort denylist of list/string *amplifier* functions (F05,
/// review follow-up): these materialize gigabyte-scale results inside
/// the engine in a single uninterruptible operator call from a tiny
/// statement (`RETURN range(0, 1_000_000_000)`), before the cell
/// budget — which only bounds our conversion — or the timeout could
/// help. This is defense in depth, not a proof: unknown amplifiers
/// remain a deployment-level concern (see the store docs). A word
/// counts as a call only when `(` follows it directly, so property
/// names like `m.range` pass.
const AMPLIFYING_FUNCTIONS: [&str; 6] = [
    "RANGE",
    "UNNEST",
    "STR_SPLIT",
    "REPEAT",
    "PAD_LEFT",
    "PAD_RIGHT",
];

/// One lexical token outside comments; string and backtick content is
/// one opaque [`Tok::Literal`] that carries no structure.
enum Tok<'a> {
    /// Maximal alphanumeric/underscore run (plus non-ASCII letters and
    /// digits; non-ASCII whitespace separates, per kuzu's WHITESPACE).
    Word {
        word: &'a str,
        start: usize,
        end: usize,
    },
    /// Single punctuation byte from [`ALLOWED_PUNCTUATION`], or `;`.
    Punct { char: u8, pos: usize },
    /// A quoted string or backtick identifier; content never parsed.
    Literal { end: usize },
}

impl Tok<'_> {
    /// Word slice, `None` for punctuation and literals.
    fn word(&self) -> Option<&str> {
        match self {
            Tok::Word { word, .. } => Some(word),
            _ => None,
        }
    }
}

/// Build the rejection error for a statement that fails the guard.
fn reject(reason: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("query rejected: {reason}"))
}

/// Lex `src` into structural tokens, dropping comments. Returns `Err`
/// (rejection) on an unterminated string, backtick identifier or block
/// comment, and on any byte outside words, whitespace,
/// [`ALLOWED_PUNCTUATION`] and `;`.
fn lex(src: &str) -> Result<Vec<Tok<'_>>, ArchguardError> {
    let bytes = src.as_bytes();
    let mut tokens: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if byte >= 0x80 {
            // Non-ASCII leading byte (i is always at a char boundary —
            // every advance path stops at ASCII delimiters or steps by
            // whole chars). kuzu's WHITESPACE admits NBSP, U+1680 …
            // U+3000, U+2007/202F and the deprecated U+180E; all of them
            // must SEPARATE tokens here, or `LIMIT\u{a0}999999999` would
            // lex as one unknown word and the enforced cap would never
            // be tracked (F04 review, F2).
            let c = src[i..]
                .chars()
                .next()
                .ok_or_else(|| reject("truncated utf-8 sequence"))?;
            if c.is_whitespace() || c == '\u{180e}' {
                i += c.len_utf8();
                continue;
            }
        }
        if byte == b'/' {
            // kuzu CypherComment: only `//` … LF|EOF and non-nested
            // `/* … */`. `--` is NOT a comment there (two minus
            // operators) and must not be one here either (F04 review,
            // F1: hidden `;`/EXPORT payloads).
            match bytes.get(i + 1) {
                Some(&b'/') => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                Some(&b'*') => {
                    // kuzu: '/*' ( ~[*] | '*' ~[/] )* '*/' — a star consumes
                    // the next character, so `**/` does not close the comment.
                    let mut end = i + 2;
                    loop {
                        match bytes.get(end) {
                            None => return Err(reject("unterminated block comment")),
                            Some(b'*') if bytes.get(end + 1) == Some(&b'/') => break,
                            Some(b'*') => end += 2,
                            Some(_) => end += 1,
                        }
                    }
                    i = end + 2;
                    continue;
                }
                // A single slash is ordinary punctuation (division).
                _ => {}
            }
        }
        if byte == b'\'' || byte == b'"' {
            i += 1;
            loop {
                match bytes.get(i) {
                    None => return Err(reject("unterminated string literal")),
                    Some(&b'\\') => {
                        if bytes.get(i + 1).is_none() {
                            return Err(reject("unterminated string literal"));
                        }
                        i += 2;
                    }
                    Some(&quote) if quote == byte => {
                        // Doubled quote is an escape for the quote itself.
                        i += if bytes.get(i + 1) == Some(&byte) {
                            2
                        } else {
                            1
                        };
                        break;
                    }
                    Some(_) => i += 1,
                }
            }
            tokens.push(Tok::Literal { end: i });
            continue;
        }
        if byte == b'`' {
            i += 1;
            loop {
                match bytes.get(i) {
                    None => return Err(reject("unterminated quoted identifier")),
                    Some(b'`') => {
                        i += 1;
                        break;
                    }
                    Some(_) => i += 1,
                }
            }
            tokens.push(Tok::Literal { end: i });
            continue;
        }
        if byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80 {
            let start = i;
            while i < bytes.len() {
                let b = bytes[i];
                if b.is_ascii_alphanumeric() || b == b'_' {
                    i += 1;
                    continue;
                }
                if b >= 0x80 {
                    let c = src[i..]
                        .chars()
                        .next()
                        .ok_or_else(|| reject("truncated utf-8 sequence"))?;
                    if c.is_whitespace() || c == '\u{180e}' {
                        break; // separator — kuzu splits here too
                    }
                    if c.is_alphabetic() || c.is_numeric() {
                        i += c.len_utf8();
                        continue;
                    }
                    // Symbols outside words: kuzu accepts a few more as
                    // identifier chars, but rejecting is the safe
                    // direction (false rejection, never a bypass).
                    return Err(reject(format!("unsupported character `{c}`")));
                }
                break;
            }
            tokens.push(Tok::Word {
                word: &src[start..i],
                start,
                end: i,
            });
            continue;
        }
        let char = byte as char;
        if ALLOWED_PUNCTUATION.contains(char) || char == ';' {
            tokens.push(Tok::Punct { char: byte, pos: i });
            i += 1;
            continue;
        }
        return Err(reject(format!("unsupported character `{char}`")));
    }
    Ok(tokens)
}

/// True when `word` is one of the [`CLAUSE_WORDS`] (case-insensitive).
fn is_clause_word(word: &str) -> bool {
    CLAUSE_WORDS
        .iter()
        .any(|clause| word.eq_ignore_ascii_case(clause))
}

/// True when `word` is a reserved statement keyword. `WITH` is allowed
/// as the second half of the `STARTS WITH` / `ENDS WITH` operators.
fn is_reserved_word(word: &str, previous_word: Option<&str>) -> bool {
    if word.eq_ignore_ascii_case("WITH")
        && previous_word.is_some_and(|prev| {
            prev.eq_ignore_ascii_case("STARTS") || prev.eq_ignore_ascii_case("ENDS")
        })
    {
        return false;
    }
    RESERVED_WORDS
        .iter()
        .any(|reserved| word.eq_ignore_ascii_case(reserved))
}

/// A clause word acts as a slot start only when it is a standalone
/// word — not a property/label name fragment after `.`, `:` or `|`.
fn starts_slot(tokens: &[Tok<'_>], index: usize) -> bool {
    !matches!(
        tokens.get(index.wrapping_sub(1)),
        Some(Tok::Punct {
            char: b'.' | b':' | b'|',
            ..
        })
    )
}

/// Consume one "opaque expression" slot starting at `*index`: any token
/// is legal, but reserved words reject, clause words end the slot only
/// when `is_slot_start` accepts them at that position (and then do not
/// belong to the slot). Returns the number of tokens consumed.
fn scan_slot(
    tokens: &[Tok<'_>],
    index: &mut usize,
    is_slot_start: &dyn Fn(&[Tok<'_>], usize) -> bool,
) -> Result<usize, ArchguardError> {
    let mut count = 0;
    while *index < tokens.len() {
        match &tokens[*index] {
            Tok::Punct { char: b';', .. } => {
                return Err(reject("multiple statements (a second `;`)"));
            }
            Tok::Word { word, .. } => {
                let previous = previous_word(tokens, *index);
                if is_reserved_word(word, previous) {
                    return Err(reject(format!("forbidden keyword `{word}`")));
                }
                if is_clause_word(word) && starts_slot(tokens, *index) {
                    if is_slot_start(tokens, *index) {
                        return Ok(count);
                    }
                    return Err(reject(format!("unexpected clause keyword `{word}`")));
                }
            }
            _ => {}
        }
        *index += 1;
        count += 1;
    }
    Ok(count)
}

/// The nearest preceding Word token, if any; a literal counts as an
/// opaque barrier (`'x' WITH …` is no `STARTS WITH` operator).
fn previous_word<'x>(tokens: &[Tok<'x>], index: usize) -> Option<&'x str> {
    tokens[..index].iter().rev().find_map(|token| match token {
        Tok::Word { word, .. } => Some(*word),
        Tok::Literal { .. } => Some(""),
        Tok::Punct { .. } => None,
    })
}

/// The numeric word directly following the current token, if any.
fn numeric_word<'s, 'x>(tokens: &'s [Tok<'x>], index: usize) -> Option<&'s Tok<'x>> {
    match tokens.get(index) {
        Some(Tok::Word { word, .. })
            if !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            Some(&tokens[index])
        }
        _ => None,
    }
}

/// Span of the `LIMIT` row count when an oversized `LIMIT n` must be
/// capped in place.
struct LimitSlot {
    start: usize,
    end: usize,
}

/// Structural parse of the token stream against the one allowed
/// statement shape; returns the LIMIT row-count span if present.
fn parse(tokens: &[Tok<'_>]) -> Result<Option<LimitSlot>, ArchguardError> {
    let mut index = 0;
    let Some(first_word) = tokens.first().and_then(Tok::word) else {
        return Err(reject("statement is empty or does not start with a word"));
    };
    if !first_word.eq_ignore_ascii_case("MATCH") && !first_word.eq_ignore_ascii_case("RETURN") {
        return Err(reject(format!(
            "statement must start with MATCH or RETURN, found `{first_word}`"
        )));
    }

    // [ MATCH pattern ]
    if first_word.eq_ignore_ascii_case("MATCH") {
        index = 1;
        let consumed = scan_slot(tokens, &mut index, &|tokens, i| {
            tokens[i].word().is_some_and(|word| {
                starts_slot(tokens, i)
                    && (word.eq_ignore_ascii_case("WHERE") || word.eq_ignore_ascii_case("RETURN"))
            })
        })?;
        if consumed == 0 {
            return Err(reject("MATCH without a pattern"));
        }
    }

    // [ WHERE expression ]
    if tokens
        .get(index)
        .and_then(Tok::word)
        .is_some_and(|word| word.eq_ignore_ascii_case("WHERE") && starts_slot(tokens, index))
    {
        index += 1;
        let consumed = scan_slot(tokens, &mut index, &|tokens, i| {
            tokens[i]
                .word()
                .is_some_and(|word| word.eq_ignore_ascii_case("RETURN") && starts_slot(tokens, i))
        })?;
        if consumed == 0 {
            return Err(reject("WHERE without a condition"));
        }
    }

    // RETURN projections (mandatory)
    if !tokens
        .get(index)
        .and_then(Tok::word)
        .is_some_and(|word| word.eq_ignore_ascii_case("RETURN") && starts_slot(tokens, index))
    {
        return Err(reject("statement must contain a RETURN clause"));
    }
    index += 1;
    let slot_starts_return_tail = |tokens: &[Tok<'_>], i: usize| {
        tokens[i].word().is_some_and(|word| {
            starts_slot(tokens, i)
                && (word.eq_ignore_ascii_case("LIMIT")
                    || word.eq_ignore_ascii_case("SKIP")
                    || (word.eq_ignore_ascii_case("ORDER")
                        && tokens
                            .get(i + 1)
                            .and_then(Tok::word)
                            .is_some_and(|next| next.eq_ignore_ascii_case("BY"))))
        })
    };
    let consumed = scan_slot(tokens, &mut index, &slot_starts_return_tail)?;
    if consumed == 0 {
        return Err(reject("RETURN without projections"));
    }

    // [ ORDER BY items ]
    if tokens
        .get(index)
        .and_then(Tok::word)
        .is_some_and(|word| word.eq_ignore_ascii_case("ORDER"))
        && tokens
            .get(index + 1)
            .and_then(Tok::word)
            .is_some_and(|word| word.eq_ignore_ascii_case("BY"))
    {
        index += 2;
        let consumed = scan_slot(tokens, &mut index, &slot_starts_return_tail)?;
        if consumed == 0 {
            return Err(reject("ORDER BY without items"));
        }
    }

    // [ SKIP n ]
    if tokens
        .get(index)
        .and_then(Tok::word)
        .is_some_and(|word| word.eq_ignore_ascii_case("SKIP"))
    {
        if numeric_word(tokens, index + 1).is_none() {
            return Err(reject("SKIP must be followed by a non-negative integer"));
        }
        index += 2;
    }

    // [ LIMIT n ]
    let limit = if tokens
        .get(index)
        .and_then(Tok::word)
        .is_some_and(|word| word.eq_ignore_ascii_case("LIMIT"))
    {
        let Some(Tok::Word { start, end, .. }) = numeric_word(tokens, index + 1) else {
            return Err(reject("LIMIT must be followed by a non-negative integer"));
        };
        index += 2;
        Some(LimitSlot {
            start: *start,
            end: *end,
        })
    } else {
        None
    };

    if index != tokens.len() {
        return Err(reject(format!(
            "unexpected token `{}` after the final clause",
            tokens[index].word().unwrap_or("?")
        )));
    }
    Ok(limit)
}

/// Guard a read-only openCypher statement (T32, hardened by F04) and
/// enforce a row `LIMIT`.
///
/// The statement must match the single read-only shape documented in
/// the module header; anything else — multiple statements, mutation or
/// administration keywords, unsupported syntax, unterminated literals —
/// is rejected before execution. String and backtick content is opaque,
/// so literal words such as `'delete me'` are data, not keywords.
///
/// A trailing `;` (plus surrounding whitespace) is stripped so an
/// appended `LIMIT` stays inside the statement; a `LIMIT n` clause is
/// mandatory: if absent, ` LIMIT {max_limit}` is appended, and every
/// oversized `LIMIT n` (including numbers too large to parse) is
/// replaced in place by `max_limit`.
pub fn sanitize_query(cypher: &str, max_limit: usize) -> Result<String, ArchguardError> {
    let trimmed = cypher.trim();
    // F05 (review): bound the parser input itself — kuzu parses and
    // compiles before its execution timeout can interrupt anything, so
    // an unbounded statement is unbounded compile cost.
    if trimmed.len() > MAX_STATEMENT_BYTES {
        return Err(reject(format!(
            "statement exceeds the {MAX_STATEMENT_BYTES}-byte query size budget"
        )));
    }
    let mut tokens = lex(trimmed)?;

    // Exactly one optional trailing `;`; every other `;` rejects below.
    if matches!(tokens.last(), Some(Tok::Punct { char: b';', .. })) {
        tokens.pop();
    }

    // F05 (review): reject list/string amplifier calls — `range(0, …)`
    // and friends materialize their full result inside one uninterruptible
    // engine operator, ahead of the cell budget (our conversion only) and
    // the timeout. Property names like `m.range` are not calls (same
    // `.`/`:`/`|` rule clause words use).
    for (index, token) in tokens.iter().enumerate() {
        if let Tok::Word { word, .. } = token
            && AMPLIFYING_FUNCTIONS
                .iter()
                .any(|amplifier| word.eq_ignore_ascii_case(amplifier))
            && starts_slot(&tokens, index)
            && matches!(tokens.get(index + 1), Some(Tok::Punct { char: b'(', .. }))
        {
            return Err(reject(format!(
                "function {word}() is not allowed in guarded queries (unbounded engine-side materialization)"
            )));
        }
    }

    let limit = parse(&tokens)?;

    // Rebuild the statement up to the end of the last retained token: this
    // drops the trailing `;`, trailing whitespace and any trailing comment
    // in one step — the appended LIMIT then always sits behind real token
    // text, never behind something kuzu comments away server-side (F04:
    // appending to raw text let `// …` swallow the row cap).
    let last_end = tokens.last().map_or(0, |token| match token {
        Tok::Word { end, .. } | Tok::Literal { end, .. } => *end,
        Tok::Punct { pos, .. } => pos + 1,
    });
    let mut statement = trimmed[..last_end].to_string();

    match limit {
        Some(slot) => {
            let number = &statement[slot.start..slot.end];
            let oversized = number
                .parse::<usize>()
                .map_or(true, |value| value > max_limit);
            if oversized {
                statement.replace_range(slot.start..slot.end, &max_limit.to_string());
            }
        }
        None => {
            statement.push(' ');
            statement.push_str("LIMIT ");
            statement.push_str(&max_limit.to_string());
        }
    }
    Ok(statement)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Table-driven: allowed statements and their guarded form.
    #[test]
    fn block_comments_end_exactly_where_kuzu_ends_them() {
        // kuzu: '/*' ( ~[*] | '*' ~[/] )* '*/'. A star consumes the next
        // character, so `**/` does not close and the text behind it is still
        // comment there, while a first-`*/` scanner sees code and vice versa.
        for input in [
            r#"/***/ RETURN ' */ LOAD FROM "/tmp/x.csv" RETURN * //'"#,
            "/* **/ RETURN 1",
            "/***/ RETURN 1",
        ] {
            assert!(sanitize_query(input, 25).is_err(), "{input}");
        }
    }

    #[test]
    fn allowed_queries_are_passed_through_with_limit_enforced() {
        let cases: Vec<(&str, usize, &str)> = vec![
            // Plain read-only query: untouched except for the appended LIMIT.
            ("MATCH (n) RETURN n", 25, "MATCH (n) RETURN n LIMIT 25"),
            // Existing in-range LIMIT stays verbatim.
            (
                "MATCH (n) RETURN n LIMIT 5",
                25,
                "MATCH (n) RETURN n LIMIT 5",
            ),
            // Oversized LIMIT is capped in place.
            (
                "MATCH (n) RETURN n LIMIT 500",
                25,
                "MATCH (n) RETURN n LIMIT 25",
            ),
            // Lowercase `limit` keyword recognised, replacement keeps the rest.
            (
                "MATCH (n) RETURN n limit 500",
                25,
                "MATCH (n) RETURN n limit 25",
            ),
            // Mixed-case LIMIT keyword in range is untouched.
            (
                "Match (n) Return n Limit 3",
                25,
                "Match (n) Return n Limit 3",
            ),
            // Bare RETURN also passes the start rule and gets a LIMIT.
            ("RETURN 1", 10, "RETURN 1 LIMIT 10"),
            // Lowercase match/return accepted.
            ("match (n) return n", 25, "match (n) return n LIMIT 25"),
            // Leading/trailing whitespace trimmed before appending.
            ("  MATCH (n) RETURN n \n", 25, "MATCH (n) RETURN n LIMIT 25"),
            // Identifier containing a forbidden word (dataset ⊃ set) survives.
            (
                "MATCH (n:Dataset) RETURN n.dataset",
                25,
                "MATCH (n:Dataset) RETURN n.dataset LIMIT 25",
            ),
            // More forbidden-word substrings inside identifiers.
            (
                "MATCH (n) RETURN n.copy_file, n.limitless, n.matched_by, n.offset",
                25,
                "MATCH (n) RETURN n.copy_file, n.limitless, n.matched_by, n.offset LIMIT 25",
            ),
            // Trailing semicolon stripped so the LIMIT lands inside the statement.
            (
                "MATCH (m:module) RETURN m.name ORDER BY m.name;",
                25,
                "MATCH (m:module) RETURN m.name ORDER BY m.name LIMIT 25",
            ),
            // Numeric literal too large to parse counts as oversized.
            (
                "MATCH (n) RETURN n LIMIT 99999999999999999999999999",
                25,
                "MATCH (n) RETURN n LIMIT 25",
            ),
            // LIMIT exactly at max_limit is untouched.
            (
                "MATCH (n) RETURN n LIMIT 25",
                25,
                "MATCH (n) RETURN n LIMIT 25",
            ),
            // The Kap. 8 recipes: relationship patterns and property lists.
            (
                "MATCH ()-[d:depends_on]->() RETURN d.ref_count, d.cyclic;",
                25,
                "MATCH ()-[d:depends_on]->() RETURN d.ref_count, d.cyclic LIMIT 25",
            ),
            (
                "MATCH (s:snapshot) RETURN s.git_sha, s.tag, s.cycles, s.largest_scc, s.taken_at LIMIT 5",
                5,
                "MATCH (s:snapshot) RETURN s.git_sha, s.tag, s.cycles, s.largest_scc, s.taken_at LIMIT 5",
            ),
            // SKIP keeps its place before LIMIT (single clause each).
            (
                "MATCH (n) RETURN n ORDER BY n.name DESC SKIP 2 LIMIT 9",
                25,
                "MATCH (n) RETURN n ORDER BY n.name DESC SKIP 2 LIMIT 9",
            ),
        ];
        for (input, max_limit, expected) in cases {
            let actual = sanitize_query(input, max_limit)
                .unwrap_or_else(|error| panic!("sanitize_query({input:?}) failed: {error}"));
            assert_eq!(actual, expected, "sanitize_query({input:?}, {max_limit})");
        }
    }

    /// F04: string, backtick and comment content is opaque — literal
    /// words are data and never keywords.
    #[test]
    fn quoted_and_commented_content_is_opaque() {
        // The acceptance case: literal words inside strings pass.
        let allowed: Vec<&str> = vec![
            // The acceptance case: literal words inside strings pass.
            "MATCH (n) WHERE n.name = 'delete me' RETURN n.name AS msg LIMIT 5",
            "MATCH (n) WHERE n.detail CONTAINS 'create table' RETURN n",
            "RETURN \"drop table\" LIMIT 5",
            // Backtick-quoted identifiers may carry any reserved word.
            "MATCH (`set`) RETURN `set`.`create`",
            // Comment content is inert on BOTH sides (kuzu CypherComment
            // = `//` … LF/EOF and non-nested `/* */`): a hidden keyword
            // is no keyword, and a `;` inside a comment is content, not
            // a separator — for the guard and for kuzu alike.
            "MATCH (n) RETURN n // DELETE everything",
            "MATCH (n) // DELETE everything\nRETURN n",
            "MATCH (n) /* ; DELETE (n) */ RETURN n",
            "MATCH (n) RETURN n /* keep; going */",
            // kuzu's grammar closes these comments at the same `*/` as the guard.
            "/**/ RETURN 1",
            "/****/ RETURN 1",
            "/* a * b */ RETURN 1",
        ];
        for input in allowed {
            let _ = sanitize_query(input, 25)
                .unwrap_or_else(|error| panic!("sanitize_query({input:?}) failed: {error}"));
        }

        // An appended LIMIT may never end up inside a trailing comment.
        let guarded = sanitize_query("MATCH (n) RETURN n // DROP everything", 25).expect("ok");
        assert!(
            guarded.ends_with("LIMIT 25"),
            "row cap survives the comment: {guarded:?}"
        );
        assert!(
            !guarded.contains("//"),
            "trailing comment is dropped, not kept around the cap: {guarded:?}"
        );

        // F04 review (F1): `--` is NOT a comment in kuzu — its "tail" is
        // ordinary text here too, so reserved words and `;` in it reject.
        for input in [
            "MATCH (n) RETURN n -- DELETE everything",
            "MATCH (n) RETURN n -- ; DELETE (n)",
        ] {
            let error = sanitize_query(input, 25)
                .expect_err("`--` is not a comment in kuzu, input must reject");
            assert!(error.to_string().contains("query rejected"), "{error}");
        }
    }

    /// F04 adversarial-review regressions (comment-grammar divergence):
    /// the exact payloads that were executable while the guard modelled
    /// `--` as a comment and ignored `//`.
    #[test]
    fn review_comment_divergence_payloads_are_rejected() {
        // Review F1 input 1: with `--` wrongly a comment, kuzu saw
        // `RETURN 1 - - 0` plus a second statement `EXPORT DATABASE …`,
        // and the following `// x` line kept `last_end` past the payload.
        for input in [
            "RETURN 1 -- 0; EXPORT DATABASE '/tmp/pwn';\n// x",
            "RETURN 1 -- 0; CALL read_csv('/etc/passwd') RETURN *;\n// x",
        ] {
            let error = sanitize_query(input, 25).expect_err("hidden second statement must reject");
            assert!(error.to_string().contains("query rejected"), "{error}");
        }

        // Review F2: a `//` tail must not survive into the emitted
        // statement, so the appended cap can never be commented out
        // server-side.
        let guarded = sanitize_query("MATCH (a:module),(b:module) RETURN a.name, b.name // x", 21)
            .expect("plain read with comment tail");
        assert!(
            guarded.ends_with("LIMIT 21"),
            "cap must be live: {guarded:?}"
        );
        assert!(
            !guarded.contains("//"),
            "comment tail must be gone: {guarded:?}"
        );

        // Review F2 variant: NBSP is whitespace for kuzu — it must
        // separate, the oversized LIMIT is tracked and capped.
        let guarded = sanitize_query("MATCH (a:module) RETURN a.name LIMIT\u{a0}999999999", 25)
            .expect("NBSP must not hide the LIMIT slot");
        assert!(!guarded.contains("999999999"), "cap applied: {guarded:?}");
        assert!(guarded.contains("LIMIT"), "limit clause kept: {guarded:?}");
        // U+180E is kuzu whitespace: it separates (never merges), so the
        // statement stays the ordinary LIMIT-clause shape.
        let guarded = sanitize_query("MATCH (a:module) RETURN a.name LIMIT\u{180e}5", 25)
            .expect("U+180E must separate, not merge");
        assert!(guarded.contains("LIMIT"), "limit kept: {guarded:?}");
    }

    /// Table-driven: each forbidden keyword is rejected as a whole word,
    /// uppercase and lowercase — including the F04 additions beyond the
    /// old lexical denylist.
    #[test]
    fn forbidden_keywords_are_rejected() {
        let keywords = [
            "CREATE",
            "DELETE",
            "DETACH",
            "SET",
            "MERGE",
            "DROP",
            "COPY",
            "LOAD",
            "CALL",
            "ALTER",
            "REMOVE",
            "UNION",
            "UNWIND",
            "OPTIONAL",
            "FOREACH",
            "IMPORT",
            "EXPORT",
            "ATTACH",
            "USE",
            "CHECKPOINT",
            "SHOW",
            "EXPLAIN",
            "PROFILE",
            "DESCRIBE",
            "BEGIN",
            "COMMIT",
            "ROLLBACK",
        ];
        for keyword in keywords {
            let upper = format!("MATCH (n) RETURN n {keyword} (m)");
            let lower = format!("match (n) return n {} (m)", keyword.to_lowercase());
            for input in [upper, lower] {
                let result = sanitize_query(&input, 25);
                assert!(result.is_err(), "expected rejection for {input:?}");
            }
        }
    }

    /// Table-driven: structural rejections and injection attempts.
    #[test]
    fn non_read_only_shapes_are_rejected() {
        let cases: Vec<&str> = vec![
            "",
            "   \n\t ",
            // Multi-statement injection: second statement carries a keyword.
            "MATCH (n) RETURN n; DELETE (n)",
            "match (n) return n; drop (n)",
            // F04 acceptance: EXPORT DATABASE as a second statement.
            "RETURN 1 LIMIT 1; EXPORT DATABASE '/tmp/pwn'",
            // Second statement without a reserved word — shape alone rejects.
            "RETURN 1; RETURN 2",
            // Right start word missing.
            "CREATE (n)",
            "WITH x RETURN x",
            "explain MATCH (n) RETURN n",
            "OPTIONAL MATCH (n) RETURN n",
            "CALL { MATCH (n) RETURN n } RETURN n",
            // No word at all.
            ";;;",
            "()--()",
            // F04: clause keywords outside their slot / wrong arity.
            "MATCH (n) RETURN n LIMIT",
            "MATCH (n) RETURN n LIMIT 'x'",
            "MATCH (n) RETURN n LIMIT 5 LIMIT 6",
            "MATCH (n) RETURN n SKIP x",
            "MATCH (n) RETURN n ORDER n",
            "MATCH (n) RETURN",
            "MATCH RETURN n",
            "MATCH (n) MATCH (m) RETURN n",
            "MATCH (n) WHERE RETURN n",
            // F04: smuggling attempts.
            "MATCH (n) RETURN n UNION MATCH (m) RETURN m",
            "MATCH (n) RETURN n ATTACH '/tmp/x' AS y",
            "MATCH (n) RETURN n; ATTACH '/tmp/x' AS y RETURN n",
            "LOAD CSV FROM 'x' AS row RETURN row",
            "MATCH (n) RETURN n CHECKPOINT",
            // F04: unsupported characters, params and unterminated literals.
            "MATCH (n) RETURN n SET n.x = $param",
            "MATCH (n) RETURN `unterminated",
            "MATCH (n) RETURN 'unterminated",
            "MATCH (n) RETURN n /* unterminated",
            "MATCH (n) RETURN n \\ DELETE (n)",
            // Homoglyph: Cyrillic Т inside RETURN is an unknown word, so
            // no RETURN clause exists and the structural parse rejects.
            "MATCH (n) REТURN n",
        ];
        for input in cases {
            let result = sanitize_query(input, 25);
            assert!(result.is_err(), "expected rejection for {input:?}");
        }
    }

    /// The `STARTS WITH` / `ENDS WITH` operators keep `WITH` as data
    /// while a standalone `WITH` clause stays forbidden.
    #[test]
    fn starts_with_operator_survives_while_with_clause_does_not() {
        assert!(
            sanitize_query(
                "MATCH (n) WHERE n.name STARTS WITH 'a' RETURN n LIMIT 5",
                25
            )
            .is_ok()
        );
        assert!(
            sanitize_query("MATCH (n) WHERE n.name ENDS WITH 'z' RETURN n LIMIT 5", 25).is_ok()
        );
        assert!(sanitize_query("WITH 1 AS x RETURN x", 25).is_err());
        assert!(sanitize_query("MATCH (n) WITH n RETURN n", 25).is_err());
    }

    /// A dot-qualified identifier may share a clause keyword's spelling
    /// without turning into a clause start.
    #[test]
    fn dotted_property_names_are_never_clauses() {
        let guarded =
            sanitize_query("MATCH (n) RETURN n.limit, n.skip, n.order, n.where", 25).expect("ok");
        assert_eq!(
            guarded,
            "MATCH (n) RETURN n.limit, n.skip, n.order, n.where LIMIT 25"
        );
    }

    /// F05 (adversarial review): the statement itself is bounded, and
    /// the engine-side list/string amplifiers are rejected before kuzu
    /// ever materializes their result.
    #[test]
    fn statement_size_budget_and_amplifier_functions_are_rejected() {
        // Oversized statements reject on the byte cap — before lexing,
        // so a megabyte of parser input never reaches kuzu's compiler.
        let huge = format!(
            "MATCH (n) WHERE n.name = '{}' RETURN n",
            "a".repeat(100_000)
        );
        let err = sanitize_query(&huge, 25).expect_err("oversized statements must reject");
        assert!(err.to_string().contains("exceeds"), "{err}");
        // A large but budget-compliant statement still passes.
        let big = format!("MATCH (n) WHERE n.name = '{}' RETURN n", "a".repeat(60_000));
        assert!(sanitize_query(&big, 25).is_ok(), "60 KiB < 64 KiB cap");

        // Amplifier calls reject, case-insensitively, with the reason.
        for call in [
            "RETURN range(0, 1000000) AS r",
            "RETURN RaNgE(0, 5) AS r",
            "MATCH (n) RETURN unnest(n.tags)",
            "RETURN str_split('a,b', ',') AS parts",
            "RETURN repeat('x', 1000000) AS pad",
            "MATCH (n) RETURN pad_left(n.name, 1000000000)",
            "MATCH (n) RETURN pad_right(n.name, 1000000000)",
        ] {
            let err = sanitize_query(call, 25).expect_err("{call} must reject");
            let msg = err.to_string();
            assert!(msg.contains("not allowed"), "{call}: {msg}");
            assert!(msg.contains("materialization"), "{call}: {msg}");
        }

        // Not calls: the same words as property names or plain data.
        assert!(sanitize_query("MATCH (n) RETURN n.range, n.repeat, n.unnest", 25).is_ok());
        assert!(sanitize_query("MATCH (n) RETURN m.range(1)", 25).is_ok());
        assert!(
            sanitize_query("MATCH (n) WHERE n.name = 'range(0, 10)' RETURN n", 25).is_ok(),
            "quoted amplifier text is opaque data"
        );
        // Ordinary functions are untouched.
        assert!(sanitize_query("MATCH (n) RETURN count(*) AS c", 25).is_ok());
    }
}
