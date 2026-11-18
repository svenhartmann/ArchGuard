//! `package` and `import` clauses of a Scala source: the graph targets for
//! the extractor and the bound names for the call resolver.

use super::symbols::ScalaImport;
use crate::extract::TargetSpec;
use crate::lex::Flavor;

/// A `package` or `import` clause: its code words and the lines it spans.
pub(super) struct Clause {
    pub(super) words: Vec<String>,
    pub(super) lines: std::ops::RangeInclusive<u32>,
}

/// The `package` and `import` clauses of a source. A clause starts at the
/// beginning of a statement and ends at a line break, `;` or an unmatched
/// `}` outside its own braces; a nested `package a {` ends at its `{`.
pub(super) fn clauses(content: &str) -> Vec<Clause> {
    let tokens = crate::lex::tokens(content, Flavor::Kotlin);
    let mut out = Vec::new();
    let mut start = true;
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        let opens = start && !token.literal && matches!(token.text.as_str(), "package" | "import");
        start = !token.literal && matches!(token.text.as_str(), "\n" | ";" | "{" | "}");
        i += 1;
        if opens {
            let clause;
            (clause, i) = clause_at(&tokens, i - 1);
            out.push(clause);
        }
    }
    out
}

/// The clause opened by the keyword at `at`, and the index after it.
fn clause_at(tokens: &[crate::lex::Token], at: usize) -> (Clause, usize) {
    let mut clause = Clause {
        words: vec![tokens[at].text.clone()],
        lines: tokens[at].line..=tokens[at].line,
    };
    let mut depth = 0usize;
    let mut i = at + 1;
    while let Some(next) = tokens.get(i).filter(|t| !t.literal) {
        match next.text.as_str() {
            "{" if clause.words[0] == "package" => break,
            "}" | "\n" | ";" if depth == 0 => break,
            "{" => depth += 1,
            "}" => depth -= 1,
            _ => {}
        }
        if next.text != "\n" {
            clause.words.push(next.text.clone());
            clause.lines = *clause.lines.start()..=next.line;
        }
        i += 1;
    }
    (clause, i)
}

/// The leading `a.b.c` names of clause words.
pub(super) fn dotted(words: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for (index, word) in words.iter().enumerate() {
        match (index % 2, *word) {
            (0, name) if is_identifier(name) => out.push(name.trim_matches('`').to_string()),
            (1, ".") => {}
            _ => break,
        }
    }
    out
}

/// One imported name: its package path, and the imported name with the
/// name it binds; `None` for a wildcard.
type Imported = (Vec<String>, Option<(String, String)>);

/// What one `import` clause imports (`import a.B, c.D` imports twice).
fn imported(words: &[&str]) -> Vec<Imported> {
    split(words, ",")
        .into_iter()
        .flat_map(|expr| {
            let prefix = dotted(expr);
            let rest = &expr[(prefix.len() * 2).min(expr.len())..];
            let selectors: Vec<_> = match rest {
                // `a.b.{C, D => E}`: the dot before the brace is consumed.
                ["{", inner @ .., "}"] => {
                    split(inner, ",").into_iter().filter_map(selector).collect()
                }
                [] => match prefix.last().map(String::as_str) {
                    Some("_" | "given") => vec![None],
                    Some(name) => vec![Some((name.to_string(), name.to_string()))],
                    None => vec![],
                },
                ["*"] => vec![None],
                _ => vec![],
            };
            let path = match rest {
                [] => prefix[..prefix.len().saturating_sub(1)].to_vec(),
                _ => prefix,
            };
            selectors
                .into_iter()
                .filter(|_| !path.is_empty())
                .map(|selector| (path.clone(), selector))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// One selector: `C`, `C => D`, `C as D` import `C` (bound as `D`); `_`,
/// `*` and `given …` are wildcards (`Some(None)`); `C => _` hides `C` (`None`).
fn selector(words: &[&str]) -> Option<Option<(String, String)>> {
    let name = |word: &str| word.trim_matches('`').to_string();
    match words {
        ["_" | "*"] | ["given", ..] => Some(None),
        [_, "=", ">", "_"] => None,
        [from, "=", ">", to] | [from, "as", to] if is_identifier(from) => {
            Some(Some((name(from), name(to))))
        }
        [from, ..] if is_identifier(from) => Some(Some((name(from), name(from)))),
        _ => None,
    }
}

/// The graph targets of one `import` clause; none for the platform
/// packages `scala`, `java` and `javax`.
pub(super) fn import_targets(words: &[&str]) -> Vec<TargetSpec> {
    imported(words)
        .into_iter()
        .filter(|(path, _)| !matches!(path[0].as_str(), "scala" | "java" | "javax"))
        .map(|(path, selector)| TargetSpec::Symbol {
            path: path.join("/"),
            stem: selector.map(|(name, _)| name),
        })
        .collect()
}

/// The names the `import` clauses of a source bind, for the call resolver.
pub(super) fn bindings(content: &str) -> Vec<ScalaImport> {
    clauses(content)
        .iter()
        .filter(|clause| clause.words[0] == "import")
        .flat_map(|clause| {
            let words: Vec<&str> = clause.words[1..].iter().map(String::as_str).collect();
            imported(&words)
        })
        .map(|(path, selector)| match selector {
            Some((name, alias)) => ScalaImport {
                path: format!("{}.{name}", path.join(".")),
                name: Some(alias),
            },
            None => ScalaImport {
                path: path.join("."),
                name: None,
            },
        })
        .collect()
}

/// `words` split at `sep` outside braces and brackets.
fn split<'a, 'b>(words: &'b [&'a str], sep: &str) -> Vec<&'b [&'a str]> {
    let mut out = Vec::new();
    let (mut depth, mut from) = (0usize, 0);
    for (index, word) in words.iter().enumerate() {
        match *word {
            "{" | "[" | "(" => depth += 1,
            "}" | "]" | ")" => depth = depth.saturating_sub(1),
            w if w == sep && depth == 0 => {
                out.push(&words[from..index]);
                from = index + 1;
            }
            _ => {}
        }
    }
    out.push(&words[from..]);
    out.retain(|part| !part.is_empty());
    out
}

// Own copy, not crate::ident: Scala allows backquoted names and `$` in identifiers.
fn is_identifier(word: &str) -> bool {
    word.starts_with('`')
        || (word.starts_with(|c: char| c.is_alphabetic() || c == '_')
            && word
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$'))
}
