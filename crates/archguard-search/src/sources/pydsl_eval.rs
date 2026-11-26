//! Python literals and DSL calls of a module read as JSON, as Elasticsearch stores them. All or
//! nothing: one part that is not a literal (f-string with a hole, unknown name, other call,
//! `**` spread) makes the whole value `None`. Names resolve to top-level constants of the file only.
use super::pydsl_module::{Callee, Member, Module};
use archguard_flow_common::lang_pyscan::{dotted, skip_ws};
use archguard_flow_common::lang_python::{Arg, call_args, split_commas, string_value};
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;

/// Nesting limit of one value; also stops a cycle of constants or inner documents.
pub(super) const DEPTH: u8 = 64;

type Range = (usize, usize);

pub(super) struct Eval<'m, 'a> {
    m: &'m Module<'a>,
    /// Analysis objects the evaluated calls defined: kind -> name -> definition.
    pub(super) analysis: BTreeMap<&'static str, Map<String, Value>>,
}

impl<'m, 'a> Eval<'m, 'a> {
    pub(super) fn new(m: &'m Module<'a>) -> Self {
        Self {
            m,
            analysis: BTreeMap::new(),
        }
    }

    pub(super) fn value(&mut self, range: Range, depth: u8) -> Option<Value> {
        let depth = depth.checked_sub(1)?;
        let b = self.m.masked.as_bytes();
        let (a, e) = trim(b, range);
        let text = self.m.src.get(a..e).filter(|t| !t.is_empty())?;
        if let Some(s) = string_value(text) {
            return Some(s.into());
        }
        let closed = matches!(b[a], b'[' | b'(' | b'{') && close(b, a) == Some(e - 1);
        match b[a] {
            b'[' | b'(' if closed => self.items(a, e, depth).map(Value::Array),
            b'{' if closed => self.dict(a, e, depth),
            c if c.is_ascii_digit() || c == b'-' => number(text),
            _ => self.name_or_call((a, e), depth),
        }
    }

    fn items(&mut self, a: usize, e: usize, depth: u8) -> Option<Vec<Value>> {
        parts(&self.m.masked, a, e)
            .into_iter()
            .map(|r| self.value(r, depth))
            .collect()
    }

    fn dict(&mut self, a: usize, e: usize, depth: u8) -> Option<Value> {
        let mut out = Map::new();
        for (x, y) in parts(&self.m.masked, a, e) {
            let colon = colon(self.m.masked.as_bytes(), x, y)?;
            let key = self.value((x, colon), depth)?;
            out.insert(key.as_str()?.to_owned(), self.value((colon + 1, y), depth)?);
        }
        Some(Value::Object(out))
    }

    fn name_or_call(&mut self, (a, e): Range, depth: u8) -> Option<Value> {
        match &self.m.src[a..e] {
            "True" => return Some(true.into()),
            "False" => return Some(false.into()),
            "None" => return Some(Value::Null),
            _ => {}
        }
        let b = self.m.masked.as_bytes();
        let (name, j) = dotted(b, a);
        if j == e && !name.contains('.') {
            return self.value(self.m.constant(&name)?, depth);
        }
        let open = skip_ws(b, j);
        if name.is_empty() || b.get(open) != Some(&b'(') {
            return None;
        }
        self.call(&name, open, e, depth)
    }

    fn call(&mut self, name: &str, open: usize, e: usize, depth: u8) -> Option<Value> {
        let (args, close) = call_args(self.m.src, &self.m.masked, open)?;
        if close != e - 1 {
            return None;
        }
        match self.m.classify(name)? {
            Callee::Analysis(kind) => self.define(kind, &args, depth),
            Callee::Field(ty) => self.field(&ty, &args, depth),
            Callee::List => match args.as_slice() {
                [only] if only.name.is_none() => self.arg(only, depth),
                _ => None,
            },
        }
    }

    fn arg(&mut self, a: &Arg, depth: u8) -> Option<Value> {
        let from = a.value.as_ptr() as usize - self.m.src.as_ptr() as usize;
        self.value((from, from + a.value.len()), depth)
    }

    /// `analyzer(name, type=None, **params)` and the other analysis builders: defines the object
    /// below its kind and yields its name. Without type and parameters it names a built-in.
    fn define(&mut self, kind: &'static str, args: &[Arg], depth: u8) -> Option<Value> {
        let mut positional = args.iter().filter(|a| a.name.is_none());
        let name = self.arg(positional.next()?, depth)?.as_str()?.to_owned();
        let mut def = Map::new();
        if let Some(ty) = positional.next() {
            def.insert("type".into(), self.arg(ty, depth)?);
        }
        if positional.next().is_some() {
            return None;
        }
        for a in args.iter().filter(|a| a.name.is_some()) {
            def.insert(a.name?.into(), self.arg(a, depth)?);
        }
        if def.is_empty() {
            return Some(name.into());
        }
        if !def.contains_key("type") {
            matches!(kind, "analyzer" | "normalizer").then_some(())?;
            def.insert("type".into(), "custom".into());
        }
        self.analysis
            .entry(kind)
            .or_default()
            .insert(name.clone(), Value::Object(def));
        Some(name.into())
    }

    /// A field class call: `type`, its keyword arguments, and for `Object`/`Nested` the
    /// properties of an inner document class given positionally.
    fn field(&mut self, ty: &str, args: &[Arg], depth: u8) -> Option<Value> {
        let mut def = Map::new();
        def.insert("type".into(), ty.into());
        for a in args {
            match a.name {
                // Options of the Python classes, not mapping parameters.
                Some("multi" | "required" | "attr") => {}
                Some(k) => {
                    def.insert(k.into(), self.arg(a, depth)?);
                }
                None if matches!(ty, "object" | "nested") => {
                    let class = self.m.class_index(a.value)?;
                    let props = self.properties(class, depth)?;
                    def.insert("properties".into(), Value::Object(props));
                }
                None => return None,
            }
        }
        Some(Value::Object(def))
    }

    /// The fields a class declares as `name = Text(…)` or typed (`name: M[Keyword]`); other class
    /// attributes are no fields. `None` when a typed field needs its type inferred.
    pub(super) fn properties(&mut self, class: usize, depth: u8) -> Option<Map<String, Value>> {
        let mut props = Map::new();
        for (name, member) in self.m.members(&self.m.defs[class]) {
            let value = match member {
                Member::Call(a, e) => self.value((a, e), depth)?,
                Member::Typed(ty) => serde_json::json!({ "type": ty? }),
            };
            props.insert(name, value);
        }
        Some(props)
    }
}

fn number(text: &str) -> Option<Value> {
    text.parse::<i64>().ok().map(Value::from).or_else(|| {
        let f = text.parse::<f64>().ok()?;
        Number::from_f64(f).map(Value::Number)
    })
}

fn trim(b: &[u8], (mut a, mut e): Range) -> Range {
    while a < e && b[a].is_ascii_whitespace() {
        a += 1;
    }
    while e > a && b[e - 1].is_ascii_whitespace() {
        e -= 1;
    }
    (a, e)
}

/// The offset of the bracket that closes the one at `open`.
fn close(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in b.iter().enumerate().skip(open) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// The non-blank comma-separated parts inside the bracket pair `a..e`.
fn parts(masked: &str, a: usize, e: usize) -> Vec<Range> {
    let mut out = split_commas(masked, a + 1, e - 1);
    out.retain(|&(x, y)| !masked[x..y].trim().is_empty());
    out
}

/// The first `:` of `x..y` outside brackets.
fn colon(b: &[u8], x: usize, y: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in b.iter().enumerate().take(y).skip(x) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b':' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}
