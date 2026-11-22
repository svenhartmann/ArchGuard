//! Access layers tab (LOGIC.md section 7, item 8): the chain per endpoint from
//! `archguard_core::access_chain`, shown as a table with a detail of the chain.
//! Unknown never reads as open: `undeclared` and "no check read" stay words of
//! their own, and a path-only or hint layer carries its own badge.

use archguard_engine::core::{
    AccessState, ChainFrom, ChainRow, EnforcedAt, ServiceHow, ServiceLayer, UiFlow, access_chain,
};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::context::Loaded;
use crate::screens_ui::words;
use crate::tab_support::{self, Actions, TabState, Texts, index};
use crate::ui::{AccessData, AccessModel, AlLine, AlRow, AppWindow};

pub struct AccessLayersState {
    flow: Result<UiFlow, String>,
    show_unguarded: bool,
    from: ChainFrom,
    only: bool,
    selected: usize,
}

pub fn wire(ui: &AppWindow) {
    let d = ui.global::<AccessData>();
    let a = Actions::<AccessLayersState>::new(ui);
    d.on_set_from(a.on(|s, _, i: i32| {
        s.from = if i == 1 {
            ChainFrom::Screen
        } else {
            ChainFrom::Endpoint
        };
        s.selected = 0;
    }));
    d.on_toggle_only(a.on0(|s, _| {
        s.only = !s.only;
        s.selected = 0;
    }));
    d.on_select_row(a.on(|s, _, i: i32| s.selected = index(i).unwrap_or(0)));
}

impl TabState for AccessLayersState {
    fn push(&mut self, ui: &AppWindow) {
        ui.global::<AccessData>()
            .set_model(build(self, &Texts::of(ui)));
    }

    fn pending(ui: &AppWindow, error: Option<&str>) {
        let model = match error {
            None => AccessModel::default(),
            Some(e) => AccessModel {
                state: 2,
                error: e.into(),
                ..Default::default()
            },
        };
        ui.global::<AccessData>().set_model(model);
    }
}

/// Show the active document's tab; the flow is read on a worker thread.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let root = data.root.to_path_buf();
        move || {
            let show_unguarded = archguard_engine::load_config(None, &root)
                .map_or(true, |l| l.config.views.access_layers.show_unguarded);
            AccessLayersState {
                flow: archguard_engine::ui_flow::analyze(&root).map_err(|e| e.to_string()),
                show_unguarded,
                from: ChainFrom::Endpoint,
                only: false,
                selected: 0,
            }
        }
    });
}

/// 0 none, 1 enforced, 2 path only, 3 hint, 4 data filter; the strongest of `layers`.
fn how_kind<'a>(layers: impl IntoIterator<Item = &'a ServiceLayer>) -> i32 {
    layers
        .into_iter()
        .map(|l| match l.how {
            ServiceHow::Enforced if matches!(l.state, AccessState::Protected(_)) => 1,
            // an enforced layer without a readable condition is no protection
            ServiceHow::Enforced | ServiceHow::PathOnly => 2,
            ServiceHow::Hint => 3,
            ServiceHow::DataFilter => 4,
        })
        .min()
        .unwrap_or(0)
}

/// `Class.method` -> its strongest layer kind over all endpoints; empty when the flow cannot be read.
pub fn method_layers(root: &std::path::Path) -> std::collections::HashMap<String, i32> {
    let Ok(flow) = archguard_engine::ui_flow::analyze(root) else {
        return Default::default();
    };
    layers_by_target(&flow)
}

fn layers_by_target(flow: &UiFlow) -> std::collections::HashMap<String, i32> {
    let mut out = std::collections::HashMap::new();
    for l in &flow.service_layers {
        let kind = how_kind([l]);
        let e = out.entry(l.target.clone()).or_insert(kind);
        *e = (*e).min(kind);
    }
    out
}

fn state_kind(s: &AccessState) -> i32 {
    match s {
        AccessState::Undeclared => 0,
        AccessState::Open => 1,
        AccessState::Protected(_) => 2,
    }
}

fn layers_text(texts: &Texts, layers: &[ServiceLayer]) -> String {
    if layers.is_empty() {
        return "-".into();
    }
    (layers.iter())
        .map(|l| format!("{}: {}", l.target, words::endpoint_access(texts, &l.state)))
        .collect::<Vec<_>>()
        .join("; ")
}

fn screens_text(r: &ChainRow) -> String {
    if r.screens.is_empty() {
        return "-".into();
    }
    (r.screens.iter())
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn row(texts: &Texts, r: &ChainRow, pressed: bool) -> AlRow {
    AlRow {
        method: r.method.as_str().into(),
        path: r.path.as_str().into(),
        screens: screens_text(r).into(),
        endpoint: words::endpoint_access(texts, &r.access).into(),
        endpoint_kind: state_kind(&r.access),
        service: layers_text(texts, &r.services).into(),
        service_kind: how_kind(&r.services),
        data: layers_text(texts, &r.data).into(),
        data_kind: how_kind(&r.data),
        effective: words::endpoint_access(texts, &r.effective).into(),
        effective_kind: state_kind(&r.effective),
        enforced_at: match r.enforced_at {
            EnforcedAt::None => 0,
            EnforcedAt::Endpoint => 1,
            EnforcedAt::Service => 2,
            EnforcedAt::Both => 3,
        },
        pressed,
    }
}

/// The chain of one row, top (screens) to bottom (data).
fn detail(texts: &Texts, r: &ChainRow) -> Vec<AlLine> {
    let line = |layer, text: String, site: String, state: String, how| AlLine {
        layer,
        text: text.into(),
        site: site.into(),
        state: state.into(),
        how,
    };
    let mut out: Vec<AlLine> = (r.screens.iter())
        .map(|s| {
            let text = s.trigger.clone().unwrap_or_default();
            line(
                0,
                format!("{} {text}", s.name),
                format!("{}:{}", s.file, s.line),
                String::new(),
                0,
            )
        })
        .collect();
    out.push(line(
        1,
        format!("{} {}", r.method, r.path),
        format!("{}:{}", r.file, r.line),
        words::endpoint_access(texts, &r.access),
        0,
    ));
    for (layer, items) in [(2, &r.services), (3, &r.data)] {
        out.extend(items.iter().map(|l| {
            let text = if l.note.is_empty() {
                l.target.clone()
            } else {
                format!("{} - {}", l.target, l.note)
            };
            line(
                layer,
                text,
                format!("{}:{}", l.file, l.line),
                words::endpoint_access(texts, &l.state),
                how_kind([l]),
            )
        }));
    }
    out
}

fn build(s: &AccessLayersState, texts: &Texts) -> AccessModel {
    let flow = match &s.flow {
        Ok(f) => f,
        Err(e) => {
            return AccessModel {
                state: 2,
                error: e.as_str().into(),
                ..Default::default()
            };
        }
    };
    let chain = access_chain(flow, s.from, s.only, s.show_unguarded);
    let selected = s.selected.min(chain.len().saturating_sub(1));
    let rows: Vec<AlRow> = (chain.iter().enumerate())
        .map(|(i, r)| row(texts, r, i == selected))
        .collect();
    let pick = chain.get(selected);
    AccessModel {
        state: 1,
        from: i32::from(s.from == ChainFrom::Screen),
        only: s.only,
        rows: ModelRc::new(VecModel::from(rows)),
        detail_title: pick
            .map(|r| format!("{} {}", r.method, r.path))
            .unwrap_or_default()
            .into(),
        detail_effective: pick
            .map(|r| words::endpoint_access(texts, &r.effective))
            .unwrap_or_default()
            .into(),
        detail: ModelRc::new(VecModel::from(
            pick.map(|r| detail(texts, r)).unwrap_or_default(),
        )),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Condition, EndpointAccess, ServiceKind};
    use slint::Model;

    fn state(only: bool) -> AccessLayersState {
        let ep = |path: &str, line, access| EndpointAccess {
            method: "GET".into(),
            path: path.into(),
            access,
            application: None,
            file: "C.java".into(),
            line,
            source: "test".into(),
            handler: None,
            sends: Default::default(),
        };
        let layer = |how, line| ServiceLayer {
            endpoint_file: "C.java".into(),
            endpoint_line: line,
            endpoint_method: "GET".into(),
            endpoint_path: format!("/{}", if line == 1 { "a" } else { "b" }),
            kind: ServiceKind::Service,
            target: "S.m".into(),
            file: "S.java".into(),
            line: 9,
            state: AccessState::Protected(Condition::Role("B".into())),
            how,
            note: String::new(),
        };
        AccessLayersState {
            flow: Ok(UiFlow {
                endpoints: vec![
                    ep("/a", 1, AccessState::Open),
                    ep("/b", 2, AccessState::Undeclared),
                ],
                service_layers: vec![
                    layer(ServiceHow::Enforced, 1),
                    layer(ServiceHow::PathOnly, 2),
                ],
                ..UiFlow::default()
            }),
            show_unguarded: true,
            from: ChainFrom::Endpoint,
            only,
            selected: 0,
        }
    }

    #[test]
    fn service_only_and_path_only_stay_apart_and_unknown_is_not_open() {
        let m = build(&state(false), &Texts::default());
        let rows: Vec<AlRow> = m.rows.iter().collect();
        // /a: open endpoint, enforced service check: flagged service only
        assert_eq!((rows[0].enforced_at, rows[0].service_kind), (2, 1));
        // /b: undeclared endpoint, path-only layer: no protection, still undeclared
        assert_eq!((rows[1].enforced_at, rows[1].service_kind), (0, 2));
        assert_eq!(
            (rows[1].effective_kind, rows[1].effective.as_str()),
            (0, "undeclared")
        );
        assert_eq!(m.detail.row_count(), 2);
        // both endpoints name S.m: the enforced layer wins over the path-only one
        assert_eq!(
            layers_by_target(state(false).flow.as_ref().unwrap())["S.m"],
            1
        );
        assert_eq!(build(&state(true), &Texts::default()).rows.row_count(), 1);
    }
}
