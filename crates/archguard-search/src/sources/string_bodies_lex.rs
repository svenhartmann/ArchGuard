//! Token table of one Java, C#, Python or TypeScript file, built in one pass: words, punctuation
//! and string literals with their decoded value, and the matching bracket of each opener. Comments
//! and string state are kept across lines, so no later step scans the file again.
//! Strings: Java text blocks `"""`; C# `@"` (doubled quotes), `"""` raw, `$` interpolated = no value;
//! Python `'''`, `"""`, `r` raw, `f` interpolated = no value; TypeScript backticks (with `${` = no value).
//! A value is `None` when it is not fully literal (interpolation, unterminated, unknown escape).
// shortcut: a TypeScript regex literal holding a quote mis-lexes; upgrade when a corpus shows it.

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Lang {
    Java,
    Cs,
    Py,
    Ts,
}

impl Lang {
    pub(super) fn of(ext: &str) -> Self {
        match ext {
            "java" => Self::Java,
            "cs" => Self::Cs,
            "py" => Self::Py,
            _ => Self::Ts,
        }
    }
}

pub(super) enum Kind<'a> {
    Word(&'a str),
    Punct(u8),
    Str(Option<String>),
}

pub(super) struct Tok<'a> {
    pub(super) kind: Kind<'a>,
    pub(super) line: u32,
}

pub(super) struct Toks<'a> {
    pub(super) t: Vec<Tok<'a>>,
    close: Vec<usize>,
}

/// Longest bracket span a call may have in tokens; a longer one is reported as a gap.
const MAX_SPAN: usize = 2_000;

impl Toks<'_> {
    pub(super) fn word(&self, i: usize) -> &str {
        match self.t.get(i).map(|t| &t.kind) {
            Some(Kind::Word(w)) => w,
            _ => "",
        }
    }

    pub(super) fn is(&self, i: usize, c: u8) -> bool {
        matches!(self.t.get(i).map(|t| &t.kind), Some(Kind::Punct(p)) if *p == c)
    }

    pub(super) fn line(&self, i: usize) -> u32 {
        self.t.get(i).map_or(0, |t| t.line)
    }

    /// The top-level arguments of the bracket at `open` as token ranges; `None` when it is
    /// never closed or spans more than `MAX_SPAN` tokens.
    pub(super) fn args(&self, open: usize) -> Option<Vec<(usize, usize)>> {
        let end = *self.close.get(open).filter(|&&e| e - open <= MAX_SPAN)?;
        let (mut out, mut start, mut i) = (vec![], open + 1, open + 1);
        while i < end {
            match self.t[i].kind {
                Kind::Punct(b'(' | b'[' | b'{') if self.close[i] != usize::MAX => {
                    i = self.close[i];
                }
                Kind::Punct(b',') => {
                    out.push((start, i));
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        if start < end {
            out.push((start, end));
        }
        Some(out)
    }

    /// The index of the matching closer of the bracket at `open`.
    pub(super) fn end(&self, open: usize) -> Option<usize> {
        self.close.get(open).copied().filter(|&e| e != usize::MAX)
    }
}

pub(super) fn lex<'a>(text: &'a str, ext: &str) -> Toks<'a> {
    let mut lx = Lexer {
        text,
        b: text.as_bytes(),
        i: 0,
        line: 1,
        lang: Lang::of(ext),
        out: vec![],
    };
    while lx.i < lx.b.len() {
        lx.step();
    }
    let close = brackets(&lx.out);
    Toks { t: lx.out, close }
}

fn brackets(t: &[Tok]) -> Vec<usize> {
    let (mut close, mut stack) = (vec![usize::MAX; t.len()], vec![]);
    for (i, tok) in t.iter().enumerate() {
        match tok.kind {
            Kind::Punct(b'(' | b'[' | b'{') => stack.push(i),
            Kind::Punct(b')' | b']' | b'}') => {
                if let Some(o) = stack.pop() {
                    close[o] = i;
                }
            }
            _ => {}
        }
    }
    close
}

fn word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// How the body of a string is read.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Escaped,
    /// Python `r''`: no decoding, a backslash still keeps the next character from ending it.
    PyRaw,
    /// C# `@""`: `""` is one quote.
    Verbatim,
    /// C# `"""`: nothing is decoded.
    CsRaw,
}

struct Open {
    quote: u8,
    at: usize,
    mode: Mode,
    interp: bool,
}

struct Lexer<'a> {
    text: &'a str,
    b: &'a [u8],
    i: usize,
    line: u32,
    lang: Lang,
    out: Vec<Tok<'a>>,
}

impl<'a> Lexer<'a> {
    fn push(&mut self, kind: Kind<'a>, line: u32) {
        self.out.push(Tok { kind, line });
    }

    fn step(&mut self) {
        let c = self.b[self.i];
        if c == b'\n' {
            self.line += 1;
            self.i += 1;
        } else if c.is_ascii_whitespace() {
            self.i += 1;
        } else if self.comment() || self.string() {
        } else if word_byte(c) {
            let s = self.i;
            while self.b.get(self.i).is_some_and(|&c| word_byte(c)) {
                self.i += 1;
            }
            self.push(Kind::Word(&self.text[s..self.i]), self.line);
        } else {
            self.push(Kind::Punct(c), self.line);
            self.i += 1;
        }
    }

    fn comment(&mut self) -> bool {
        let (c, n) = (self.b[self.i], self.b.get(self.i + 1).copied());
        let py = self.lang == Lang::Py;
        if (py && c == b'#') || (!py && c == b'/' && n == Some(b'/')) {
            while self.b.get(self.i).is_some_and(|&c| c != b'\n') {
                self.i += 1;
            }
            return true;
        }
        if !py && c == b'/' && n == Some(b'*') {
            self.i += 2;
            while self.i < self.b.len() && !self.b[self.i..].starts_with(b"*/") {
                self.line += u32::from(self.b[self.i] == b'\n');
                self.i += 1;
            }
            self.i = (self.i + 2).min(self.b.len());
            return true;
        }
        false
    }

    /// The string literal that starts here, with the prefix characters of its language.
    fn opener(&self) -> Option<Open> {
        let (mut j, mut mode, mut interp) = (self.i, Mode::Escaped, false);
        match self.lang {
            Lang::Cs => {
                while j - self.i < 2 && matches!(self.b.get(j), Some(b'@' | b'$')) {
                    interp |= self.b[j] == b'$';
                    if self.b[j] == b'@' {
                        mode = Mode::Verbatim;
                    }
                    j += 1;
                }
            }
            Lang::Py => {
                let len = self.b[j..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphabetic())
                    .count();
                let prefix = &self.b[j..j + len];
                if (1..=2).contains(&len) && prefix.iter().all(|c| b"rRbBuUfF".contains(c)) {
                    interp = prefix.iter().any(|c| matches!(c, b'f' | b'F'));
                    if prefix.iter().any(|c| matches!(c, b'r' | b'R')) {
                        mode = Mode::PyRaw;
                    }
                    j += len;
                }
            }
            _ => {}
        }
        let quote = *self.b.get(j)?;
        let ok = matches!(quote, b'"' | b'\'') || (self.lang == Lang::Ts && quote == b'`');
        ok.then_some(Open {
            quote,
            at: j,
            mode,
            interp,
        })
    }

    fn string(&mut self) -> bool {
        let Some(mut o) = self.opener() else {
            return false;
        };
        let run = self.b[o.at..].iter().take_while(|&&c| c == o.quote).count();
        let long = run >= 3
            && match self.lang {
                Lang::Ts => false,
                Lang::Java | Lang::Cs => o.quote == b'"',
                Lang::Py => true,
            };
        let k = match (long, self.lang) {
            (true, Lang::Cs) => {
                o.mode = Mode::CsRaw;
                run
            }
            (true, _) => 3,
            _ => 1,
        };
        let start = o.at + k;
        let (end, closed) = self.find_end(start, &o, k);
        let body = &self.text[start..end];
        let literal = closed && !o.interp && !(o.quote == b'`' && body.contains("${"));
        let value = literal.then(|| decode(body, o.mode)).flatten();
        let line = self.line;
        self.line += body.bytes().filter(|&c| c == b'\n').count() as u32;
        self.push(Kind::Str(value), line);
        self.i = if closed { end + k } else { end };
        true
    }

    /// Where the body that starts at `start` ends, and whether a closing delimiter was found.
    fn find_end(&self, start: usize, o: &Open, k: usize) -> (usize, bool) {
        let b = self.b;
        let escapes = matches!(o.mode, Mode::Escaped | Mode::PyRaw);
        let single = k == 1 && o.quote != b'`' && o.mode != Mode::Verbatim;
        let mut j = start;
        while j < b.len() {
            if b[j] == o.quote && (0..k).all(|d| b.get(j + d) == Some(&o.quote)) {
                if o.mode == Mode::Verbatim && b.get(j + 1) == Some(&o.quote) {
                    j += 2;
                    continue;
                }
                return (j, true);
            }
            if escapes && b[j] == b'\\' {
                j += 2;
                continue;
            }
            if single && b[j] == b'\n' {
                return (j, false);
            }
            j += 1;
        }
        (b.len(), false)
    }
}

fn decode(body: &str, mode: Mode) -> Option<String> {
    match mode {
        Mode::PyRaw | Mode::CsRaw => Some(body.to_owned()),
        Mode::Verbatim => Some(body.replace("\"\"", "\"")),
        Mode::Escaped => unescape(body),
    }
}

/// Backslash escapes JSON bodies use; any other escape makes the value unreadable.
fn unescape(body: &str) -> Option<String> {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next()? {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'b' => '\u{8}',
            'f' => '\u{c}',
            '\n' => continue,
            c @ ('\\' | '"' | '\'' | '`' | '/') => c,
            'u' => {
                let hex: String = chars.by_ref().take(4).collect();
                char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?
            }
            _ => return None,
        });
    }
    Some(out)
}
