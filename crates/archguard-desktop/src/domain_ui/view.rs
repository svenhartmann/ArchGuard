//! The Domain tab's window model: the entity diagram, the selected
//! entity's fields and state machine (diagram and table), the events.

use archguard_engine::core::domain_model::{DomainModel, EntityKind, StateMachine};
use archguard_engine::domain_model::summary;

use super::state::DomainState;
use super::{entities, states};
use crate::tab_support::{Texts, model};
use crate::ui::{
    DomArrow, DomBand, DomCard, DomEvent, DomField, DomState, DomStateArrow, DomTransition,
    DomainViewModel,
};

fn kind(k: EntityKind) -> i32 {
    match k {
        EntityKind::AggregateRoot => 0,
        EntityKind::Entity => 1,
        EntityKind::ValueObject => 2,
        EntityKind::Unknown => 3,
    }
}

pub fn build(s: &DomainState, texts: &Texts) -> DomainViewModel {
    let d = match &*s.model {
        Ok(d) => d,
        Err(e) => {
            return DomainViewModel {
                state: 2,
                error: e.into(),
                ..Default::default()
            };
        }
    };
    let sum = summary(d);
    let mut m = DomainViewModel {
        state: 1,
        entities: sum.entities as i32,
        machines: sum.state_machines as i32,
        transitions: sum.transitions as i32,
        use_cases: sum.use_cases as i32,
        event_count: sum.events as i32,
        complete: sum.complete,
        selected: -1,
        events: model(d.events.iter().map(|e| DomEvent {
            name: e.type_name.as_str().into(),
            raised: or_none(&e.raised_by, texts).into(),
            handled: or_none(&e.handled_by, texts).into(),
        })),
        ..Default::default()
    };
    diagram(&mut m, s, d, texts);
    if let Some(i) = s.selected.filter(|&i| i < d.entities.len()) {
        selection(&mut m, d, i);
    }
    m
}

fn or_none(items: &[String], texts: &Texts) -> String {
    if items.is_empty() {
        texts.t("none").into()
    } else {
        items.join(", ")
    }
}

fn diagram(m: &mut DomainViewModel, s: &DomainState, d: &DomainModel, texts: &Texts) {
    let g = entities::layout(d);
    let machine = |id: &str| d.state_machines.iter().any(|sm| sm.entity == id);
    m.cards = model(g.cards.iter().map(|c| {
        let e = &d.entities[c.entity];
        DomCard {
            title: e.name.as_str().into(),
            kind: kind(e.kind),
            x: c.x,
            y: c.y,
            w: entities::CARD_W,
            h: entities::CARD_H,
            entity: c.entity as i32,
            selected: s.selected == Some(c.entity),
            machine: machine(&e.id),
        }
    }));
    m.arrows = model(g.arrows.into_iter().map(|a| DomArrow {
        path: a.path.into(),
        dashed: a.dashed,
        label: a.label.into(),
        lx: a.at.0,
        ly: a.at.1,
    }));
    m.bands = model(g.bands.into_iter().map(|b| {
        DomBand {
            title: match b.root {
                Some(r) => texts
                    .f("Aggregate {name}", &[("name", d.entities[r].name.clone())])
                    .into(),
                None => texts.t("Not part of an aggregate").into(),
            },
            y: b.y,
            h: b.h,
        }
    }));
    m.width = g.width;
    m.height = g.height;
}

fn selection(m: &mut DomainViewModel, d: &DomainModel, i: usize) {
    let e = &d.entities[i];
    m.selected = i as i32;
    m.title = e.name.as_str().into();
    m.kind = kind(e.kind);
    m.location = format!("{}:{}", e.file, e.line).into();
    m.file = e.file.as_str().into();
    m.fields = model(e.fields.iter().map(|f| {
        DomField {
            name: f.name.as_str().into(),
            r#type: if f.collection {
                format!("{}[]", f.type_name)
            } else {
                f.type_name.clone()
            }
            .into(),
        }
    }));
    if let Some(sm) = d.state_machines.iter().find(|sm| sm.entity == e.id) {
        machine(m, sm);
    }
}

fn machine(m: &mut DomainViewModel, sm: &StateMachine) {
    let g = states::layout(sm);
    m.machine_title = format!("{}.{} ({})", sm.entity, sm.field, sm.enum_type).into();
    m.states = model(g.nodes.iter().map(|n| DomState {
        name: n.state.clone().unwrap_or_default().into(),
        initial: n.initial,
        any: n.state.is_none(),
        x: n.x,
        y: n.y,
        w: states::NODE_W,
        h: states::NODE_H,
    }));
    m.state_arrows = model(g.arrows.iter().map(|a| DomStateArrow {
        path: a.path.as_str().into(),
        number: a.number as i32,
        lx: a.at.0,
        ly: a.at.1,
    }));
    m.state_w = g.width;
    m.state_h = g.height;
    m.rows = model(table(sm));
}

/// The transition table, numbered as the diagram's arrows.
pub fn table(sm: &StateMachine) -> Vec<DomTransition> {
    (sm.transitions.iter().enumerate())
        .map(|(i, t)| {
            let from = if t.from.is_empty() {
                "*".to_string()
            } else {
                t.from.join(", ")
            };
            DomTransition {
                number: (i + 1).to_string().into(),
                from_to: format!("{from} → {}", t.to).into(),
                method: t.method.as_str().into(),
                guard: t.guard.clone().unwrap_or_default().into(),
                events: t.events.join(", ").into(),
                location: format!("{}:{}", t.file, t.line).into(),
                file: t.file.as_str().into(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::domain_model::{Entity, StateTransition};
    use slint::Model;

    fn shop() -> DomainModel {
        DomainModel {
            entities: vec![Entity {
                id: "Order".into(),
                name: "Order".into(),
                kind: EntityKind::AggregateRoot,
                fields: vec![],
                file: "Order.cs".into(),
                line: 5,
                source: "t".into(),
            }],
            state_machines: vec![StateMachine {
                entity: "Order".into(),
                field: "OrderStatus".into(),
                enum_type: "OrderStatus".into(),
                states: vec!["Submitted".into(), "Paid".into()],
                initial: Some("Submitted".into()),
                transitions: vec![StateTransition {
                    from: vec!["Submitted".into()],
                    to: "Paid".into(),
                    method: "Order.SetPaidStatus".into(),
                    guard: Some("OrderStatus == OrderStatus.Submitted".into()),
                    events: vec!["OrderPaid".into()],
                    file: "Order.cs".into(),
                    line: 125,
                }],
            }],
            ..DomainModel::default()
        }
    }

    #[test]
    fn selecting_an_entity_shows_its_state_machine_and_table() {
        let mut s = super::super::state::build_from(Ok(shop()));
        let m = build(&s, &Texts::default());
        assert_eq!(
            (m.selected, m.states.row_count()),
            (-1, 0),
            "nothing selected"
        );
        assert!(m.cards.row_data(0).unwrap().machine);
        s.select_entity(0);
        let m = build(&s, &Texts::default());
        assert_eq!(m.machine_title, "Order.OrderStatus (OrderStatus)");
        assert_eq!(m.states.row_count(), 2);
        let row = m.rows.row_data(0).unwrap();
        assert_eq!(
            (
                row.number.as_str(),
                row.from_to.as_str(),
                row.location.as_str()
            ),
            ("1", "Submitted → Paid", "Order.cs:125")
        );
        assert_eq!(row.events, "OrderPaid");
        assert_eq!(m.state_arrows.row_data(0).unwrap().number, 1);
        assert_eq!(m.bands.row_data(0).unwrap().title, "Aggregate Order");
    }
}
