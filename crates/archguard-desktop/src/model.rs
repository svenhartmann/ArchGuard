//! What the window shows, built from an analysis run and independent of
//! Slint: placed modules, placed dependencies and the selection state.

use std::collections::BTreeMap;

use archguard_engine::AnalysisRun;
use archguard_engine::core::{Graph, ModuleMetrics};

use crate::layout::{Dep, layout};

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub name: String,
    /// Last path segment, the label on the card.
    pub label: String,
    /// Everything before the label, shown small above it.
    pub scope: String,
    pub fan_in: u32,
    pub fan_out: u32,
    pub instability: f32,
    pub in_cycle: bool,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dependency {
    pub from: usize,
    pub to: usize,
    /// Number of source references behind the edge.
    pub weight: u64,
    pub path: String,
    pub cyclic: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Overview {
    pub project: String,
    pub modules: Vec<Module>,
    pub deps: Vec<Dependency>,
    pub width: f32,
    pub height: f32,
    pub cycles: u32,
    pub passed: bool,
    pub acd: f64,
}

/// One row of the detail panel: the module on the other end and the
/// number of references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neighbor {
    pub index: usize,
    pub weight: u64,
}

impl Overview {
    pub fn from_run(project: String, run: &AnalysisRun) -> Self {
        let mut o = Self::from_graph(project, &run.view, &run.report.metrics);
        o.cycles = run.report.summary.sccs;
        o.passed = run.report.summary.status == archguard_engine::core::Status::Pass;
        o.acd = run.report.summary.acd;
        o
    }

    /// Modules are the view's nodes, sorted by name as the view is.
    pub fn from_graph(
        project: String,
        view: &Graph,
        metrics: &BTreeMap<String, ModuleMetrics>,
    ) -> Self {
        let names: Vec<String> = view.nodes.iter().map(|n| n.name.clone()).collect();
        let index: BTreeMap<_, usize> = view
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i))
            .collect();
        let edges: Vec<(usize, usize, u64)> = view
            .edges
            .iter()
            .filter_map(|e| Some((*index.get(&e.from)?, *index.get(&e.to)?, e.weight())))
            .filter(|(a, b, _)| a != b)
            .collect();
        let placed = layout(
            &names,
            &edges
                .iter()
                .map(|&(from, to, _)| Dep { from, to })
                .collect::<Vec<_>>(),
        );
        let modules = names
            .iter()
            .zip(&placed.nodes)
            .map(|(name, p)| {
                let m = metrics.get(name);
                let (scope, label) = split_name(name);
                Module {
                    name: name.clone(),
                    label,
                    scope,
                    fan_in: m.map_or(0, |m| m.fan_in),
                    fan_out: m.map_or(0, |m| m.fan_out),
                    instability: m.map_or(0.0, |m| m.instability),
                    in_cycle: m.is_some_and(|m| m.in_cycle),
                    x: p.x,
                    y: p.y,
                }
            })
            .collect();
        let deps = edges
            .iter()
            .zip(placed.edges)
            .map(|(&(from, to, weight), p)| Dependency {
                from,
                to,
                weight,
                path: p.path,
                cyclic: p.cyclic,
            })
            .collect();
        Overview {
            project,
            modules,
            deps,
            width: placed.width,
            height: placed.height,
            cycles: 0,
            passed: true,
            acd: 0.0,
        }
    }

    /// Modules `selected` depends on, heaviest first.
    pub fn outgoing(&self, selected: usize) -> Vec<Neighbor> {
        self.neighbors(|d| (d.from == selected).then_some(d.to))
    }

    /// Modules that depend on `selected`, heaviest first.
    pub fn incoming(&self, selected: usize) -> Vec<Neighbor> {
        self.neighbors(|d| (d.to == selected).then_some(d.from))
    }

    fn neighbors(&self, other: impl Fn(&Dependency) -> Option<usize>) -> Vec<Neighbor> {
        let mut out: Vec<Neighbor> = self
            .deps
            .iter()
            .filter_map(|d| {
                other(d).map(|index| Neighbor {
                    index,
                    weight: d.weight,
                })
            })
            .collect();
        out.sort_by(|a, b| {
            b.weight
                .cmp(&a.weight)
                .then(self.modules[a.index].name.cmp(&self.modules[b.index].name))
        });
        out
    }

    /// Which modules stay bright for a selection: the module itself and
    /// its direct neighbours. Without a selection everything is bright.
    pub fn bright(&self, selected: Option<usize>) -> Vec<bool> {
        let Some(s) = selected else {
            return vec![true; self.modules.len()];
        };
        let mut bright = vec![false; self.modules.len()];
        bright[s] = true;
        for d in &self.deps {
            if d.from == s {
                bright[d.to] = true;
            }
            if d.to == s {
                bright[d.from] = true;
            }
        }
        bright
    }

    /// Indices of modules whose name contains `query`, ignoring case.
    pub fn search(&self, query: &str) -> Vec<usize> {
        let q = query.trim().to_lowercase();
        (0..self.modules.len())
            .filter(|&i| q.is_empty() || self.modules[i].name.to_lowercase().contains(&q))
            .collect()
    }
}

/// Folder names that say where sources live, not what the module is.
const GENERIC: &[&str] = &[
    "src", "lib", "main", "java", "kotlin", "scala", "python", "source", "sources",
];

/// Card label and the scope above it. Trailing generic source folders
/// move into the scope, so `crates/archguard-cli/src` reads as
/// `archguard-cli` with the scope `crates · src`.
fn split_name(name: &str) -> (String, String) {
    let parts: Vec<&str> = name.trim_end_matches('/').split('/').collect();
    let tail = parts
        .iter()
        .rev()
        .take_while(|p| GENERIC.contains(p))
        .count()
        .min(parts.len() - 1);
    let at = parts.len() - 1 - tail;
    let before = parts[..at].join("/");
    let after = parts[at + 1..].join("/");
    let scope = match (before.is_empty(), after.is_empty()) {
        (true, true) => String::new(),
        (false, true) => before,
        (true, false) => after,
        (false, false) => format!("{before} · {after}"),
    };
    (scope, parts[at].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Edge, Node, NodeId, NodeKind, Ref};

    fn graph() -> (Graph, BTreeMap<String, ModuleMetrics>) {
        let node = |id: u32, name: &str| Node {
            id: NodeId(id),
            name: name.into(),
            kind: NodeKind::Module,
            parent: None,
            lang: None,
        };
        let edge = |from: u32, to: u32, count: u64| Edge {
            from: NodeId(from),
            to: NodeId(to),
            refs: vec![Ref {
                file: "x".into(),
                line: 1,
                raw: "import".into(),
            }],
            count: Some(count),
        };
        let g = Graph {
            schema_version: 1,
            root: ".".into(),
            nodes: vec![
                node(0, "src/api"),
                node(1, "src/core"),
                node(2, "src/db"),
                node(3, "tools"),
            ],
            edges: vec![edge(0, 1, 5), edge(0, 2, 2), edge(1, 2, 7)],
            semantic: None,
            review_kpis: None,
        };
        let mut metrics = BTreeMap::new();
        metrics.insert(
            "src/api".to_string(),
            ModuleMetrics {
                fan_in: 0,
                fan_out: 2,
                instability: 1.0,
                in_cycle: false,
            },
        );
        (g, metrics)
    }

    #[test]
    fn modules_carry_labels_metrics_and_positions() {
        let (g, m) = graph();
        let o = Overview::from_graph("shop".into(), &g, &m);
        assert_eq!(o.modules.len(), 4);
        let api = &o.modules[0];
        assert_eq!((api.scope.as_str(), api.label.as_str()), ("src", "api"));
        assert_eq!((api.fan_out, api.instability), (2, 1.0));
        assert_eq!(o.modules[3].scope, "");
        assert!(o.modules[0].y < o.modules[1].y, "api sits above core");
        assert_eq!(o.deps.len(), 3);
        assert_eq!(o.deps[2].weight, 7);
    }

    #[test]
    fn generic_source_folders_do_not_become_the_label() {
        assert_eq!(
            split_name("crates/archguard-cli/src"),
            ("crates · src".to_string(), "archguard-cli".to_string())
        );
        assert_eq!(
            split_name("services/catalog/src/main/java"),
            (
                "services · src/main/java".to_string(),
                "catalog".to_string()
            )
        );
        assert_eq!(
            split_name("web/app"),
            ("web".to_string(), "app".to_string())
        );
        assert_eq!(split_name("src"), (String::new(), "src".to_string()));
    }

    #[test]
    fn neighbours_are_sorted_by_weight() {
        let (g, m) = graph();
        let o = Overview::from_graph("shop".into(), &g, &m);
        let out = o.outgoing(0);
        assert_eq!(
            out,
            vec![
                Neighbor {
                    index: 1,
                    weight: 5
                },
                Neighbor {
                    index: 2,
                    weight: 2
                }
            ]
        );
        let into_db: Vec<usize> = o.incoming(2).iter().map(|n| n.index).collect();
        assert_eq!(into_db, vec![1, 0]);
    }

    #[test]
    fn selection_keeps_only_direct_neighbours_bright() {
        let (g, m) = graph();
        let o = Overview::from_graph("shop".into(), &g, &m);
        assert_eq!(o.bright(None), vec![true; 4]);
        assert_eq!(o.bright(Some(1)), vec![true, true, true, false]);
    }

    #[test]
    fn search_ignores_case_and_empty_query_matches_all() {
        let (g, m) = graph();
        let o = Overview::from_graph("shop".into(), &g, &m);
        assert_eq!(o.search("  "), vec![0, 1, 2, 3]);
        assert_eq!(o.search("SRC/C"), vec![1]);
    }
}
