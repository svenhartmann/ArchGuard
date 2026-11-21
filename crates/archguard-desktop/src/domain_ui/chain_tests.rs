//! The chain view and the state machine against a small flow and model.

use archguard_engine::core::domain_model::{
    DomainEvent, DomainModel, Entity, EntityKind, EventHandlerSite, IntegrationEvent,
    MessageHandler, Publisher, StateMachine, StateTransition, UseCase, UseCaseKind,
};
use archguard_engine::core::{
    AccessState, Action, Call, Condition, EndpointAccess, Origin, Screen, UiFlow,
};
use slint::Model;

use super::{state::build_from, view::build};
use crate::tab_support::Texts;
use crate::ui::DomChain;

fn model() -> DomainModel {
    let paid = StateTransition {
        from: vec!["Submitted".into()],
        to: "Paid".into(),
        method: "Order.SetPaidStatus".into(),
        guard: Some("Status == Submitted".into()),
        events: vec!["OrderPaid".into()],
        file: "Order.cs".into(),
        line: 40,
    };
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
            field: "Status".into(),
            enum_type: "OrderStatus".into(),
            states: vec!["Submitted".into(), "Paid".into()],
            initial: Some("Submitted".into()),
            transitions: vec![paid],
        }],
        use_cases: vec![UseCase {
            id: "Pay".into(),
            name: "PayHandler".into(),
            kind: UseCaseKind::Command,
            request: Some("PayCommand".into()),
            calls: vec!["Order.SetPaidStatus".into()],
            file: "Pay.cs".into(),
            line: 3,
            source: "t".into(),
        }],
        events: vec![DomainEvent {
            type_name: "OrderPaid".into(),
            raised_by: vec!["Order.SetPaidStatus".into()],
            handled_by: vec!["Mailer".into()],
            source: "t".into(),
        }],
        ..across_services()
    }
}

/// The integration event the use case publishes, its handler in Catalog,
/// the domain-event handler's declaration.
fn across_services() -> DomainModel {
    DomainModel {
        integration_events: vec![IntegrationEvent {
            type_name: "OrderPaidIntegrationEvent".into(),
            published_by: vec![Publisher {
                by: "PayHandler.Handle".into(),
                service: "Ordering".into(),
                file: "Pay.cs".into(),
                line: 7,
            }],
            handled_by: vec!["Stock".into()],
            source: "t".into(),
        }],
        message_handlers: vec![MessageHandler {
            id: "Stock".into(),
            message: "OrderPaidIntegrationEvent".into(),
            sends: vec![],
            file: "Stock.cs".into(),
            line: 2,
            source: "t".into(),
            service: "Catalog".into(),
        }],
        event_handler_sites: vec![EventHandlerSite {
            event: "OrderPaid".into(),
            handler: "Mailer".into(),
            file: "Mailer.cs".into(),
            line: 9,
        }],
        ..DomainModel::default()
    }
}

fn flow() -> UiFlow {
    UiFlow {
        screens: vec![Screen {
            weak: false,
            id: "checkout".into(),
            name: "Checkout".into(),
            kind: archguard_engine::core::ScreenKind::Page,
            route: Some("/checkout".into()),
            application: Some("Web".into()),
            file: "Checkout.razor".into(),
            line: 1,
            access: archguard_engine::core::AccessState::Undeclared,
            source: "t".into(),
        }],
        actions: vec![Action {
            weak: false,
            screen: Origin::Screen("checkout".into()),
            trigger: Some("Pay".into()),
            call: Call::Http {
                method: "PUT".into(),
                path: "/api/orders/pay".into(),
            },
            file: "Checkout.razor".into(),
            line: 9,
            condition: archguard_engine::core::AccessState::Undeclared,
            source: "t".into(),
        }],
        endpoints: vec![EndpointAccess {
            method: "PUT".into(),
            path: "/api/orders/pay".into(),
            access: AccessState::Protected(Condition::Role("Admin".into())),
            application: None,
            file: "OrdersApi.cs".into(),
            line: 12,
            source: "t".into(),
            handler: None,
            sends: vec!["PayCommand".into()],
        }],
        ..UiFlow::default()
    }
}

fn steps(c: &DomChain) -> Vec<(String, String, i32, i32)> {
    (c.steps.iter())
        .map(|s| (s.kind.to_string(), s.location.to_string(), s.tone, s.depth))
        .collect()
}

#[test]
fn a_chain_into_a_state_lists_nested_chains_indented_with_the_verdict() {
    let mut s = build_from(Ok(model()), Ok(flow()));
    assert_eq!(
        s.selected(),
        Some(0),
        "the root with a state machine is focused"
    );
    s.select_state("Paid");
    let m = build(&s, &Texts::default());
    assert_eq!(m.chosen_state, "Paid");
    assert_eq!(
        m.role_names.row_count(),
        2,
        "\"Role …\" and Admin; anonymous and signed in are the buttons"
    );
    assert_eq!(
        m.chains.row_count(),
        1,
        "the message chain alone does not touch Order"
    );
    let c = m.triggers.row_data(0).expect("a chain into Paid");
    let want = |tone| {
        vec![
            ("Screen action".into(), "Checkout.razor:9".into(), -1, 0),
            ("Endpoint".into(), "OrdersApi.cs:12".into(), tone, 0),
            ("Use case".into(), "Pay.cs:3".into(), -1, 0),
            ("Transition".into(), "Order.cs:40".into(), -1, 0),
            ("Event".into(), String::new(), -1, 0),
            ("Event handler".into(), "Mailer.cs:9".into(), -1, 1),
            ("Publishes".into(), "Pay.cs:7".into(), -1, 1),
            ("Message handler".into(), "Stock.cs:2".into(), -1, 1),
        ]
    };
    assert_eq!(steps(&c), want(1), "anonymous is denied");
    assert_eq!((c.verdict.as_str(), c.tone), ("denied", 1));
    let endpoint = c.steps.row_data(1).unwrap().text;
    assert_eq!(endpoint, "PUT /api/orders/pay · role Admin · denied");
    let nested = c.steps.row_data(7).unwrap().text;
    assert_eq!(
        nested, "OrderPaidIntegrationEvent → Stock · Catalog",
        "with its service"
    );
    s.select_role(2);
    let m = build(&s, &Texts::default());
    let c = m.triggers.row_data(0).unwrap();
    assert_eq!(steps(&c), want(0), "role Admin is allowed");
    assert_eq!(
        (c.verdict.as_str(), c.tone),
        ("unknown", 2),
        "the endpoint allows Admin, but no start screen leads to Checkout"
    );
    s.select_state("Submitted");
    assert_eq!(build(&s, &Texts::default()).triggers.row_count(), 0);
}

#[test]
fn the_entity_shows_its_state_machine_table_and_the_whole_model_on_request() {
    let mut s = build_from(Ok(model()), Err("no flow".into()));
    let m = build(&s, &Texts::default());
    assert_eq!((m.focus, m.selected), (0, 0));
    assert_eq!(m.machine_title, "Order.Status (OrderStatus)");
    assert_eq!(m.states.row_count(), 2);
    let row = m.rows.row_data(0).unwrap();
    assert_eq!(
        (
            row.number.as_str(),
            row.from_to.as_str(),
            row.location.as_str(),
            row.line
        ),
        ("1", "Submitted → Paid", "Order.cs:40", 40)
    );
    assert_eq!(row.events, "OrderPaid");
    assert_eq!(m.state_arrows.row_data(0).unwrap().number, 1);
    assert_eq!(m.flow_note, "no flow");
    assert!(m.cards.row_data(0).unwrap().machine);
    assert_eq!(m.bands.row_data(0).unwrap().title, "Aggregate Order");
    s.select(super::state::Focus::Integration(0));
    let m = build(&s, &Texts::default());
    assert_eq!(
        (m.focus, m.title.as_str()),
        (1, "OrderPaidIntegrationEvent")
    );
    assert_eq!(
        m.flow_bands.row_count(),
        2,
        "Ordering publishes, Catalog handles"
    );
    assert_eq!(
        m.note.id,
        "view-integration_event:Pay.cs:OrderPaidIntegrationEvent"
    );
}
