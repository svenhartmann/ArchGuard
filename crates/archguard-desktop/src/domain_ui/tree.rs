//! The Domain tab's left tree: per service its aggregates with their
//! entities and value objects, then its use cases; then the integration
//! events and the domain events. Rows carry the marks: state machine,
//! number of rules, noted for the agent.

use archguard_engine::core::domain_model::{DomainModel, EntityKind};

use super::entities::membership;
use super::ids;
use super::state::{DomainState, Focus};
use crate::tab_support::Texts;
use crate::ui::DomTreeRow;

fn group(depth: i32, key: String, label: String, count: usize, open: bool) -> DomTreeRow {
    DomTreeRow {
        depth,
        key: key.into(),
        label: label.into(),
        group: true,
        open,
        count: count as i32,
        kind: -1,
        index: -1,
        ..Default::default()
    }
}

fn item(s: &DomainState, depth: i32, focus: Focus, label: &str) -> DomTreeRow {
    DomTreeRow {
        depth,
        label: label.into(),
        kind: focus.kind(),
        index: focus.index() as i32,
        selected: s.focus == Some(focus),
        ..Default::default()
    }
}

/// An event's name without the suffix its group already says.
pub fn short(name: &str) -> &str {
    ["IntegrationEvent", "DomainEvent"]
        .iter()
        .find_map(|x| name.strip_suffix(x).filter(|s| !s.is_empty()))
        .unwrap_or(name)
}

pub fn rows(s: &DomainState, d: &DomainModel, texts: &Texts) -> Vec<DomTreeRow> {
    let rule_ids = ids::rules(d);
    let noted = |id: &str| s.notes.contains_key(id);
    let member = membership(d);
    let mut out = Vec::new();
    let services = (s.services.names.iter()).filter(|n| s.service.as_ref().is_none_or(|x| x == *n));
    for svc in services {
        let of = |file: &str| s.services.of(file) == svc;
        let mut ents: Vec<usize> = (0..d.entities.len())
            .filter(|&i| of(&d.entities[i].file))
            .collect();
        let ucs: Vec<usize> = (0..d.use_cases.len())
            .filter(|&i| of(&d.use_cases[i].file))
            .collect();
        if ents.is_empty() && ucs.is_empty() {
            continue;
        }
        let key = format!("s:{svc}");
        let open = !s.folded.contains(&key);
        out.push(group(0, key, svc.clone(), ents.len() + ucs.len(), open));
        if !open {
            continue;
        }
        // Aggregates by root name, entities of none last.
        let band = |i: usize| member[i].0.map(|r| d.entities[r].name.as_str());
        ents.sort_by_key(|&i| (band(i).is_none(), band(i), member[i].1, &d.entities[i].name));
        let mut k = 0;
        while k < ents.len() {
            let b = band(ents[k]);
            let n = ents[k..].iter().take_while(|&&i| band(i) == b).count();
            let key = format!("a:{svc}:{}", b.unwrap_or("-"));
            let open = !s.folded.contains(&key);
            let label = match b {
                Some(name) => texts.f("Aggregate {name}", &[("name", name.to_string())]),
                None => texts.t("Not part of an aggregate").to_string(),
            };
            out.push(group(1, key, label, n, open));
            for &i in ents[k..k + n].iter().filter(|_| open) {
                let e = &d.entities[i];
                let mut r = item(s, 2, Focus::Entity(i), &e.name);
                r.machine = d.state_machines.iter().any(|m| m.entity == e.id);
                let mine = || (d.rules.iter().zip(&rule_ids)).filter(|(x, _)| x.entity == e.id);
                r.rules = mine().count() as i32;
                r.noted = mine().any(|(_, id)| noted(id));
                if e.kind == EntityKind::ValueObject {
                    r.note = texts.t("value object").into();
                }
                out.push(r);
            }
            k += n;
        }
        if !ucs.is_empty() {
            let key = format!("u:{svc}");
            let open = !s.folded.contains(&key);
            out.push(group(1, key, texts.t("Use cases").into(), ucs.len(), open));
            for &i in ucs.iter().filter(|_| open) {
                out.push(item(s, 2, Focus::UseCase(i), &d.use_cases[i].name));
            }
        }
    }
    events(s, d, texts, &mut out);
    out
}

/// The integration events published or handled in the shown service, and
/// the domain events raised in it.
fn events(s: &DomainState, d: &DomainModel, texts: &Texts, out: &mut Vec<DomTreeRow>) {
    let svc = s.service.as_deref();
    let ie: Vec<usize> = (0..d.integration_events.len())
        .filter(|&i| {
            let e = &d.integration_events[i];
            svc.is_none_or(|svc| {
                e.published_by.iter().any(|p| p.service == svc)
                    || (d.message_handlers.iter())
                        .any(|h| h.message == e.type_name && h.service == svc)
            })
        })
        .collect();
    let raised_in = |method: &str| {
        let class = method.split_once('.').map_or(method, |(c, _)| c);
        (d.entities.iter()).any(|e| e.name == class && s.in_service(&e.file))
    };
    let de: Vec<usize> = (0..d.events.len())
        .filter(|&i| svc.is_none() || d.events[i].raised_by.iter().any(|m| raised_in(m)))
        .collect();
    let handlers = |n: usize| texts.plural(n, "{count} handler", "{count} handlers");
    for (key, label, list, integration) in [
        ("ie", texts.t("Integration events"), ie, true),
        ("de", texts.t("Domain events"), de, false),
    ] {
        if list.is_empty() {
            continue;
        }
        let open = !s.folded.contains(key);
        out.push(group(0, key.into(), label.into(), list.len(), open));
        for &i in list.iter().filter(|_| open) {
            let (focus, name, id, n) = if integration {
                let e = &d.integration_events[i];
                let n = (d.message_handlers.iter())
                    .filter(|h| h.message == e.type_name)
                    .count();
                (
                    Focus::Integration(i),
                    &e.type_name,
                    ids::integration_event(d, i),
                    n,
                )
            } else {
                let e = &d.events[i];
                (
                    Focus::Event(i),
                    &e.type_name,
                    ids::domain_event(d, i),
                    e.handled_by.len(),
                )
            };
            let mut r = item(s, 1, focus, short(name));
            r.note = handlers(n).into();
            r.noted = s.notes.contains_key(&id);
            out.push(r);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::domain_ui::services::Services;
    use crate::domain_ui::state::build_from;
    use archguard_engine::core::domain_model::{
        DomainEvent, DomainRule, Entity, IntegrationEvent, MessageHandler, Publisher, Relation,
        RelationKind, StateMachine, UseCase, UseCaseKind,
    };

    fn entity(name: &str, kind: EntityKind, file: &str) -> Entity {
        Entity {
            id: name.into(),
            name: name.into(),
            kind,
            fields: vec![],
            file: file.into(),
            line: 1,
            source: "t".into(),
        }
    }

    pub fn shop() -> DomainModel {
        DomainModel {
            entities: vec![
                entity("Order", EntityKind::AggregateRoot, "Ordering/Order.cs"),
                entity("Address", EntityKind::ValueObject, "Ordering/Address.cs"),
                entity("CatalogItem", EntityKind::Entity, "Catalog/CatalogItem.cs"),
            ],
            relations: vec![Relation {
                from: "Order".into(),
                to: "Address".into(),
                kind: RelationKind::HasOne,
                via: "Address".into(),
                source: "t".into(),
            }],
            state_machines: vec![StateMachine {
                entity: "Order".into(),
                field: "Status".into(),
                enum_type: "OrderStatus".into(),
                states: vec![],
                initial: None,
                transitions: vec![],
            }],
            rules: (0..2)
                .map(|k| DomainRule {
                    entity: "Order".into(),
                    method: "Order.Ship".into(),
                    condition: None,
                    message: None,
                    exception: "X".into(),
                    file: "Ordering/Order.cs".into(),
                    line: 10 + k,
                    source: "t".into(),
                })
                .collect(),
            use_cases: vec![UseCase {
                id: "Ship".into(),
                name: "ShipHandler".into(),
                kind: UseCaseKind::Command,
                request: None,
                calls: vec![],
                file: "Ordering/Ship.cs".into(),
                line: 1,
                source: "t".into(),
            }],
            events: vec![DomainEvent {
                type_name: "OrderShippedDomainEvent".into(),
                raised_by: vec!["Order.Ship".into()],
                handled_by: vec!["H".into()],
                source: "t".into(),
            }],
            integration_events: vec![IntegrationEvent {
                type_name: "OrderPaidIntegrationEvent".into(),
                published_by: vec![Publisher {
                    by: "P.Handle".into(),
                    service: "Ordering".into(),
                    file: "Ordering/P.cs".into(),
                    line: 4,
                }],
                handled_by: vec!["C".into()],
                source: "t".into(),
            }],
            message_handlers: vec![MessageHandler {
                id: "C".into(),
                message: "OrderPaidIntegrationEvent".into(),
                sends: vec![],
                file: "Catalog/C.cs".into(),
                line: 2,
                source: "t".into(),
                service: "Catalog".into(),
            }],
            ..DomainModel::default()
        }
    }

    pub fn state() -> DomainState {
        let mut s = build_from(Ok(shop()), Err("no flow".into()));
        s.services = Services::of_pairs(&[
            ("Ordering/Order.cs", "Ordering"),
            ("Ordering/Address.cs", "Ordering"),
            ("Ordering/Ship.cs", "Ordering"),
            ("Ordering/P.cs", "Ordering"),
            ("Catalog/CatalogItem.cs", "Catalog"),
            ("Catalog/C.cs", "Catalog"),
        ]);
        s
    }

    fn lines(s: &DomainState) -> Vec<String> {
        let d = s.model().unwrap();
        (rows(s, d, &Texts::default()).iter())
            .map(|r| {
                let marks = format!(
                    "{}{}{}{}",
                    if r.machine { " ⟳" } else { "" },
                    if r.rules > 0 {
                        format!(" {}r", r.rules)
                    } else {
                        String::new()
                    },
                    if r.noted { " ✎" } else { "" },
                    if r.selected { " *" } else { "" },
                );
                format!("{}{}{marks}", "  ".repeat(r.depth as usize), r.label)
            })
            .collect()
    }

    #[test]
    fn the_tree_groups_by_service_then_aggregate_then_events() {
        let mut s = state();
        assert_eq!(
            lines(&s),
            [
                "Catalog",
                "  Not part of an aggregate",
                "    CatalogItem",
                "Ordering",
                "  Aggregate Order",
                "    Order ⟳ 2r *",
                "    Address",
                "  Use cases",
                "    ShipHandler",
                "Integration events",
                "  OrderPaid",
                "Domain events",
                "  OrderShipped",
            ],
            "the root with a state machine is focused first"
        );
        s.notes.insert(
            "view-domain_rule:Ordering/Order.cs:Order.Ship#2".into(),
            Default::default(),
        );
        s.select_service(1);
        s.toggle_group("u:Catalog");
        let got = lines(&s);
        assert_eq!(
            got[0..3],
            ["Catalog", "  Not part of an aggregate", "    CatalogItem *"]
        );
        assert_eq!(
            got[3..],
            ["Integration events", "  OrderPaid"],
            "handled in Catalog"
        );
        s.select_service(2);
        s.toggle_group("a:Ordering:Order");
        assert_eq!(lines(&s)[1], "  Aggregate Order", "closed: no entities");
        s.toggle_group("a:Ordering:Order");
        assert_eq!(
            lines(&s)[2],
            "    Order ⟳ 2r ✎ *",
            "a noted rule marks its entity"
        );
        let mut t = state();
        t.fold_other_services();
        assert_eq!(
            lines(&t)[0..3],
            ["Catalog", "Ordering", "  Aggregate Order"],
            "opens at the focus"
        );
        t.select_service(1);
        assert_eq!(
            lines(&t)[1],
            "  Not part of an aggregate",
            "a picked service opens"
        );
    }
}
