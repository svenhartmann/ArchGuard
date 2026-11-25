//! Form of the first expression that blocks reading an index attribute or a watch input; counts only.
use hcl_edit::expr::{Expression, ObjectKey, TraversalOperator};
use hcl_edit::structure::Body;
use hcl_edit::template::Element;
use std::collections::BTreeMap;
use std::path::Path;

pub type Counts = BTreeMap<String, usize>;
const INDEX: &str = "elasticstack_elasticsearch_index";
/// Attributes of the index resource that are no settings, mapping or analysis.
const SKIP: &[&str] = &[
    "name",
    "alias",
    "deletion_protection",
    "master_timeout",
    "timeout",
    "wait_for_active_shards",
    "settings_raw",
    "elasticsearch_connection",
    "sort_order",
];

/// Locals and resources of one directory (one module).
struct Ctx {
    locals: Vec<(String, Expression)>,
    res: Vec<(String, String, Body)>,
}

fn attrs(b: &Body) -> impl Iterator<Item = (&str, &Expression)> {
    b.attributes().map(|a| (a.key.as_str(), &a.value))
}

fn ctx_of(bodies: &[Body]) -> Ctx {
    let mut c = Ctx {
        locals: vec![],
        res: vec![],
    };
    for b in bodies {
        for l in b.get_blocks("locals") {
            c.locals
                .extend(attrs(&l.body).map(|(k, v)| (k.to_owned(), v.clone())));
        }
        for r in b.get_blocks("resource") {
            if let [k, n] = r.labels.as_slice() {
                c.res
                    .push((k.as_str().into(), n.as_str().into(), r.body.clone()));
            }
        }
    }
    c
}

/// Literal text of a string or heredoc; `Err(label)` when it interpolates.
fn text(e: &Expression) -> Result<String, &'static str> {
    let (els, label): (Vec<&Element>, _) = match e {
        Expression::String(s) => return Ok(s.value().clone()),
        Expression::StringTemplate(t) => (t.iter().collect(), "${ in string"),
        Expression::HeredocTemplate(h) => (h.template.iter().collect(), "heredoc interpolation"),
        _ => return Err("other"),
    };
    els.iter()
        .map(|x| x.as_literal().map(|l| l.value().as_str()).ok_or(label))
        .collect()
}

fn traversal_root(e: &Expression) -> Option<(&str, Vec<&str>)> {
    let Expression::Traversal(t) = e else {
        return None;
    };
    let Expression::Variable(v) = &t.expr else {
        return None;
    };
    let keys = t.operators.iter().map_while(|o| match o.value() {
        TraversalOperator::GetAttr(n) => Some(n.as_str()),
        _ => None,
    });
    Some((v.as_str(), keys.collect()))
}

fn other(e: &Expression) -> String {
    let d = format!("{e:?}");
    format!("other:{}", d.split(['(', ' ', '{']).next().unwrap_or("?"))
}

/// First blocking form of `e`, `None` when it is literal. `depth` 0 follows one local or sibling.
fn form(e: &Expression, c: &Ctx, depth: u8) -> Option<String> {
    match e {
        Expression::Null(_)
        | Expression::Bool(_)
        | Expression::Number(_)
        | Expression::String(_) => None,
        Expression::StringTemplate(_) | Expression::HeredocTemplate(_) => {
            text(e).err().map(Into::into)
        }
        Expression::Array(a) => a.iter().find_map(|x| form(x, c, depth)),
        Expression::Object(o) => o.iter().find_map(|(_, v)| form(v.expr(), c, depth)),
        Expression::FuncCall(f) => match f.name.name.as_str() {
            "jsonencode" | "jsondecode" => f.args.iter().find_map(|x| form(x, c, depth)),
            n => Some(format!("function {n}")),
        },
        Expression::Traversal(_) => Some(traversal_form(e, c, depth)).filter(|f| f != "-"),
        _ => Some(other(e)),
    }
}

fn traversal_form(e: &Expression, c: &Ctx, depth: u8) -> String {
    let Some((root, keys)) = traversal_root(e) else {
        return other(e);
    };
    let found = match (root, keys.as_slice()) {
        ("local", [k, ..]) => c.locals.iter().find(|(n, _)| n == k).map(|x| &x.1),
        (INDEX, [n, a, ..]) => c
            .res
            .iter()
            .find(|r| r.0 == INDEX && r.1 == *n)
            .and_then(|r| attrs(&r.2).find(|x| x.0 == *a))
            .map(|x| x.1),
        _ => None,
    };
    let kind = match root {
        "local" | "var" | "each" | "module" => root.to_owned(),
        r if r == INDEX => "same-module index resource".into(),
        r => format!("resource {r}"),
    };
    match (found, depth) {
        (Some(x), 0) => form(x, c, 1).map_or("-".into(), |f| format!("{kind} -> {f}")),
        (None, 0) if root == "local" || root == INDEX => format!("{kind} -> unresolved"),
        _ => kind,
    }
}

fn bump(m: &mut Counts, k: String) {
    *m.entry(k).or_default() += 1;
}

/// Index attributes by group and blocking form.
fn index_attrs(b: &Body, c: &Ctx, out: &mut Counts) {
    for (k, v) in attrs(b).filter(|(k, _)| !SKIP.contains(k)) {
        let group = match k {
            "mappings" => "mappings",
            _ if k.starts_with("analysis_") => "analysis",
            _ => "settings",
        };
        if let Some(f) = form(v, c, 0) {
            bump(out, format!("index {group}: {f}"));
        }
    }
}

fn key_is(k: &ObjectKey, name: &str) -> bool {
    match k {
        ObjectKey::Ident(i) => i.as_str() == name,
        ObjectKey::Expression(Expression::String(s)) => s.value() == name,
        ObjectKey::Expression(_) => false,
    }
}

/// The value at `path` of an object, a `jsonencode`d object or a JSON text.
fn dig(e: &Expression, path: &[&str]) -> Option<Expression> {
    let Some((first, rest)) = path.split_first() else {
        return Some(e.clone());
    };
    let obj = match e {
        Expression::FuncCall(f) => return dig(f.args.iter().next()?, path),
        Expression::Object(o) => o.clone(),
        _ => {
            let body = hcl_edit::parser::parse_body(&format!("x = {}", text(e).ok()?)).ok()?;
            let Expression::Object(o) = body.get_attribute("x")?.value.clone() else {
                return None;
            };
            o
        }
    };
    let v = obj.iter().find(|(k, _)| key_is(k, first))?.1.expr().clone();
    dig(&v, rest)
}

fn watch_input(b: &Body, c: &Ctx, out: &mut Counts) {
    let Some((_, e)) = attrs(b).find(|a| a.0 == "input") else {
        return bump(out, "watch input missing".into());
    };
    let Some(f) = form(e, c, 0) else {
        let hit = dig(e, &["search"]).is_some();
        return bump(
            out,
            format!(
                "watch input literal, {}",
                if hit { "with search" } else { "without search" }
            ),
        );
    };
    bump(out, format!("watch input blocked: {f}"));
    if let Some(i) = dig(e, &["search", "request", "indices"]) {
        let inner = form(&i, c, 0).unwrap_or_else(|| "literal".into());
        bump(out, format!("watch indices present, form: {inner}"));
    }
}

fn tf_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let n = e.file_name();
        if p.is_dir() && !n.to_string_lossy().starts_with('.') && n != "node_modules" {
            tf_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "tf") {
            out.push(p);
        }
    }
}

/// Counts per blocking form, plus `index blocks` and `watch blocks` read from the sources.
pub fn breakdown(root: &Path) -> Counts {
    let mut files = vec![];
    tf_files(root, &mut files);
    let mut by_dir: BTreeMap<_, Vec<Body>> = BTreeMap::new();
    for f in files {
        let parsed = std::fs::read_to_string(&f)
            .ok()
            .and_then(|s| hcl_edit::parser::parse_body(&s).ok());
        by_dir
            .entry(f.parent().map(Path::to_path_buf))
            .or_default()
            .extend(parsed);
    }
    let mut out = Counts::new();
    for bodies in by_dir.values() {
        let c = ctx_of(bodies);
        for (kind, _, b) in &c.res {
            match kind.as_str() {
                INDEX => {
                    bump(&mut out, "index blocks (sources)".into());
                    index_attrs(b, &c, &mut out);
                }
                "elasticstack_elasticsearch_watch" => {
                    bump(&mut out, "watch blocks (sources)".into());
                    watch_input(b, &c, &mut out);
                }
                _ => {}
            }
        }
    }
    out
}
