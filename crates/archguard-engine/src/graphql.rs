//! Small GraphQL reader (ADR 0014): type definitions of SDL documents and the
//! root fields of operation documents. It reads, it does not validate.
use std::collections::{BTreeMap, BTreeSet};

const MAX_TOKENS: usize = 200_000;
const MAX_DEPTH: usize = 32;
const PUNCTUATORS: &[u8] = b"!$&():=@[]{|}";

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Name,
    Punct,
    Spread,
    Str,
    Num,
}
#[derive(Debug, Clone, Copy)]
struct Tok<'a> {
    kind: Kind,
    text: &'a str,
    at: usize,
    end: usize,
}

#[derive(Debug, Default)]
pub(crate) struct Document {
    pub types: Vec<TypeDef>,
    /// `query` → root type name, from a `schema { … }` definition.
    pub roots: BTreeMap<String, String>,
    /// Directive uses as written, prefixed with their schema coordinate
    /// (`User @key(fields: "id")`); recorded, never interpreted.
    pub directives: Vec<String>,
    pub operations: Vec<Operation>,
}
#[derive(Debug)]
pub(crate) struct TypeDef {
    pub name: String,
    pub kind: String,
    /// Fields, input fields, enum values or union members (with empty type).
    pub fields: Vec<(String, Field)>,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Field {
    pub ty: String,
    /// Argument name → (type, has default value).
    pub args: BTreeMap<String, (String, bool)>,
}
#[derive(Debug)]
pub(crate) struct Operation {
    pub kind: String,
    pub name: Option<String>,
    pub root_fields: BTreeSet<String>,
    /// A root-level fragment spread names no fragment of this document.
    pub unknown_spreads: Vec<String>,
    /// Byte offset of the operation in the parsed text.
    pub at: usize,
}
type R<T> = Result<T, (usize, String)>;
/// Name, type, has a default value, directive uses.
type InputValue = (String, String, bool, Vec<String>);
/// Root fields and root-level fragment spreads of one selection set.
#[derive(Default)]
struct Selection {
    fields: BTreeSet<String>,
    spreads: Vec<String>,
}

fn string_end(src: &str, start: usize) -> R<usize> {
    if let Some(rest) = src[start..].strip_prefix("\"\"\"") {
        let mut from = 0;
        while let Some(found) = rest[from..].find("\"\"\"") {
            let at = from + found;
            if !rest[..at].ends_with('\\') {
                return Ok(start + 3 + at + 3);
            }
            from = at + 3;
        }
        return Err((start, "unterminated block string".into()));
    }
    let b = src.as_bytes();
    let mut i = start + 1;
    loop {
        match b.get(i) {
            None | Some(b'\n') => return Err((start, "unterminated string".into())),
            Some(b'\\') => i += 2,
            Some(b'"') => return Ok(i + 1),
            _ => i += 1,
        }
    }
}
fn run(b: &[u8], mut i: usize, keep: impl Fn(u8) -> bool) -> usize {
    while i < b.len() && keep(b[i]) {
        i += 1;
    }
    i
}
fn token(src: &str, i: usize) -> R<(Kind, usize)> {
    let b = src.as_bytes();
    let c = b[i];
    if c == b'"' {
        return Ok((Kind::Str, string_end(src, i)?));
    }
    if src[i..].starts_with("...") {
        return Ok((Kind::Spread, i + 3));
    }
    if PUNCTUATORS.contains(&c) {
        return Ok((Kind::Punct, i + 1));
    }
    if c == b'_' || c.is_ascii_alphabetic() {
        return Ok((
            Kind::Name,
            run(b, i, |x| x == b'_' || x.is_ascii_alphanumeric()),
        ));
    }
    if c == b'-' || c.is_ascii_digit() {
        let end = run(b, i + 1, |x| {
            x.is_ascii_alphanumeric() || b".+-".contains(&x)
        });
        return Ok((Kind::Num, end));
    }
    Err((i, "unexpected character".into()))
}
fn lex(src: &str) -> R<Vec<Tok<'_>>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        if out.len() > MAX_TOKENS {
            return Err((i, "document exceeds the token limit".into()));
        }
        let c = src.as_bytes()[i];
        if c.is_ascii_whitespace() || c == b',' {
            i += 1;
        } else if c == b'#' {
            i = src[i..].find('\n').map_or(src.len(), |n| i + n);
        } else if src[i..].starts_with('\u{feff}') {
            i += 3;
        } else {
            let (kind, end) = token(src, i)?;
            out.push(Tok {
                kind,
                text: &src[i..end],
                at: i,
                end,
            });
            i = end;
        }
    }
    Ok(out)
}
fn record(doc: &mut Document, at: &str, list: Vec<String>) {
    doc.directives
        .extend(list.into_iter().map(|d| format!("{at} {d}")));
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Tok<'a>>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<Tok<'a>> {
        self.toks.get(self.pos).copied()
    }
    fn is(&self, text: &str) -> bool {
        self.peek()
            .is_some_and(|t| t.kind != Kind::Str && t.text == text)
    }
    fn eat(&mut self, text: &str) -> bool {
        let found = self.is(text);
        if found {
            self.pos += 1;
        }
        found
    }
    fn fail<T>(&self, what: &str) -> R<T> {
        let at = self.peek().map_or(self.src.len(), |t| t.at);
        Err((at, format!("expected {what}")))
    }
    fn expect(&mut self, text: &str) -> R<()> {
        if self.eat(text) {
            Ok(())
        } else {
            self.fail(&format!("'{text}'"))
        }
    }
    fn name(&mut self) -> R<&'a str> {
        match self.peek() {
            Some(t) if t.kind == Kind::Name => {
                self.pos += 1;
                Ok(t.text)
            }
            _ => self.fail("a name"),
        }
    }
    /// `A | B | C` or `A & B` with an optional leading separator.
    fn names(&mut self, separator: &str) -> R<Vec<String>> {
        self.eat(separator);
        let mut out = vec![self.name()?.to_owned()];
        while self.eat(separator) {
            out.push(self.name()?.to_owned());
        }
        Ok(out)
    }
    fn description(&mut self) {
        if self.peek().is_some_and(|t| t.kind == Kind::Str) {
            self.pos += 1;
        }
    }
    /// Skips a bracketed group starting at the current opening token.
    fn skip_group(&mut self) -> R<()> {
        let mut depth = 0usize;
        while let Some(t) = self.peek() {
            self.pos += 1;
            if t.kind != Kind::Punct {
                continue;
            }
            if "([{".contains(t.text) {
                depth += 1;
            } else if ")]}".contains(t.text) {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
        }
        self.fail("a closing bracket")
    }
    fn value(&mut self) -> R<()> {
        if self.is("[") || self.is("{") {
            return self.skip_group();
        }
        if self.eat("$") {
            return self.name().map(|_| ());
        }
        match self.peek() {
            Some(t) if t.kind != Kind::Punct && t.kind != Kind::Spread => {
                self.pos += 1;
                Ok(())
            }
            _ => self.fail("a value"),
        }
    }
    /// Directive uses as written.
    fn directives(&mut self) -> R<Vec<String>> {
        let mut out = vec![];
        while let Some(start) = self.peek().filter(|_| self.is("@")) {
            self.pos += 1;
            self.name()?;
            if self.is("(") {
                self.skip_group()?;
            }
            let end = self.toks[self.pos - 1].end;
            out.push(self.src[start.at..end].to_owned());
        }
        Ok(out)
    }
    fn type_ref(&mut self, depth: usize) -> R<String> {
        if depth > MAX_DEPTH {
            return self.fail("less nesting");
        }
        let mut ty = if self.eat("[") {
            let inner = self.type_ref(depth + 1)?;
            self.expect("]")?;
            format!("[{inner}]")
        } else {
            self.name()?.to_owned()
        };
        if self.eat("!") {
            ty.push('!');
        }
        Ok(ty)
    }
    /// `name: Type = default @dir` items up to `close`.
    fn input_values(&mut self, close: &str) -> R<Vec<InputValue>> {
        let mut out = vec![];
        while !self.eat(close) {
            self.description();
            let name = self.name()?.to_owned();
            self.expect(":")?;
            let ty = self.type_ref(0)?;
            let default = self.eat("=");
            if default {
                self.value()?;
            }
            out.push((name, ty, default, self.directives()?));
        }
        Ok(out)
    }
    fn type_def(&mut self, doc: &mut Document) -> R<()> {
        let kind = self.name()?.to_owned();
        let name = self.name()?.to_owned();
        if self.eat("implements") {
            self.names("&")?;
        }
        let own = self.directives()?;
        record(doc, &name, own);
        let fields = members(self, doc, &kind, &name)?;
        doc.types.push(TypeDef { name, kind, fields });
        Ok(())
    }
    fn schema_def(&mut self, doc: &mut Document) -> R<()> {
        let used = self.directives()?;
        record(doc, "schema", used);
        if self.eat("{") {
            while !self.eat("}") {
                let kind = self.name()?.to_owned();
                self.expect(":")?;
                doc.roots.insert(kind, self.name()?.to_owned());
            }
        }
        Ok(())
    }
    fn directive_def(&mut self) -> R<()> {
        self.expect("@")?;
        self.name()?;
        if self.is("(") {
            self.skip_group()?;
        }
        self.eat("repeatable");
        self.expect("on")?;
        self.names("|").map(|_| ())
    }
    /// After `...`: a named spread or an inline fragment of the same root.
    fn spread(&mut self, out: &mut Selection, depth: usize) -> R<()> {
        let inline_on = self.is("on")
            && self
                .toks
                .get(self.pos + 1)
                .is_some_and(|t| t.kind == Kind::Name);
        if inline_on {
            self.pos += 2;
        } else if !self.is("@") && !self.is("{") {
            out.spreads.push(self.name()?.to_owned());
            return self.directives().map(|_| ());
        }
        self.directives()?;
        self.selection(out, depth + 1)
    }
    /// Root-level field names and fragment spreads of a selection set.
    fn selection(&mut self, out: &mut Selection, depth: usize) -> R<()> {
        if depth > MAX_DEPTH {
            return self.fail("less nesting");
        }
        self.expect("{")?;
        while !self.eat("}") {
            if self.peek().is_some_and(|t| t.kind == Kind::Spread) {
                self.pos += 1;
                self.spread(out, depth)?;
                continue;
            }
            let mut field = self.name()?;
            if self.eat(":") {
                field = self.name()?;
            }
            if self.is("(") {
                self.skip_group()?;
            }
            self.directives()?;
            if self.is("{") {
                self.skip_group()?;
            }
            if !field.starts_with("__") {
                out.fields.insert(field.to_owned());
            }
        }
        Ok(())
    }
    /// `query Name($v: T) @dir { … }` after the keyword.
    fn operation(&mut self) -> R<(Option<String>, Selection)> {
        let name = self.peek().filter(|t| t.kind == Kind::Name).map(|t| {
            self.pos += 1;
            t.text.to_owned()
        });
        if self.is("(") {
            self.skip_group()?;
        }
        self.directives()?;
        let mut selection = Selection::default();
        self.selection(&mut selection, 0)?;
        Ok((name, selection))
    }
}

fn object_fields(p: &mut Parser<'_>, doc: &mut Document, owner: &str) -> R<Vec<(String, Field)>> {
    let mut fields = vec![];
    while !p.eat("}") {
        p.description();
        let field = p.name()?.to_owned();
        let args = if p.eat("(") {
            p.input_values(")")?
        } else {
            vec![]
        };
        p.expect(":")?;
        let ty = p.type_ref(0)?;
        let used = p.directives()?;
        record(doc, &format!("{owner}.{field}"), used);
        let args = args.into_iter().map(|(n, t, d, _)| (n, (t, d))).collect();
        fields.push((field, Field { ty, args }));
    }
    Ok(fields)
}

fn input_fields(p: &mut Parser<'_>, doc: &mut Document, owner: &str) -> R<Vec<(String, Field)>> {
    let mut fields = vec![];
    for (field, ty, _, used) in p.input_values("}")? {
        record(doc, &format!("{owner}.{field}"), used);
        let ty = Field {
            ty,
            ..Field::default()
        };
        fields.push((field, ty));
    }
    Ok(fields)
}

fn enum_values(p: &mut Parser<'_>, doc: &mut Document, owner: &str) -> R<Vec<(String, Field)>> {
    let mut values = vec![];
    while !p.eat("}") {
        p.description();
        let value = p.name()?.to_owned();
        let used = p.directives()?;
        record(doc, &format!("{owner}.{value}"), used);
        values.push((value, Field::default()));
    }
    Ok(values)
}
/// What a type definition lists: fields, input fields, enum values or union members.
fn members(
    p: &mut Parser<'_>,
    doc: &mut Document,
    kind: &str,
    name: &str,
) -> R<Vec<(String, Field)>> {
    if kind == "union" {
        if !p.eat("=") {
            return Ok(vec![]);
        }
        let names = p.names("|")?;
        return Ok(names.into_iter().map(|m| (m, Field::default())).collect());
    }
    if !p.eat("{") {
        return Ok(vec![]);
    }
    match kind {
        "input" => input_fields(p, doc, name),
        "enum" => enum_values(p, doc, name),
        _ => object_fields(p, doc, name),
    }
}

struct Pending {
    kind: String,
    name: Option<String>,
    selection: Selection,
    at: usize,
}
#[derive(Default)]
struct Reader {
    doc: Document,
    fragments: BTreeMap<String, Selection>,
    pending: Vec<Pending>,
}
impl Reader {
    fn definition(&mut self, p: &mut Parser<'_>, t: Tok<'_>) -> R<()> {
        match t.text {
            "{" if t.kind == Kind::Punct => {
                let mut selection = Selection::default();
                p.selection(&mut selection, 0)?;
                self.pending.push(Pending {
                    kind: "query".into(),
                    name: None,
                    selection,
                    at: t.at,
                });
            }
            "query" | "mutation" | "subscription" => {
                p.pos += 1;
                let (name, selection) = p.operation()?;
                self.pending.push(Pending {
                    kind: t.text.into(),
                    name,
                    selection,
                    at: t.at,
                });
            }
            "fragment" => {
                p.pos += 1;
                let name = p.name()?.to_owned();
                p.expect("on")?;
                p.name()?;
                let (_, selection) = p.operation()?;
                self.fragments.insert(name, selection);
            }
            _ => return self.type_system(p, t),
        }
        Ok(())
    }
    fn type_system(&mut self, p: &mut Parser<'_>, t: Tok<'_>) -> R<()> {
        p.pos += 1;
        match t.text {
            "extend" if p.eat("schema") => p.schema_def(&mut self.doc),
            "extend" => p.type_def(&mut self.doc),
            "schema" => p.schema_def(&mut self.doc),
            "directive" => p.directive_def(),
            "type" | "interface" | "input" | "enum" | "union" | "scalar" => {
                p.pos -= 1;
                p.type_def(&mut self.doc)
            }
            _ => {
                p.pos -= 1;
                p.fail("a definition")
            }
        }
    }
    /// Root-level spreads take the root fields of fragments in the same document.
    fn finish(mut self) -> Document {
        for op in std::mem::take(&mut self.pending) {
            let Selection {
                fields: mut root_fields,
                mut spreads,
            } = op.selection;
            let mut seen = BTreeSet::new();
            let mut unknown_spreads = vec![];
            while let Some(spread) = spreads.pop() {
                if !seen.insert(spread.clone()) {
                    continue;
                }
                let Some(fragment) = self.fragments.get(&spread) else {
                    unknown_spreads.push(spread);
                    continue;
                };
                root_fields.extend(fragment.fields.iter().cloned());
                spreads.extend(fragment.spreads.iter().cloned());
            }
            self.doc.operations.push(Operation {
                kind: op.kind,
                name: op.name,
                root_fields,
                unknown_spreads,
                at: op.at,
            });
        }
        self.doc
    }
}

/// Parses SDL definitions and executable definitions of one document.
/// Errors carry the byte offset they refer to.
pub(crate) fn parse(src: &str) -> Result<Document, (usize, String)> {
    let mut p = Parser {
        src,
        toks: lex(src)?,
        pos: 0,
    };
    let mut reader = Reader::default();
    loop {
        p.description();
        let Some(t) = p.peek() else { break };
        reader.definition(&mut p, t)?;
    }
    Ok(reader.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdl_types_arguments_extensions_and_directives_are_read() {
        let doc = parse(
            r#""""Users"""
extend schema @link(url: "https://specs.apollo.dev/federation/v2.3", import: ["@key"])
schema { query: Root }
directive @auth(role: String = "x") repeatable on FIELD_DEFINITION | OBJECT
type Root { user(id: ID!, "doc" first: Int = 10, filter: F = {a: [1, 2]}): User @auth }
type User implements Node & Entity @key(fields: "id") { id: ID! @external tags: [String!]! }
extend type User { email: String }
input F { a: [Int] = [] }
enum Role { ADMIN @deprecated(reason: "no") USER }
union Item = | User | Role
scalar Date @specifiedBy(url: "u")"#,
        )
        .unwrap();
        assert_eq!(doc.roots["query"], "Root");
        let root = &doc.types[0];
        let user = &root.fields[0].1;
        assert_eq!(user.ty, "User");
        assert_eq!(user.args["id"], ("ID!".into(), false));
        assert_eq!(user.args["first"], ("Int".into(), true));
        assert_eq!(doc.types[1].fields[1].1.ty, "[String!]!");
        assert_eq!(doc.types[2].name, "User");
        assert_eq!(doc.types[2].fields[0].0, "email");
        assert_eq!(doc.types[5].fields.len(), 2);
        assert!(
            doc.directives
                .contains(&r#"User @key(fields: "id")"#.to_owned())
        );
        assert!(doc.directives.contains(&"User.id @external".to_owned()));
        assert!(
            doc.directives
                .iter()
                .any(|d| d.starts_with("schema @link("))
        );
        assert!(doc.operations.is_empty());
    }

    #[test]
    fn operations_report_root_fields_through_aliases_fragments_and_inline_fragments() {
        let doc = parse(
            "query GetUser($id: ID! = \"1\") @cached { u: user(id: $id) { ...F id } ...Top ... on Query { users } __typename ...Gone }
             fragment Top on Query { me { id } ...More }
             fragment More on Query { node(id: 1) { id } }
             fragment F on User { name }
             mutation { createUser(input: {name: \"x\"}) { id } }
             { viewer }",
        )
        .unwrap();
        let get = &doc.operations[0];
        assert_eq!(get.name.as_deref(), Some("GetUser"));
        assert_eq!(
            get.root_fields
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["me", "node", "user", "users"]
        );
        assert_eq!(get.unknown_spreads, ["Gone"]);
        assert_eq!(doc.operations[1].kind, "mutation");
        assert_eq!(doc.operations[2].kind, "query");
        assert!(doc.operations[2].root_fields.contains("viewer"));
    }

    #[test]
    fn malformed_documents_report_the_offset() {
        let (at, message) = parse("type User { id: }").unwrap_err();
        assert_eq!(at, 16);
        assert!(message.contains("name"), "{message}");
        assert!(parse("query { a ").is_err());
        assert!(parse("\"open").is_err());
        assert!(parse("\"\"\" never closed").is_err());
        assert_eq!(
            parse("\"\"\"a \\\"\"\" b\"\"\" scalar X")
                .unwrap()
                .types
                .len(),
            1
        );
    }

    #[test]
    fn deeply_nested_list_types_fail_instead_of_overflowing_the_stack() {
        let deep = format!(
            "type Q {{ f: {}Int{} }}",
            "[".repeat(99_000),
            "]".repeat(99_000)
        );
        let (_, message) = parse(&deep).unwrap_err();
        assert!(message.contains("less nesting"), "{message}");
        assert_eq!(
            parse("type Q { f: [[Int!]!] }").unwrap().types[0].fields[0]
                .1
                .ty,
            "[[Int!]!]"
        );
    }
}
