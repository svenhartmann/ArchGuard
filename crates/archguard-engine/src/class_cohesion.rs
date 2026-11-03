//! LCOM4 over explicit instance bodies, own state and resolved internal calls.
use archguard_core::{CallAnalysis, ClassCohesion};
use std::collections::{BTreeMap, BTreeSet};

pub fn class_cohesion(calls: &CallAnalysis) -> Vec<ClassCohesion> {
    let mut methods = BTreeMap::<_, Vec<_>>::new();
    for method in &calls.methods {
        methods
            .entry(method.class_id.as_str())
            .or_default()
            .push(method);
    }
    let mut edges = BTreeMap::<_, Vec<_>>::new();
    for call in &calls.calls {
        edges
            .entry(call.from_class.as_str())
            .or_default()
            .push(call);
    }
    let mut states = BTreeMap::new();
    if let Some(details) = calls
        .details
        .as_ref()
        .filter(|d| d.definition == "source-types-state/v1")
    {
        for state in &details.class_states {
            states
                .entry(state.class_id.as_str())
                .and_modify(|s| *s = None)
                .or_insert(Some(state));
        }
    }
    let mut result = vec![];
    for class in &calls.classes {
        if class.kind == "module" {
            continue;
        }
        let eligible: Vec<_> = methods
            .get(class.id.as_str())
            .into_iter()
            .flatten()
            .copied()
            .filter(|m| {
                !m.is_static
                    && !matches!(
                        m.name.as_str(),
                        "<init>" | "<clinit>" | ".ctor" | ".cctor" | "constructor"
                    )
                    && m.id.matches('#').count() <= 1
                    && m.complexity
                        .as_ref()
                        .is_some_and(|c| c.definition == "ast-decisions-v1")
            })
            .collect();
        let mut adjacency: BTreeMap<_, BTreeSet<String>> = eligible
            .iter()
            .map(|m| (m.id.clone(), BTreeSet::new()))
            .collect();
        let state = states.get(class.id.as_str()).copied().flatten();
        let fields: BTreeSet<_> = state
            .into_iter()
            .flat_map(|s| &s.fields)
            .filter(|f| !f.is_static)
            .map(|f| f.id.as_str())
            .collect();
        let mut field_first = BTreeMap::<&String, String>::new();
        let mut joins = vec![];
        for access in state.into_iter().flat_map(|s| &s.accesses) {
            if !fields.contains(access.field.as_str()) || !adjacency.contains_key(&access.method) {
                continue;
            }
            if let Some(first) = field_first.get(&access.field) {
                joins.push((access.method.clone(), (*first).clone()));
            } else {
                field_first.insert(&access.field, access.method.clone());
            }
        }
        let mut unresolved = false;
        for call in edges.get(class.id.as_str()).into_iter().flatten() {
            let Some(from) = call
                .from_method
                .as_ref()
                .filter(|m| adjacency.contains_key(*m))
            else {
                continue;
            };
            unresolved |= call.resolution == "unresolved";
            if call.resolution == "project"
                && call.to_class.as_deref() == Some(&class.id)
                && let Some(to) = call
                    .to_method
                    .as_ref()
                    .filter(|m| adjacency.contains_key(*m))
            {
                joins.push((from.clone(), to.clone()));
            }
        }
        for (a, b) in joins {
            adjacency.get_mut(&a).unwrap().insert(b.clone());
            adjacency.get_mut(&b).unwrap().insert(a);
        }
        let mut groups = vec![];
        let mut seen = BTreeSet::new();
        for id in adjacency.keys() {
            if seen.contains(id) {
                continue;
            }
            let mut pending = vec![id.clone()];
            let mut group = vec![];
            while let Some(id) = pending.pop() {
                if seen.insert(id.clone()) {
                    pending.extend(adjacency[&id].iter().cloned());
                    group.push(id);
                }
            }
            group.sort();
            groups.push(group);
        }
        let status = if state.is_none() {
            "unavailable"
        } else if state.is_some_and(|s| s.status != "ready")
            || calls
                .details
                .as_ref()
                .is_some_and(|d| d.enumeration_complete == Some(false))
            || unresolved
            || adjacency.len() != eligible.len()
        {
            "partial"
        } else if eligible.is_empty() {
            "not_applicable"
        } else {
            "ready"
        };
        let reason = match status {
            "unavailable" => {
                "No compiler state-access evidence for this class; cohesion is unknown."
            }
            "partial" => {
                "Incomplete state or call resolution; groups show observed connections, LCOM4 remains unknown."
            }
            "not_applicable" => "No explicit non-static method bodies to compare.",
            _ => {
                "Connected components of instance methods through own non-static fields/properties or resolved internal calls. Constructors, static members and nested local functions are excluded. Multiple groups are review candidates, not proof of unrelated responsibilities."
            }
        };
        result.push(ClassCohesion {
            class_id: class.id.clone(),
            definition: "lcom4-source/v1".into(),
            status: status.into(),
            methods: eligible.len(),
            fields: fields.len(),
            lcom4: (status == "ready").then_some(groups.len()),
            groups,
            cyclomatic_sum: (!eligible.is_empty()).then(|| {
                eligible
                    .iter()
                    .map(|m| m.complexity.as_ref().unwrap().cyclomatic)
                    .sum()
            }),
            reason: reason.into(),
        });
    }
    result.sort_by(|a, b| a.class_id.cmp(&b.class_id));
    result
}
