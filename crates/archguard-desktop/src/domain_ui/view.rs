//! The Domain tab's window model: top bar, tree, the focused entity (fields,
//! relations, rules, state machine, chains, validations, reactions), event
//! or use case, the whole model on request, the note form and code overlay.

use archguard_engine::core::domain_model::{DomainModel, EntityKind, StateMachine};
use archguard_engine::domain_model::summary;

use super::state::{DomainState, Focus};
use super::{chain, entities, flow, focus, ids, overview, states, tree};
use crate::screens_ui::inspector::{note_form, row};
use crate::tab_support::{Texts, model};
use crate::ui::{
    DomArrow, DomBand, DomBox, DomCard, DomField, DomHubChip, DomState, DomStateArrow,
    DomTransition, DomainViewModel, FlowRow,
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
    let flow = (*s.flow).as_ref().ok();
    let mut m = DomainViewModel {
        state: 1,
        entities: sum.entities as i32,
        machines: sum.state_machines as i32,
        rule_count: sum.rules as i32,
        validation_count: sum.validations as i32,
        event_count: (sum.events + sum.integration_events) as i32,
        complete: sum.complete,
        services: model(
            std::iter::once(texts.t("All services").into())
                .chain(s.services.names.iter().map(|n| n.as_str().into())),
        ),
        service_index: (s.service.as_ref())
            .and_then(|x| s.services.names.iter().position(|n| n == x))
            .map_or(0, |i| i as i32 + 1),
        role: s.role as i32,
        role_names: crate::screens_ui::role_picker(texts, flow, &s.roles),
        tree: model(tree::rows(s, d, texts)),
        details_open: s.details_open,
        whole: s.whole,
        focus: -1,
        selected: -1,
        problem: s.problem.as_str().into(),
        code: s.code.view(),
        ..Default::default()
    };
    if let Some((id, subject)) = &s.note_for {
        m.inspector = note_form(id, subject, texts);
    }
    diagram(&mut m, s, d, texts);
    match s.focus {
        Some(Focus::Entity(i)) if i < d.entities.len() => {
            entity(&mut m, s, d, i, texts);
            chains(&mut m, s, d, i, texts);
        }
        Some(f @ (Focus::Integration(_) | Focus::Event(_))) => event(&mut m, s, d, f, texts),
        Some(Focus::UseCase(i)) if i < d.use_cases.len() => use_case(&mut m, s, d, i, texts),
        _ => {}
    }
    m
}

/// The chains into the chosen state, every chain of the entity.
fn chains(m: &mut DomainViewModel, s: &DomainState, d: &DomainModel, i: usize, texts: &Texts) {
    let flow = match &*s.flow {
        Ok(f) => f,
        Err(e) => {
            m.flow_note = e.as_str().into();
            return;
        }
    };
    let id = &d.entities[i].id;
    let a = s.assignment();
    m.chains = model(chain::of_entity(flow, d, &s.services, id, &a, texts));
    if let Some(state) = &s.state {
        m.chosen_state = state.as_str().into();
        m.triggers = model(chain::triggers(
            flow,
            d,
            &s.services,
            (id, state),
            &a,
            texts,
        ));
        use slint::Model;
        m.states = model(m.states.iter().map(|mut n| {
            n.selected = n.name == state.as_str() && !n.any;
            n
        }));
    }
}

fn diagram(m: &mut DomainViewModel, s: &DomainState, d: &DomainModel, texts: &Texts) {
    let scene = overview::scene(d, s.module.as_deref());
    let g = scene.diagram;
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
            selected: s.selected() == Some(c.entity),
            machine: machine(&e.id),
        }
    }));
    let links = (scene.links.into_iter()).map(|l| (l.path, false, l.count.to_string(), l.at));
    let arrows = (g.arrows.into_iter()).map(|a| (a.path, a.dashed, a.label.to_string(), a.at));
    m.arrows = model(
        arrows
            .chain(links)
            .map(|(path, dashed, label, at)| DomArrow {
                path: path.into(),
                dashed,
                label: label.into(),
                lx: at.0,
                ly: at.1,
            }),
    );
    m.boxes = model(scene.boxes.into_iter().map(|b| DomBox {
        title: b.title.into(),
        sub: b.sub.into(),
        module: b.module as i32,
        x: b.x,
        y: b.y,
        w: b.w,
        h: b.h,
    }));
    m.hubs = model(scene.hubs.iter().map(|h| DomHubChip {
        text: format!("{} \u{2190} {}", d.entities[h.entity].name, h.from).into(),
        index: h.entity as i32,
    }));
    m.module = s.module.as_deref().unwrap_or_default().into();
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

fn entity(m: &mut DomainViewModel, s: &DomainState, d: &DomainModel, i: usize, texts: &Texts) {
    let e = &d.entities[i];
    m.focus = 0;
    m.selected = i as i32;
    m.title = e.name.as_str().into();
    m.kind = kind(e.kind);
    m.service = s.services.of(&e.file).into();
    m.location = format!("{}:{}", e.file, e.line).into();
    m.file = e.file.as_str().into();
    m.line = e.line as i32;
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
    m.relations = model(focus::relations(d, i, texts));
    m.reactions = model(focus::reactions(d, i, texts));
    let rule_ids = ids::rules(d);
    let groups = focus::rule_groups(s, d, i, &rule_ids, texts);
    m.entity_rules = d.rules.iter().filter(|r| r.entity == e.id).count() as i32;
    m.rule_groups = model(groups);
    m.validations = model(focus::validations(d, i));
    if let Some(sm) = d.state_machines.iter().find(|sm| sm.entity == e.id) {
        machine(m, sm);
    }
}

fn event(m: &mut DomainViewModel, s: &DomainState, d: &DomainModel, f: Focus, texts: &Texts) {
    let (lanes, name, id, what) = match f {
        Focus::Integration(i) if i < d.integration_events.len() => (
            flow::integration(d, i, texts),
            &d.integration_events[i].type_name,
            ids::integration_event(d, i),
            texts.t("integration event"),
        ),
        Focus::Event(i) if i < d.events.len() => (
            flow::domain(d, i, &s.services, texts),
            &d.events[i].type_name,
            ids::domain_event(d, i),
            texts.t("domain event"),
        ),
        _ => return,
    };
    m.focus = f.kind();
    m.title = name.as_str().into();
    let lanes_n = lanes.bands.len();
    m.service = texts
        .f(
            "{what} · {lanes}",
            &[
                ("what", what.to_string()),
                (
                    "lanes",
                    texts.plural(lanes_n, "{count} service", "{count} services"),
                ),
            ],
        )
        .into();
    let mut note = FlowRow {
        id: id.as_str().into(),
        text: name.as_str().into(),
        ..Default::default()
    };
    if let Some(n) = s.notes.get(&id) {
        note.badge = texts.t(crate::screens_ui::notes::badge(n)).into();
        note.note = n.note.as_str().into();
    }
    m.note = note;
    m.flow_cards = model(lanes.cards);
    m.flow_arrows = model(lanes.arrows);
    m.flow_bands = model(lanes.bands);
    m.flow_w = lanes.width;
    m.flow_h = lanes.height;
}

fn use_case(m: &mut DomainViewModel, s: &DomainState, d: &DomainModel, i: usize, texts: &Texts) {
    let u = &d.use_cases[i];
    m.focus = 3;
    m.title = u.name.as_str().into();
    m.service = s.services.of(&u.file).into();
    m.location = format!("{}:{}", u.file, u.line).into();
    m.file = u.file.as_str().into();
    m.line = u.line as i32;
    let mut items = Vec::new();
    if let Some(r) = &u.request {
        items.push(row(
            texts.f("handles {request}", &[("request", r.clone())]),
            &u.file,
            u.line,
        ));
    }
    let transitions = || d.state_machines.iter().flat_map(|sm| &sm.transitions);
    for c in &u.calls {
        let text = texts.f("calls {method}", &[("method", c.clone())]);
        items.push(match transitions().find(|t| t.method == *c) {
            Some(t) => row(format!("{text} → {}", t.to), &t.file, t.line),
            None => FlowRow {
                text: text.into(),
                ..Default::default()
            },
        });
    }
    m.items = model(items);
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
        selected: false,
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
                to: t.to.as_str().into(),
                from_to: format!("{from} → {}", t.to).into(),
                method: t.method.as_str().into(),
                guard: t.guard.clone().unwrap_or_default().into(),
                events: t.events.join(", ").into(),
                location: format!("{}:{}", t.file, t.line).into(),
                file: t.file.as_str().into(),
                line: t.line as i32,
            }
        })
        .collect()
}
