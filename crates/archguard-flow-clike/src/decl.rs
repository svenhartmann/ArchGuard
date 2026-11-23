//! Class and method declarations with their attributes in Java, Kotlin and C# text.
use crate::lex::{self, ident_before, literal_end, split_top};
use crate::source::Source;

#[derive(Clone)]
pub struct Attr {
    pub name: String,
    pub args: String,
    pub at: usize,
}

#[derive(Clone)]
pub struct Decl {
    pub class: bool,
    pub name: String,
    pub attrs: Vec<Attr>,
    /// Offset of a method's parameter list; a class's header start.
    pub open: usize,
    /// A class's body end (`}`, or `;` without a body); 0 for a method.
    pub end: usize,
    /// A class's base list as simple names (`: Base<T>, IFoo`, `extends`,
    /// `implements`); empty for a method.
    pub bases: Vec<String>,
    /// `interface` for a class; `public` and not `static` for a method.
    pub flag: bool,
}

/// The header of a type declaration: from its modifiers to the body (or `;`).
pub fn class_header<'a>(src: &'a Source, d: &Decl) -> &'a str {
    let rest = &src.text[d.open..];
    &rest[..rest.find(['{', ';']).unwrap_or(rest.len())]
}

/// Calls `f(class, method)` for each method with the innermost class around it.
pub fn each_method(src: &Source, annotations: bool, mut f: impl FnMut(&Decl, &Decl)) {
    let mut open: Vec<Decl> = Vec::new();
    for d in decls(src, annotations) {
        while open.last().is_some_and(|c| c.end < d.open) {
            open.pop();
        }
        if d.class {
            open.push(d);
        } else if let Some(class) = open.last() {
            f(class, &d);
        }
    }
}

/// Last segment of a qualified attribute name, without `Attribute` and a target.
pub fn simple(name: &str) -> String {
    if let Some((head, generic)) = name.split_once('<') {
        return format!("{}<{generic}", simple(head));
    }
    let name = name.rsplit([':', '.']).next().unwrap_or(name).trim();
    name.strip_suffix("Attribute")
        .filter(|n| !n.is_empty())
        .unwrap_or(name)
        .to_string()
}

/// Class and annotated method declarations with their attributes, in order.
/// `annotations`: Java/Kotlin `@X(…)`; otherwise C# `[X(…), Y]`.
pub fn decls(src: &Source, annotations: bool) -> Vec<Decl> {
    let b = src.bytes();
    let (mut out, mut pending, mut i) = (Vec::new(), Vec::new(), 0);
    while i < b.len() {
        i = match b[i] {
            c if c.is_ascii_whitespace() => i + 1,
            b'[' if !annotations => attribute_list(src, i, &mut pending),
            b'@' if annotations && !src.text[i..].starts_with("@interface") => {
                annotation(src, i, &mut pending)
            }
            _ => {
                let end = header_end(src, i);
                let goes_on = src.header_goes_on(i, end);
                if !(goes_on && !pending.is_empty()) {
                    out.extend(declaration(src, i, end, std::mem::take(&mut pending)));
                }
                end + 1
            }
        };
    }
    out
}

/// The attributes of a Razor `@attribute [A(…)] [B]` directive (text after
/// the keyword); `at` is the offset of the list in `rest`.
pub fn directive_attrs(rest: &str) -> Vec<Attr> {
    let src = Source::new(String::new(), rest);
    let (mut out, mut i) = (Vec::new(), 0);
    while let Some(at) = src.text[i..].find(|c: char| !c.is_whitespace()) {
        if src.bytes()[i + at] != b'[' {
            break;
        }
        let next = attribute_list(&src, i + at, &mut out);
        if next == i + at + 1 && src.pair(i + at).is_none() {
            break;
        }
        i = next;
    }
    out
}

/// A C# `[A(…), B]` list at `i`; returns the position after it.
fn attribute_list(src: &Source, i: usize, pending: &mut Vec<Attr>) -> usize {
    let Some(close) = src.pair(i) else {
        return i + 1;
    };
    for part in split_top(&src.text[i + 1..close]) {
        let (name, args) = match part.find('(') {
            Some(p) if part.ends_with(')') => (&part[..p], &part[p + 1..part.len() - 1]),
            _ => (part, ""),
        };
        pending.push(Attr {
            name: simple(name),
            args: args.into(),
            at: i,
        });
    }
    close + 1
}

/// A Java/Kotlin `@A(…)` at `i`; returns the position after it.
fn annotation(src: &Source, i: usize, pending: &mut Vec<Attr>) -> usize {
    let b = src.bytes();
    let j = i
        + 1
        + b[i + 1..]
            .iter()
            .take_while(|c| lex::is_ident(**c) || matches!(c, b'.' | b':'))
            .count();
    let k = j + b[j..]
        .iter()
        .take_while(|c| matches!(c, b' ' | b'\t'))
        .count();
    let close = (b.get(k) == Some(&b'(')).then(|| src.pair(k)).flatten();
    pending.push(Attr {
        name: simple(&src.text[i + 1..j]),
        args: close.map_or("", |c| &src.text[k + 1..c]).into(),
        at: i,
    });
    close.map_or(j, |c| c + 1).max(i + 1)
}

/// End of a declaration or statement head: `{`, `;`, `}` or a line end outside brackets.
fn header_end(src: &Source, mut j: usize) -> usize {
    let b = src.bytes();
    while j < b.len() {
        if let Some(end) = literal_end(b, j) {
            j = end;
            continue;
        }
        match b[j] {
            b'(' | b'[' => j = src.pair(j).map_or(j + 1, |c| c + 1),
            b'{' | b';' | b'}' | b'\n' => return j,
            _ => j += 1,
        }
    }
    b.len()
}

/// A class, or a method with attributes; in C# also a method without any
/// (conventional routing needs every public action).
fn declaration(src: &Source, start: usize, end: usize, attrs: Vec<Attr>) -> Option<Decl> {
    let header = &src.text[start..end];
    if let Some((interface, name)) = class_name(header, src.rel.ends_with(".kt")) {
        let (bases, end) = class_shape(src, start);
        return Some(Decl {
            class: true,
            name,
            attrs,
            open: start,
            end,
            bases,
            flag: interface,
        });
    }
    let p = header.find('(')?;
    let modifiers: Vec<String> = (header[..p].split_whitespace())
        .map(str::to_owned)
        .collect();
    let has = |w: &str| modifiers.iter().any(|m| m == w);
    let public = has("public") && !has("static") && !header[..p].contains('=');
    if attrs.is_empty() && !(public && src.rel.ends_with(".cs")) {
        return None;
    }
    let (s, e) = ident_before(header.as_bytes(), p);
    (s < e).then(|| Decl {
        class: false,
        name: header[s..e].into(),
        attrs,
        open: start + p,
        end: 0,
        bases: vec![],
        flag: public,
    })
}

/// `(is an interface, name)` of a type declaration header.
fn class_name(header: &str, kotlin: bool) -> Option<(bool, String)> {
    let head = header.split('(').next().unwrap_or(header);
    let mut words = head
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty());
    while let Some(w) = words.next() {
        if matches!(w, "class" | "interface" | "record" | "struct") || kotlin && w == "object" {
            return words.next().map(|n| (w == "interface", n.to_owned()));
        }
    }
    None
}

/// The base list and body end of the type declared at `start`.
fn class_shape(src: &Source, start: usize) -> (Vec<String>, usize) {
    let b = src.bytes();
    let mut j = start;
    while j < b.len() && !matches!(b[j], b'{' | b';') {
        if let Some(end) = literal_end(b, j) {
            j = end;
            continue;
        }
        j = match b[j] {
            b'(' | b'[' => src.pair(j).map_or(j + 1, |c| c + 1),
            _ => j + 1,
        };
    }
    let end = match b.get(j) {
        Some(b'{') => src.pair(j).unwrap_or(b.len()),
        _ => j,
    };
    (base_names(&src.text[start..j.min(b.len())]), end)
}

/// Simple names in the base list of a type header (`: Base<T>, IFoo`,
/// `extends`, `implements`), primary constructors and generics left out.
fn base_names(header: &str) -> Vec<String> {
    let (mut head, mut depth) = (String::new(), 0i32);
    for c in header.chars() {
        match c {
            '(' | '[' | '<' => depth += 1,
            ')' | ']' | '>' => depth -= 1,
            _ if depth == 0 => head.push(c),
            _ => {}
        }
    }
    let head = head.split(" where ").next().unwrap_or("");
    let list = match head.find(':') {
        Some(p) => &head[p + 1..],
        None => match head.find(" extends ").or_else(|| head.find(" implements ")) {
            Some(p) => &head[p..],
            None => "",
        },
    };
    (list.replace(" implements ", ",").replace(" extends ", ","))
        .split(',')
        .filter_map(|item| {
            let item = item.split_whitespace().next()?;
            let name = item.rsplit('.').next().unwrap_or(item);
            let name: String = name
                .chars()
                .take_while(|c| lex::is_ident(*c as u8))
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .collect()
}
