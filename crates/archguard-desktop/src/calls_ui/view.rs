//! The tab state as the window's `CallsModel` value.

use std::collections::BTreeSet;

use slint::SharedString;

use super::evidence_view::{ends, fill_evidence, int, model, resolution_index};
use super::model::{ElementKind, file_module, language_label};
use super::notes;
use super::rankings;
use super::state::{Mode, State};
use super::tree::Kind;
use crate::tab_support::Texts;
use crate::ui::{
    ArcFigure, CallsAnalyzer, CallsCycleRow, CallsModel, CallsRankRow, CallsRelationRow, CallsStep,
    CallsTreeRow,
};
use archguard_engine::core::CallClass;

/// Names the window translates but Rust places in lists.
pub struct UiLabels {
    pub all_languages: SharedString,
    pub all_packages: SharedString,
    pub default_package: SharedString,
    pub external_group: SharedString,
    pub unresolved_group: SharedString,
    /// Translates the engine's fixed notes ([`notes::diagnostic`]).
    pub texts: Texts,
}

/// Fill the whole model from the state.
pub fn fill(s: &State, ui: &UiLabels) -> CallsModel {
    let _span = archguard_engine::self_trace::tracing::info_span!("fill").entered();
    let mut m = CallsModel {
        state: 1,
        mode: match s.mode {
            Mode::Connections => 0,
            Mode::Rankings => 1,
            Mode::Cycles => 2,
        },
        ..Default::default()
    };
    fill_toolbar(&mut m, s, ui);
    fill_coverage(&mut m, s, &ui.texts);
    fill_tree(&mut m, s, ui);
    match s.mode {
        Mode::Connections => fill_relations(&mut m, s),
        Mode::Rankings => fill_rankings(&mut m, s),
        Mode::Cycles => fill_cycles(&mut m, s),
    }
    fill_evidence(&mut m, s);
    m.sequence = sequence(s, &ui.texts);
    m
}

/// The selected method's calls between classes, as in arc42 chapter 6.
fn sequence(s: &State, texts: &Texts) -> ArcFigure {
    let method = s.element.as_ref().filter(|e| e.kind == ElementKind::Method);
    method
        .and_then(|e| archguard_engine::report::class_sequence(texts.doc_lang(), &s.idx.a, &e.id))
        .map(|d| crate::architecture_ui::sequence::figure(&d, String::new(), String::new(), texts))
        .unwrap_or_default()
}

fn fill_toolbar(m: &mut CallsModel, s: &State, ui: &UiLabels) {
    let l = &s.labels;
    let languages = s.idx.languages();
    m.language_index = s
        .filter
        .language
        .as_ref()
        .and_then(|x| languages.iter().position(|y| y == x))
        .map_or(0, |i| int(i + 1));
    m.languages = model(
        std::iter::once(ui.all_languages.clone())
            .chain(languages.iter().map(|x| language_label(x, l).into())),
    );
    let packages = s.idx.packages(s.filter.language.as_deref());
    m.package_index = s
        .filter
        .package
        .as_ref()
        .and_then(|x| packages.iter().position(|y| y == x))
        .map_or(0, |i| int(i + 1));
    m.packages = model(
        std::iter::once(ui.all_packages.clone()).chain(packages.iter().map(|p| match p.as_str() {
            "" => ui.default_package.clone(),
            p => p.replace('/', ".").into(),
        })),
    );
    m.query = s.filter.query.as_str().into();
    m.resolution = s.filter.resolution.index();
    m.internal = s.filter.internal;
}

fn fill_coverage(m: &mut CallsModel, s: &State, texts: &Texts) {
    let a = &s.idx.a;
    let say = |d: &str| SharedString::from(notes::diagnostic(texts, d));
    let count = |r: &str| int(a.calls.iter().filter(|c| c.resolution == r).count());
    m.status = a.status.as_str().into();
    m.files = int(a.files);
    m.classes = int(a.classes.len());
    m.project = count("project");
    m.external = count("external");
    m.unresolved = count("unresolved");
    m.java_release = i32::try_from(a.java_release).unwrap_or(0);
    m.unreported = a.analyzers.is_empty() && a.status != "not_applicable";
    m.diagnostic_line = a
        .diagnostics
        .iter()
        .map(|d| notes::diagnostic(texts, d))
        .collect::<Vec<_>>()
        .join(" ")
        .into();
    m.diagnostics = model(notes::extra_diagnostics(a).iter().map(|d| say(d)));
    m.no_calls = a.calls.is_empty() && a.status != "not_applicable";
    m.analyzers = model(a.analyzers.iter().map(|x| CallsAnalyzer {
        note: notes::note(x) as i32,
        language: language_label(&x.language, &s.labels).into(),
        backend: x.backend.as_str().into(),
        status: x.status.as_str().into(),
        files: int(x.files),
        version: x.version.clone().unwrap_or_default().into(),
        project: x.project.clone().unwrap_or_default().into(),
        diagnostics: model(x.diagnostics.iter().map(|d| say(d))),
    }));
    m.analysis_open = s.analysis_open;
    let rels = &s.view.relations;
    m.connections = int(rels.len());
    m.sites = int(rels.iter().map(|r| r.calls.len()).sum());
}

fn tree_label(s: &State, kind: Kind, n: &super::tree::Node, ui: &UiLabels) -> SharedString {
    let label = n.label.clone().unwrap_or_default();
    match kind {
        Kind::Language => language_label(&n.id, &s.labels).into(),
        Kind::Group if n.id == "external" => ui.external_group.clone(),
        Kind::Group => ui.unresolved_group.clone(),
        Kind::Package if label.is_empty() => ui.default_package.clone(),
        _ => label.into(),
    }
}

/// The selected element, or the selected external/unresolved connection.
fn pressed(s: &State, node: &super::tree::Node) -> bool {
    let element = s.element.as_ref().is_some_and(|e| {
        e.id == node.id
            && matches!(
                (e.kind, node.kind),
                (ElementKind::Class, Kind::Class) | (ElementKind::Method, Kind::Method)
            )
    });
    let sel = s.view.selected.as_ref();
    element
        || (node.kind == Kind::Leaf && sel.is_some_and(|r| Some(&r.key) == node.relation.as_ref()))
}

fn fill_tree(m: &mut CallsModel, s: &State, ui: &UiLabels) {
    let v = &s.view;
    let t = &v.tree;
    let sel = v.selected.as_ref();
    let methods = |pick: fn(&archguard_engine::core::MethodCall) -> &Option<String>| {
        sel.map(|r| {
            r.calls
                .iter()
                .filter_map(|&i| pick(&s.idx.a.calls[i]).clone())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default()
    };
    let (callers, targets) = (methods(|c| &c.from_method), methods(|c| &c.to_method));
    let mut rows = Vec::new();
    let mut skipped = 0;
    m.tree_focus = -1;
    for &n in &v.visible {
        if !v.shown.contains(&n) {
            skipped += 1;
            continue;
        }
        let node = &t.nodes[n];
        let pressed = pressed(s, node);
        if pressed && node.kind != Kind::Leaf {
            m.tree_focus = int(rows.len());
        }
        let (from, to) = match node.kind {
            Kind::Class => (
                sel.is_some_and(|r| r.from == node.id),
                sel.is_some_and(|r| r.to.as_deref() == Some(&node.id)),
            ),
            Kind::Method => (callers.contains(&node.id), targets.contains(&node.id)),
            _ => (false, false),
        };
        rows.push(CallsTreeRow {
            node: int(n),
            skipped: std::mem::take(&mut skipped),
            kind: node.kind as i32,
            depth: int(node.depth),
            label: tree_label(s, node.kind, node, ui),
            expandable: !node.children.is_empty(),
            expanded: s.expanded.contains(&node.key),
            role: i32::from(from) + 2 * i32::from(to),
            count: int(node.count),
            module: node.kind == Kind::Class
                && s.idx
                    .class(Some(&node.id))
                    .is_some_and(|c| c.kind == "module"),
            cycle: s.is_cycle_member(node.kind, &node.id),
            pressed,
        });
    }
    m.tree = model(rows);
    m.tree_shown = int(v.shown.len());
    m.tree_total = int(v.visible.len());
    m.tree_more = v.visible.len() > s.tree_limit;
    m.tree_missing = int(t.missing_methods);
    m.tree_nodes = int(t.nodes.len());
    m.tree_element = s
        .element
        .as_ref()
        .map(|e| match e.kind {
            ElementKind::Class => s.idx.class_label(Some(&e.id), &s.labels),
            ElementKind::Method => s.idx.method_label(Some(&e.id), &s.labels),
        })
        .unwrap_or_default()
        .into();
}

/// The type name without its package; a file name stays whole.
fn short(label: String, cls: Option<&CallClass>) -> String {
    if cls.is_some_and(file_module) {
        return label;
    }
    label.rsplit('.').next().unwrap_or_default().to_string()
}

fn fill_relations(m: &mut CallsModel, s: &State) {
    let v = &s.view;
    let selected = v.selected.as_ref().map(|r| r.key.as_str());
    m.relations = model(v.relations.iter().take(s.limit).map(|r| {
        let (from, to) = ends(s, r);
        let short_to = if r.to.is_some() {
            short(to.clone(), s.idx.class(r.to.as_deref()))
        } else {
            to.clone()
        };
        CallsRelationRow {
            title: format!(
                "{} → {}",
                short(from.clone(), s.idx.class(Some(&r.from))),
                short_to
            )
            .into(),
            language: s
                .idx
                .class(Some(&r.from))
                .map(|c| s.idx.language(c))
                .filter(|x| !x.is_empty())
                .map(|x| language_label(&x, &s.labels))
                .unwrap_or_default()
                .into(),
            sites: int(r.calls.len()),
            methods: int(r.target_methods(&s.idx.a)),
            resolution: resolution_index(&r.resolution),
            full: format!("{from} → {to}").into(),
            selected: Some(r.key.as_str()) == selected,
        }
    }));
    m.relations_more = v.relations.len() > s.limit;
}

fn fill_rankings(m: &mut CallsModel, s: &State) {
    let v = &s.view;
    let (idx, l) = (&s.idx, &s.labels);
    m.rank_method = s.rank_method;
    m.rank_sort = s.rank_sort as i32;
    m.rank_total = int(v.rank_rows.len());
    m.rank_more = v.rank_rows.len() > s.rank_limit;
    m.rank_unresolved = int(v.rankings.unresolved_targets);
    m.rank_missing_callers = int(v.rankings.missing_callers);
    m.rank_missing_targets = int(v.rankings.missing_targets);
    m.rank_unmeasured = int(v.rank_missing);
    let kind = if s.rank_method {
        ElementKind::Method
    } else {
        ElementKind::Class
    };
    m.ranks = model(v.rank_rows.iter().take(s.rank_limit).map(|r| {
        let method = s.rank_method.then(|| idx.method(Some(&r.id))).flatten();
        let metric = method.and_then(rankings::complexity);
        let cls = idx.class(Some(method.map_or(r.id.as_str(), |m| m.class_id.as_str())));
        let language = cls.map(|c| idx.language(c)).filter(|x| !x.is_empty());
        CallsRankRow {
            label: rankings::label(idx, s.rank_method, &r.id, l).into(),
            incoming: int(r.incoming),
            outgoing: int(r.outgoing),
            total: int(r.total),
            method: method.is_some(),
            measured: metric.is_some(),
            cyclomatic: metric.map_or(0, |x| x.cyclomatic as i32),
            nesting: metric.map_or(0, |x| x.max_nesting as i32),
            language: language
                .map(|x| language_label(&x, l))
                .unwrap_or_default()
                .into(),
            module: cls.is_some_and(|c| c.kind == "module"),
            pressed: s
                .element
                .as_ref()
                .is_some_and(|e| e.kind == kind && e.id == r.id),
        }
    }));
}

fn step(label: String, sites: i32, pressed: bool) -> CallsStep {
    CallsStep {
        label: label.into(),
        sites,
        pressed,
    }
}

fn fill_cycles(m: &mut CallsModel, s: &State) {
    let v = &s.view;
    let (idx, l) = (&s.idx, &s.labels);
    let kind = s.cycle_kind;
    let groups = s.cycles.groups(kind);
    m.cycle_method = kind == ElementKind::Method;
    m.cycle_count = int(v.groups.len());
    m.cycle_total = int(groups.len());
    m.cycle_unresolved = int(s.cycles.unresolved_targets);
    m.cycle_missing = int(s.cycles.missing_methods);
    m.cycles_more = s.cycle.groups < v.groups.len();
    let mut shown: Vec<usize> = (0..v.groups.len().min(s.cycle.groups)).collect();
    if let Some(pos) = v.group.and_then(|g| v.groups.iter().position(|&x| x == g))
        && !shown.contains(&pos)
    {
        shown.push(pos);
    }
    let label = |id: &str| idx.qualified(kind, id, l);
    m.cycle_rows = model(shown.into_iter().map(|pos| {
        let g = &groups[v.groups[pos]];
        let mut preview = g
            .members
            .iter()
            .take(2)
            .map(|id| label(id))
            .collect::<Vec<_>>()
            .join(", ");
        if g.members.len() > 2 {
            preview.push_str(&format!(" … +{}", g.members.len() - 2));
        }
        CallsCycleRow {
            number: int(pos + 1),
            preview: preview.into(),
            members: int(g.members.len()),
            edges: int(g.edges.len()),
            sites: int(g.call_sites),
            selected: Some(v.groups[pos]) == v.group,
        }
    }));
    fill_group(m, s);
}

/// The selected group: path, members and internal connections.
fn fill_group(m: &mut CallsModel, s: &State) {
    let (v, idx, l, kind) = (&s.view, &s.idx, &s.labels, s.cycle_kind);
    let label = |id: &str| idx.qualified(kind, id, l);
    let Some(g) = s.group() else { return };
    m.has_group = true;
    m.group_members = int(g.members.len());
    m.group_edges = int(g.edges.len());
    let steps = g.witness.len().saturating_sub(1);
    m.witness_steps = int(steps);
    if steps <= 20 {
        m.witness = g
            .witness
            .iter()
            .map(|id| label(id))
            .collect::<Vec<_>>()
            .join(" → ")
            .into();
    }
    m.path = model((0..steps.min(s.cycle.witness)).map(|i| {
        let (a, b) = (&g.witness[i], &g.witness[i + 1]);
        step(format!("{}. {} → {}", i + 1, label(a), label(b)), -1, false)
    }));
    m.path_more = s.cycle.witness < steps;
    let el = s.element.as_ref();
    m.members = model(g.members.iter().take(s.cycle.members).map(|id| {
        step(
            label(id),
            -1,
            el.is_some_and(|e| e.kind == kind && &e.id == id),
        )
    }));
    m.members_more = s.cycle.members < g.members.len();
    let sel = s.relation.as_deref();
    m.edges = model(v.cycle_relations.iter().take(s.cycle.edges).map(|r| {
        let (a, b) = r.cycle.clone().unwrap_or_default();
        step(
            format!("{} → {}", label(&a), label(&b)),
            int(r.calls.len()),
            Some(r.key.as_str()) == sel,
        )
    }));
    m.edges_more = s.cycle.edges < g.edges.len();
}

#[cfg(test)]
mod tests {
    use super::super::model::fixture;
    use super::*;

    #[test]
    fn short_names_drop_the_package_but_keep_file_names() {
        let java = fixture::class("blue.Blue", "blue");
        assert_eq!(short("blue.Blue".into(), Some(&java)), "Blue");
        let mut module = fixture::class("api/http.py::(module)", "api");
        module.name = "(module)".into();
        module.kind = "module".into();
        module.file = "api/http.py".into();
        assert_eq!(short("http.py".into(), Some(&module)), "http.py");
        assert_eq!(short("x.Unknown".into(), None), "Unknown");
    }

    /// The tree scrolls to the focused element's row (`tree-focus`).
    #[test]
    fn the_focused_element_names_its_tree_row() {
        use slint::Model;
        let labels = UiLabels {
            all_languages: "".into(),
            all_packages: "".into(),
            default_package: "".into(),
            external_group: "".into(),
            unresolved_group: "".into(),
            texts: Texts::default(),
        };
        let mut s = State::new(fixture::analysis());
        s.refresh();
        assert_eq!(fill(&s, &labels).tree_focus, -1, "no element, no row");
        super::super::actions::focus_class(&mut s, "red.Red");
        s.refresh();
        let m = fill(&s, &labels);
        let row = m.tree.row_data(m.tree_focus as usize).expect("a row");
        assert!(row.pressed);
        assert_eq!(row.label, "Red");
    }

    /// A selected method shows its calls between classes as a sequence.
    #[test]
    fn a_selected_method_shows_its_sequence() {
        use super::super::actions::select_element;
        use slint::Model;
        let labels = UiLabels {
            all_languages: "".into(),
            all_packages: "".into(),
            default_package: "".into(),
            external_group: "".into(),
            unresolved_group: "".into(),
            texts: Texts::default(),
        };
        let mut s = State::new(fixture::analysis());
        select_element(&mut s, ElementKind::Method, "blue.Util#hint()");
        s.refresh();
        let m = fill(&s, &labels);
        assert_eq!(m.sequence.id, "runtime-scenario-classes-blue-util-hint");
        let names: Vec<_> = m
            .sequence
            .cards
            .iter()
            .map(|c| c.name1.to_string())
            .collect();
        // Heads above and below the lifelines.
        assert_eq!(
            names,
            ["blue.Util", "green.Leaf", "blue.Util", "green.Leaf"]
        );
        select_element(&mut s, ElementKind::Class, "blue.Util");
        s.refresh();
        assert_eq!(fill(&s, &labels).sequence.id, "", "a class has no sequence");
    }
}
