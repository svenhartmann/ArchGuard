//! Baseline (T17): the cycle-hash ratchet of ARCHITECTURE.md Kap. 7
//! ("Baseline" row) — accept the cycles that exist, block new ones.
//!
//! A [`Baseline`] is the set of [`cycle_hash`]es of the SCCs that were
//! accepted at snapshot time, persisted as `.archguard/baseline.json` (Kap.
//! 9/11) and read/written via serde. [`diff`] compares the SCCs found on
//! the current [`Graph`] against it: an SCC whose hash is **not** in the
//! baseline is *new* (reported by its `scc-N` id), a baseline hash no
//! current SCC produces anymore is *removed*.
//!
//! The hash itself follows Kap. 7 exactly: SHA-256 over the SCC's node
//! **names**, sorted and joined with `\n` — so membership decides the
//! identity of a cycle, independent of ids and edge shape.

use std::collections::BTreeSet;

use archguard_core::Graph;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::scc::Scc;

/// SHA-256 hex digest identifying a cycle by its membership (Kap. 7):
/// the SCC's node names, sorted, joined with `\n` and hashed. Input
/// order is irrelevant; two SCCs with the same member names share a
/// hash, and the hash survives renumbering of `scc-N` ids.
pub fn cycle_hash(scc_node_names: &[String]) -> String {
    let mut names: Vec<String> = scc_node_names.to_vec();
    names.sort();
    let mut hasher = Sha256::new();
    hasher.update(names.join("\n").as_bytes());
    format!("{:x}", hasher.finalize())
}

/// The accepted cycle hashes of a previous run — the "ratchet" state
/// persisted as `.archguard/baseline.json` (Kap. 9/11). Serialized with
/// serde; `hashes` is a `BTreeSet` so the JSON is sorted and the file
/// is diffable (Kap. 12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    /// Version of this file's schema; the current version is `1`,
    /// matching the canonical graph schema version (Kap. 5).
    pub schema_version: u32,
    /// [`cycle_hash`] of every accepted SCC, sorted.
    pub hashes: BTreeSet<String>,
}

/// The delta between the current SCCs and a [`Baseline`], as produced
/// by [`diff`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineDiff {
    /// `scc-N` ids of current SCCs whose hash is not in the baseline —
    /// the cycles a `--baseline` gate should block. In input order
    /// (= the canonical `find_sccs` order).
    pub new: Vec<String>,
    /// Baseline hashes no current SCC produces anymore — cycles that
    /// were resolved. Sorted (baseline order).
    pub removed: Vec<String>,
}

/// Compare `current` (SCCs found on `g`) against `baseline`.
///
/// Each SCC is identified by the [`cycle_hash`] of its member names
/// resolved through `g`; node ids unknown to `g` hash as the empty
/// name. `new` lists the `scc-N` ids of SCCs whose hash is absent from
/// the baseline, `removed` the baseline hashes no current SCC produces.
pub fn diff(current: &[Scc], g: &Graph, baseline: &Baseline) -> BaselineDiff {
    let mut new: Vec<String> = Vec::new();
    let mut current_hashes: BTreeSet<String> = BTreeSet::new();
    for scc in current {
        let names: Vec<String> = scc
            .node_ids
            .iter()
            .map(|&id| g.node_name(id).unwrap_or("").to_string())
            .collect();
        let hash = cycle_hash(&names);
        if !baseline.hashes.contains(&hash) {
            new.push(scc.id.clone());
        }
        current_hashes.insert(hash);
    }
    let removed: Vec<String> = baseline
        .hashes
        .iter()
        .filter(|hash| !current_hashes.contains(*hash))
        .cloned()
        .collect();
    BaselineDiff { new, removed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Edge, Node, NodeId, NodeKind};

    /// Hand-assembled graph: `nodes` gives names in *insertion* order
    /// (id = position), `edges` references nodes by name. Same shape as
    /// the `scc.rs` test helper.
    fn graph(nodes: &[&str], edges: &[(&str, &str)]) -> Graph {
        let id_of = |name: &str| -> NodeId {
            NodeId(
                nodes
                    .iter()
                    .position(|n| *n == name)
                    .expect("test: edge endpoint must be a listed node") as u32,
            )
        };
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/abs".to_string(),
            nodes: nodes
                .iter()
                .enumerate()
                .map(|(i, n)| Node {
                    id: NodeId(i as u32),
                    name: (*n).to_string(),
                    kind: NodeKind::File,
                    parent: None,
                    lang: None,
                })
                .collect(),
            edges: edges
                .iter()
                .map(|(from, to)| Edge {
                    from: id_of(from),
                    to: id_of(to),
                    refs: Vec::new(),
                    count: None,
                })
                .collect(),
        }
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    /// Build `Scc` values directly: one per name group, ids assigned in
    /// the given order (`scc-1`, `scc-2`, …), matching `g`'s node ids.
    fn sccs(g: &Graph, groups: &[&[&str]]) -> Vec<Scc> {
        groups
            .iter()
            .enumerate()
            .map(|(i, group)| Scc {
                id: format!("scc-{}", i + 1),
                node_ids: group
                    .iter()
                    .map(|name| {
                        NodeId(
                            g.nodes
                                .iter()
                                .position(|n| &n.name == name)
                                .expect("test: scc member must be a listed node")
                                as u32,
                        )
                    })
                    .collect(),
                edges: Vec::new(),
            })
            .collect()
    }

    fn baseline_of(v: &[&[&str]]) -> Baseline {
        Baseline {
            schema_version: 1,
            hashes: v.iter().map(|group| cycle_hash(&names(group))).collect(),
        }
    }

    #[test]
    fn cycle_hash_is_ordered_join_sha256() {
        // Golden value: SHA-256 over "a\nb\nc".
        assert_eq!(
            cycle_hash(&names(&["a", "b", "c"])),
            "ea7fb08b7a2dc4619ffb7c7bb38d95a2047935fa165d71b12efd3852a2e6d0cc"
        );
    }

    #[test]
    fn cycle_hash_ignores_input_order() {
        assert_eq!(
            cycle_hash(&names(&["c", "a", "b"])),
            cycle_hash(&names(&["a", "b", "c"]))
        );
        assert_ne!(
            cycle_hash(&names(&["a", "b", "c"])),
            cycle_hash(&names(&["a", "b", "d"]))
        );
    }

    #[test]
    fn baseline_serializes_to_sorted_json() {
        let baseline = baseline_of(&[&["a", "b"], &["c", "d", "e"]]);
        let json = serde_json::to_string(&baseline).unwrap();
        let restored: Baseline = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, baseline);
        assert_eq!(restored.schema_version, 1);
        // BTreeSet order: hashes serialize sorted, so the file diffs cleanly.
        let hashes = restored.hashes.iter().collect::<Vec<_>>();
        let mut sorted = hashes.clone();
        sorted.sort();
        assert_eq!(hashes, sorted);
    }

    #[test]
    fn diff_unchanged_is_empty() {
        let g = graph(
            &["a", "b", "c", "d"],
            &[("a", "b"), ("b", "a"), ("c", "d"), ("d", "c")],
        );
        let current = sccs(&g, &[&["a", "b"], &["c", "d"]]);
        let baseline = baseline_of(&[&["a", "b"], &["c", "d"]]);

        assert_eq!(
            diff(&current, &g, &baseline),
            BaselineDiff {
                new: Vec::new(),
                removed: Vec::new(),
            }
        );
    }

    #[test]
    fn diff_reports_new_cycle_by_scc_id() {
        let g = graph(
            &["a", "b", "c", "d"],
            &[("a", "b"), ("b", "a"), ("c", "d"), ("d", "c")],
        );
        let current = sccs(&g, &[&["a", "b"], &["c", "d"]]);
        // Baseline knows only the a↔b cycle; c↔d is new.
        let baseline = baseline_of(&[&["a", "b"]]);

        assert_eq!(
            diff(&current, &g, &baseline),
            BaselineDiff {
                new: vec!["scc-2".to_string()],
                removed: Vec::new(),
            }
        );
    }

    #[test]
    fn diff_reports_removed_hash() {
        let g = graph(&["a", "b"], &[("a", "b"), ("b", "a")]);
        let current = sccs(&g, &[&["a", "b"]]);
        // Baseline knows a↔b plus a cycle that has since been resolved.
        let gone = cycle_hash(&names(&["x", "y"]));
        let mut baseline = baseline_of(&[&["a", "b"]]);
        baseline.hashes.insert(gone.clone());

        assert_eq!(
            diff(&current, &g, &baseline),
            BaselineDiff {
                new: Vec::new(),
                removed: vec![gone],
            }
        );
    }

    #[test]
    fn diff_against_empty_baseline_reports_all_as_new() {
        let g = graph(&["a", "b"], &[("a", "b"), ("b", "a")]);
        let current = sccs(&g, &[&["a", "b"]]);
        let baseline = Baseline {
            schema_version: 1,
            hashes: BTreeSet::new(),
        };

        assert_eq!(
            diff(&current, &g, &baseline),
            BaselineDiff {
                new: vec!["scc-1".to_string()],
                removed: Vec::new(),
            }
        );
    }
}
