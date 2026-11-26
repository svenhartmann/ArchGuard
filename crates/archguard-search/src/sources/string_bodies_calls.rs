//! The calls that name an index or template and carry its definition as a string, found on the token
//! table of one file (rules of the format are listed in `string_bodies.rs`). The name position of
//! the API decides: a method of the same name on any other object does not match.
use super::constants::Name;
use super::string_bodies_lex::{Kind, Lang, Toks};
use std::collections::HashMap;

/// What stands where a body is expected.
pub(super) enum Arg {
    /// A string literal; `None` when it is not fully literal.
    Text(Option<String>),
    /// A possibly qualified identifier that a string constant may resolve.
    Ref(String),
    /// Anything composed or computed: `+`, `%`, a call, `**kwargs`.
    Gap,
    /// An object or array literal, `null` or nothing: no string body, not this reader's.
    Other,
}

/// One recognized call: the name of the index or template, its body, the line of the call.
pub(super) struct Found {
    pub(super) name: Name,
    pub(super) body: Arg,
    pub(super) line: u32,
    pub(super) api: String,
}

/// `group`, Python method, TypeScript method, keyword of the name.
const KW: [(&str, &str, &str, &str); 7] = [
    ("indices", "create", "create", "index"),
    ("indices", "put_template", "putTemplate", "name"),
    ("indices", "put_index_template", "putIndexTemplate", "name"),
    (
        "indices",
        "put_component_template",
        "putComponentTemplate",
        "name",
    ),
    ("indices", "put_mapping", "putMapping", "index"),
    ("ingest", "put_pipeline", "putPipeline", "id"),
    ("ilm", "put_lifecycle", "putLifecycle", "name"),
];
const JAVA_REQUESTS: [&str; 3] = [
    "CreateIndexRequest",
    "PutIndexTemplateRequest",
    "PutMappingRequest",
];
const JAVA_METHODS: [(&str, &str); 3] = [
    ("create", "index"),
    ("putIndexTemplate", "name"),
    ("putComponentTemplate", "name"),
];
const CS_METHODS: [&str; 4] = [
    "Create",
    "PutTemplate",
    "PutIndexTemplate",
    "PutComponentTemplate",
];

pub(super) fn find(t: &Toks, lang: Lang) -> Vec<Found> {
    let mut out = vec![];
    for i in 0..t.t.len() {
        match lang {
            Lang::Py | Lang::Ts => keyword_call(t, i, lang == Lang::Py, &mut out),
            Lang::Java => java_call(t, i, &mut out),
            Lang::Cs => cs_call(t, i, &mut out),
        }
    }
    if lang == Lang::Java {
        out.extend(java_request_calls(t));
    }
    out
}

/// What the tokens `a..b` are.
fn classify(t: &Toks, (a, b): (usize, usize)) -> Arg {
    let words = || {
        (a..b).all(|i| {
            if (i - a) % 2 == 0 {
                word_ident(t.word(i))
            } else {
                t.is(i, b'.')
            }
        })
    };
    match (t.t.get(a).map(|x| &x.kind), b - a) {
        (None, _) | (_, 0) | (Some(Kind::Punct(b'{' | b'[')), _) => Arg::Other,
        (Some(Kind::Str(v)), 1) => Arg::Text(v.clone()),
        (Some(Kind::Word("null" | "None" | "undefined")), 1) => Arg::Other,
        (Some(Kind::Word(_)), n) if n % 2 == 1 && words() => Arg::Ref(
            (a..b)
                .map(|i| if t.is(i, b'.') { "." } else { t.word(i) })
                .collect(),
        ),
        _ => Arg::Gap,
    }
}

fn word_ident(w: &str) -> bool {
    w.chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
}

fn name_of(arg: Arg) -> Name {
    match arg {
        Arg::Text(Some(s)) if !s.is_empty() => Name::Lit(s),
        Arg::Ref(r) => Name::Ref(r),
        _ => Name::Gap,
    }
}

fn gap_call(t: &Toks, at: usize, api: &str) -> Found {
    Found {
        name: Name::Gap,
        body: Arg::Gap,
        line: t.line(at),
        api: api.into(),
    }
}

/// The value of `key` in the arguments of a Python call (`key=value`) or in the object literal of
/// a TypeScript call (`key: value`, or the shorthand `key`).
fn keyed(t: &Toks, args: &[(usize, usize)], key: &str, py: bool) -> Option<(usize, usize)> {
    args.iter().find_map(|&(a, b)| {
        let named = t.word(a) == key || matches!(&t.t[a].kind, Kind::Str(Some(s)) if s == key);
        let sep = if py { b'=' } else { b':' };
        if !named {
            None
        } else if t.is(a + 1, sep) && !t.is(a + 2, b'=') {
            Some((a + 2, b))
        } else {
            (!py && b == a + 1).then_some((a, b))
        }
    })
}

/// `.indices.create(index="x", body=B)` and `.indices.create({ index: 'x', body: B })`.
fn keyword_call(t: &Toks, i: usize, py: bool, out: &mut Vec<Found>) {
    if !(t.is(i, b'.') && t.is(i + 2, b'.') && t.is(i + 4, b'(')) {
        return;
    }
    let (group, method) = (t.word(i + 1), t.word(i + 3));
    let Some(row) = KW
        .iter()
        .find(|r| r.0 == group && method == if py { r.1 } else { r.2 })
    else {
        return;
    };
    let api = format!("{group}.{method}");
    let Some(call_args) = t.args(i + 4) else {
        out.push(gap_call(t, i + 3, &api));
        return;
    };
    let args = if py {
        call_args
    } else {
        let Some(&(a, _)) = call_args.first().filter(|&&(a, _)| t.is(a, b'{')) else {
            return;
        };
        let Some(inner) = t.args(a) else {
            out.push(gap_call(t, i + 3, &api));
            return;
        };
        inner
    };
    let body = keyed(t, &args, "body", py).map(|r| classify(t, r));
    if let Some(body @ (Arg::Text(_) | Arg::Ref(_) | Arg::Gap)) = body {
        let name = keyed(t, &args, row.3, py).map_or(Name::Gap, |r| name_of(classify(t, r)));
        out.push(Found {
            name,
            body,
            line: t.line(i + 3),
            api,
        });
    }
}

/// `new CreateIndexRequest("x")` and the new client's `.indices().create(b -> b.index("x").withJson(...))`.
fn java_call(t: &Toks, i: usize, out: &mut Vec<Found>) {
    if !(t.is(i, b'.') && t.word(i + 1) == "indices" && t.is(i + 2, b'(') && t.is(i + 3, b')')) {
        return;
    }
    let Some(&(_, key)) = JAVA_METHODS.iter().find(|m| m.0 == t.word(i + 5)) else {
        return;
    };
    if !t.is(i + 4, b'.') || !t.is(i + 6, b'(') {
        return;
    }
    let api = format!("indices().{}", t.word(i + 5));
    let Some(span) = t.end(i + 6) else {
        out.push(gap_call(t, i + 5, &api));
        return;
    };
    let call = |name: &str| {
        (i + 7..span).find(|&j| t.is(j, b'.') && t.word(j + 1) == name && t.is(j + 2, b'('))
    };
    let Some(json) = call("withJson") else { return };
    let first = |at: usize| t.args(at + 2).and_then(|a| a.first().copied());
    let reader =
        first(json).filter(|&(a, _)| t.word(a) == "new" && t.word(a + 1) == "StringReader");
    let body = reader
        .and_then(|(a, _)| t.args(a + 2))
        .and_then(|a| a.first().copied())
        .map_or(Arg::Gap, |r| classify(t, r));
    let name = call(key)
        .and_then(first)
        .map_or(Name::Gap, |r| name_of(classify(t, r)));
    out.push(Found {
        name,
        body,
        line: t.line(i + 5),
        api,
    });
}

/// `new CreateIndexRequest("x").source(B, XContentType.JSON)`, chained or on the variable it is
/// assigned to.
fn java_request_calls(t: &Toks) -> Vec<Found> {
    let mut sources: HashMap<&str, Vec<usize>> = HashMap::new();
    for i in 1..t.t.len() {
        if t.is(i, b'.') && t.word(i + 1) == "source" && t.is(i + 2, b'(') {
            sources.entry(t.word(i - 1)).or_default().push(i + 2);
        }
    }
    let mut out = vec![];
    for i in 0..t.t.len() {
        if t.word(i) != "new" || !JAVA_REQUESTS.contains(&t.word(i + 1)) || !t.is(i + 2, b'(') {
            continue;
        }
        let api = format!("new {}", t.word(i + 1));
        let (Some(args), Some(close)) = (t.args(i + 2), t.end(i + 2)) else {
            out.push(gap_call(t, i + 1, &api));
            continue;
        };
        let chained = chain_source(t, close + 1);
        let held = t
            .is(i.wrapping_sub(1), b'=')
            .then(|| sources.get(t.word(i.wrapping_sub(2))))
            .flatten();
        let source = chained.or_else(|| held.and_then(|v| v.iter().copied().find(|&s| s > close)));
        let body = source
            .and_then(|s| t.args(s))
            .and_then(|a| a.first().copied());
        let Some(body) = body.map(|r| classify(t, r)) else {
            continue;
        };
        let name = args.first().map_or(Name::Gap, |&r| name_of(classify(t, r)));
        out.push(Found {
            name,
            body,
            line: t.line(i + 1),
            api,
        });
    }
    out
}

/// The `(` of `.source(` in the method chain that starts at `at`.
fn chain_source(t: &Toks, mut at: usize) -> Option<usize> {
    while t.is(at, b'.') && t.is(at + 2, b'(') {
        if t.word(at + 1) == "source" {
            return Some(at + 2);
        }
        at = t.end(at + 2)? + 1;
    }
    None
}

/// `low.Indices.Create<StringResponse>("x", PostData.String(B))` of the low-level client.
fn cs_call(t: &Toks, i: usize, out: &mut Vec<Found>) {
    if !(t.is(i, b'.') && t.word(i + 1) == "Indices" && t.is(i + 2, b'.')) {
        return;
    }
    let method = t.word(i + 3);
    if !CS_METHODS.contains(&method.trim_end_matches("Async")) {
        return;
    }
    let mut open = i + 4;
    if t.is(open, b'<') {
        let Some(gt) = (open..open + 16).find(|&j| t.is(j, b'>')) else {
            return;
        };
        open = gt + 1;
    }
    if !t.is(open, b'(') {
        return;
    }
    let api = format!("Indices.{method}");
    let Some(args) = t.args(open) else {
        out.push(gap_call(t, i + 3, &api));
        return;
    };
    let post = args.get(1).copied().filter(|&(a, _)| {
        t.word(a) == "PostData"
            && t.is(a + 1, b'.')
            && t.word(a + 2) == "String"
            && t.is(a + 3, b'(')
    });
    let Some((a, _)) = post else { return };
    let body = t
        .args(a + 3)
        .and_then(|v| v.first().copied())
        .map_or(Arg::Gap, |r| classify(t, r));
    let name = name_of(classify(t, args[0]));
    out.push(Found {
        name,
        body,
        line: t.line(i + 3),
        api,
    });
}

/// `NAME = "..."` where the string is the whole value and starts a JSON object: (name, text, line).
pub(super) fn json_constants(t: &Toks) -> Vec<(String, String, u32)> {
    let mut out = vec![];
    for i in 1..t.t.len() {
        let Kind::Str(Some(text)) = &t.t[i].kind else {
            continue;
        };
        if !t.is(i - 1, b'=') || !text.trim_start().starts_with('{') || !whole_value(t, i) {
            continue;
        }
        // `NAME: type = "..."` (TypeScript, Python) names the constant before the colon.
        let at = if t.is(i.wrapping_sub(3), b':') {
            i.wrapping_sub(4)
        } else {
            i.wrapping_sub(2)
        };
        // A keyword argument (`body='{...}'`) follows `(` or `,`; a declaration does not.
        let argument = t.is(at.wrapping_sub(1), b'(') || t.is(at.wrapping_sub(1), b',');
        let name = t.word(at);
        if !name.is_empty() && !argument {
            out.push((name.to_owned(), text.clone(), t.line(i)));
        }
    }
    out
}

/// Whether nothing is joined to the string literal at `i`: `+`, `.format`, `%`, a second literal.
fn whole_value(t: &Toks, i: usize) -> bool {
    match t.t.get(i + 1).map(|x| &x.kind) {
        None | Some(Kind::Punct(b';' | b',' | b')' | b'}')) => true,
        Some(Kind::Word(_)) => t.line(i + 1) > t.line(i),
        _ => false,
    }
}
