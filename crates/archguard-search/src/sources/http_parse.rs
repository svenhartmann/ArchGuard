//! Requests out of `.http` text and out of shell-like text (curl at the start of a command);
//! pure text work, no file access. The rules are listed in `http.rs`.

/// What a request carries as its body.
#[derive(Debug, PartialEq)]
pub(super) enum Body {
    None,
    Inline(String),
    /// `-d @file`: the path as written.
    File(String),
}

#[derive(Debug)]
pub(super) struct Request {
    pub(super) method: String,
    pub(super) url: String,
    /// 1-based line of the request line, or of the first line of a continued command.
    pub(super) line: u32,
    pub(super) body: Body,
}

const METHODS: &[&str] = &["GET", "PUT", "POST", "DELETE", "HEAD", "PATCH", "OPTIONS"];
/// curl options that take a value; the value is never the URL.
const VALUE_FLAGS: &[&str] = &[
    "-H",
    "--header",
    "-u",
    "--user",
    "-o",
    "--output",
    "-w",
    "--write-out",
    "-A",
    "--user-agent",
    "-e",
    "--referer",
    "--cacert",
    "--cert",
    "--key",
    "-m",
    "--max-time",
    "--connect-timeout",
    "--retry",
    "-b",
    "--cookie",
    "-c",
    "--cookie-jar",
    "-K",
    "--config",
];
const DATA_FLAGS: &[&str] = &[
    "-d",
    "--data",
    "--data-raw",
    "--data-binary",
    "--data-ascii",
];

fn line_no(i: usize) -> u32 {
    u32::try_from(i + 1).unwrap_or(u32::MAX)
}

fn request_line(line: &str) -> Option<(String, String)> {
    let mut words = line.split_whitespace();
    let method = words.next().filter(|m| METHODS.contains(m))?;
    Some((method.into(), words.next()?.into()))
}

fn separator(line: &str) -> bool {
    line.trim_start().starts_with("###")
}

/// `.http` text: a request line, header lines, a blank line, the body up to the next `###` or
/// request line. A body that starts right after the request line (`{`, `[`) needs no blank line.
pub(super) fn http_requests(text: &str) -> Vec<Request> {
    let lines: Vec<&str> = text.lines().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < lines.len() {
        let Some((method, url)) = request_line(lines[i]) else {
            i += 1;
            continue;
        };
        let line = line_no(i);
        i += 1;
        let header = |l: &str| {
            let t = l.trim_start();
            !t.is_empty()
                && !t.starts_with(['{', '['])
                && !separator(l)
                && request_line(l).is_none()
        };
        while i < lines.len() && header(lines[i]) {
            i += 1;
        }
        let start = i;
        while i < lines.len() && !separator(lines[i]) && request_line(lines[i]).is_none() {
            i += 1;
        }
        let body = lines[start..i].join("\n");
        let body = body.trim();
        let body = if body.is_empty() {
            Body::None
        } else {
            Body::Inline(body.into())
        };
        out.push(Request {
            method,
            url,
            line,
            body,
        });
    }
    out
}

/// Shell, Dockerfile and Makefile text: every command whose first word is the literal `curl`
/// (after `&&`, `;`, `|`, `RUN`, a leading `@` or `-`). Lines ending in `\` continue.
pub(super) fn shell_requests(text: &str) -> Vec<Request> {
    let mut out = Vec::new();
    let (mut joined, mut start) = (String::new(), 0);
    for (i, raw) in text.lines().enumerate() {
        if joined.is_empty() {
            start = i;
        }
        match raw.trim_end().strip_suffix('\\') {
            Some(head) => {
                joined.push_str(head);
                joined.push(' ');
            }
            None => {
                joined.push_str(raw);
                out.extend(
                    commands(&joined)
                        .iter()
                        .filter_map(|c| curl(c, line_no(start))),
                );
                joined.clear();
            }
        }
    }
    out
}

/// A curl command as a request: `-X`/`-XPUT`, one of the data flags, the first free word is the URL.
fn curl(words: &[String], line: u32) -> Option<Request> {
    let mut it = words.iter().map(String::as_str);
    let first = it.next()?;
    let first = if first == "RUN" { it.next()? } else { first };
    if first.trim_start_matches(['@', '-', '+']) != "curl" {
        return None;
    }
    let (method, url, body) = options(it);
    let method = method.unwrap_or_else(|| if body == Body::None { "GET" } else { "POST" }.into());
    Some(Request {
        method,
        url: url?.into(),
        line,
        body,
    })
}

type Options<'a> = (Option<String>, Option<&'a str>, Body);

fn options<'a>(mut it: impl Iterator<Item = &'a str>) -> Options<'a> {
    let (mut method, mut url, mut body) = (None, None, Body::None);
    while let Some(w) = it.next() {
        match w {
            "-X" | "--request" => method = it.next().map(str::to_uppercase),
            w if DATA_FLAGS.contains(&w) => body = it.next().map_or(Body::None, data),
            w if VALUE_FLAGS.contains(&w) => {
                it.next();
            }
            w if w.starts_with("-X") => method = Some(w[2..].to_uppercase()),
            w if w.starts_with('-') => {}
            w => url = url.or(Some(w)),
        }
    }
    (method, url, body)
}

fn data(arg: &str) -> Body {
    arg.strip_prefix('@')
        .map_or_else(|| Body::Inline(arg.into()), |f| Body::File(f.into()))
}

/// Words of one logical line, split into commands at `&&`, `||`, `;` and `|`. Quotes group, a
/// backslash escapes, `#` at the start of a word ends the line. Nothing is expanded.
fn commands(line: &str) -> Vec<Vec<String>> {
    let mut lx = Lexer::default();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        let go_on = match lx.quote {
            Some(q) => lx.quoted(q, c, &mut chars),
            None => lx.plain(c, &mut chars),
        };
        if !go_on {
            break;
        }
    }
    lx.flush();
    lx.cmds.push(lx.cmd);
    lx.cmds
}

#[derive(Default)]
struct Lexer {
    cmds: Vec<Vec<String>>,
    cmd: Vec<String>,
    word: String,
    /// A word has begun, so an empty quoted string still counts.
    active: bool,
    quote: Option<char>,
}

impl Lexer {
    fn flush(&mut self) {
        if std::mem::take(&mut self.active) {
            self.cmd.push(std::mem::take(&mut self.word));
        }
    }

    fn quoted(&mut self, q: char, c: char, chars: &mut std::str::Chars) -> bool {
        match c {
            _ if c == q => self.quote = None,
            '\\' if q == '"' => match chars.next() {
                Some(n @ ('"' | '\\' | '$' | '`')) => self.word.push(n),
                Some(n) => self.word.extend(['\\', n]),
                None => self.word.push('\\'),
            },
            _ => self.word.push(c),
        }
        true
    }

    /// `false` ends the line (a comment).
    fn plain(&mut self, c: char, chars: &mut std::str::Chars) -> bool {
        match c {
            '#' if !self.active => return false,
            '\'' | '"' => (self.quote, self.active) = (Some(c), true),
            '&' | '|' | ';' => {
                self.flush();
                self.cmds.push(std::mem::take(&mut self.cmd));
            }
            _ if c.is_whitespace() => self.flush(),
            _ => {
                let c = if c == '\\' {
                    chars.next().unwrap_or(c)
                } else {
                    c
                };
                self.word.push(c);
                self.active = true;
            }
        }
        true
    }
}
