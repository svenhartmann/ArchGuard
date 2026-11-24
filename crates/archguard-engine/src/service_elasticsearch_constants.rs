//! String constants a service declares and the index references they resolve (ADR 0023, Q1).
//! One declaration in the service, a literal value and a matching qualifier resolve; every
//! other case stays a gap.
use archguard_core::{ServiceEvidence, ServiceTopology};
use archguard_flow_common::support::diagnostic;
use std::collections::BTreeMap;

/// A string constant with its owner (Java/C# class, else file stem); `value` is `None` when not a literal.
struct Constant {
    owner: String,
    value: Option<String>,
    evidence: ServiceEvidence,
}

/// A reference waiting for the constants of its whole service.
struct Pending {
    service: String,
    rel: String,
    ext: &'static str,
    reference: String,
    access: &'static str,
    line: u32,
    pointer: &'static str,
}

/// Constants by (service, language, name) and the references that wait for them.
#[derive(Default)]
pub(super) struct Constants {
    declared: BTreeMap<(String, &'static str, String), Vec<Constant>>,
    pending: Vec<Pending>,
}

type Found = BTreeMap<(String, String, &'static str), Vec<ServiceEvidence>>;

impl Constants {
    pub(super) fn collect(&mut self, service: &str, ext: &str, rel: &str, text: &str) {
        for (name, constant) in declarations(text, ext, rel) {
            let key = (service.to_owned(), lang(ext), name);
            self.declared.entry(key).or_default().push(constant);
        }
    }

    pub(super) fn defer(
        &mut self,
        service: &str,
        ext: &str,
        rel: &str,
        reference: String,
        at: (&'static str, u32, &'static str),
    ) {
        self.pending.push(Pending {
            service: service.to_owned(),
            rel: rel.to_owned(),
            ext: lang(ext),
            reference,
            access: at.0,
            line: at.1,
            pointer: at.2,
        });
    }

    /// Turns each waiting reference into a usage with call site and declaration, or a gap.
    pub(super) fn settle(self, found: &mut Found, report: &mut ServiceTopology) {
        for p in &self.pending {
            match self.resolve(p) {
                Some((name, declared)) => {
                    let call = site(
                        &p.rel,
                        p.line,
                        p.pointer,
                        "Index name taken from a constant of the same service.",
                    );
                    let key = (p.service.clone(), name, p.access);
                    found.entry(key).or_default().extend([call, declared]);
                }
                None => diagnostic(report, &p.rel, Some(short(&p.service)), GAP),
            }
        }
    }

    /// The literal of the one constant the reference names in its service; any other count, a
    /// non-literal value or a qualifier that is not the declaring class (file stem) is a gap.
    fn resolve(&self, p: &Pending) -> Option<(String, ServiceEvidence)> {
        let (qualifier, name) = match p.reference.rsplit_once('.') {
            Some((q, n)) => (Some(q.rsplit('.').next().unwrap_or(q)), n),
            None => (None, p.reference.as_str()),
        };
        let key = (p.service.clone(), p.ext, name.to_owned());
        let [c] = self.declared.get(&key)?.as_slice() else {
            return None;
        };
        let owned = qualifier.is_none_or(|q| q == c.owner);
        owned.then(|| c.value.clone().map(|v| (v, c.evidence.clone())))?
    }
}

pub(super) fn site(rel: &str, line: u32, pointer: &str, description: &str) -> ServiceEvidence {
    ServiceEvidence {
        file: rel.to_owned(),
        pointer: pointer.into(),
        line: Some(line),
        description: description.into(),
    }
}

/// `Name` or `Class.NAME` (any depth) is a reference; everything else is a gap.
pub(super) fn reference(arg: &str) -> Name {
    let ident = |s: &str| {
        s.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
            && s.chars().all(|c| c.is_alphanumeric() || c == '_')
    };
    if arg.split('.').all(ident) {
        Name::Ref(arg.to_owned())
    } else {
        Name::Gap
    }
}

/// `tsx` is `ts`: constants and references of one language group meet.
fn lang(ext: &str) -> &'static str {
    match ext {
        "java" => "java",
        "cs" => "cs",
        "py" => "py",
        _ => "ts",
    }
}

/// The type bodies a Java/C# file has open: (name, brace depth of the body, is interface).
#[derive(Default)]
struct Scope {
    open: Vec<(String, usize, bool)>,
    depth: usize,
    waiting: Option<(String, bool)>,
    keyword: Option<bool>,
}

impl Scope {
    /// Follows the braces and type declarations of `code`; `.class` literals are not declarations.
    fn feed(&mut self, code: &str) {
        let mut quoted = false;
        let mut word = String::new();
        let mut prev = ' ';
        for ch in code.chars().chain([' ']) {
            if ch.is_alphanumeric() || ch == '_' {
                word.push(ch);
                continue;
            }
            if !word.is_empty() && !quoted {
                self.on_word(&word, prev);
            }
            if !word.is_empty() {
                prev = 'a';
                word.clear();
            }
            match ch {
                '"' => quoted = !quoted,
                '{' if !quoted => self.open_body(),
                '}' if !quoted => self.close_body(),
                _ => {}
            }
            if !ch.is_whitespace() {
                prev = ch;
            }
        }
    }

    /// A word after a type keyword names the type; `prev` is the character before this word.
    fn on_word(&mut self, w: &str, prev: char) {
        match (self.keyword.take(), w) {
            (Some(iface), w) => self.waiting = Some((w.to_owned(), iface)),
            (None, "class" | "enum" | "record" | "struct") if prev != '.' => {
                self.keyword = Some(false)
            }
            (None, "interface") if prev != '.' => self.keyword = Some(true),
            _ => {}
        }
    }

    fn open_body(&mut self) {
        self.depth += 1;
        if let Some((name, iface)) = self.waiting.take() {
            self.open.push((name, self.depth, iface));
        }
    }

    fn close_body(&mut self) {
        while self.open.last().is_some_and(|(_, d, _)| *d >= self.depth) {
            self.open.pop();
        }
        self.depth = self.depth.saturating_sub(1);
    }

    /// The innermost type and whether a field declared here is a member of an interface.
    fn owner(&self) -> Option<(&str, bool)> {
        let (name, depth, iface) = self.open.last()?;
        Some((name, *iface && *depth == self.depth))
    }
}

/// The constant's name when `head` (the text before `=`) declares a string constant:
/// Java `static final String` (or a bare `String` field of an interface), C# `const string`
/// or `static readonly string`, TypeScript `const`, Python `NAME`; the last two only at top level.
fn declared_name<'a>(ext: &str, head: &[&'a str], top: bool, interface: bool) -> Option<&'a str> {
    let has = |w: &str| head.contains(&w);
    let ok = match ext {
        "java" => java_constant(head, interface),
        "cs" => (has("const") || (has("static") && has("readonly"))) && has("string"),
        "py" => head.len() == 1 && top,
        _ => top && (head.starts_with(&["const"]) || head.starts_with(&["export", "const"])),
    };
    let name = head.last().filter(|_| ok)?;
    Some(name.split(':').next().unwrap_or(name))
}

fn java_constant(head: &[&str], interface: bool) -> bool {
    let has = |w: &str| head.contains(&w);
    let bare = interface && head.len() >= 2 && head[head.len() - 2] == "String";
    (has("static") && has("final") && has("String")) || bare
}

/// The literal on the right of `=`; a `;` inside it cuts it, which makes it a gap.
fn value_of(ext: &str, value: &str) -> Option<String> {
    let value = value.split(';').next().unwrap_or("").trim();
    literal(value).filter(|_| matches!(ext, "java" | "cs") || !value.starts_with('`'))
}

/// String constants one file declares.
// shortcut: unqualified references ignore imports and nesting, and `=` splits the line at its first
// occurrence; upgrade if a name declared in two classes of one service must be told apart.
fn declarations(text: &str, ext: &str, rel: &str) -> Vec<(String, Constant)> {
    let stem = rel.rsplit('/').next().unwrap_or(rel);
    let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
    let jvm_like = matches!(ext, "java" | "cs");
    let mut scope = Scope::default();
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with("//") || line.starts_with('*') || line.starts_with("/*") {
            continue;
        }
        let split = line
            .split_once('=')
            .filter(|(h, v)| !v.starts_with('=') && !h.ends_with(['!', '<', '>']));
        let (before, after) = split.map_or((line, ""), |(h, v)| (h, v));
        if jvm_like {
            scope.feed(before);
        }
        let owner = scope.owner();
        let found = split.and_then(|(head, value)| {
            // The brace that closes a one-line body sits after the value; it is read below.
            let head = head.rsplit(['{', ';']).next().unwrap_or(head);
            let head: Vec<&str> = head.split_whitespace().collect();
            let top = !raw.starts_with(char::is_whitespace);
            let name = declared_name(ext, &head, top, owner.is_some_and(|o| o.1))?;
            Some((name, value))
        });
        if let Some((name, value)) = found {
            let owner_name = owner.map_or(stem, |o| o.0);
            let owner_name = if jvm_like { owner_name } else { stem };
            out.push((
                name.to_owned(),
                Constant {
                    owner: owner_name.to_owned(),
                    value: value_of(ext, value),
                    evidence: site(
                        rel,
                        i as u32 + 1,
                        name,
                        "Index name constant declared here.",
                    ),
                },
            ));
        }
        if jvm_like {
            scope.feed(after);
        }
    }
    out
}

/// What an index argument is: a literal, a possibly qualified identifier that a constant of
/// the same service may resolve, or neither.
pub(super) enum Name {
    Lit(String),
    Ref(String),
    Gap,
}

pub(super) const GAP: &str =
    "Index name is not a literal (constant, variable or expression); the index stays unknown.";

pub(super) fn short(service: &str) -> &str {
    service.rsplit(':').next().unwrap_or(service)
}

pub(super) fn literal(value: &str) -> Option<String> {
    let quote = value
        .chars()
        .next()
        .filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let inner = value.strip_prefix(quote)?.strip_suffix(quote)?;
    (!inner.is_empty() && !inner.contains(quote) && !inner.contains("${")).then(|| inner.to_owned())
}
