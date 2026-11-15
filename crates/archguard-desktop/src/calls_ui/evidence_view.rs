//! The detail column as `CallsModel` values: the selected connection's
//! call sites and candidates, or the selected declaration. Also the value
//! helpers the other columns use.

use archguard_engine::core::MethodCall;
use slint::{ModelRc, SharedString, VecModel};

use super::evidence::{self, Proposal};
use super::model::ElementKind;
use super::model::Relation;
use super::rankings;
use super::state::State;
use crate::ui::{
    CallsCohesionGroup, CallsDeclaration, CallsMethodGroup, CallsMetric, CallsModel, CallsSite,
    CallsSuggestion,
};

pub fn model<T: Clone + 'static>(items: impl IntoIterator<Item = T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items.into_iter().collect::<Vec<_>>()))
}

pub fn int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

pub fn resolution_index(r: &str) -> i32 {
    match r {
        "project" => 0,
        "external" => 1,
        _ => 2,
    }
}

/// `text` relative to the selected package ([`super::model::relative`]).
pub fn rel(s: &State, text: &str) -> String {
    super::model::relative(text, s.filter.package.as_deref())
}

pub fn ends(s: &State, r: &Relation) -> (String, String) {
    let l = &s.labels;
    let from = s.idx.class_label(Some(&r.from), l);
    let to = match &r.to {
        Some(to) => s.idx.class_label(Some(to), l),
        None => r.label.clone(),
    };
    (from, to)
}

/// `role`: 0 none, 1 caller, 2 target, 3 both (`CallsMetric.role`).
fn metric(s: &State, id: &str, role: i32) -> Option<CallsMetric> {
    let method = s.idx.method(Some(id))?;
    let c = rankings::complexity(method);
    Some(CallsMetric {
        label: rel(s, &s.idx.qualified(ElementKind::Method, id, &s.labels)).into(),
        role,
        measured: c.is_some(),
        cyclomatic: c.map_or(0, |c| c.cyclomatic as i32),
        nesting: c.map_or(0, |c| c.max_nesting as i32),
    })
}

/// Caller and target of a method pair; a self-call is one line.
fn metrics(s: &State, first: &MethodCall) -> Vec<CallsMetric> {
    let (from, to) = (first.from_method.as_deref(), first.to_method.as_deref());
    if from.is_some() && from == to {
        return from.and_then(|id| metric(s, id, 3)).into_iter().collect();
    }
    let caller = from.and_then(|id| metric(s, id, 1));
    caller
        .into_iter()
        .chain(to.and_then(|id| metric(s, id, 2)))
        .collect()
}

fn method_group(s: &State, g: &evidence::MethodGroup, open: bool) -> CallsMethodGroup {
    let (idx, l) = (&s.idx, &s.labels);
    let first = &idx.a.calls[g.calls[0]];
    CallsMethodGroup {
        title: rel(
            s,
            &format!(
                "{} → {}",
                idx.method_label(first.from_method.as_deref(), l),
                idx.target_label(first, false, l)
            ),
        )
        .into(),
        shown: int(g.calls.len()),
        total: int(g.total),
        open,
        metrics: model(metrics(s, first)),
        sites: model(g.calls.iter().map(|&i| {
            let c = &idx.a.calls[i];
            CallsSite {
                location: format!("{}:{}:{}", c.file, c.line, c.column).into(),
                dispatch: c.dispatch.as_str().into(),
                resolution: resolution_index(&c.resolution),
                declared: if c.to_method.is_some() {
                    rel(s, &idx.target_label(c, true, l)).into()
                } else {
                    SharedString::new()
                },
                code: c.code.as_str().into(),
            }
        })),
    }
}

fn suggestion(s: &State, x: &evidence::Suggestion, open: bool) -> CallsSuggestion {
    CallsSuggestion {
        kind: match x.kind {
            Proposal::Interface => 0,
            Proposal::Adapter => 1,
            Proposal::Move => 2,
        },
        source: rel(s, &x.source).into(),
        target: rel(s, &x.target).into(),
        methods: rel(s, &x.methods).into(),
        cycle_size: int(x.cycle_size),
        target_calls: int(x.target_calls),
        outgoing: int(x.outgoing),
        own_calls: int(x.own_calls),
        same_file: int(x.same_file),
        unresolved: int(x.unresolved),
        partial: x.partial,
        evidence: x
            .evidence
            .iter()
            .take(5)
            .map(|e| rel(s, e))
            .collect::<Vec<_>>()
            .join(" · ")
            .into(),
        evidence_more: int(x.evidence.len().saturating_sub(5)),
        open,
    }
}

pub fn fill_evidence(m: &mut CallsModel, s: &State) {
    let Some(r) = s.view.selected.as_ref() else {
        m.declaration = declaration(s);
        return;
    };
    let (from, to) = ends(s, r);
    m.has_relation = true;
    let title = format!("{from} → {to}");
    m.evidence_title = rel(s, &title).into();
    if m.evidence_title != title.as_str() {
        m.evidence_full = title.into();
    }
    m.evidence_sites = int(r.calls.len());
    m.evidence_resolution = resolution_index(&r.resolution);
    let groups = evidence::method_groups(r, &s.idx, s.site_limit);
    m.groups = model(groups.iter().enumerate().map(|(i, g)| {
        let open = s.open_groups.get(&g.key).copied().unwrap_or(i == 0);
        method_group(s, g, open)
    }));
    m.sites_more = r.calls.len() > s.site_limit;
    let list = evidence::suggestions(r, &s.idx, &s.cycles.classes, &s.labels);
    m.suggestions = model(
        list.iter()
            .enumerate()
            .map(|(i, x)| suggestion(s, x, s.open_suggestions.contains(&i))),
    );
}

fn declaration(s: &State) -> CallsDeclaration {
    let mut d = CallsDeclaration::default();
    let Some(el) = &s.element else { return d };
    let (idx, l) = (&s.idx, &s.labels);
    match el.kind {
        ElementKind::Method => {
            let Some(m) = idx.method(Some(&el.id)) else {
                return d;
            };
            d.kind = 2;
            d.title = rel(s, &rankings::label(idx, true, &el.id, l)).into();
            d.location = format!("{}:{}", m.file, m.line).into();
            d.metric = metric(s, &el.id, 0).unwrap_or_default();
        }
        ElementKind::Class => {
            let Some(c) = idx.class(Some(&el.id)) else {
                return d;
            };
            d.kind = 1;
            d.title = rel(s, &super::model::class_name(c)).into();
            d.location = format!("{}:{}", c.file, c.line).into();
            cohesion(&mut d, s, &c.id);
        }
    }
    d
}

fn cohesion(d: &mut CallsDeclaration, s: &State, class: &str) {
    let details = s.idx.a.details.as_ref();
    let Some(c) = details.and_then(|x| {
        x.cohesion
            .iter()
            .find(|r| r.class_id == class && r.definition == "lcom4-source/v1")
    }) else {
        return;
    };
    d.cohesion = if c.lcom4.is_some() { 2 } else { 1 };
    d.lcom4 = c.lcom4.map_or(0, int);
    d.methods = int(c.methods);
    d.fields = int(c.fields);
    d.complexity = c
        .cyclomatic_sum
        .map_or_else(|| "—".to_string(), |x| x.to_string())
        .into();
    d.groups = model(c.groups.iter().map(|g| {
        let known: Vec<(String, &String)> = g
            .iter()
            .filter_map(|id| s.idx.method(Some(id)).map(|m| (m.signature.clone(), id)))
            .collect();
        CallsCohesionGroup {
            methods: model(
                known
                    .iter()
                    .map(|(sig, _)| SharedString::from(sig.as_str())),
            ),
            ids: model(known.iter().map(|(_, id)| SharedString::from(id.as_str()))),
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::super::model::fixture;
    use super::*;

    fn rows(m: &[CallsMetric]) -> Vec<(i32, String)> {
        m.iter().map(|x| (x.role, x.label.to_string())).collect()
    }

    /// Caller and target with the same signature in different classes
    /// read apart by role and owner; a self-call shows one line.
    #[test]
    fn caller_and_target_metrics_name_their_role() {
        let mut a = fixture::analysis();
        a.methods.push(fixture::method("red.Red", "name()", 1));
        a.calls.push(fixture::call(
            "blue.Blue#name()",
            Some("red.Red#name()"),
            "project",
            30,
        ));
        let mut s = State::new(a);
        // All packages: full names.
        s.filter.package = None;
        assert_eq!(
            rows(&metrics(&s, &s.idx.a.calls[6])),
            [(1, "blue.Blue#name()".into()), (2, "red.Red#name()".into())]
        );
        assert_eq!(
            rows(&metrics(&s, &s.idx.a.calls[5])),
            [(3, "blue.Blue#name()".into())]
        );
        s.filter.package = Some("blue".into());
        assert_eq!(
            rows(&metrics(&s, &s.idx.a.calls[6])),
            [(1, "Blue#name()".into()), (2, "red.Red#name()".into())]
        );
    }
}
