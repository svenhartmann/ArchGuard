//! The access chain per endpoint (LOGIC.md section 7, item 8): screens that call
//! it, the endpoint's own check, the checks below it and the effective result.
use crate::ui_flow::{
    AccessState, EndpointAccess, ServiceHow, ServiceKind, ServiceLayer, UiFlow,
    join_actions_to_endpoints,
};
use crate::ui_flow::{Action, Origin};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainFrom {
    Endpoint,
    Screen,
}

/// Where a check is enforced. `None` means no check was read, not that the
/// endpoint is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcedAt {
    None,
    Endpoint,
    Service,
    Both,
}

impl std::fmt::Display for EnforcedAt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::None => "none",
            Self::Endpoint => "endpoint",
            Self::Service => "service",
            Self::Both => "both",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainScreen {
    pub name: String,
    pub trigger: Option<String>,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainRow {
    pub method: String,
    pub path: String,
    pub application: Option<String>,
    pub file: String,
    pub line: u32,
    pub screens: Vec<ChainScreen>,
    pub access: AccessState,
    pub effective: AccessState,
    pub services: Vec<ServiceLayer>,
    pub data: Vec<ServiceLayer>,
    pub enforced_at: EnforcedAt,
    /// Protected only below the endpoint: the endpoint itself is open or undeclared.
    pub service_guarded: bool,
}

/// A state in words; unknown stays `undeclared`, never open.
pub fn state_text(s: &AccessState) -> String {
    match s {
        AccessState::Protected(c) => c.text(),
        AccessState::Open => "open".into(),
        AccessState::Undeclared => "undeclared".into(),
    }
}

/// `Svc.run: role B`, with `(path only)`, `(hint)` or `(data filter)` for a
/// layer that does not protect on every path.
pub fn layer_text(l: &ServiceLayer) -> String {
    let tag = match l.how {
        ServiceHow::Enforced => "",
        ServiceHow::PathOnly => " (path only)",
        ServiceHow::Hint => " (hint)",
        ServiceHow::DataFilter => " (data filter)",
    };
    format!("{}: {}{tag}", l.target, state_text(&l.state))
}

fn enforced(l: &ServiceLayer) -> bool {
    l.how == ServiceHow::Enforced && matches!(l.state, AccessState::Protected(_))
}

/// Where `e` is enforced: its own check, an enforced layer below it, both or none.
pub fn enforced_at(flow: &UiFlow, e: &EndpointAccess) -> EnforcedAt {
    let own = matches!(e.access, AccessState::Protected(_));
    let below = flow.service_layers_of(e).any(enforced);
    match (own, below) {
        (true, true) => EnforcedAt::Both,
        (true, false) => EnforcedAt::Endpoint,
        (false, true) => EnforcedAt::Service,
        _ => EnforcedAt::None,
    }
}

fn row(flow: &UiFlow, e: &EndpointAccess, show_unguarded: bool) -> ChainRow {
    let layers = |kind| {
        (flow.service_layers_of(e))
            .filter(|l| l.kind == kind && (show_unguarded || enforced(l)))
            .cloned()
            .collect::<Vec<_>>()
    };
    let at = enforced_at(flow, e);
    ChainRow {
        method: e.method.clone(),
        path: e.path.clone(),
        application: e.application.clone(),
        file: e.file.clone(),
        line: e.line,
        screens: vec![],
        access: e.access.clone(),
        effective: flow.effective_access(e),
        services: layers(ServiceKind::Service),
        data: layers(ServiceKind::Data),
        enforced_at: at,
        service_guarded: at == EnforcedAt::Service,
    }
}

fn screen_of(flow: &UiFlow, a: &Action) -> (String, ChainScreen) {
    let name = match &a.screen {
        Origin::Screen(id) => (flow.screens.iter().find(|s| s.id == *id))
            .map_or(id.clone(), |s| s.route.clone().unwrap_or(s.name.clone())),
        Origin::AnyScreen { application } => format!("*{application}"),
    };
    let screen = ChainScreen {
        name: name.clone(),
        trigger: a.trigger.clone(),
        file: a.file.clone(),
        line: a.line,
    };
    (name, screen)
}

/// One row per endpoint (`Endpoint`, screens listed on it) or one per
/// screen/endpoint pair sorted by screen (`Screen`, endpoints no screen calls
/// are left out). `only_service_guarded` keeps rows protected only below the endpoint.
pub fn access_chain(
    flow: &UiFlow,
    from: ChainFrom,
    only_service_guarded: bool,
    show_unguarded: bool,
) -> Vec<ChainRow> {
    let mut rows: Vec<ChainRow> = flow
        .endpoints
        .iter()
        .map(|e| row(flow, e, show_unguarded))
        .collect();
    for (action, hits) in join_actions_to_endpoints(flow) {
        let (_, screen) = screen_of(flow, action);
        for e in hits {
            for r in rows.iter_mut().filter(|r| same(r, e)) {
                if !r.screens.contains(&screen) {
                    r.screens.push(screen.clone());
                }
            }
        }
    }
    if from == ChainFrom::Screen {
        let mut split: Vec<ChainRow> = rows
            .into_iter()
            .flat_map(|r| {
                r.screens.clone().into_iter().map(move |s| ChainRow {
                    screens: vec![s],
                    ..r.clone()
                })
            })
            .collect();
        split.sort_by(|a, b| a.screens[0].name.cmp(&b.screens[0].name));
        rows = split;
    }
    rows.retain(|r| !only_service_guarded || r.service_guarded);
    rows
}

fn same(r: &ChainRow, e: &EndpointAccess) -> bool {
    r.file == e.file && r.line == e.line && r.method == e.method && r.path == e.path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_flow::{Call, Condition};

    fn endpoint(path: &str, access: AccessState) -> EndpointAccess {
        EndpointAccess {
            method: "GET".into(),
            path: path.into(),
            access,
            application: None,
            file: "C.java".into(),
            line: 5,
            source: "test".into(),
            handler: None,
            sends: Default::default(),
        }
    }

    fn layer(e: &EndpointAccess, how: ServiceHow, state: AccessState) -> ServiceLayer {
        ServiceLayer {
            endpoint_file: e.file.clone(),
            endpoint_line: e.line,
            endpoint_method: e.method.clone(),
            endpoint_path: e.path.clone(),
            kind: ServiceKind::Service,
            target: "S.m".into(),
            file: "S.java".into(),
            line: 3,
            state,
            how,
            note: String::new(),
        }
    }

    #[test]
    fn service_only_protection_is_flagged_and_unknown_is_not_open() {
        let admin = AccessState::Protected(Condition::Role("ADMIN".into()));
        let a = endpoint("/a", AccessState::Open);
        let b = endpoint("/b", AccessState::Undeclared);
        let c = endpoint("/c", admin.clone());
        let mut flow = UiFlow {
            endpoints: vec![a.clone(), b.clone(), c.clone()],
            ..UiFlow::default()
        };
        // same file/line for all three: give distinct lines
        flow.endpoints[1].line = 6;
        flow.endpoints[2].line = 7;
        let (b, c) = (flow.endpoints[1].clone(), flow.endpoints[2].clone());
        flow.service_layers = vec![
            layer(&a, ServiceHow::Enforced, admin.clone()),
            layer(&b, ServiceHow::PathOnly, admin.clone()),
            layer(&c, ServiceHow::Enforced, admin.clone()),
        ];
        let rows = access_chain(&flow, ChainFrom::Endpoint, false, true);
        assert_eq!(
            rows.iter().map(|r| r.enforced_at).collect::<Vec<_>>(),
            [EnforcedAt::Service, EnforcedAt::None, EnforcedAt::Both]
        );
        assert_eq!(rows[1].effective, AccessState::Undeclared);
        let only = access_chain(&flow, ChainFrom::Endpoint, true, true);
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].path, "/a");
        // show_unguarded = false hides the path-only layer
        let hidden = access_chain(&flow, ChainFrom::Endpoint, false, false);
        assert!(hidden[1].services.is_empty());
    }

    #[test]
    fn layer_words_mark_what_does_not_protect() {
        let e = endpoint("/a", AccessState::Open);
        let l = layer(&e, ServiceHow::Hint, AccessState::Undeclared);
        assert_eq!(layer_text(&l), "S.m: undeclared (hint)");
    }

    #[test]
    fn screens_join_by_call_and_split_by_screen() {
        let a = endpoint("/a", AccessState::Open);
        let flow = UiFlow {
            endpoints: vec![a],
            actions: vec![Action {
                screen: Origin::AnyScreen {
                    application: "web".into(),
                },
                trigger: None,
                call: Call::Http {
                    method: "GET".into(),
                    path: "/a".into(),
                },
                file: "a.ts".into(),
                line: 1,
                condition: AccessState::Undeclared,
                weak: false,
                source: "test".into(),
            }],
            ..UiFlow::default()
        };
        let by_ep = access_chain(&flow, ChainFrom::Endpoint, false, true);
        assert_eq!(by_ep[0].screens.len(), 1);
        let by_screen = access_chain(&flow, ChainFrom::Screen, false, true);
        assert_eq!(by_screen[0].screens[0].name, "*web");
    }
}
