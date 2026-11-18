//! Ruby lexing into the shared [`Token`] model. Comments (`#`, `=begin`…
//! `=end`, everything after `__END__`), strings with `#{…}` interpolation,
//! `%w[…]`/`%i[…]`/`%q(…)` and the other percent literals, regex literals,
//! symbols and heredoc bodies never reach the code tokens. Heredoc bodies
//! produce no tokens at all, so the rest of the line that opened them stays
//! ordinary code. An identifier keeps a trailing `?` or `!` (`empty?`), and
//! a label keeps its colon (`if:` in `validates :x, if: :y` is no keyword).
//! Multi-character operators (`::`, `&&`, `&.`, `<=>`, …) are one token.
/// One token, mapped to the shared `Token` by the parent module.
pub(super) struct Lexeme {
    pub text: String,
    pub literal: bool,
    pub line: u32,
    pub end_line: u32,
    pub start: usize,
    pub end: usize,
}

/// Operators of more than one character, longest first.
const OPERATORS: [&str; 33] = [
    "**=", "<=>", "===", "...", "<<=", ">>=", "&&=", "||=", "**", "==", "!=", "=~", "!~", ">=",
    "<=", "&&", "||", "<<", ">>", "+=", "-=", "*=", "/=", "%=", "|=", "&=", "^=", "::", "..", "->",
    "=>", "&.", "@@",
];

/// Words after which an operand, not an operator, follows (`return /x/`).
const EXPRESSION_KEYWORDS: [&str; 14] = [
    "if", "unless", "while", "until", "when", "and", "or", "not", "return", "then", "else",
    "elsif", "in", "case",
];

pub(super) fn tokens(source: &str) -> Vec<Lexeme> {
    let mut lexer = Lexer {
        source,
        bytes: source.as_bytes(),
        i: 0,
        line: 1,
        out: Vec::new(),
        heredocs: Vec::new(),
    };
    lexer.run();
    lexer.out
}

/// A heredoc whose body starts on the next line: its label and whether the
/// label may be indented (`<<~`, `<<-`).
struct Heredoc {
    label: String,
    indented: bool,
}

struct Lexer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    i: usize,
    line: u32,
    out: Vec<Lexeme>,
    heredocs: Vec<Heredoc>,
}

/// Skip the bodies of the heredocs opened on the line just ended.
fn heredoc_bodies(lexer: &mut Lexer) {
    for heredoc in std::mem::take(&mut lexer.heredocs) {
        while lexer.i < lexer.bytes.len() {
            let end = lexer.source[lexer.i..]
                .find('\n')
                .map_or(lexer.bytes.len(), |n| lexer.i + n);
            let text = lexer.source[lexer.i..end].trim_end_matches('\r');
            let text = if heredoc.indented { text.trim() } else { text };
            let closes = text == heredoc.label;
            lexer.i = (end + 1).min(lexer.bytes.len());
            lexer.line += u32::from(end < lexer.bytes.len());
            if closes {
                break;
            }
        }
    }
}

/// The closing delimiter of a percent literal.
fn closing(open: u8) -> u8 {
    match open {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        b'<' => b'>',
        other => other,
    }
}

/// A name, number, `@ivar` or `$global` starts here.
fn starts_word(byte: u8) -> bool {
    is_word(byte) || byte == b'@' || byte == b'$'
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

impl Lexer<'_> {
    fn at(&self, offset: usize) -> u8 {
        self.bytes.get(self.i + offset).copied().unwrap_or(0)
    }

    fn run(&mut self) {
        self.line_start();
        while self.i < self.bytes.len() {
            let c = self.at(0);
            if c == b'\n' {
                self.push(self.i, self.i + 1, self.line, false);
                self.i += 1;
                self.line += 1;
                heredoc_bodies(self);
                self.line_start();
            } else if c == b'\\' && self.at(1) == b'\n' {
                // A continued line is one statement.
                self.i += 2;
                self.line += 1;
            } else if c.is_ascii_whitespace() {
                self.i += 1;
            } else if c == b'#' {
                self.skip_line();
            } else {
                let (start, line) = (self.i, self.line);
                let literal = self.scan();
                self.push(start, self.i, line, literal);
            }
        }
    }

    /// `=begin` … `=end` block comments and the `__END__` data section, both
    /// only at the start of a line.
    fn line_start(&mut self) {
        while self.source[self.i..].starts_with("=begin")
            && !self.source[self.i..][6..].starts_with(|c: char| is_word(c as u8))
        {
            loop {
                self.skip_line();
                if self.i >= self.bytes.len() {
                    return;
                }
                self.i += 1;
                self.line += 1;
                if self.source[self.i..].starts_with("=end") {
                    self.skip_line();
                    break;
                }
            }
        }
        let first = self.source[self.i..].lines().next().unwrap_or("");
        if first.trim_end() == "__END__" {
            self.i = self.bytes.len();
        }
    }

    fn skip_line(&mut self) {
        while self.i < self.bytes.len() && self.at(0) != b'\n' {
            self.i += 1;
        }
    }

    fn push(&mut self, start: usize, end: usize, line: u32, literal: bool) {
        self.out.push(Lexeme {
            text: self.source[start..end].into(),
            literal,
            line,
            end_line: self.line,
            start,
            end,
        });
    }

    /// The previous code token ends an operand (`x`, `1`, `)`), so `/`, `%`
    /// and `<<` after it are operators.
    fn after_operand(&self) -> bool {
        self.out.last().filter(|t| t.text != "\n").is_some_and(|t| {
            t.literal
                || matches!(t.text.as_str(), ")" | "]" | "}")
                || (t
                    .text
                    .starts_with(|c: char| is_word(c as u8) || c == '@' || c == '$')
                    && !EXPRESSION_KEYWORDS.contains(&t.text.as_str()))
        })
    }

    /// `name /re/` or `puts %w[a]`: an identifier, a space before and none
    /// after reads as a command call with a literal argument.
    fn command_argument(&self) -> bool {
        let spaced_before = self.i > 0 && self.bytes[self.i - 1] == b' ';
        let spaced_after = matches!(self.at(1), b' ' | b'=' | b'\n');
        let after_name = self.out.last().is_some_and(|t| {
            !t.literal
                && t.text
                    .starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        });
        spaced_before && !spaced_after && after_name
    }

    /// Consume one token; true if it is a literal.
    fn scan(&mut self) -> bool {
        let c = self.at(0);
        match c {
            b'\'' | b'"' | b'`' => {
                self.i += 1;
                self.string(c, c, c != b'\'');
                true
            }
            b'%' if self.percent_literal() => true,
            b'/' if self.operand_expected() => {
                self.i += 1;
                self.string(b'/', b'/', true);
                self.flags();
                true
            }
            b'<' if self.heredoc_start() => true,
            b':' if self.symbol() => true,
            b'?' if self.char_literal() => true,
            _ if starts_word(c) => {
                self.word();
                false
            }
            _ => {
                self.operator();
                false
            }
        }
    }

    /// An operator, the longest of [`OPERATORS`] or one character.
    fn operator(&mut self) {
        let width = OPERATORS
            .iter()
            .find(|op| self.source[self.i..].starts_with(**op))
            .map_or(1, |op| op.len());
        self.i += width;
    }

    /// `/`, `%` and `<<` here start a literal, not an operator.
    fn operand_expected(&self) -> bool {
        // After `def` or a dot comes a method name: `def /(other)`, `x.%(y)`.
        let method_name = self
            .out
            .last()
            .is_some_and(|t| matches!(t.text.as_str(), "def" | "." | "&."));
        !method_name && (!self.after_operand() || self.command_argument())
    }

    /// A body up to `close`, nesting `open`/`close` pairs when they differ;
    /// `#{…}` interpolation is skipped when `interpolates`. Starts after the
    /// opening delimiter.
    fn string(&mut self, open: u8, close: u8, interpolates: bool) {
        let mut depth = 1usize;
        while self.i < self.bytes.len() {
            let c = self.at(0);
            if c == b'\\' {
                self.i += 1;
                if self.i == self.bytes.len() {
                    break;
                }
            } else if interpolates && c == b'#' && self.at(1) == b'{' {
                self.interpolation();
                continue;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    self.i += 1;
                    return;
                }
            } else if c == open && open != close {
                depth += 1;
            }
            self.advance();
        }
    }

    /// One byte, counting a newline.
    fn advance(&mut self) {
        if self.at(0) == b'\n' {
            self.line += 1;
        }
        self.i += 1;
    }

    /// `#{ … }` with any strings nested in it.
    fn interpolation(&mut self) {
        self.i += 2;
        let mut depth = 1usize;
        while self.i < self.bytes.len() {
            match self.at(0) {
                q @ (b'\'' | b'"' | b'`') => {
                    self.i += 1;
                    self.string(q, q, q != b'\'');
                    continue;
                }
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.i += 1;
                        return;
                    }
                }
                _ => {}
            }
            self.advance();
        }
    }

    /// Regex flags after the closing delimiter.
    fn flags(&mut self) {
        while self.at(0).is_ascii_lowercase() {
            self.i += 1;
        }
    }

    /// `%w[…]`, `%i(…)`, `%q{…}`, `%r{…}x`, `%(…)` and the other forms.
    fn percent_literal(&mut self) -> bool {
        let kind = self.at(1);
        let lettered = b"qQwWiIrsx".contains(&kind);
        let delimiter = self.at(1 + usize::from(lettered));
        let valid = !delimiter.is_ascii_alphanumeric() && !delimiter.is_ascii_whitespace();
        if delimiter == 0 || !valid || !self.operand_expected() {
            return false;
        }
        self.i += 2 + usize::from(lettered);
        self.string(
            delimiter,
            closing(delimiter),
            !lettered || b"QWIrx".contains(&kind),
        );
        if kind == b'r' {
            self.flags();
        }
        true
    }

    /// `<<~LABEL`, `<<-LABEL`, `<<LABEL` and their quoted forms; the body is
    /// skipped at the end of the line.
    fn heredoc_start(&mut self) -> bool {
        if self.at(1) != b'<' {
            return false;
        }
        let indented = matches!(self.at(2), b'~' | b'-');
        let at = 2 + usize::from(indented);
        let first = self.at(at);
        let quoted = matches!(first, b'\'' | b'"' | b'`');
        let bare = if indented {
            first.is_ascii_alphabetic() || first == b'_'
        } else {
            first.is_ascii_uppercase() && self.operand_expected()
        };
        if !quoted && !bare {
            return false;
        }
        let start = self.i + at + usize::from(quoted);
        let mut end = start;
        while end < self.bytes.len()
            && (is_word(self.bytes[end])
                || (quoted && self.bytes[end] != first && self.bytes[end] != b'\n'))
        {
            end += 1;
        }
        if end == start {
            return false;
        }
        self.heredocs.push(Heredoc {
            label: self.source[start..end].to_string(),
            indented,
        });
        self.i = end + usize::from(quoted);
        true
    }

    /// `:name`, `:name?`, `:@ivar` or `:"quoted"`; not `::` or `a ? b : c`.
    fn symbol(&mut self) -> bool {
        let next = self.at(1);
        let previous = self.i.checked_sub(1).map(|k| self.bytes[k]);
        if previous == Some(b':') || next == b':' {
            return false;
        }
        if next == b'"' || next == b'\'' {
            self.i += 2;
            self.string(next, next, next == b'"');
            return true;
        }
        if !(is_word(next) || next == b'@' || next == b'$') || next.is_ascii_digit() {
            return false;
        }
        self.i += 1;
        self.word();
        if self.at(0) == b'=' && !matches!(self.at(1), b'=' | b'~' | b'>') {
            self.i += 1;
        }
        true
    }

    /// `?a` character literal where an operand is expected.
    fn char_literal(&mut self) -> bool {
        if self.after_operand() || !self.at(1).is_ascii_graphic() || is_word(self.at(2)) {
            return false;
        }
        self.i += 2;
        true
    }

    /// The `@`, `@@` or `$` of a variable; true if a punctuation global
    /// (`$"`, `$'`, `$/`) is complete with its one character.
    fn sigils(&mut self) -> bool {
        let global = self.at(0) == b'$';
        while matches!(self.at(0), b'@' | b'$') {
            self.i += 1;
        }
        let punctuation = global && self.at(0).is_ascii_punctuation() && self.at(0) != b'_';
        self.i += usize::from(punctuation);
        punctuation
    }

    /// An identifier, constant, `@ivar`, `@@cvar`, `$global` or number, with
    /// a trailing `?`/`!` of a method name and the colon of a label.
    fn word(&mut self) {
        let number = self.at(0).is_ascii_digit();
        if self.sigils() {
            return;
        }
        while self.i < self.bytes.len()
            && (is_word(self.at(0))
                || (number && self.at(0) == b'.' && self.at(1).is_ascii_digit()))
        {
            self.i += 1;
        }
        if number {
            return;
        }
        if matches!(self.at(0), b'?' | b'!') && !matches!(self.at(1), b'=' | b':') {
            self.i += 1;
        }
        if self.at(0) == b':' && self.at(1) != b':' && self.out.last().is_none_or(|t| t.text != "?")
        {
            self.i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tokens;

    /// The code tokens (no literals, no newlines).
    fn code(source: &str) -> Vec<String> {
        tokens(source)
            .into_iter()
            .filter(|t| !t.literal && t.text != "\n")
            .map(|t| t.text)
            .collect()
    }

    #[test]
    fn comments_strings_and_interpolation_hide_code() {
        let source = "# require 'hidden'\nx = \"a #{y + \"}\"} b\" # tail\n=begin\nrequire 'x'\n=end\nz = 'it''s'\n";
        assert_eq!(code(source), ["x", "=", "z", "="]);
    }

    #[test]
    fn heredocs_skip_their_body_but_not_the_rest_of_the_line() {
        let source = "run(<<~SQL, <<-'TXT', 1)\n  select end\n  SQL\n  def x\n  TXT\nafter\n";
        assert_eq!(code(source), ["run", "(", ",", ",", "1", ")", "after"]);
        let lines: Vec<u32> = tokens(source)
            .into_iter()
            .filter(|t| t.text == "after")
            .map(|t| t.line)
            .collect();
        assert_eq!(lines, [6]);
        // `class << self` and `a << b` stay operators.
        assert_eq!(code("class << self\nend\n"), ["class", "<<", "self", "end"]);
        assert_eq!(code("a << B\n"), ["a", "<<", "B"]);
    }

    #[test]
    fn percent_literals_regexes_and_symbols_are_literals() {
        let source = "a = %w[if end] + %i(do end) + %q{x {nested} end}\nb = x % 2\nc = /end#{d}/i\ne = f / 2\nsplit /,/\ng = :end\nh = {if: 1}\nk = A::B\n";
        assert_eq!(
            code(source),
            [
                "a", "=", "+", "+", "b", "=", "x", "%", "2", "c", "=", "e", "=", "f", "/", "2",
                "split", "g", "=", "h", "=", "{", "if:", "1", "}", "k", "=", "A", "::", "B",
            ]
        );
    }

    #[test]
    fn method_names_keep_predicate_suffixes_and_operators_merge() {
        assert_eq!(
            code("x.empty? && y.save! || z&.a <=> b != c\n"),
            [
                "x", ".", "empty?", "&&", "y", ".", "save!", "||", "z", "&.", "a", "<=>", "b",
                "!=", "c",
            ]
        );
        assert_eq!(code("t = a ? b : c\n"), ["t", "=", "a", "?", "b", ":", "c"]);
    }

    #[test]
    fn a_backslash_at_the_end_of_an_open_literal_ends_the_input() {
        for source in [
            "class A\n  def f\n    x = \"abc\\",
            "x = :\"a\\",
            "x = %w[a\\",
        ] {
            let last = tokens(source).pop().unwrap();
            assert!(last.literal && last.end == source.len(), "{source:?}");
        }
    }

    #[test]
    fn operator_method_names_and_punctuation_globals_are_no_literals() {
        assert_eq!(
            code("def /(other)\nend\ndef self.%(x)\nend\n"),
            [
                "def", "/", "(", "other", ")", "end", "def", "self", ".", "%", "(", "x", ")", "end"
            ]
        );
        assert_eq!(
            code("puts $\", $', $/, $:\n"),
            ["puts", "$\"", ",", "$'", ",", "$/", ",", "$:"]
        );
    }

    #[test]
    fn data_after_end_marker_is_ignored() {
        assert_eq!(code("a\n__END__\nb c\n"), ["a"]);
    }
}
