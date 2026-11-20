//! Permission matrix over a [`UiFlow`] (ADR 0021): one row per screen or
//! endpoint, one column per role, each cell the simulator's own evaluation.
use crate::ui_flow::{
    AccessState, Assignment, Outcome, RoleKind, UiFlow, evaluate_access, evaluate_condition,
    resolve_assignment,
};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixKind {
    Screens,
    Endpoints,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixRowKind {
    Screen,
    Endpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatrixRow {
    pub kind: MatrixRowKind,
    /// Screen route (or name without one), or `METHOD path`.
    pub label: String,
    pub file: String,
    pub line: u32,
    /// The guard in words: `none`, `declared open`, `undeclared`, or the condition.
    pub requires: String,
    /// One outcome per [`PermissionMatrix::roles`] entry.
    pub cells: Vec<Outcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PermissionMatrix {
    pub roles: Vec<String>,
    pub rows: Vec<MatrixRow>,
}

/// Columns: `anonymous`, `signed in`, then the catalog's roles and groups
/// (not its permission ids: a project can define over a hundred). A role
/// column is a signed-in user holding exactly that role, with the catalog's
/// hierarchy and policies applied as in the simulation, and holding both `X`
/// and `ROLE_X` (Spring's prefix). Screens are judged by
/// their own access (no guard allowed), not by a navigation path; an
/// undeclared endpoint and every unreadable condition stay `unknown`.
pub fn permission_matrix(flow: &UiFlow, kind: MatrixKind) -> PermissionMatrix {
    let names: BTreeSet<&str> = (flow.roles.iter())
        .filter(|d| d.kind != RoleKind::Permission)
        .map(|d| d.name.as_str())
        .collect();
    let assignment = |authenticated, roles: &[&str]| {
        resolve_assignment(
            flow,
            &Assignment {
                authenticated,
                // Spring writes the role `ADMIN` as `ROLE_ADMIN` in guards; either spelling is held.
                roles: (roles.iter())
                    .flat_map(|r| [r.to_string(), format!("ROLE_{r}")])
                    .collect(),
                ..Assignment::default()
            },
        )
    };
    let mut assignments = vec![assignment(false, &[]), assignment(true, &[])];
    assignments.extend(names.iter().map(|n| assignment(true, &[n])));
    let roles = ["anonymous", "signed in"]
        .into_iter()
        .chain(names.iter().copied())
        .map(String::from)
        .collect();
    let mut rows = Vec::new();
    if kind != MatrixKind::Endpoints {
        for s in &flow.screens {
            rows.push(MatrixRow {
                kind: MatrixRowKind::Screen,
                label: s.route.clone().unwrap_or_else(|| s.name.clone()),
                file: s.file.clone(),
                line: s.line,
                requires: s.access.as_ref().map_or("none".into(), |c| c.text()),
                cells: (assignments.iter())
                    .map(|a| {
                        s.access
                            .as_ref()
                            .map_or(Outcome::Allowed, |c| evaluate_condition(c, a))
                    })
                    .collect(),
            });
        }
    }
    if kind != MatrixKind::Screens {
        for e in &flow.endpoints {
            rows.push(MatrixRow {
                kind: MatrixRowKind::Endpoint,
                label: format!("{} {}", e.method, e.path),
                file: e.file.clone(),
                line: e.line,
                requires: match &e.access {
                    AccessState::Protected(c) => c.text(),
                    AccessState::Open => "declared open".into(),
                    AccessState::Undeclared => "undeclared".into(),
                },
                cells: assignments
                    .iter()
                    .map(|a| evaluate_access(&e.access, a))
                    .collect(),
            });
        }
    }
    PermissionMatrix { roles, rows }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_flow::{Condition, EndpointAccess, PolicyDef, RoleDef, Screen, ScreenKind};
    use Outcome::{Allowed as A, Denied as D, Unknown as U};

    fn screen(route: &str, access: Option<Condition>) -> Screen {
        Screen {
            id: route.into(),
            name: route.into(),
            kind: ScreenKind::Page,
            route: Some(route.into()),
            application: None,
            file: "S.razor".into(),
            line: 1,
            access,
            source: "test".into(),
        }
    }

    fn endpoint(path: &str, access: AccessState) -> EndpointAccess {
        EndpointAccess {
            method: "GET".into(),
            path: path.into(),
            access,
            application: None,
            file: "Api.cs".into(),
            line: 2,
            source: "test".into(),
            handler: None,
            sends: vec![],
        }
    }

    fn def(name: &str, kind: RoleKind, includes: &[&str]) -> RoleDef {
        RoleDef {
            name: name.into(),
            kind,
            includes: includes.iter().map(|s| s.to_string()).collect(),
            file: "R.cs".into(),
            line: 1,
            source: "test".into(),
        }
    }

    #[test]
    fn cells_follow_the_simulator_and_unknown_is_never_allowed() {
        let flow = UiFlow {
            screens: vec![
                screen("/", None),
                screen("/account", Some(Condition::Authenticated)),
                screen("/admin", Some(Condition::Role("admin".into()))),
                screen("/purge", Some(Condition::Policy("CanPurge".into()))),
                screen("/odd", Some(Condition::Unknown("x()".into()))),
                screen("/spring", Some(Condition::Role("ROLE_admin".into()))),
            ],
            endpoints: vec![
                endpoint("/open", AccessState::Open),
                endpoint("/legacy", AccessState::Undeclared),
                endpoint(
                    "/api",
                    AccessState::Protected(Condition::Custom("tenant".into())),
                ),
            ],
            roles: vec![
                def("admin", RoleKind::Role, &["user"]),
                def("user", RoleKind::Role, &[]),
                def("app.read", RoleKind::Permission, &[]),
            ],
            policies: vec![PolicyDef {
                name: "CanPurge".into(),
                requires: Condition::Role("admin".into()),
                file: "P.cs".into(),
                line: 1,
                source: "test".into(),
            }],
            ..UiFlow::default()
        };
        let m = permission_matrix(&flow, MatrixKind::Both);
        // permissions are no columns
        assert_eq!(m.roles, ["anonymous", "signed in", "admin", "user"]);
        let cells: Vec<(&str, Vec<Outcome>)> = m
            .rows
            .iter()
            .map(|r| (r.label.as_str(), r.cells.clone()))
            .collect();
        assert_eq!(
            cells,
            [
                ("/", vec![A, A, A, A]),
                ("/account", vec![D, A, A, A]),
                ("/admin", vec![D, D, A, D]),
                ("/purge", vec![D, D, A, D]),
                ("/odd", vec![U, U, U, U]),
                ("/spring", vec![D, D, A, D]),
                ("GET /open", vec![A, A, A, A]),
                ("GET /legacy", vec![U, U, U, U]),
                ("GET /api", vec![U, U, U, U]),
            ]
        );
        assert_eq!(m.rows[8].requires, "custom tenant");
        let only = |k| permission_matrix(&flow, k).rows.len();
        assert_eq!(
            (only(MatrixKind::Screens), only(MatrixKind::Endpoints)),
            (6, 3)
        );
    }
}
