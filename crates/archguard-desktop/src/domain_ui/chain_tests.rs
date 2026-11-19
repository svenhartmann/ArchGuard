//! The chain view against a small flow and model.

use archguard_engine::core::domain_model::{
    DomainEvent, DomainModel, Entity, EntityKind, StateMachine, StateTransition, UseCase,
    UseCaseKind,
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
        guard: None,
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
        ..DomainModel::default()
    }
}

fn flow() -> UiFlow {
    UiFlow {
        screens: vec![Screen {
            id: "checkout".into(),
            name: "Checkout".into(),
            kind: archguard_engine::core::ScreenKind::Page,
            route: Some("/checkout".into()),
            application: Some("Web".into()),
            file: "Checkout.razor".into(),
            line: 1,
            access: None,
            source: "t".into(),
        }],
        actions: vec![Action {
            screen: Origin::Screen("checkout".into()),
            trigger: Some("Pay".into()),
            call: Call::Http {
                method: "PUT".into(),
                path: "/api/orders/pay".into(),
            },
            file: "Checkout.razor".into(),
            line: 9,
            condition: None,
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

fn steps(c: &DomChain) -> Vec<(String, String, i32)> {
    (c.steps.iter())
        .map(|s| (s.kind.to_string(), s.location.to_string(), s.tone))
        .collect()
}

#[test]
fn a_selected_state_lists_the_chains_into_it_with_the_endpoint_access() {
    let mut s = build_from(Ok(model()), Ok(flow()));
    s.select_entity(0);
    s.select_state("Paid");
    let m = build(&s, &Texts::default());
    assert_eq!(m.chosen_state, "Paid");
    assert_eq!(m.chains.row_count(), 1, "every chain of the entity");
    let c = m.triggers.row_data(0).expect("a chain into Paid");
    let want = |tone| {
        vec![
            ("Screen action".into(), "Checkout.razor:9".into(), -1),
            ("Endpoint".into(), "OrdersApi.cs:12".into(), tone),
            ("Use case".into(), "Pay.cs:3".into(), -1),
            ("Transition".into(), "Order.cs:40".into(), -1),
            ("Event".into(), String::new(), -1),
        ]
    };
    assert_eq!(steps(&c), want(1), "anonymous is denied");
    let endpoint = c.steps.row_data(1).unwrap().text;
    assert_eq!(endpoint, "PUT /api/orders/pay · role Admin · denied");
    s.select_role(2);
    let m = build(&s, &Texts::default());
    let c = m.triggers.row_data(0).unwrap();
    assert_eq!(steps(&c), want(0), "role Admin is allowed");
    assert_eq!(m.roles.row_count(), 3);
    s.select_state("Submitted");
    assert_eq!(build(&s, &Texts::default()).triggers.row_count(), 0);
}
