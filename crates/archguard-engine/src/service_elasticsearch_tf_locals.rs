//! One-level lookup of `local.<name>[.<key>…]` and `<index resource>.<name>.<attr>` inside the
//! module instance of an index (ADR 0023). The found expression is read as a literal; whatever it
//! references itself is not followed, so a local that uses another local is a gap.
use super::service_terraform::ModuleCtx;
use super::service_terraform_hcl::{attr, get};
use hcl_edit::expr::{Expression, TraversalOperator};

fn local<'a>(ctx: &ModuleCtx<'a>, name: &str) -> Option<&'a Expression> {
    ctx.locals.iter().find_map(|b| attr(&b.body, name))
}

fn sibling<'a>(ctx: &ModuleCtx<'a>, kind: &str, name: &str, key: &str) -> Option<&'a Expression> {
    let r = ctx.resources.iter().find(
        |b| matches!(b.labels.as_slice(), [k, n] if k.as_str() == kind && n.as_str() == name),
    )?;
    attr(&r.body, key)
}

pub(super) fn lookup<'a>(ctx: &ModuleCtx<'a>, e: &Expression) -> Option<&'a Expression> {
    let Expression::Traversal(t) = e else {
        return None;
    };
    let Expression::Variable(root) = &t.expr else {
        return None;
    };
    let mut keys = t.operators.iter().map(|o| match o.value() {
        TraversalOperator::GetAttr(n) => Some(n.as_str()),
        _ => None,
    });
    let first = keys.next()??;
    // resource references stop at the attribute; local objects are walked key by key
    let found = match root.as_str() {
        "local" => local(ctx, first)?,
        kind @ "elasticstack_elasticsearch_index" => sibling(ctx, kind, first, keys.next()??)?,
        _ => return None,
    };
    keys.try_fold(found, |e, k| get(e, k?))
}
