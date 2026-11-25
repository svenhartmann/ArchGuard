//! Evaluates the literal part of Terraform expressions for index detail (ADR 0023): literals,
//! `merge()`/`concat()`, `local.*` and sibling-index references up to depth 3 with cycle
//! protection, and attribute/index access on resolved values. Everything else is `None`.
use super::service_elasticsearch_tf_locals::{local, lookup};
use super::service_terraform::ModuleCtx;
use super::service_terraform_hcl::{attr, literal};
use hcl_edit::expr::{Expression, ObjectKey, Traversal, TraversalOperator};
use hcl_edit::parser;

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

/// Reference hops allowed from an attribute: index -> local -> local is depth 3.
const MAX_HOPS: usize = 2;

/// Module instance and the addresses of the expressions followed so far; the path length is the
/// depth, a repeat is a cycle.
struct Cx<'a, 'c> {
    ctx: Option<&'c ModuleCtx<'a>>,
    path: Vec<usize>,
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
    let found = lookup(cx.ctx?, e)?;
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

/// A traversal of a resolved value: a call result, a literal or a local, then access steps.
fn access(t: &Traversal, cx: &mut Cx) -> Option<Json> {
    let mut ops = t.operators.iter();
    let base = match &t.expr {
        Expression::Variable(v) if v.as_str() == "local" => {
            let TraversalOperator::GetAttr(n) = ops.next()?.value() else {
                return None;
            };
            let found = local(cx.ctx?, n.as_str())?;
            guard(found, cx, tree)?
        }
        Expression::Variable(_) => return None,
        other => tree(other, cx)?,
    };
    ops.try_fold(base, |j, o| step(j, o.value(), cx))
}

fn scalar(e: &Expression) -> Option<Json> {
    Some(match e {
        Expression::Null(_) => Json::Null,
        Expression::Bool(b) => Json::Bool(*b.value()),
        Expression::Number(n) => Json::Num(n.value().to_string()),
        Expression::String(_) | Expression::StringTemplate(_) | Expression::HeredocTemplate(_) => {
            Json::Str(literal(e)?)
        }
        _ => return None,
    })
}

fn func(c: &hcl_edit::expr::FuncCall, cx: &mut Cx) -> Option<Json> {
    match (c.name.name.as_str(), c.args.iter().next()) {
        ("merge" | "concat", _) => call(c, cx),
        ("jsondecode", Some(a)) => json_at(a, cx),
        _ => None,
    }
}

fn tree(e: &Expression, cx: &mut Cx) -> Option<Json> {
    match e {
        Expression::Array(a) => Some(Json::Arr(
            a.iter().map(|x| tree(x, cx)).collect::<Option<_>>()?,
        )),
        Expression::Object(o) => Some(Json::Obj(object(o, cx)?)),
        Expression::Parenthesis(p) => tree(p.inner(), cx),
        Expression::FuncCall(c) => func(c, cx),
        Expression::Traversal(t) => follow(e, cx, tree).or_else(|| access(t, cx)),
        _ => scalar(e),
    }
}

/// The literal value of `e` (no JSON text parsing), `None` when any part is not literal.
pub(super) fn value(e: &Expression, ctx: Option<&ModuleCtx>) -> Option<Json> {
    tree(e, &mut Cx { ctx, path: vec![] })
}

/// `jsonencode(<literal>)` or a literal JSON text (string or heredoc). JSON is read as HCL,
/// whose object syntax accepts it; `file()`, `var.*` and unresolved references are not literal.
pub(super) fn json_attr(e: &Expression, ctx: Option<&ModuleCtx>) -> Option<Json> {
    json_at(e, &mut Cx { ctx, path: vec![] })
}

fn json_at(e: &Expression, cx: &mut Cx) -> Option<Json> {
    if let Expression::Traversal(t) = e {
        return follow(e, cx, json_at).or_else(|| access(t, cx));
    }
    if let Expression::FuncCall(c) = e {
        let mut args = c.args.iter();
        return match (c.name.name.as_str(), args.next(), args.next()) {
            ("jsonencode", Some(a), None) => tree(a, cx),
            ("jsondecode", Some(a), None) => json_at(a, cx),
            _ => None,
        };
    }
    let text = literal(e).filter(|_| !matches!(e, Expression::Number(_) | Expression::Bool(_)))?;
    let body = parser::parse_body(&format!("x = {text}")).ok()?;
    tree(
        attr(&body, "x")?,
        &mut Cx {
            ctx: None,
            path: vec![],
        },
    )
}
