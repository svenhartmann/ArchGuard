//! HTTP calls of JavaScript front ends shared by the React and Vue extractors
//! (ADR 0021): `fetch`, `axios` and `axios.create` instances with a literal
//! `baseURL`.
use std::collections::BTreeMap;

use archguard_core::Call;

use crate::lang_text::{find_top, matching};
use crate::lang_ts::{
    Item, Parser, Value, close, fill, ident_at, is_ident_char, skip_generic, skip_ws,
    trailing_ident, url_path, words,
};
use crate::lang_ts_decl::{declared_value, imports};

/// Request methods as `axios` and the Next.js and Nuxt route files name them.
pub(crate) const HTTP_METHODS: [&str; 7] =
    ["get", "post", "put", "patch", "delete", "head", "options"];

/// An instance's `baseURL` path, or its source when not literal.
pub(crate) type Base = Result<String, String>;

/// Axios instances by (file, exported name or `default`).
pub(crate) struct Clients {
    instances: BTreeMap<(String, String), Base>,
}

pub(crate) fn axios_names(text: &str) -> Vec<String> {
    imports(text)
        .into_iter()
        .filter(|i| i.module == "axios" && i.imported.as_deref().is_none_or(|n| n == "*"))
        .map(|i| i.local)
        .collect()
}

/// The `baseURL` path of an `axios.create(…)` argument.
pub(crate) fn base_of(text: &str, arg: &Item) -> Base {
    match &arg.value {
        Value::Obj(fields) => match fields.iter().find(|(k, _)| k == "baseURL") {
            None => Ok(String::new()),
            Some((_, v)) => v
                .value
                .literal()
                .map(|u| url_path(&u).trim_end_matches('/').to_string())
                .ok_or_else(|| v.source(text)),
        },
        Value::Raw(r) if r.is_empty() => Ok(String::new()),
        _ => Err(arg.source(text)),
    }
}

/// The name `axios.create` at `pos` is bound to: `const name =` or `export default`.
pub(crate) fn instance_name(text: &str, pos: usize) -> Option<String> {
    let before = text[..pos].trim_end();
    match before.strip_suffix('=') {
        Some(lhs) => trailing_ident(lhs.trim_end()),
        None => before.ends_with("export default").then(|| "default".into()),
    }
}

/// `const name = axios.create({ baseURL })` and `export default axios.create(…)`.
pub(crate) fn instances(text: &str) -> BTreeMap<String, Base> {
    let mut out = BTreeMap::new();
    for a in axios_names(text) {
        for pos in words(text, &a) {
            let at = pos + a.len();
            if !text[at..].starts_with(".create(") {
                continue;
            }
            let arg = Parser {
                s: text,
                i: at + ".create(".len(),
            }
            .value(0);
            if let Some(name) = instance_name(text, pos) {
                out.insert(name, base_of(text, &arg));
            }
        }
    }
    let exported = words(text, "default").into_iter().find_map(|pos| {
        let name = ident_at(text, skip_ws(text.as_bytes(), pos + "default".len()));
        let export = text[..pos].trim_end().ends_with("export");
        export.then(|| out.get(name).cloned()).flatten()
    });
    if let Some(base) = exported {
        out.insert("default".into(), base);
    }
    out
}

impl Clients {
    /// The instances declared in `files` (path to text).
    pub(crate) fn new(files: &BTreeMap<String, String>) -> Self {
        let mut instances = BTreeMap::new();
        for (file, text) in files {
            for (name, base) in self::instances(text) {
                instances.insert((file.clone(), name), base);
            }
        }
        Clients { instances }
    }

    /// HTTP receivers usable in `file`: axios itself (no base) and instances.
    /// `module` resolves an import specifier of `file` to a scanned file.
    pub(crate) fn receivers<'m>(
        &self,
        file: &str,
        text: &str,
        module: impl Fn(&str) -> Option<&'m str>,
    ) -> Vec<(String, Option<Base>)> {
        let mut out: Vec<(String, Option<Base>)> =
            axios_names(text).into_iter().map(|n| (n, None)).collect();
        for ((f, name), base) in &self.instances {
            if f == file && name != "default" {
                out.push((name.clone(), Some(base.clone())));
            }
        }
        for i in imports(text) {
            let Some(module) = module(&i.module) else {
                continue;
            };
            let name = i.imported.unwrap_or_else(|| "default".into());
            if let Some(base) = self.instances.get(&(module.to_string(), name)) {
                out.push((i.local, Some(base.clone())));
            }
        }
        out
    }
}

/// The method and URL argument of `recv.method(…)` at `pos`.
pub(crate) fn client_call(text: &str, pos: usize, recv: &str) -> Option<(&'static str, Item)> {
    let b = text.as_bytes();
    let rest = text[pos + recv.len()..].strip_prefix('.')?;
    let method = HTTP_METHODS.into_iter().find(|m| {
        rest.strip_prefix(m)
            .is_some_and(|r| !r.starts_with(is_ident_char))
    })?;
    let mut j = skip_ws(b, pos + recv.len() + 1 + method.len());
    if b.get(j) == Some(&b'<') {
        j = skip_ws(b, skip_generic(b, j));
    }
    (b.get(j) == Some(&b'(')).then(|| (method, Parser { s: text, i: j + 1 }.value(0)))
}

/// The call, or why it is unknown.
pub(crate) fn http_call(
    method: Result<String, String>,
    url: Option<String>,
    base: Option<&Base>,
) -> Result<Call, String> {
    let url = url.ok_or("URL is not a literal")?;
    let method = method.map_err(|m| format!("method is not a literal ({m})"))?;
    let prefix = match base {
        Some(Err(b)) => return Err(format!("baseURL is not a literal ({b})")),
        Some(Ok(p)) if !url.contains("://") => p.as_str(),
        _ => "",
    };
    Ok(Call::Http {
        method,
        path: format!("{prefix}{}", url_path(&url)),
    })
}

/// Whether the call at `pos` is the global function, not a method or declaration.
pub(crate) fn global_call(text: &str, pos: usize) -> bool {
    let before = &text[..pos];
    let member =
        before.ends_with('.') && !before.ends_with("window.") && !before.ends_with("globalThis.");
    let declared = matches!(
        trailing_ident(before.trim_end()).as_deref(),
        Some("function" | "async")
    );
    !member && !declared
}

/// The method of a fetch options argument; `GET` without one.
pub(crate) fn fetch_method(text: &str, init: Option<&Item>) -> Result<String, String> {
    let Some(init) = init else {
        return Ok("GET".into());
    };
    let Value::Obj(fields) = &init.value else {
        return Err(init.source(text));
    };
    match fields.iter().find(|(k, _)| k == "method") {
        None => Ok("GET".into()),
        Some((_, v)) => match &v.value {
            Value::Str(m) => Ok(m.to_ascii_uppercase()),
            _ => Err(v.source(text)),
        },
    }
}

/// Calls that fetch with a function's own arguments; the first one in a body
/// is its request.
const FETCHERS: [&str; 4] = ["fetch", "fetchJson", "fetchText", "fetchBlob"];

/// The exported functions of a generated API client (oazapfts `fetch-client.ts`,
/// `sdk.gen.ts`) in `files` (path, text), by name, with their request: the first
/// `fetch`/`fetchJson`/`fetchText`/`fetchBlob` call gives the path (a final
/// `${…}` not after `/` is the query and dropped) and `method: "…"` (else `GET`),
/// prefixed with the literal `baseUrl` of an exported `defaults` object. A
/// function without such a call is left out; a non-literal one is `Err`.
pub(crate) fn generated_calls(
    files: &[(String, String)],
) -> BTreeMap<String, Result<Call, String>> {
    let base = files.iter().find_map(|(_, t)| generated_base(t));
    let base = base.as_deref().unwrap_or("");
    (files.iter())
        .flat_map(|(_, text)| exported_functions(text))
        .filter_map(|(name, body)| Some((name, generated_call(&body, base)?)))
        .collect()
}

/// The literal `baseUrl` of `export const defaults = { … }` (oazapfts).
fn generated_base(t: &str) -> Option<String> {
    let pos = (words(t, "defaults").into_iter())
        .find(|&p| t[..p].trim_end().ends_with("export const"))?;
    let open = pos + t[pos..].find('{')?;
    let fields = Parser { s: t, i: open }.object(0);
    fields.into_iter().find_map(|(k, v)| match v.value {
        Value::Str(s) if k == "baseUrl" => Some(s.trim_end_matches('/').to_string()),
        _ => None,
    })
}

/// Name and source of every exported function and arrow constant.
fn exported_functions(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for kw in ["function", "const"] {
        for pos in words(text, kw) {
            let before = text[..pos].trim_end();
            if !(before.ends_with("export") || before.ends_with("export async")) {
                continue;
            }
            let name = ident_at(text, skip_ws(text.as_bytes(), pos + kw.len()));
            let Some(body) = declared_value(text, name).map(|b| b.source(text)) else {
                continue;
            };
            if kw == "function" || find_top(&body, "=>").is_some() {
                found.push((name.to_string(), body));
            }
        }
    }
    found
}

/// The `(` of the first `fetch`, `fetchJson`, … call, type arguments skipped.
fn fetch_open(body: &str) -> Option<usize> {
    let b = body.as_bytes();
    (FETCHERS.iter())
        .flat_map(|f| words(body, f).into_iter().map(move |p| p + f.len()))
        .filter_map(|after| {
            let mut j = skip_ws(b, after);
            if b.get(j) == Some(&b'<') {
                j = skip_ws(b, matching(b, j)? + 1);
            }
            (b.get(j) == Some(&b'(')).then_some(j)
        })
        .min()
}

/// The first fetch call of a function body; none without one.
fn generated_call(body: &str, base: &str) -> Option<Result<Call, String>> {
    let open = fetch_open(body)?;
    let url = Parser {
        s: body,
        i: open + 1,
    }
    .value(0);
    let args = &body[open..close(body, open)];
    let method = match args.find("method:") {
        None => Ok("GET".to_string()),
        Some(m) => match (Parser {
            s: args,
            i: m + "method:".len(),
        })
        .value(0)
        .value
        {
            Value::Str(s) => Ok(s.to_ascii_uppercase()),
            _ => Err(format!("method is not a literal in {}", args.trim())),
        },
    };
    let path = match &url.value {
        Value::Str(s) => Some(s.clone()),
        // A final hole not after `/` is the query (`${QS.query(…)}`).
        Value::Tpl(t) => fill(
            match t.rfind("${") {
                Some(k) if k > 0 && !t[..k].ends_with('/') && t.ends_with('}') => &t[..k],
                _ => t.as_str(),
            },
            |_| None,
        ),
        _ => None,
    };
    Some(match (method, path) {
        (Ok(method), Some(path)) => Ok(Call::Http {
            method,
            path: format!("{base}{}", url_path(&path)),
        }),
        (Err(why), _) => Err(why),
        (_, None) => Err(format!("URL is not a literal: {}", url.source(body))),
    })
}
