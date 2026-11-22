//! Service layers on the flow; declared in `lib.rs` so the test file does not import its parent.
use crate::ui_flow::*;

fn endpoint(access: AccessState) -> EndpointAccess {
    EndpointAccess {
        method: "GET".into(),
        path: "/a".into(),
        access,
        application: None,
        file: "A.java".into(),
        line: 3,
        source: "test".into(),
        handler: None,
        sends: vec![],
    }
}

fn layer(e: &EndpointAccess, how: ServiceHow, role: &str) -> ServiceLayer {
    ServiceLayer {
        endpoint_file: e.file.clone(),
        endpoint_line: e.line,
        endpoint_method: e.method.clone(),
        endpoint_path: e.path.clone(),
        kind: ServiceKind::Service,
        target: "S.m".into(),
        file: "S.java".into(),
        line: 9,
        state: AccessState::Protected(Condition::Role(role.into())),
        how,
        note: String::new(),
    }
}

#[test]
fn only_enforced_layers_change_the_effective_access() {
    let e = endpoint(AccessState::Protected(Condition::Role("A".into())));
    let flow = UiFlow {
        endpoints: vec![e.clone()],
        service_layers: vec![
            layer(&e, ServiceHow::Enforced, "B"),
            layer(&e, ServiceHow::PathOnly, "C"),
            layer(&e, ServiceHow::Hint, "D"),
            layer(&e, ServiceHow::DataFilter, "E"),
        ],
        ..UiFlow::default()
    };
    assert_eq!(
        flow.effective_access(&e),
        AccessState::Protected(Condition::All(vec![
            Condition::Role("A".into()),
            Condition::Role("B".into())
        ]))
    );
    let other = EndpointAccess {
        line: 4,
        ..e.clone()
    };
    assert_eq!(flow.service_layers_of(&other).count(), 0);
    assert_eq!(flow.effective_access(&other), other.access);
}

#[test]
fn an_enforced_layer_tightens_undeclared_and_open() {
    for before in [AccessState::Undeclared, AccessState::Open] {
        let e = endpoint(before);
        let flow = UiFlow {
            endpoints: vec![e.clone()],
            service_layers: vec![layer(&e, ServiceHow::Enforced, "B")],
            ..UiFlow::default()
        };
        assert_eq!(
            flow.effective_access(&e),
            AccessState::Protected(Condition::Role("B".into()))
        );
    }
}

#[test]
fn layers_round_trip_merge_and_old_json_reads() {
    let e = endpoint(AccessState::Undeclared);
    let l = layer(&e, ServiceHow::Enforced, "B");
    let flow = UiFlow {
        endpoints: vec![e.clone()],
        service_layers: vec![l.clone()],
        ..UiFlow::default()
    };
    let back: UiFlow = serde_json::from_str(&serde_json::to_string(&flow).unwrap()).unwrap();
    assert_eq!(back, flow);
    let merged = merge_ui_flows(flow.clone(), flow);
    assert_eq!(merged.service_layers, vec![l]);
    let old: UiFlow =
        serde_json::from_str(r#"{"definition":"ui-flow/v1","complete":true}"#).unwrap();
    assert!(old.service_layers.is_empty());
    assert!(
        !serde_json::to_string(&old)
            .unwrap()
            .contains("service_layers")
    );
}
