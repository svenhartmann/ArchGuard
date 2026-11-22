//! Access layers tab (LOGIC.md section 7, item 8): the chain per endpoint from
//! `archguard_core::access_chain`, shown as a table with a detail of the chain.
//! Unknown never reads as open: `undeclared` and "no check read" stay words of
//! their own, and a path-only or hint layer carries its own badge.

use archguard_engine::core::{
    AccessState, CallMethod, ChainFrom, ChainRow, EnforcedAt, ServiceHow, ServiceLayer, UiFlow,
    access_chain,
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::sync::Arc;

use crate::context::Loaded;
use crate::screens_ui::words;
use crate::tab_support::{self, Actions, Flow, TabState, Texts, index};
use crate::ui::{AccessData, AccessModel, AlLine, AlRow, AppWindow};

pub struct AccessLayersState {
    flow: Arc<Result<UiFlow, String>>,
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
        let flow = Flow::of(data);
        move || {
            let show_unguarded = archguard_engine::load_config(None, &root)
                .map_or(true, |l| l.config.views.access_layers.show_unguarded);
            AccessLayersState {
                flow: flow.get(),
                show_unguarded,
                from: ChainFrom::Endpoint,
                only: false,
                selected: 0,
            }
        }
    });
}

/// 0 none, 1 enforced, 2 enforced with an unreadable condition (never open), 3 path only,
/// 4 hint, 5 data filter; the strongest of `layers`.
fn how_kind<'a>(layers: impl IntoIterator<Item = &'a ServiceLayer>) -> i32 {
    layers
        .into_iter()
        .map(|l| match l.how {
            ServiceHow::Enforced if matches!(l.state, AccessState::Protected(_)) => 1,
            ServiceHow::Enforced => 2,
            ServiceHow::PathOnly => 3,
            ServiceHow::Hint => 4,
            ServiceHow::DataFilter => 5,
        })
        .min()
        .unwrap_or(0)
}

/// (file, `Class.method`) -> its strongest layer kind over all endpoints; empty when the flow cannot be read.
pub type MethodLayers = std::collections::HashMap<(String, String), i32>;

pub fn method_layers(flow: &Result<UiFlow, String>) -> MethodLayers {
    flow.as_ref().map(layers_by_target).unwrap_or_default()
}

fn layers_by_target(flow: &UiFlow) -> MethodLayers {
    let mut out = std::collections::HashMap::new();
    for l in &flow.service_layers {
        let kind = how_kind([l]);
        let e = out
            .entry((l.file.clone(), l.target.clone()))
            .or_insert(kind);
        *e = (*e).min(kind);
    }
    out
}

/// The badge of method `m`: none unless the pairing is bijective, i.e. exactly one layer names
/// `m`'s file (up to a leading path prefix) and `Class.method`, and `m` is the only method of
/// `methods` that layer maps to (overloads and same-named classes elsewhere get none).
pub fn badge(layers: &MethodLayers, methods: &[CallMethod], m: &CallMethod) -> i32 {
    let target = |m: &CallMethod| format!("{}.{}", short_class(&m.class_id), m.name);
    // `analyze_calls` paths drop the project-relative prefix (`src/`): match on a path boundary.
    let fits = |layer_file: &str, m: &CallMethod| {
        layer_file == m.file || layer_file.ends_with(&format!("/{}", m.file))
    };
    let mine = target(m);
    let mut hits = (layers.iter()).filter(|((f, t), _)| *t == mine && fits(f, m));
    let (Some(((file, _), kind)), None) = (hits.next(), hits.next()) else {
        return 0;
    };
    let rivals = methods
        .iter()
        .filter(|o| target(o) == mine && fits(file, o));
    if rivals.count() == 1 { *kind } else { 0 }
}

fn short_class(class_id: &str) -> &str {
    class_id.rsplit('.').next().unwrap_or(class_id)
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
    let flow = match &*s.flow {
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
            flow: Arc::new(Ok(UiFlow {
                endpoints: vec![
                    ep("/a", 1, AccessState::Open),
                    ep("/b", 2, AccessState::Undeclared),
                ],
                service_layers: vec![
                    layer(ServiceHow::Enforced, 1),
                    layer(ServiceHow::PathOnly, 2),
                ],
                ..UiFlow::default()
            })),
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
        assert_eq!((rows[1].enforced_at, rows[1].service_kind), (0, 3));
        assert_eq!(
            (rows[1].effective_kind, rows[1].effective.as_str()),
            (0, "undeclared")
        );
        assert_eq!(m.detail.row_count(), 2);
        // both endpoints name S.m: the enforced layer wins over the path-only one
        assert_eq!(
            layers_by_target((*state(false).flow).as_ref().unwrap())
                [&("S.java".into(), "S.m".into())],
            1
        );
        assert_eq!(build(&state(true), &Texts::default()).rows.row_count(), 1);
    }

    /// Two classes named `UserService` in different files: only the guarded one is badged.
    #[test]
    fn a_badge_needs_the_same_file_and_no_overload() {
        let mut st = state(false);
        let Ok(flow) = Arc::get_mut(&mut st.flow).unwrap().as_mut() else {
            panic!()
        };
        flow.service_layers.truncate(1);
        flow.service_layers[0].target = "UserService.delete".into();
        flow.service_layers[0].file = "admin/UserService.java".into();
        let map = layers_by_target(flow);
        let ms = [
            method("admin.UserService", "delete", "admin/UserService.java"),
            method("web.UserService", "delete", "web/UserService.java"),
        ];
        assert_eq!(badge(&map, &ms, &ms[0]), 1);
        assert_eq!(badge(&map, &ms, &ms[1]), 0);
        // an overload in the same file cannot be told apart: no badge
        let over = [ms[0].clone(), ms[0].clone()];
        assert_eq!(badge(&map, &over, &over[0]), 0);
    }

    fn method(class_id: &str, name: &str, file: &str) -> CallMethod {
        CallMethod {
            id: format!("{class_id}.{name}"),
            class_id: class_id.into(),
            name: name.into(),
            signature: String::new(),
            file: file.into(),
            line: 1,
            end_line: None,
            is_static: false,
            complexity: None,
        }
    }

    #[test]
    fn enforced_with_unreadable_condition_is_not_path_only() {
        let mut st = state(false);
        let Ok(flow) = Arc::get_mut(&mut st.flow).unwrap().as_mut() else {
            panic!()
        };
        flow.service_layers[0].state = AccessState::Undeclared;
        assert_eq!(how_kind([&flow.service_layers[0]]), 2);
        assert_eq!(how_kind([&flow.service_layers[1]]), 3, "path only");
    }

    /// Both paths come from the real producers (`ui_flow::analyze` and `analyze_calls`) on a
    /// Spring project (needs a JDK, skipped without one).
    #[test]
    fn real_layer_and_call_paths_meet() {
        let jdk = |t: &str| {
            std::process::Command::new(t)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !(jdk("java") && jdk("javac")) {
            eprintln!("skipped: no JDK");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let svc = |pkg: &str, ann: &str| {
            format!(
                "package {pkg};\nimport org.springframework.stereotype.Service;\nimport org.springframework.security.access.prepost.*;\n@Service\nclass Svc {{\n{ann} String run() {{ return \"x\"; }}\n}}\n"
            )
        };
        for (path, text) in [
            ("pom.xml", "<project/>".to_string()),
            ("src/main/java/app/Conf.java", "package app;\nimport org.springframework.security.config.annotation.method.configuration.EnableMethodSecurity;\n@EnableMethodSecurity\nclass Conf {}\n".into()),
            ("src/main/java/app/Ctl.java", "package app;\nimport org.springframework.web.bind.annotation.*;\n@RestController\nclass Ctl {\n  private Svc svc;\n  @GetMapping(\"/a\")\n  String a() { return svc.run(); }\n}\n".into()),
            ("src/main/java/app/Svc.java", svc("app", "@PreAuthorize(\"hasRole('B')\")")),
            ("src/main/java/other/Svc.java", svc("other", "")),
        ] {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let layers = method_layers(&Flow::unshared(dir.path()).get());
        let config = archguard_engine::load_config(None, dir.path())
            .unwrap()
            .config;
        let analysis = archguard_engine::analyze_calls(dir.path(), &config);
        let badge_of = |class: &str| {
            let m = analysis
                .methods
                .iter()
                .find(|m| m.name == "run" && m.class_id.ends_with(class))
                .unwrap_or_else(|| panic!("no {class}.run in {:?}", analysis.methods));
            badge(&layers, &analysis.methods, m)
        };
        assert_eq!(badge_of("app.Svc"), 1, "layers: {layers:?}");
        assert_eq!(badge_of("other.Svc"), 0);
    }

    /// Non-standard layout: the root `src/main/...` and `admin/main/...` both hold a `Svc.run`;
    /// only the admin one is guarded. Paths come from the real producers (needs a JDK).
    #[test]
    fn a_shortened_path_that_fits_two_layers_gets_no_badge() {
        let jdk = |t: &str| {
            std::process::Command::new(t)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !(jdk("java") && jdk("javac")) {
            eprintln!("skipped: no JDK");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let svc = |pkg: &str, ann: &str| {
            format!(
                "package {pkg};\nimport org.springframework.stereotype.Service;\nimport org.springframework.security.access.prepost.*;\n@Service\npublic class Svc {{\n{ann} public String run() {{ return \"x\"; }}\n}}\n"
            )
        };
        for (path, text) in [
            ("pom.xml", "<project/>".to_string()),
            ("src/main/java/app/Conf.java", "package app;\nimport org.springframework.security.config.annotation.method.configuration.EnableMethodSecurity;\n@EnableMethodSecurity\nclass Conf {}\n".into()),
            ("src/main/java/app/Ctl.java", "package app;\nimport adm.Svc;\nimport org.springframework.web.bind.annotation.*;\n@RestController\nclass Ctl {\n  private Svc svc;\n  @GetMapping(\"/a\")\n  String a() { return svc.run(); }\n}\n".into()),
            ("src/main/java/app/Svc.java", svc("app", "")),
            ("admin/main/java/app/Svc.java", svc("adm","@PreAuthorize(\"hasRole('B')\")")),
        ] {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let layers = method_layers(&Flow::unshared(dir.path()).get());
        let config = archguard_engine::load_config(None, dir.path())
            .unwrap()
            .config;
        let analysis = archguard_engine::analyze_calls(dir.path(), &config);
        let runs: Vec<&CallMethod> = analysis
            .methods
            .iter()
            .filter(|m| m.name == "run")
            .collect();
        eprintln!(
            "layers {layers:?}\nmethods {:?}",
            runs.iter().map(|m| &m.file).collect::<Vec<_>>()
        );
        let unguarded = runs
            .iter()
            .find(|m| !m.file.starts_with("admin"))
            .expect("root Svc.run");
        assert_eq!(
            badge(&layers, &analysis.methods, unguarded),
            0,
            "layers: {layers:?}"
        );
    }
}
