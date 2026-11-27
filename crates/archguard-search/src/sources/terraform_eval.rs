//! Evaluates the literal part of Terraform expressions for index detail (ADR 0023): literals,
//! `merge()`/`concat()`, `local.*` and sibling-index references up to depth 5 with cycle
//! protection, `jsondecode(file("${path.module}/<name>.json"))` below the project root, and
//! attribute/index access on resolved values. Everything else is `None`.
use super::terraform_locals::{local, lookup};
use super::terraform_scope::Scope;
use archguard_terraform::hcl::{attr, literal};
use hcl_edit::expr::{
    BinaryOperator, Expression, ForExpr, ObjectKey, Traversal, TraversalOperator, UnaryOperator,
};
use hcl_edit::parser;
use hcl_edit::template::{Element, Template};

/// A fully literal value; objects keep the written order.
#[derive(Clone)]
pub(super) enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub(super) fn compact(&self) -> String {
        let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
        match self {
            Json::Null => "null".into(),
            Json::Bool(b) => b.to_string(),
            Json::Num(n) => n.clone(),
            Json::Str(s) => quote(s),
            Json::Arr(a) => format!(
                "[{}]",
                a.iter().map(Json::compact).collect::<Vec<_>>().join(",")
            ),
            Json::Obj(o) => format!(
                "{{{}}}",
                o.iter()
                    .map(|(k, v)| format!("{}:{}", quote(k), v.compact()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }

    /// Object keys in byte order at every level, as Terraform's `jsonencode` writes them.
    pub(super) fn sorted(self) -> Json {
        match self {
            Json::Obj(mut o) => {
                o.sort_by(|a, b| a.0.cmp(&b.0));
                Json::Obj(o.into_iter().map(|(k, v)| (k, v.sorted())).collect())
            }
            Json::Arr(a) => Json::Arr(a.into_iter().map(Json::sorted).collect()),
            other => other,
        }
    }

    pub(super) fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(o) => o.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(super) fn str(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Json::Str(s) => Some(s.clone()),
            _ => None,
        }
    }
}

/// Reference hops allowed from an attribute: index -> local x4 is depth 5. Raised from 3 on
/// 2026-11-26 (ticket 30) for mappings built from chained locals.
const MAX_HOPS: usize = 4;

/// Elements one attribute may iterate over in `for` expressions before it is a gap.
const MAX_FOR: usize = 10_000;

/// Module instance, the addresses of the expressions followed so far (the path length is the
/// depth, a repeat is a cycle), the variables bound by enclosing `for` expressions and the
/// number of `for` elements evaluated.
#[derive(Default)]
struct Cx<'a, 'c> {
    scope: Option<&'c Scope<'a, 'c>>,
    path: Vec<usize>,
    env: Vec<(String, Json)>,
    used: usize,
}

fn object(o: &hcl_edit::expr::Object, cx: &mut Cx) -> Option<Vec<(String, Json)>> {
    o.iter()
        .map(|(k, v)| {
            let key = match k {
                ObjectKey::Ident(i) => i.as_str().to_owned(),
                ObjectKey::Expression(k) => literal(k)?,
            };
            Some((key, tree(v.expr(), cx)?))
        })
        .collect()
}

/// Reads the expression a reference points to, once per path entry and at most `MAX_HOPS` deep.
fn guard<T>(
    found: &Expression,
    cx: &mut Cx,
    read: impl FnOnce(&Expression, &mut Cx) -> Option<T>,
) -> Option<T> {
    let id = std::ptr::from_ref(found) as usize;
    if cx.path.len() >= MAX_HOPS || cx.path.contains(&id) {
        return None;
    }
    cx.path.push(id);
    let out = read(found, cx);
    cx.path.pop();
    out
}

/// Follows a `local.*` or sibling-index reference and reads what it points to with `read`.
fn follow<T>(
    e: &Expression,
    cx: &mut Cx,
    read: impl FnOnce(&Expression, &mut Cx) -> Option<T>,
) -> Option<T> {
    let found = lookup(cx.scope?.module, e)?;
    guard(found, cx, read)
}

/// `merge()` of objects (shallow, the right value wins) or `concat()` of lists, all arguments
/// literal.
fn call(c: &hcl_edit::expr::FuncCall, cx: &mut Cx) -> Option<Json> {
    let name = c.name.name.as_str();
    if c.args.expand_final() {
        return None;
    }
    let mut objects: Vec<(String, Json)> = vec![];
    let mut items = vec![];
    for a in c.args.iter() {
        match (name, tree(a, cx)?) {
            ("merge", Json::Obj(o)) => {
                for (k, v) in o {
                    match objects.iter_mut().find(|(x, _)| *x == k) {
                        Some(slot) => slot.1 = v,
                        None => objects.push((k, v)),
                    }
                }
            }
            ("concat", Json::Arr(a)) => items.extend(a),
            _ => return None,
        }
    }
    Some(if name == "merge" {
        Json::Obj(objects)
    } else {
        Json::Arr(items)
    })
}

/// One `.attr`, `["key"]` or `[0]` step into a resolved value; a missing key is `None`.
fn step(j: Json, op: &TraversalOperator, cx: &mut Cx) -> Option<Json> {
    let key = match op {
        TraversalOperator::GetAttr(n) => Json::Str(n.as_str().to_owned()),
        TraversalOperator::LegacyIndex(n) => Json::Num(n.value().to_string()),
        TraversalOperator::Index(e) => tree(e, cx)?,
        _ => return None,
    };
    match (j, key) {
        (Json::Obj(o), Json::Str(k)) => o.into_iter().find(|(x, _)| *x == k).map(|x| x.1),
        (Json::Arr(a), Json::Num(i)) => a.into_iter().nth(i.parse().ok()?),
        _ => None,
    }
}

/// `path.module`: the module directory relative to the root, as `file()` reads it.
fn path_module(op: &TraversalOperator, cx: &Cx) -> Option<Json> {
    let TraversalOperator::GetAttr(n) = op else {
        return None;
    };
    let dir = cx.scope?.dir;
    (n.as_str() == "module").then(|| {
        Json::Str(if dir.is_empty() {
            ".".into()
        } else {
            dir.into()
        })
    })
}

/// A traversal of a resolved value: a call result, a literal or a local, then access steps.
fn access(t: &Traversal, cx: &mut Cx) -> Option<Json> {
    let mut ops = t.operators.iter();
    let base = match &t.expr {
        Expression::Variable(v) if v.as_str() == "local" => {
            let TraversalOperator::GetAttr(n) = ops.next()?.value() else {
                return None;
            };
            let found = local(cx.scope?.module, n.as_str())?;
            guard(found, cx, tree)?
        }
        Expression::Variable(v) if v.as_str() == "path" => path_module(ops.next()?.value(), cx)?,
        other => tree(other, cx)?,
    };
    ops.try_fold(base, |j, o| step(j, o.value(), cx))
}

/// `==`, `!=`, `&&`, `||` and `!` on resolved values, enough for `for ... if` filters.
fn op(e: &Expression, cx: &mut Cx) -> Option<Json> {
    match e {
        Expression::UnaryOp(u) if matches!(u.operator.value(), UnaryOperator::Not) => {
            match tree(&u.expr, cx)? {
                Json::Bool(b) => Some(Json::Bool(!b)),
                _ => None,
            }
        }
        Expression::BinaryOp(b) => {
            let (l, r) = (tree(&b.lhs_expr, cx)?, tree(&b.rhs_expr, cx)?);
            Some(Json::Bool(match (b.operator.value(), l, r) {
                (BinaryOperator::Eq, l, r) => l.compact() == r.compact(),
                (BinaryOperator::NotEq, l, r) => l.compact() != r.compact(),
                (BinaryOperator::And, Json::Bool(x), Json::Bool(y)) => x && y,
                (BinaryOperator::Or, Json::Bool(x), Json::Bool(y)) => x || y,
                _ => return None,
            }))
        }
        _ => None,
    }
}

/// One element of a `for`: `Some(None)` when the `if` filter drops it, `None` when not literal.
fn entry(f: &ForExpr, cx: &mut Cx) -> Option<Option<(String, Json)>> {
    if let Some(c) = &f.cond {
        match tree(&c.expr, cx)? {
            Json::Bool(true) => {}
            Json::Bool(false) => return Some(None),
            _ => return None,
        }
    }
    let key = match f.key_expr.as_ref().map(|k| tree(k, cx)) {
        None => String::new(),
        Some(Some(Json::Str(s) | Json::Num(s))) => s,
        Some(_) => return None,
    };
    Some(Some((key, tree(&f.value_expr, cx)?)))
}

/// `[for x in <literal> : v if c]` or `{for k, v in <literal> : k => e if c}`; grouping (`...`),
/// duplicate keys and more than `MAX_FOR` elements are gaps.
fn for_expr(f: &ForExpr, cx: &mut Cx) -> Option<Json> {
    let items: Vec<(Json, Json)> = match tree(&f.intro.collection_expr, cx)? {
        Json::Obj(mut o) => {
            // Terraform iterates a map in lexical key order
            o.sort_by(|a, b| a.0.cmp(&b.0));
            o.into_iter().map(|(k, v)| (Json::Str(k), v)).collect()
        }
        Json::Arr(a) => a
            .into_iter()
            .enumerate()
            .map(|(i, v)| (Json::Num(i.to_string()), v))
            .collect(),
        _ => return None,
    };
    cx.used += items.len();
    if f.grouping || cx.used > MAX_FOR {
        return None;
    }
    let (mark, mut out) = (cx.env.len(), vec![]);
    for (k, v) in items {
        if let Some(kv) = &f.intro.key_var {
            cx.env.push((kv.as_str().to_owned(), k));
        }
        cx.env.push((f.intro.value_var.as_str().to_owned(), v));
        let r = entry(f, cx);
        cx.env.truncate(mark);
        match r? {
            None => {}
            Some(e) if f.key_expr.is_some() && out.iter().any(|x: &(String, Json)| x.0 == e.0) => {
                return None;
            }
            Some(e) => out.push(e),
        }
    }
    Some(if f.key_expr.is_some() {
        Json::Obj(out)
    } else {
        Json::Arr(out.into_iter().map(|e| e.1).collect())
    })
}

/// A template whose interpolations are all resolved strings or numbers; directives are gaps.
fn template(t: &Template, cx: &mut Cx) -> Option<Json> {
    let mut out = String::new();
    for el in t.iter() {
        match el {
            Element::Literal(l) => out.push_str(l.as_str()),
            Element::Interpolation(i) => match tree(&i.expr, cx)? {
                Json::Str(s) | Json::Num(s) => out.push_str(&s),
                _ => return None,
            },
            Element::Directive(_) => return None,
        }
    }
    Some(Json::Str(out))
}

fn scalar(e: &Expression) -> Option<Json> {
    Some(match e {
        Expression::Null(_) => Json::Null,
        Expression::Bool(b) => Json::Bool(*b.value()),
        Expression::Number(n) => Json::Num(n.value().to_string()),
        Expression::String(s) => Json::Str(s.value().clone()),
        _ => return None,
    })
}

fn func(c: &hcl_edit::expr::FuncCall, cx: &mut Cx) -> Option<Json> {
    match (c.name.name.as_str(), c.args.iter().next()) {
        ("merge" | "concat", _) => call(c, cx),
        ("jsondecode", Some(a)) => json_at(a, cx),
        ("jsonencode", Some(a)) => Some(Json::Str(tree(a, cx)?.sorted().compact())),
        _ => None,
    }
}

fn tree(e: &Expression, cx: &mut Cx) -> Option<Json> {
    match e {
        Expression::Array(a) => Some(Json::Arr(
            a.iter().map(|x| tree(x, cx)).collect::<Option<_>>()?,
        )),
        Expression::Object(o) => Some(Json::Obj(object(o, cx)?)),
        Expression::StringTemplate(t) => template(t, cx),
        Expression::HeredocTemplate(h) => template(&h.template, cx),
        Expression::Parenthesis(p) => tree(p.inner(), cx),
        Expression::FuncCall(c) => func(c, cx),
        Expression::Traversal(t) => follow(e, cx, tree).or_else(|| access(t, cx)),
        Expression::Variable(v) => cx
            .env
            .iter()
            .rfind(|x| x.0 == v.as_str())
            .map(|x| x.1.clone()),
        Expression::ForExpr(f) => for_expr(f, cx),
        Expression::UnaryOp(_) | Expression::BinaryOp(_) => op(e, cx),
        _ => scalar(e),
    }
}

/// The literal value of `e` (no JSON text parsing), `None` when any part is not literal.
pub(super) fn value(e: &Expression, scope: Option<&Scope>) -> Option<Json> {
    tree(
        e,
        &mut Cx {
            scope,
            ..Cx::default()
        },
    )
}

/// A string value of `e` (local or `file()` JSON allowed), the one rule for names of all artifacts.
pub(super) fn string(e: &Expression, scope: &Scope) -> Option<String> {
    match value(e, Some(scope))? {
        Json::Str(s) => Some(s),
        _ => None,
    }
}

/// `jsonencode(<literal>)` or a literal JSON text (string or heredoc). JSON is read as HCL,
/// whose object syntax accepts it; `var.*` and unresolved references are not literal.
pub(super) fn json_attr(e: &Expression, scope: Option<&Scope>) -> Option<Json> {
    json_at(
        e,
        &mut Cx {
            scope,
            ..Cx::default()
        },
    )
}

/// A `jsonencode` reached through a reference is the text Terraform would send: sorted.
fn json_ref(f: &Expression, cx: &mut Cx) -> Option<Json> {
    let j = json_at(f, cx)?;
    Some(match f {
        Expression::FuncCall(c) if c.name.name.as_str() == "jsonencode" => j.sorted(),
        _ => j,
    })
}

fn json_at(e: &Expression, cx: &mut Cx) -> Option<Json> {
    if let Expression::Traversal(t) = e {
        return follow(e, cx, json_ref).or_else(|| access(t, cx));
    }
    let text = if let Expression::FuncCall(c) = e {
        let mut args = c.args.iter();
        match (c.name.name.as_str(), args.next(), args.next()) {
            ("jsonencode", Some(a), None) => return tree(a, cx),
            ("jsondecode", Some(a), None) => return json_at(a, cx),
            ("file", Some(a), None) => match tree(a, cx)? {
                Json::Str(rel) => cx.scope?.file(&rel)?,
                _ => return None,
            },
            _ => return None,
        }
    } else {
        literal(e).filter(|_| !matches!(e, Expression::Number(_) | Expression::Bool(_)))?
    };
    let body = parser::parse_body(&format!("x = {text}")).ok()?;
    tree(attr(&body, "x")?, &mut Cx::default())
}
