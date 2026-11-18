//! GraphQL schema changes against the saved baseline (ADR 0014), in the
//! style of the OpenAPI comparison: breaking, review or info.
use crate::graphql_model::{
    GraphqlArgument, GraphqlChange, GraphqlContract, GraphqlField, GraphqlLink, GraphqlType, key,
};
use std::collections::{BTreeMap, BTreeSet};

struct Diff<'a> {
    out: Vec<GraphqlChange>,
    links: &'a [GraphqlLink],
    provider: Option<&'a String>,
}
impl Diff<'_> {
    fn add(&mut self, at: &str, kind: &str, severity: &str, message: String) {
        // Consumers are known for root fields only: `Query.user(id:)` → `Query.user`.
        let coordinate = at.split('(').next().unwrap_or(at);
        let consumers = self
            .links
            .iter()
            .filter(|l| Some(&l.provider) == self.provider)
            .filter(|l| l.root_fields.iter().any(|f| f == coordinate))
            .map(|l| l.consumer.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self.out.push(GraphqlChange {
            operation: at.into(),
            kind: kind.into(),
            severity: severity.into(),
            message,
            consumers,
        });
    }
    fn types(&mut self, old: &BTreeMap<String, GraphqlType>, new: &BTreeMap<String, GraphqlType>) {
        for (name, t) in old {
            let Some(n) = new.get(name) else {
                self.add(
                    name,
                    "type_removed",
                    "breaking",
                    format!("Type {name} was removed."),
                );
                continue;
            };
            if n.kind != t.kind {
                let message = format!("{name} changed from {} to {}.", t.kind, n.kind);
                self.add(name, "type_kind_changed", "breaking", message);
            }
            for (field, f) in &t.fields {
                let at = format!("{name}.{field}");
                match n.fields.get(field) {
                    Some(g) => self.field(&at, f, g),
                    None => {
                        let message = format!("Field or value {at} was removed.");
                        self.add(&at, "field_removed", "breaking", message);
                    }
                }
            }
            for field in n.fields.keys().filter(|f| !t.fields.contains_key(*f)) {
                let at = format!("{name}.{field}");
                self.add(&at, "field_added", "info", format!("{at} was added."));
            }
        }
    }
    fn field(&mut self, at: &str, f: &GraphqlField, g: &GraphqlField) {
        if g.ty != f.ty {
            let message = format!(
                "{at} type changed from {} to {}; compatibility needs review.",
                f.ty, g.ty
            );
            self.add(at, "field_type_changed", "review", message);
        }
        for (arg, a) in &f.arguments {
            let at = format!("{at}({arg}:)");
            if let Some((kind, severity, message)) = argument(&at, a, g.arguments.get(arg)) {
                self.add(&at, kind, severity, message);
            }
        }
        for (arg, b) in &g.arguments {
            if !f.arguments.contains_key(arg) && b.ty.ends_with('!') && !b.has_default {
                let at = format!("{at}({arg}:)");
                let message = format!("New required argument {at}.");
                self.add(&at, "required_argument_added", "breaking", message);
            }
        }
    }
}
fn argument(
    at: &str,
    a: &GraphqlArgument,
    b: Option<&GraphqlArgument>,
) -> Option<(&'static str, &'static str, String)> {
    let Some(b) = b else {
        return Some((
            "argument_removed",
            "breaking",
            format!("Argument {at} was removed."),
        ));
    };
    let (from, to) = (&a.ty, &b.ty);
    if *to == format!("{from}!") && !b.has_default {
        let message = format!("Argument {at} became non-null ({from} → {to}).");
        Some(("argument_nullability_changed", "breaking", message))
    } else if *from == format!("{to}!") {
        let message = format!("Argument {at} became nullable ({from} → {to}).");
        Some(("argument_nullability_changed", "info", message))
    } else if to != from {
        let message =
            format!("Argument {at} type changed from {from} to {to}; compatibility needs review.");
        Some(("argument_type_changed", "review", message))
    } else {
        None
    }
}
/// Changes from `before` to `after`; a contract missing now loses all types.
pub(crate) fn changes(
    before: &[GraphqlContract],
    after: &[GraphqlContract],
    links: &[GraphqlLink],
) -> Vec<GraphqlChange> {
    let empty = BTreeMap::new();
    let now: BTreeMap<_, _> = after.iter().map(|c| (key(c), c)).collect();
    let mut out = vec![];
    for old in before {
        let mut diff = Diff {
            out: vec![],
            links,
            provider: old.provider.as_ref(),
        };
        diff.types(&old.types, now.get(&key(old)).map_or(&empty, |c| &c.types));
        out.extend(diff.out);
    }
    out
}
