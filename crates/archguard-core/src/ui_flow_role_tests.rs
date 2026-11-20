//! Role and policy catalog in the simulation (ADR 0021, roles entry).
use crate::ui_flow::*;

fn def(name: &str, kind: RoleKind, includes: &[&str]) -> RoleDef {
    RoleDef {
        name: name.into(),
        kind,
        includes: includes.iter().map(|s| s.to_string()).collect(),
        file: "Roles.cs".into(),
        line: 1,
        source: "test".into(),
    }
}
fn policy(name: &str, requires: Condition) -> PolicyDef {
    PolicyDef {
        name: name.into(),
        requires,
        file: "Startup.cs".into(),
        line: 2,
        source: "test".into(),
    }
}
fn holding(roles: &[&str]) -> Assignment {
    Assignment {
        authenticated: true,
        roles: roles.iter().map(|s| s.to_string()).collect(),
        ..Assignment::default()
    }
}
fn eval(flow: &UiFlow, condition: &Condition, assignment: &Assignment) -> Outcome {
    evaluate_condition(condition, &resolve_assignment(flow, assignment))
}
fn can_purge() -> UiFlow {
    UiFlow {
        policies: vec![policy("CanPurge", Condition::Role("Administrator".into()))],
        ..UiFlow::default()
    }
}

#[test]
fn policy_resolves_to_what_it_requires_and_an_explicit_switch_wins() {
    let flow = can_purge();
    let purge = Condition::Policy("CanPurge".into());
    let admin = holding(&["Administrator"]);
    assert_eq!(eval(&flow, &purge, &admin), Outcome::Allowed);
    assert_eq!(eval(&flow, &purge, &holding(&[])), Outcome::Denied);
    assert_eq!(eval(&flow, &purge, &Assignment::default()), Outcome::Denied);
    let mut switched = admin.clone();
    switched.policies.insert("CanPurge".into(), false);
    assert_eq!(eval(&flow, &purge, &switched), Outcome::Denied);
    // A policy may require another; a policy requiring itself stays unknown.
    let outer = Condition::All(vec![Condition::Authenticated, purge.clone()]);
    let flow = UiFlow {
        policies: vec![
            policy("Outer", outer),
            policy("CanPurge", Condition::Role("Administrator".into())),
            policy("Loop", Condition::Policy("Loop".into())),
        ],
        ..UiFlow::default()
    };
    let outer = Condition::Policy("Outer".into());
    assert_eq!(eval(&flow, &outer, &admin), Outcome::Allowed);
    let looped = Condition::Policy("Loop".into());
    assert_eq!(eval(&flow, &looped, &admin), Outcome::Unknown);
}

#[test]
fn policy_defs_of_one_name_that_disagree_stay_unknown() {
    let flow = UiFlow {
        policies: vec![
            policy("Application", Condition::Authenticated),
            policy("Application", Condition::Role("Api".into())),
        ],
        ..UiFlow::default()
    };
    let p = Condition::Policy("Application".into());
    assert_eq!(eval(&flow, &p, &holding(&[])), Outcome::Unknown);
    assert_eq!(eval(&flow, &p, &holding(&["Api"])), Outcome::Allowed);
    assert_eq!(eval(&flow, &p, &Assignment::default()), Outcome::Denied);
}

#[test]
fn hierarchy_implies_transitively_and_survives_cycles() {
    let flow = UiFlow {
        roles: vec![
            def("ADMIN", RoleKind::Role, &["STAFF"]),
            def("STAFF", RoleKind::Role, &["USER"]),
            def("USER", RoleKind::Role, &["ADMIN"]), // cycle
            def("GUEST", RoleKind::Role, &[]),
        ],
        ..UiFlow::default()
    };
    let resolved = resolve_assignment(&flow, &holding(&["STAFF"]));
    let roles: Vec<&str> = resolved.roles.iter().map(String::as_str).collect();
    assert_eq!(roles, ["ADMIN", "STAFF", "USER"]);
    let flow = UiFlow {
        roles: vec![def("ADMIN", RoleKind::Role, &["USER"])],
        ..UiFlow::default()
    };
    let (user, admin) = (
        Condition::Role("USER".into()),
        Condition::Role("ADMIN".into()),
    );
    assert_eq!(eval(&flow, &user, &holding(&["ADMIN"])), Outcome::Allowed);
    assert_eq!(eval(&flow, &admin, &holding(&["USER"])), Outcome::Denied);
}

#[test]
fn wildcard_permissions_cover_the_defined_ids_below_them() {
    let flow = UiFlow {
        roles: vec![
            def("sq.apps.{app}.*", RoleKind::Permission, &[]),
            def("sq.apps.{app}.contents.read", RoleKind::Permission, &[]),
            def("sq.admin.users", RoleKind::Permission, &[]),
        ],
        ..UiFlow::default()
    };
    let read = Condition::Role("sq.apps.{app}.contents.read".into());
    assert_eq!(
        eval(&flow, &read, &holding(&["sq.apps.{app}.*"])),
        Outcome::Allowed
    );
    assert_eq!(eval(&flow, &read, &holding(&["sq.*"])), Outcome::Allowed);
    let wild = Condition::Role("sq.apps.{app}.*".into());
    let reader = holding(&["sq.apps.{app}.contents.read"]);
    assert_eq!(eval(&flow, &wild, &reader), Outcome::Denied);
    let users = Condition::Role("sq.admin.users".into());
    assert_eq!(
        eval(&flow, &users, &holding(&["sq.apps.{app}.*"])),
        Outcome::Denied
    );
    assert!(permission_covers("a.b", "a.b.c") && !permission_covers("a.b.c", "a.b"));
}

#[test]
fn nothing_defined_keeps_the_old_behaviour() {
    let flow = UiFlow::default();
    let assignment = holding(&["ADMIN"]);
    assert_eq!(resolve_assignment(&flow, &assignment), assignment);
    let p = Condition::Policy("CanPurge".into());
    assert_eq!(eval(&flow, &p, &assignment), Outcome::Unknown);
}

fn purge_flow() -> UiFlow {
    UiFlow {
        screens: vec![Screen {
            id: "purge".into(),
            name: "Purge".into(),
            kind: ScreenKind::Page,
            route: Some("/purge".into()),
            application: None,
            file: "Purge.razor".into(),
            line: 1,
            access: Some(Condition::Policy("CanPurge".into())),
            source: "test".into(),
        }],
        start_screens: vec!["purge".into()],
        ..can_purge()
    }
}

#[test]
fn screen_reaches_resolve_policies_and_hierarchy() {
    let flow = purge_flow();
    let reach = |a: &Assignment| screen_reaches(&flow, a)["purge"];
    assert_eq!(reach(&holding(&["Administrator"])), ScreenReach::Sure);
    assert_eq!(reach(&holding(&[])), ScreenReach::Denied);
    let flow = UiFlow {
        roles: vec![def("Owner", RoleKind::Role, &["Administrator"])],
        ..purge_flow()
    };
    let owner = holding(&["Owner"]);
    assert_eq!(screen_reaches(&flow, &owner)["purge"], ScreenReach::Sure);
    let reached = reachable_screens(&flow, "purge", &owner);
    assert_eq!(reached.get("purge"), Some(&Reach::Sure));
}

#[test]
fn findings_resolve_policies() {
    let open = Screen {
        access: None,
        ..purge_flow().screens[0].clone()
    };
    let action = Action {
        screen: Origin::Screen("purge".into()),
        trigger: Some("purge".into()),
        call: Call::Http {
            method: "POST".into(),
            path: "/api/purge".into(),
        },
        file: "Purge.razor".into(),
        line: 5,
        condition: None,
        source: "test".into(),
    };
    let endpoint = EndpointAccess {
        method: "POST".into(),
        path: "/api/purge".into(),
        access: AccessState::Protected(Condition::Policy("CanPurge".into())),
        application: None,
        file: "Api.cs".into(),
        line: 3,
        source: "test".into(),
        handler: None,
        sends: vec![],
    };
    let flow = UiFlow {
        screens: vec![open],
        actions: vec![action],
        endpoints: vec![endpoint],
        ..purge_flow()
    };
    let kinds = |flow: &UiFlow| -> Vec<UiFlowFindingKind> {
        ui_flow_findings(flow).iter().map(|f| f.kind).collect()
    };
    // Anonymous reaches the open screen; the backend policy resolves to denied.
    assert_eq!(kinds(&flow), [UiFlowFindingKind::ShownButRefused]);
    let undefined = UiFlow {
        policies: vec![],
        ..flow
    };
    assert!(
        kinds(&undefined).is_empty(),
        "{:#?}",
        ui_flow_findings(&undefined)
    );
}

#[test]
fn catalog_merges_as_sorted_union_and_is_left_out_when_empty() {
    let a = UiFlow {
        roles: vec![
            def("USER", RoleKind::Role, &[]),
            def("ADMIN", RoleKind::Role, &["USER"]),
        ],
        ..UiFlow::default()
    };
    let b = UiFlow {
        roles: vec![def("ADMIN", RoleKind::Role, &["USER"])],
        ..can_purge()
    };
    let ab = merge_ui_flows(a, b);
    let names: Vec<&str> = ab.roles.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["ADMIN", "USER"]);
    assert_eq!(ab.policies.len(), 1);
    let json = serde_json::to_value(&ab).unwrap();
    assert_eq!(json["roles"][0]["kind"], "role");
    assert_eq!(json["roles"][0]["includes"][0], "USER");
    assert_eq!(json["policies"][0]["requires"]["role"], "Administrator");
    assert_eq!(serde_json::from_value::<UiFlow>(json).unwrap(), ab);
    let empty = serde_json::to_value(UiFlow::default()).unwrap();
    assert!(empty.get("roles").is_none() && empty.get("policies").is_none());
}
