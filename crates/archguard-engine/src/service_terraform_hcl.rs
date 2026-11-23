//! Reading literal values and references from HCL expressions (ADR 0015).
use hcl_edit::expr::{Expression, ObjectKey, Traversal, TraversalOperator};
use hcl_edit::structure::Body;
use hcl_edit::visit::{self, Visit};
use std::collections::BTreeMap;

pub(super) fn attr<'b>(body: &'b Body, key: &str) -> Option<&'b Expression> {
    body.get_attribute(key).map(|a| &a.value)
}

pub(super) fn block<'b>(body: &'b Body, ident: &'b str) -> Option<&'b Body> {
    body.get_blocks(ident).next().map(|b| &b.body)
}

/// A literal string, number or bool, or a template without interpolation.
pub(crate) fn literal(expr: &Expression) -> Option<String> {
    match expr {
        Expression::String(s) => Some(s.value().clone()),
        Expression::Number(n) => Some(n.value().to_string()),
        Expression::Bool(b) => Some(b.value().to_string()),
        Expression::StringTemplate(t) => t
            .iter()
            .map(|e| e.as_literal().map(|l| l.as_str()))
            .collect(),
        Expression::HeredocTemplate(h) => h
            .template
            .iter()
            .map(|e| e.as_literal().map(|l| l.as_str()))
            .collect(),
        _ => None,
    }
}

/// Entries of an object expression with literal keys.
pub(crate) fn object(expr: &Expression) -> Vec<(String, &Expression)> {
    let Expression::Object(object) = expr else {
        return vec![];
    };
    object
        .iter()
        .filter_map(|(k, v)| {
            let key = match k {
                ObjectKey::Ident(i) => i.as_str().to_owned(),
                ObjectKey::Expression(e) => literal(e)?,
            };
            Some((key, v.expr()))
        })
        .collect()
}

pub(crate) fn get<'e>(expr: &'e Expression, key: &str) -> Option<&'e Expression> {
    object(expr)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

pub(super) fn labels(expr: Option<&Expression>) -> BTreeMap<String, String> {
    expr.map(object)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| Some((k, literal(v)?)))
        .collect()
}

/// Paths of the references in an expression, e.g. `["aws_db_instance", "orders", "address"]`.
#[derive(Default)]
pub(super) struct References(pub Vec<Vec<String>>);

impl Visit for References {
    fn visit_traversal(&mut self, node: &Traversal) {
        if let Expression::Variable(root) = &node.expr {
            let mut path = vec![root.as_str().to_owned()];
            path.extend(node.operators.iter().map_while(|op| match op.value() {
                TraversalOperator::GetAttr(name) => Some(name.as_str().to_owned()),
                _ => None,
            }));
            self.0.push(path);
        }
        visit::visit_traversal(self, node);
    }
}

pub(super) fn http(expr: &Expression) -> bool {
    let start = match expr {
        Expression::StringTemplate(t) => t
            .iter()
            .next()
            .and_then(|e| e.as_literal())
            .map(|l| l.as_str()),
        _ => None,
    };
    start.is_some_and(|s| s.starts_with("http://") || s.starts_with("https://"))
}
