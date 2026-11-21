//! Role and policy catalog in the simulation (ADR 0021, roles entry).
use crate::ui_flow::*;

fn def(name: &str, kind: RoleKind, includes: &[&str]) -> RoleDef {
    RoleDef {
        application: None,
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
        application: None,
        name: name.into(),
        requires,
        file: "Startup.cs".into(),
        line: 2,
        source: "test".into(),
    }
}
fn dotted() -> RoleConvention {
    RoleConvention {
        permissions: PermissionFormat::Dotted {
            wildcard: "*".into(),
        },
        ..RoleConvention::default()
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
        role_conventions: vec![dotted()],
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
            weak: false,
            id: "purge".into(),
            name: "Purge".into(),
            kind: ScreenKind::Page,
            route: Some("/purge".into()),
            application: None,
            file: "Purge.razor".into(),
            line: 1,
            access: AccessState::Protected(Condition::Policy("CanPurge".into())),
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
        access: AccessState::Undeclared,
        ..purge_flow().screens[0].clone()
    };
    let action = Action {
        weak: false,
        screen: Origin::Screen("purge".into()),
        trigger: Some("purge".into()),
        call: Call::Http {
            method: "POST".into(),
            path: "/api/purge".into(),
        },
        file: "Purge.razor".into(),
        line: 5,
        condition: AccessState::Undeclared,
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

fn names(items: &[&str]) -> std::collections::BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn holds_compares_by_the_convention() {
    let exact = RoleConvention::default();
    let (role, policy) = (AtomKind::Role, AtomKind::Policy);
    assert!(holds(&exact, role, &names(&["Admin"]), "Admin"));
    assert!(!holds(&exact, role, &names(&["Admin"]), "admin"));
    let loose = RoleConvention {
        role_case: NameCase::Insensitive,
        ..RoleConvention::default()
    };
    assert!(holds(&loose, role, &names(&["Admin"]), "admin"));
    assert!(
        !holds(&loose, policy, &names(&["Admin"]), "admin"),
        "policy_case is its own"
    );
    let dot = dotted();
    assert!(holds(&dot, policy, &names(&["a.b"]), "a.b.c"));
    assert!(holds(&dot, policy, &names(&["a.*"]), "a.b.c"));
    assert!(!holds(&dot, policy, &names(&["a.b.c"]), "a.b"));
    assert!(!holds(&exact, policy, &names(&["a.b"]), "a.b.c"));
    let casl = RoleConvention {
        permissions: PermissionFormat::ActionSubject {
            any_action: "manage".into(),
            any_subject: "all".into(),
        },
        action_aliases: vec![("read".into(), vec!["index".into(), "show".into()])],
        ..RoleConvention::default()
    };
    assert!(holds(&casl, policy, &names(&["manage all"]), "delete Post"));
    assert!(holds(&casl, policy, &names(&["read Post"]), "show Post"));
    assert!(!holds(
        &casl,
        policy,
        &names(&["read Post"]),
        "show Comment"
    ));
    assert!(!holds(&casl, policy, &names(&["read Post"]), "update Post"));
    assert!(holds(&casl, policy, &names(&["update all"]), "update Post"));
}

#[test]
fn hierarchy_is_expanded_before_holds_and_canonical_label_use_the_prefix() {
    let flow = UiFlow {
        roles: vec![def("ROLE_ADMIN", RoleKind::Role, &["ROLE_USER"])],
        role_conventions: vec![RoleConvention {
            role_prefix: Some("ROLE_".into()),
            ..RoleConvention::default()
        }],
        ..UiFlow::default()
    };
    let conv = flow.convention(None);
    let admin = canonical(&conv, AtomKind::Role, "ADMIN");
    assert_eq!(admin, "ROLE_ADMIN");
    assert_eq!(canonical(&conv, AtomKind::Role, "ROLE_ADMIN"), "ROLE_ADMIN");
    assert_eq!(label(&conv, AtomKind::Role, &admin), "ADMIN");
    let user = Condition::Role("ROLE_USER".into());
    assert_eq!(eval(&flow, &user, &holding(&[&admin])), Outcome::Allowed);
    // a typed name meets the stored authority at match time, never rewritten
    assert_eq!(eval(&flow, &user, &holding(&["USER"])), Outcome::Allowed);
    assert_eq!(
        eval(&flow, &user, &holding(&["ADMIN"])),
        Outcome::Allowed,
        "hierarchy"
    );
}

fn spring(app: &str) -> RoleConvention {
    RoleConvention {
        application: Some(app.into()),
        role_prefix: Some("ROLE_".into()),
        ..RoleConvention::default()
    }
}

#[test]
fn an_unprefixed_authority_is_held_as_typed() {
    // hasAuthority('AUDITOR') stores AUDITOR; the prefix convention must not change that
    let flow = UiFlow {
        role_conventions: vec![spring("api")],
        ..UiFlow::default()
    };
    let audit = Condition::Role("AUDITOR".into());
    let prefixed = Condition::Role("ROLE_AUDITOR".into());
    let held = holding(&["AUDITOR"]);
    assert_eq!(eval(&flow, &audit, &held), Outcome::Allowed);
    assert_eq!(
        eval(&flow, &prefixed, &holding(&["OTHER"])),
        Outcome::Denied
    );
    assert_eq!(held.roles.len(), 1, "held names are not rewritten");
}

#[test]
fn another_applications_convention_does_not_leak() {
    let admin = Condition::Role("ADMIN".into());
    let mut node = screen("n", AccessState::Protected(admin.clone()));
    node.application = Some("web".into());
    let mut backend = screen(
        "s",
        AccessState::Protected(Condition::Role("ROLE_ADMIN".into())),
    );
    backend.application = Some("api".into());
    let flow = UiFlow {
        screens: vec![node, backend],
        role_conventions: vec![spring("api")],
        ..UiFlow::default()
    };
    // another application's convention never applies to it
    assert_eq!(flow.convention(Some("web")).role_prefix, None);
    assert_eq!(
        flow.convention(Some("api")).role_prefix.as_deref(),
        Some("ROLE_")
    );
    // the Node screen keeps its name: ADMIN is allowed as typed
    assert_eq!(eval(&flow, &admin, &holding(&["ADMIN"])), Outcome::Allowed);
    assert_eq!(
        eval(&flow, &admin, &holding(&["ROLE_ADMIN"])),
        Outcome::Denied
    );
    let single = UiFlow {
        screens: vec![flow.screens[1].clone()],
        role_conventions: vec![spring("api")],
        ..UiFlow::default()
    };
    assert_eq!(
        single.convention(None).role_prefix.as_deref(),
        Some("ROLE_")
    );
}

fn screen(id: &str, access: AccessState) -> Screen {
    Screen {
        id: id.into(),
        name: id.into(),
        kind: ScreenKind::Page,
        route: Some(if id == "home" {
            "/".into()
        } else {
            format!("/{id}")
        }),
        application: None,
        file: "A.tsx".into(),
        line: 1,
        access,
        weak: false,
        source: "test".into(),
    }
}

#[test]
fn a_screen_without_declaration_is_undeclared_not_sure() {
    let link = |to: &str, condition| Transition {
        from: Origin::Screen("home".into()),
        target: Target::Screen(to.into()),
        trigger: None,
        file: "A.tsx".into(),
        line: 1,
        condition,
        weak: false,
        source: "test".into(),
    };
    let flow = UiFlow {
        screens: vec![
            screen("home", AccessState::Open),
            screen("plain", AccessState::Undeclared),
            screen("open", AccessState::Open),
            screen("shut", AccessState::Protected(Condition::Never)),
        ],
        transitions: vec![
            link("plain", AccessState::Open),
            link("open", AccessState::Open),
            link("shut", AccessState::Open),
        ],
        ..UiFlow::default()
    };
    let reached = reachable_screens(&flow, "home", &Assignment::default());
    assert_eq!(reached["home"], Reach::Sure);
    assert_eq!(reached["plain"], Reach::Undeclared);
    assert_eq!(reached["open"], Reach::Sure);
    assert!(!reached.contains_key("shut"));
    assert_eq!(
        screen_reaches(&flow, &Assignment::default())["plain"],
        ScreenReach::Undeclared
    );
}

#[test]
fn access_state_reads_old_and_new_files() {
    let read = |json: &str| serde_json::from_str::<Screen>(json).unwrap().access;
    let base = |access: &str| {
        format!(r#"{{"id":"a","name":"a","kind":"page","file":"f","line":1,{access}"source":"t"}}"#)
    };
    assert_eq!(read(&base("")), AccessState::Undeclared);
    assert_eq!(read(&base(r#""access":null,"#)), AccessState::Undeclared);
    assert_eq!(read(&base(r#""access":"open","#)), AccessState::Open);
    assert_eq!(
        read(&base(r#""access":"undeclared","#)),
        AccessState::Undeclared
    );
    let auth = AccessState::Protected(Condition::Authenticated);
    assert_eq!(
        read(&base(r#""access":"authenticated","#)),
        auth,
        "old bare condition"
    );
    assert_eq!(
        read(&base(r#""access":{"role":"Admin"},"#)),
        AccessState::Protected(Condition::Role("Admin".into()))
    );
    assert_eq!(
        read(&base(r#""access":{"protected":"authenticated"},"#)),
        auth
    );
    // Round trip: undeclared is omitted, the others survive.
    let s = screen("a", AccessState::Open);
    assert_eq!(
        serde_json::from_str::<Screen>(&serde_json::to_string(&s).unwrap()).unwrap(),
        s
    );
    assert!(
        !serde_json::to_string(&screen("a", AccessState::Undeclared))
            .unwrap()
            .contains("access")
    );
}

#[test]
fn access_state_layers_and_defaults() {
    use AccessState::*;
    let role = |n: &str| Protected(Condition::Role(n.into()));
    assert_eq!(Undeclared.and(role("a")), role("a"));
    assert_eq!(Open.and(role("a")), role("a"));
    assert_eq!(Open.and(Undeclared), Open);
    assert_eq!(
        role("a").and(role("b")),
        Protected(Condition::All(vec![
            Condition::Role("a".into()),
            Condition::Role("b".into())
        ]))
    );
    assert_eq!(Undeclared.or_default(Open), Open);
    assert_eq!(role("a").or_default(Open), role("a"));
    assert_eq!(AccessState::from_parts(None, true), Open);
    assert_eq!(
        AccessState::from_parts(Some(Condition::Never), true),
        Protected(Condition::Never)
    );
}
