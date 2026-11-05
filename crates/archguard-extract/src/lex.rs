//! Lexical shielding for lightweight import scanners. This is not a compiler parser.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flavor {
    C,
    /// C-like, plus nested block comments.
    Kotlin,
    Rust,
    Js,
    Python,
    Php,
}
#[derive(Debug)]
pub(crate) struct Token {
    pub text: String,
    pub literal: bool,
    pub line: u32,
    pub end_line: u32,
    pub start: usize,
    pub end: usize,
}
pub(crate) fn tokens(source: &str, flavor: Flavor) -> Vec<Token> {
    let mut lexer = Lexer {
        source,
        chars: source.char_indices().collect(),
        flavor,
        i: 0,
        line: 1,
        out: Vec::new(),
        control_parens: Vec::new(),
        after_control: false,
        php: flavor != Flavor::Php || !source.contains("<?"),
    };
    while lexer.i < lexer.chars.len() {
        lexer.step();
    }
    lexer.out
}

/// Cursor over `source`; `i` indexes `chars`, `line` is the 1-based line at `i`.
struct Lexer<'a> {
    source: &'a str,
    chars: Vec<(usize, char)>,
    flavor: Flavor,
    i: usize,
    line: u32,
    out: Vec<Token>,
    /// One entry per open JS paren: whether it follows a control keyword.
    control_parens: Vec<bool>,
    after_control: bool,
    /// Inside PHP code (as opposed to inline HTML).
    php: bool,
}

impl Lexer<'_> {
    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.i + offset).map(|p| p.1)
    }

    /// Advance one char, counting a newline.
    fn bump(&mut self) {
        if self.peek(0) == Some('\n') {
            self.line += 1;
        }
        self.i += 1;
    }

    fn last_code_token(&self) -> Option<&Token> {
        self.out.iter().rev().find(|t| t.text != "\n")
    }

    fn step(&mut self) {
        if self.skip_php_markup() || self.skip_layout() || self.skip_comment() {
            return;
        }
        let start = self.i;
        let start_line = self.line;
        let literal = self.scan_token();
        self.emit(start, start_line, literal);
    }

    /// Skip PHP open/close tags and everything outside them.
    fn skip_php_markup(&mut self) -> bool {
        if self.flavor != Flavor::Php {
            return false;
        }
        let tail = &self.source[self.chars[self.i].0..];
        let (width, php) = if tail.starts_with("<?php") {
            (5, true)
        } else if tail.starts_with("<?") {
            (2, true)
        } else if tail.starts_with("?>") {
            (2, false)
        } else if !self.php {
            self.bump();
            return true;
        } else {
            return false;
        };
        self.php = php;
        self.i += width;
        true
    }

    /// Emit newline tokens and skip other whitespace.
    fn skip_layout(&mut self) -> bool {
        let (at, c) = self.chars[self.i];
        if c == '\n' {
            self.out.push(Token {
                text: "\n".into(),
                literal: false,
                line: self.line,
                end_line: self.line,
                start: at,
                end: at + 1,
            });
            self.bump();
            return true;
        }
        if c.is_whitespace() {
            self.i += 1;
            return true;
        }
        false
    }

    fn skip_comment(&mut self) -> bool {
        let (c, next) = (self.chars[self.i].1, self.peek(1));
        let hash =
            matches!(self.flavor, Flavor::Python | Flavor::Php) && c == '#' && next != Some('[');
        let slash = self.flavor != Flavor::Python && c == '/';
        if hash || (slash && next == Some('/')) {
            while self.i < self.chars.len() && self.peek(0) != Some('\n') {
                self.i += 1;
            }
            return true;
        }
        if slash && next == Some('*') {
            self.block_comment();
            return true;
        }
        false
    }

    /// Skip `/* ... */`; Rust and Kotlin block comments nest.
    fn block_comment(&mut self) {
        let nests = matches!(self.flavor, Flavor::Rust | Flavor::Kotlin);
        self.i += 2;
        let mut depth = 1usize;
        while self.i < self.chars.len() {
            if nests && self.peek(0) == Some('/') && self.peek(1) == Some('*') {
                depth += 1;
                self.i += 2;
                continue;
            }
            if self.peek(0) == Some('*') && self.peek(1) == Some('/') {
                self.i += 2;
                depth -= 1;
                if depth == 0 {
                    break;
                }
                continue;
            }
            self.bump();
        }
    }

    /// Consume one token and return whether it is a literal.
    fn scan_token(&mut self) -> bool {
        if let Some((quote, hashes)) = self.raw_string_start() {
            self.raw_string(quote, hashes);
        } else if self.flavor == Flavor::Php
            && self.peek(0) == Some('<')
            && self.peek(1) == Some('<')
            && self.peek(2) == Some('<')
        {
            self.heredoc();
        } else if self.at_quote() {
            self.quoted();
        } else if self.at_regex() {
            self.regex();
        } else {
            self.word_or_punct();
            return false;
        }
        true
    }

    /// Rust `r"`, `r#"`, `br"`, `cr"`: the quote's index and the number of hashes.
    fn raw_string_start(&self) -> Option<(usize, usize)> {
        if self.flavor != Flavor::Rust {
            return None;
        }
        let mut q = usize::from(matches!(self.peek(0), Some('b' | 'c')));
        if self.peek(q) != Some('r') {
            return None;
        }
        q += 1;
        let hash_start = q;
        while self.peek(q) == Some('#') {
            q += 1;
        }
        (self.peek(q) == Some('"')).then_some((self.i + q, q - hash_start))
    }

    fn raw_string(&mut self, quote: usize, hashes: usize) {
        self.i = quote + 1;
        while self.i < self.chars.len() {
            if self.peek(0) == Some('"') && (0..hashes).all(|n| self.peek(1 + n) == Some('#')) {
                self.i += 1 + hashes;
                break;
            }
            self.bump();
        }
    }

    /// PHP heredoc or nowdoc up to and including its closing label line.
    fn heredoc(&mut self) {
        let end = self.chars[self.i].0 + 3 + heredoc_len(&self.source[self.chars[self.i].0 + 3..]);
        while self.i < self.chars.len() && self.chars[self.i].0 < end {
            self.bump();
        }
    }

    /// A string quote; a Rust `'a` lifetime is not one.
    fn at_quote(&self) -> bool {
        let c = self.chars[self.i].1;
        matches!(c, '\'' | '"' | '`')
            && !(self.flavor == Flavor::Rust
                && c == '\''
                && self.peek(1).is_some_and(|c| c.is_alphabetic() || c == '_')
                && self.peek(2) != Some('\''))
    }

    /// String with single or triple quotes; C and Kotlin also accept longer quote runs.
    fn quoted(&mut self) {
        let c = self.chars[self.i].1;
        if self.flavor == Flavor::Kotlin && c == '"' {
            kotlin_string(self);
            return;
        }
        let run = self.chars[self.i..].iter().take_while(|p| p.1 == c).count();
        let width = match (c != '`' && run >= 3, self.flavor) {
            (true, Flavor::C | Flavor::Kotlin) => run,
            (true, _) => 3,
            (false, _) => 1,
        };
        self.i += width;
        while self.i < self.chars.len() {
            if self.peek(0) == Some('\\') {
                self.i += 1;
                if self.i < self.chars.len() {
                    self.bump();
                }
                continue;
            }
            if self.chars[self.i..]
                .iter()
                .take(width)
                .filter(|p| p.1 == c)
                .count()
                == width
            {
                self.i += width;
                break;
            }
            self.bump();
        }
    }

    /// A JS `/` starts a regex after a control condition or an operator-like token.
    fn at_regex(&self) -> bool {
        self.flavor == Flavor::Js
            && self.peek(0) == Some('/')
            && (self.after_control
                || self.last_code_token().is_none_or(|t| {
                    matches!(
                        t.text.as_str(),
                        "=" | "(" | "[" | ":" | "," | ";" | "!" | "return" | ">" | "{"
                    )
                }))
    }

    /// JS regex literal up to its closing `/` outside a character class, or the line end.
    fn regex(&mut self) {
        self.i += 1;
        let mut class = false;
        while self.i < self.chars.len() && self.peek(0) != Some('\n') {
            match self.peek(0) {
                Some('\\') => {
                    self.i = (self.i + 2).min(self.chars.len());
                    continue;
                }
                Some('[') => class = true,
                Some(']') => class = false,
                Some('/') if !class => {
                    self.i += 1;
                    break;
                }
                _ => {}
            }
            self.i += 1;
        }
    }

    /// Identifier or number run, otherwise a single punctuation char.
    fn word_or_punct(&mut self) {
        let word = |c: char| c.is_alphanumeric() || matches!(c, '_' | '$');
        self.i += 1;
        if !word(self.chars[self.i - 1].1) {
            return;
        }
        while self.i < self.chars.len() && word(self.chars[self.i].1) {
            self.i += 1;
        }
    }

    fn emit(&mut self, start: usize, start_line: u32, literal: bool) {
        let source = self.source;
        let start = self.chars[start].0;
        let end = self.chars.get(self.i).map_or(source.len(), |p| p.0);
        let text = &source[start..end];
        if self.flavor == Flavor::Js {
            self.track_control(text, literal);
        }
        self.out.push(Token {
            text: text.into(),
            start,
            end,
            literal,
            line: start_line,
            end_line: self.line,
        });
        if self.flavor == Flavor::Php && text.starts_with("<<<") && text.trim_end().ends_with(';') {
            let line = self.line.saturating_sub(1);
            self.out.push(Token {
                text: ";".into(),
                literal: false,
                line,
                end_line: line,
                start: end,
                end,
            });
        }
    }

    /// Remember whether a JS `)` closes an `if (...)`-style condition, after which `/` is a regex.
    fn track_control(&mut self, text: &str, literal: bool) {
        if !literal && text == "(" {
            let control = self.last_code_token().is_some_and(|t| {
                matches!(
                    t.text.as_str(),
                    "if" | "while" | "for" | "with" | "catch" | "switch"
                )
            });
            self.control_parens.push(control);
            self.after_control = false;
        } else if !literal && text == ")" {
            self.after_control = self.control_parens.pop().unwrap_or(false);
        } else {
            self.after_control = false;
        }
    }
}

/// Kotlin `"…"` or raw `"""…"""`: a raw string has no escapes and ends at
/// the last quote of a run of three or more; `${…}` templates, with any
/// strings nested in them, stay part of the literal.
fn kotlin_string(lexer: &mut Lexer) {
    let quotes = |lexer: &Lexer| {
        lexer.chars[lexer.i..]
            .iter()
            .take_while(|p| p.1 == '"')
            .count()
    };
    let raw = quotes(lexer) >= 3;
    lexer.i += if raw { 3 } else { 1 };
    while lexer.i < lexer.chars.len() {
        match lexer.peek(0) {
            Some('\\') if !raw => {
                lexer.i += 1;
                if lexer.i < lexer.chars.len() {
                    lexer.bump();
                }
            }
            Some('"') if !raw => {
                lexer.i += 1;
                return;
            }
            Some('"') if quotes(lexer) >= 3 => {
                lexer.i += quotes(lexer);
                return;
            }
            Some('$') if lexer.peek(1) == Some('{') => kotlin_template(lexer),
            _ => lexer.bump(),
        }
    }
}

/// Skip a `${…}` template up to its matching `}`.
fn kotlin_template(lexer: &mut Lexer) {
    lexer.i += 2;
    let mut depth = 1usize;
    while lexer.i < lexer.chars.len() {
        match lexer.peek(0) {
            Some('"') => {
                kotlin_string(lexer);
                continue;
            }
            Some('\'') => {
                lexer.quoted();
                continue;
            }
            Some('{') => depth += 1,
            Some('}') => {
                depth -= 1;
                if depth == 0 {
                    lexer.i += 1;
                    return;
                }
            }
            _ => {}
        }
        lexer.bump();
    }
}

/// Byte length of a heredoc body after `<<<`, through the line holding the closing label.
fn heredoc_len(tail: &str) -> usize {
    let label = tail
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches(['\'', '"']);
    let mut consumed = 0;
    for (index, part) in tail.split_inclusive('\n').enumerate() {
        consumed += part.len();
        if index > 0
            && part
                .trim_start()
                .strip_prefix(label)
                .is_some_and(|s| s.trim().is_empty() || s.trim_start().starts_with([';', ',', ')']))
        {
            break;
        }
    }
    consumed
}

pub(crate) struct Statement {
    pub code: String,
    pub line: u32,
    pub raw: String,
    pub exact: String,
}
/// Split statement boundaries outside literals and Python continuations. Keep PHP group-use braces.
pub(crate) fn statements(source: &str, flavor: Flavor) -> Vec<Statement> {
    let ts = tokens(source, flavor);
    let mut splitter = Splitter {
        flavor,
        source,
        lines: source.lines().collect(),
        out: Vec::new(),
        current: Vec::new(),
        depth: 0,
        group: 0,
    };
    for token in &ts {
        splitter.push(token);
    }
    if matches!(flavor, Flavor::Python | Flavor::Php) {
        splitter.flush();
    }
    splitter.out
}

/// Statement being collected, plus paren depth and open PHP group-use braces.
struct Splitter<'s, 't> {
    flavor: Flavor,
    source: &'s str,
    lines: Vec<&'s str>,
    out: Vec<Statement>,
    current: Vec<&'t Token>,
    depth: usize,
    group: usize,
}

impl<'t> Splitter<'_, 't> {
    fn push(&mut self, token: &'t Token) {
        if self.starts_nested_namespace(token) {
            self.current.clear();
        }
        if token.text == "\n" {
            self.newline();
            return;
        }
        if !token.literal && self.closes_group(token) {
            return;
        }
        self.current.push(token);
        if self.ends_statement(token) {
            self.flush();
        }
    }

    /// `using`/`namespace` on a later line than a pending `namespace` starts a new statement.
    fn starts_nested_namespace(&self, token: &Token) -> bool {
        !token.literal
            && matches!(token.text.as_str(), "using" | "namespace")
            && self
                .current
                .first()
                .is_some_and(|t| t.text == "namespace" && t.line < token.line)
            && self.depth == 0
    }

    /// Python ends a statement at a newline outside brackets unless a `\` continues it.
    fn newline(&mut self) {
        if self.flavor != Flavor::Python {
            return;
        }
        if self.current.last().is_some_and(|t| t.text == "\\") {
            self.current.pop();
        } else if self.depth == 0 {
            self.flush();
        }
    }

    /// Track brackets and PHP group-use braces; true when `token` closed a group and was taken.
    fn closes_group(&mut self, token: &'t Token) -> bool {
        match token.text.as_str() {
            "(" | "[" => self.depth += 1,
            ")" | "]" => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        if self.flavor != Flavor::Php {
            return false;
        }
        if token.text == "{" && self.current.first().is_some_and(|t| t.text == "use") {
            self.group += 1;
        } else if token.text == "}" && self.group > 0 {
            self.group -= 1;
            self.current.push(token);
            return true;
        }
        false
    }

    fn ends_statement(&self, token: &Token) -> bool {
        !token.literal
            && self.depth == 0
            && self.group == 0
            && (token.text == ";"
                || (self.flavor != Flavor::Python && matches!(token.text.as_str(), "{" | "}")))
    }

    fn flush(&mut self) {
        flush(&mut self.current, &self.lines, self.source, &mut self.out);
    }
}
fn flush(current: &mut Vec<&Token>, lines: &[&str], source: &str, out: &mut Vec<Statement>) {
    if current.is_empty() {
        return;
    }
    let start = current[0].line;
    let end = current.last().unwrap().end_line;
    let mut code = String::new();
    let mut previous = "";
    for token in current.iter() {
        let text = token.text.as_str();
        let adjacent = matches!(
            text,
            "." | "\\" | ":" | ";" | "," | "(" | ")" | "{" | "}" | "[" | "]"
        ) || matches!(previous, "." | "\\" | ":" | "(" | "[" | "@");
        if !code.is_empty()
            && (!adjacent
                || matches!(
                    previous,
                    "from"
                        | "import"
                        | "use"
                        | "require"
                        | "require_once"
                        | "include"
                        | "include_once"
                )
                || text == "import")
        {
            code.push(' ');
        }
        code.push_str(text);
        previous = text;
    }
    out.push(Statement {
        exact: source[current[0].start..current.last().unwrap().end].to_owned(),
        code,
        line: start,
        raw: lines[(start - 1) as usize..(end as usize).min(lines.len())].join("\n"),
    });
    current.clear();
}

/// Dotted identifier chains `a.b.C` in code with their line, outside literals,
/// comments and statements opened by one of `skip` (`import`, `using`, …;
/// a two-word entry such as `global using` matches the first two tokens).
/// Segments must touch their dots, so builder calls split across lines stay
/// apart; a chain never starts right after a `.`.
pub(crate) fn dotted_chains(
    source: &str,
    flavor: Flavor,
    skip: &[&str],
) -> Vec<(u32, Vec<String>)> {
    let tokens: Vec<_> = tokens(source, flavor)
        .into_iter()
        .filter(|token| token.text != "\n")
        .collect();
    let mut out = Vec::new();
    let mut statement_start = true;
    let mut skip_statement = false;
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if statement_start && !token.literal {
            skip_statement = opens_skipped(&tokens, index, skip);
        }
        statement_start = !token.literal && matches!(token.text.as_str(), ";" | "{" | "}");
        let after_dot = index > 0 && {
            let previous = &tokens[index - 1];
            previous.text == "." && previous.end == token.start
        };
        index += 1;
        if skip_statement || after_dot || !is_name(token) {
            continue;
        }
        let segments = chain_from(&tokens, &mut index);
        if segments.len() > 1 {
            out.push((token.line, segments));
        }
    }
    out
}

/// The statement at `tokens[index]` starts with a `skip` word or two-word pair.
fn opens_skipped(tokens: &[Token], index: usize, skip: &[&str]) -> bool {
    let token = &tokens[index];
    let pair = tokens
        .get(index + 1)
        .map(|next| format!("{} {}", token.text, next.text));
    skip.contains(&token.text.as_str()) || pair.is_some_and(|pair| skip.contains(&pair.as_str()))
}

/// Segments of the chain whose first name is `tokens[*index - 1]`; moves `index` past it.
fn chain_from(tokens: &[Token], index: &mut usize) -> Vec<String> {
    let first = &tokens[*index - 1];
    let mut segments = vec![first.text.clone()];
    let mut end = first.end;
    while let (Some(dot), Some(name)) = (tokens.get(*index), tokens.get(*index + 1)) {
        if !links(dot, name, end) {
            break;
        }
        segments.push(name.text.clone());
        end = name.end;
        *index += 2;
    }
    segments
}

/// `dot` is a `.` touching the chain end at `end` and the following `name`.
fn links(dot: &Token, name: &Token, end: usize) -> bool {
    !dot.literal && dot.text == "." && dot.start == end && name.start == dot.end && is_name(name)
}

fn is_name(token: &Token) -> bool {
    !token.literal
        && token
            .text
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
}

/// Preserve source positions while hiding comments and literal contents from visibility scans.
pub(crate) fn code_only(source: &str, flavor: Flavor) -> String {
    let mut bytes: Vec<_> = source
        .bytes()
        .map(|b| if b == b'\n' || b == b'\r' { b } else { b' ' })
        .collect();
    for token in tokens(source, flavor).iter().filter(|t| !t.literal) {
        bytes[token.start..token.end].copy_from_slice(&source.as_bytes()[token.start..token.end]);
    }
    String::from_utf8(bytes).expect("tokens retain complete UTF-8 codepoints")
}
