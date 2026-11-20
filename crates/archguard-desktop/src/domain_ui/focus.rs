//! The entity focus beside fields and state machine: its relations, its
//! rules grouped by method, the validations of requests that reach it, and
//! the events it raises with the services that react.

use archguard_engine::core::domain_model::{DomainModel, RelationKind};

use super::state::DomainState;
use super::tree::short;
use crate::screens_ui::inspector::row;
use crate::tab_support::{Texts, model};
use crate::ui::{DomLink, DomRuleGroup, FlowRow};

fn simple(t: &str) -> &str {
    let t = t.split('<').next().unwrap_or(t);
    t.rsplit('.').next().unwrap_or(t)
}

/// The method's name without its class; a constructor says so.
fn method_label(method: &str, entity: &str, texts: &Texts) -> String {
    let name = method.rsplit('.').next().unwrap_or(method);
    if name == simple(entity) {
        texts.f("{name} (constructor)", &[("name", name.to_string())])
    } else {
        name.to_string()
    }
}

/// "throws when <condition> → <message>", per method in source order.
pub fn rule_groups(
    s: &DomainState,
    d: &DomainModel,
    i: usize,
    ids: &[String],
    texts: &Texts,
) -> Vec<DomRuleGroup> {
    let e = &d.entities[i];
    let mut mine: Vec<(usize, _)> = (d.rules.iter().enumerate())
        .filter(|(_, r)| r.entity == e.id)
        .collect();
    mine.sort_by_key(|(_, r)| (r.file.clone(), r.line));
    let mut groups: Vec<(String, Vec<FlowRow>)> = Vec::new();
    for (k, r) in mine {
        let when = match &r.condition {
            Some(c) => texts.f("throws when {condition}", &[("condition", c.clone())]),
            None => texts.t("throws").to_string(),
        };
        let text = match &r.message {
            Some(m) => format!("{when} → “{m}”"),
            None => when,
        };
        let mut f = row(text, &r.file, r.line);
        f.id = ids[k].as_str().into();
        if let Some(n) = s.notes.get(&ids[k]) {
            f.badge = texts.t(crate::screens_ui::notes::badge(n)).into();
            f.note = n.note.as_str().into();
        }
        f.text = format!("{} · {}", f.text, r.exception).into();
        let label = method_label(&r.method, &e.name, texts);
        match groups.iter_mut().find(|(m, _)| *m == label) {
            Some((_, rows)) => rows.push(f),
            None => groups.push((label, vec![f])),
        }
    }
    (groups.into_iter())
        .map(|(method, rules)| DomRuleGroup {
            method: method.into(),
            rules: model(rules),
        })
        .collect()
}

/// The validators of requests whose use case calls a method of entity
/// `i`, or of the entity's own type.
pub fn validations(d: &DomainModel, i: usize) -> Vec<FlowRow> {
    let e = &d.entities[i];
    let calls_it = |request: &str| {
        (d.use_cases.iter()).any(|u| {
            u.request.as_deref().is_some_and(|r| simple(r) == request)
                && (u.calls.iter()).any(|c| {
                    c.starts_with(&format!("{}.", e.id)) || c.starts_with(&format!("{}.", e.name))
                })
        })
    };
    let mut rows: Vec<_> = (d.validations.iter())
        .filter(|v| simple(&v.target_type) == e.name || calls_it(simple(&v.target_type)))
        .collect();
    rows.sort_by_key(|v| (v.target_type.clone(), v.line));
    (rows.into_iter())
        .map(|v| {
            let text = format!(
                "{}.{}: {}",
                simple(&v.target_type),
                v.property,
                v.rules.join(" · ")
            );
            row(text, &v.file, v.line)
        })
        .collect()
}

/// Outgoing relations ("has many OrderItem · OrderItems"), then incoming.
pub fn relations(d: &DomainModel, i: usize, texts: &Texts) -> Vec<DomLink> {
    let e = &d.entities[i];
    let index = |id: &str| d.entities.iter().position(|x| x.id == id);
    let kind = |k: RelationKind| {
        texts.t(match k {
            RelationKind::HasOne => "has one",
            RelationKind::HasMany => "has many",
            RelationKind::References => "references",
        })
    };
    let out = (d.relations.iter())
        .filter(|r| r.from == e.id)
        .filter_map(|r| {
            let to = index(&r.to)?;
            Some(DomLink {
                text: format!("{} {}", kind(r.kind), d.entities[to].name).into(),
                note: r.via.as_str().into(),
                kind: 0,
                index: to as i32,
            })
        });
    let into = (d.relations.iter())
        .filter(|r| r.to == e.id)
        .filter_map(|r| {
            let from = index(&r.from)?;
            Some(DomLink {
                text: format!("← {} {} {}", d.entities[from].name, kind(r.kind), e.name).into(),
                note: r.via.as_str().into(),
                kind: 0,
                index: from as i32,
            })
        });
    out.chain(into).collect()
}

/// The domain events the entity raises, each followed by the integration
/// events its handlers publish and the services handling those.
pub fn reactions(d: &DomainModel, i: usize, texts: &Texts) -> Vec<DomLink> {
    let e = &d.entities[i];
    let mut out = Vec::new();
    let raised = |m: &String| {
        m.split_once('.')
            .is_some_and(|(c, _)| c == e.name || c == e.id)
    };
    for (k, ev) in d
        .events
        .iter()
        .enumerate()
        .filter(|(_, ev)| ev.raised_by.iter().any(raised))
    {
        let by = ev
            .raised_by
            .iter()
            .filter(|m| raised(m))
            .cloned()
            .collect::<Vec<_>>();
        out.push(DomLink {
            text: short(&ev.type_name).into(),
            note: texts
                .f(
                    "raised by {method} · handled by {handlers}",
                    &[
                        ("method", by.join(", ")),
                        ("handlers", or_none(&ev.handled_by, texts)),
                    ],
                )
                .into(),
            kind: 2,
            index: k as i32,
        });
        for (j, ie) in d.integration_events.iter().enumerate() {
            let class = |by: &str| by.split_once('.').map_or(by, |(c, _)| c).to_string();
            if !ie
                .published_by
                .iter()
                .any(|p| ev.handled_by.contains(&class(&p.by)))
            {
                continue;
            }
            let mut services: Vec<&str> = (d.message_handlers.iter())
                .filter(|h| h.message == ie.type_name)
                .map(|h| h.service.as_str())
                .collect();
            services.sort_unstable();
            services.dedup();
            out.push(DomLink {
                text: format!("   → {}", short(&ie.type_name)).into(),
                note: texts
                    .f(
                        "integration event · handled in {services}",
                        &[(
                            "services",
                            or_none(
                                &services.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                                texts,
                            ),
                        )],
                    )
                    .into(),
                kind: 1,
                index: j as i32,
            });
        }
    }
    out
}

pub fn or_none(items: &[String], texts: &Texts) -> String {
    if items.is_empty() {
        texts.t("none").into()
    } else {
        items.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain_ui::tree::tests::state;
    use archguard_engine::core::domain_model::{DomainRule, UseCase, UseCaseKind, Validation};
    use slint::Model;

    #[test]
    fn rules_group_by_method_in_source_order_with_condition_and_message() {
        let mut s = state();
        let mut d = s.model().unwrap().clone();
        let rule = |method: &str, condition: Option<&str>, line| DomainRule {
            entity: "Order".into(),
            method: method.into(),
            condition: condition.map(Into::into),
            message: Some("Invalid units".into()),
            exception: "OrderingDomainException".into(),
            file: "Ordering/Order.cs".into(),
            line,
            source: "t".into(),
        };
        d.rules = vec![
            rule("Order.Ship", Some("Status != Paid"), 40),
            rule("Order.Order", Some("units <= 0"), 10),
            rule("Order.Ship", None, 44),
        ];
        let ids = crate::domain_ui::ids::rules(&d);
        s.notes.insert(ids[2].clone(), Default::default());
        let g = rule_groups(&s, &d, 0, &ids, &Texts::default());
        let methods: Vec<String> = g.iter().map(|g| g.method.to_string()).collect();
        assert_eq!(methods, ["Order (constructor)", "Ship"]);
        let ship: Vec<FlowRow> = g[1].rules.iter().collect();
        assert_eq!(
            ship[0].text,
            "throws when Status != Paid → “Invalid units” · OrderingDomainException"
        );
        assert_eq!(
            ship[1].text,
            "throws → “Invalid units” · OrderingDomainException"
        );
        assert_eq!(
            (ship[1].location.as_str(), ship[1].id.as_str()),
            ("Ordering/Order.cs:44", ids[2].as_str())
        );
        assert_eq!(ship[1].badge, "noted: fix", "a stored note shows");
    }

    #[test]
    fn validations_reach_the_entity_through_the_use_case_of_their_request() {
        let mut d = state().model().unwrap().clone();
        d.use_cases = vec![UseCase {
            id: "Ship".into(),
            name: "ShipHandler".into(),
            kind: UseCaseKind::Command,
            request: Some("App.ShipCommand".into()),
            calls: vec!["Order.Ship".into()],
            file: "Ship.cs".into(),
            line: 1,
            source: "t".into(),
        }];
        let v = |target: &str| Validation {
            target_type: target.into(),
            property: "Id".into(),
            rules: vec!["NotEmpty()".into(), "Length(3)".into()],
            file: "V.cs".into(),
            line: 5,
            source: "t".into(),
        };
        d.validations = vec![v("ShipCommand"), v("OtherCommand")];
        let rows = validations(&d, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text, "ShipCommand.Id: NotEmpty() · Length(3)");
        assert!(validations(&d, 2).is_empty(), "CatalogItem has none");
    }

    #[test]
    fn reactions_follow_domain_events_into_integration_events() {
        let mut d = state().model().unwrap().clone();
        d.integration_events[0].published_by[0].by = "H.Handle".into();
        let r = reactions(&d, 0, &Texts::default());
        let text: Vec<(String, i32)> = r.iter().map(|l| (l.text.to_string(), l.kind)).collect();
        assert_eq!(
            text,
            [("OrderShipped".into(), 2), ("   → OrderPaid".into(), 1)]
        );
        assert_eq!(r[1].note, "integration event · handled in Catalog");
        let rel: Vec<String> = relations(&d, 1, &Texts::default())
            .iter()
            .map(|l| l.text.to_string())
            .collect();
        assert_eq!(rel, ["← Order has one Address"]);
    }
}
