//! Layer drift: declared versus derived layer model (R08).
//!
//! Compares the node assignments of the `[[layers]]` entries written in
//! `archguard.toml` with the zero-config derivation. Assignment follows
//! the same rule as [`check_layers`]: slice order is priority, the first
//! layer whose `contains` globs match a node name wins. Agreement is the
//! share of assigned nodes both models assign identically; divergences
//! are listed per node, never guessed away.

use std::collections::BTreeMap;

use archguard_core::{Graph, LayerConfig, LayerDrift};
use globset::{Glob, GlobSet, GlobSetBuilder};

/// Drift between the declared and the derived layer model over the
/// nodes of `graph`.
///
/// `agreement` is `None` when nothing is comparable: an empty declared
/// model (no layers, or nothing assigned) leaves no reference model to
/// measure drift against — reported, not defaulted to zero.
pub fn layer_drift(declared: &[LayerConfig], derived: &[LayerConfig], graph: &Graph) -> LayerDrift {
    let assignment = |layers: &[LayerConfig]| -> BTreeMap<String, String> {
        let sets = compile(layers);
        let mut assigned = BTreeMap::new();
        for node in &graph.nodes {
            if let Some(index) = sets.iter().position(|set| set.is_match(&node.name)) {
                assigned.insert(node.name.clone(), layers[index].name.clone());
            }
        }
        assigned
    };
    let declared_map = assignment(declared);
    let derived_map = assignment(derived);

    let mut drift = LayerDrift::default();
    if declared_map.is_empty() {
        // Without a declared reference there is no drift to measure.
        drift.derived_only = derived_map.keys().cloned().collect();
        return drift;
    }
    let considered: Vec<&String> = declared_map
        .keys()
        .chain(derived_map.keys())
        .collect::<std::collections::BTreeSet<&String>>()
        .into_iter()
        .collect();
    if considered.is_empty() {
        return drift;
    }
    let mut agreed = 0usize;
    for name in considered {
        match (declared_map.get(name), derived_map.get(name)) {
            (Some(d), Some(r)) if d == r => agreed += 1,
            (Some(_), Some(_)) => drift.violated.push(name.clone()),
            (Some(_), None) => drift.declared_only.push(name.clone()),
            (None, Some(_)) => drift.derived_only.push(name.clone()),
            (None, None) => {}
        }
    }
    let considered =
        agreed + drift.declared_only.len() + drift.derived_only.len() + drift.violated.len();
    drift.agreement = Some(agreed as f64 / considered as f64);
    drift
}

/// One [`GlobSet`] per layer (empty `contains` matches nothing).
fn compile(layers: &[LayerConfig]) -> Vec<GlobSet> {
    layers
        .iter()
        .map(|layer| {
            let mut builder = GlobSetBuilder::new();
            for pattern in &layer.contains {
                if let Ok(glob) = Glob::new(pattern) {
                    builder.add(glob);
                }
            }
            builder.build().unwrap_or_else(|_| GlobSet::empty())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_helpers::graph;
    use archguard_core::LayerConfig;

    fn layer(name: &str, contains: &[&str]) -> LayerConfig {
        LayerConfig {
            name: name.to_string(),
            contains: contains.iter().map(|p| p.to_string()).collect(),
            may_depend_on: Vec::new(),
            origin: archguard_core::LayerOrigin::Declared,
        }
    }

    #[test]
    fn identical_models_agree_completely() {
        let g = graph(&["domain/money", "app/billing"], &[]);
        let declared = vec![layer("domain", &["domain/**"]), layer("app", &["app/**"])];
        let derived = vec![layer("domain", &["domain/**"]), layer("app", &["app/**"])];
        let drift = layer_drift(&declared, &derived, &g);
        assert_eq!(drift.agreement, Some(1.0));
        assert!(drift.declared_only.is_empty());
        assert!(drift.derived_only.is_empty());
        assert!(drift.violated.is_empty());
    }

    #[test]
    fn completely_different_assignments_agree_on_nothing() {
        let g = graph(&["domain/money"], &[]);
        let declared = vec![layer("domain", &["domain/**"])];
        let derived = vec![layer("util", &["domain/**"])];
        let drift = layer_drift(&declared, &derived, &g);
        assert_eq!(drift.agreement, Some(0.0));
        assert_eq!(drift.violated, ["domain/money"]);
    }

    #[test]
    fn empty_declared_model_has_no_agreement_value() {
        let g = graph(&["domain/money"], &[]);
        let derived = vec![layer("domain", &["domain/**"])];
        let drift = layer_drift(&[], &derived, &g);
        assert_eq!(drift.agreement, None, "no reference model to compare");
        assert_eq!(drift.derived_only, ["domain/money"]);
    }

    #[test]
    fn one_sided_and_mismatched_nodes_are_listed() {
        let g = graph(&["a/x", "b/y", "c/z"], &[]);
        let declared = vec![layer("l", &["a/**", "c/**"])];
        let derived = vec![layer("l", &["a/**", "b/**"])];
        let drift = layer_drift(&declared, &derived, &g);
        assert_eq!(drift.declared_only, ["c/z"]);
        assert_eq!(drift.derived_only, ["b/y"]);
        assert!(drift.violated.is_empty());
        // Only a/x agrees out of three considered nodes.
        assert!((drift.agreement.unwrap() - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn signature_is_stable() {
        let f: fn(&[LayerConfig], &[LayerConfig], &Graph) -> LayerDrift = layer_drift;
        let _ = f;
    }
}
