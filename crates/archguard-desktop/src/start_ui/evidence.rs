//! The world's evidence and its scoped view: the port of `worldScope`,
//! `classMetricValue` and `classMetricAssessment` from the web viewer.
//! Entities come from `archguard_engine::architecture_world`, areas and
//! package cycles from `archguard_engine::architecture_atlas`, class metrics
//! from `archguard_engine::code_metrics`, as in the CLI's report.

use std::collections::HashMap;

use archguard_engine::core::{CallAnalysis, Finding, Graph, NodeKind};
use archguard_engine::{ArchitectureWorld, ClassMetrics, WorldEntity};

use crate::rating::Cohorts;
pub use crate::rating::Rating;

/// The class metrics of the Explore lens, in the web's selector order.
pub use crate::rating::METRICS as KPIS;

/// One method of a class, for the detail panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Method {
    pub name: String,
    pub cyclomatic: Option<u32>,
    pub nesting: Option<u32>,
    pub file: String,
    pub line: u32,
}

/// Everything the tab derives its views from; `Send`, so a worker thread
/// can collect it.
pub struct Evidence {
    pub graph: Graph,
    pub findings: Vec<Finding>,
    pub world: ArchitectureWorld,
    /// Class id -> value and rating per entry of [`KPIS`].
    pub kpis: HashMap<String, [(Option<f64>, Rating); 6]>,
    /// Class id (or file node for files without classes) -> methods.
    pub methods: HashMap<String, Vec<Method>>,
}

impl Evidence {
    pub fn new(
        graph: Graph,
        findings: Vec<Finding>,
        calls: &CallAnalysis,
        world: ArchitectureWorld,
    ) -> Self {
        let metrics = archguard_engine::code_metrics(calls);
        Evidence {
            kpis: rate_classes(&metrics.classes),
            methods: methods(calls),
            graph,
            findings,
            world,
        }
    }

    pub fn entity(&self, i: usize) -> &WorldEntity {
        &self.world.entities[i]
    }

    /// The value and rating of a class metric (`kpi` 1..=6) for an entity.
    pub fn kpi(&self, entity: usize, kpi: usize) -> (Option<f64>, Rating) {
        self.entity(entity)
            .class_id
            .as_ref()
            .and_then(|id| self.kpis.get(id))
            .map_or((None, Rating::Unknown), |row| row[kpi - 1])
    }
}

fn methods(calls: &CallAnalysis) -> HashMap<String, Vec<Method>> {
    let mut out: HashMap<String, Vec<Method>> = HashMap::new();
    for m in &calls.methods {
        let row = Method {
            name: if m.signature.is_empty() {
                m.name.clone()
            } else {
                m.signature.clone()
            },
            cyclomatic: m.complexity.as_ref().map(|c| c.cyclomatic),
            nesting: m.complexity.as_ref().map(|c| c.max_nesting),
            file: m.file.clone(),
            line: m.line,
        };
        out.entry(m.file.clone()).or_default().push(row.clone());
        out.entry(m.class_id.clone()).or_default().push(row);
    }
    out
}

/// Value and rating per entry of [`KPIS`] for every class, from the one
/// rating the Quality heatmap uses.
fn rate_classes(classes: &[ClassMetrics]) -> HashMap<String, [(Option<f64>, Rating); 6]> {
    let cohorts = Cohorts::new(classes);
    classes
        .iter()
        .map(|row| {
            let rated = std::array::from_fn(|key| {
                let a = cohorts.assess(row, KPIS[key]);
                (a.value, a.state)
            });
            (row.class_id.clone(), rated)
        })
        .collect()
}

/// One immediate package or file of the scope.
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub name: String,
    pub module: bool,
    pub cyclic: bool,
    /// Indices into the evidence's findings that involve this area.
    pub findings: Vec<usize>,
    /// Indices into the world's entities, sorted by entity id.
    pub entities: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub from: usize,
    pub to: usize,
    pub cyclic: bool,
    /// Dependencies behind the connection, when the atlas counts them.
    pub references: Option<u64>,
}

/// `worldScope`: the atlas of one scope with entities and findings per area.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scope {
    pub name: Option<String>,
    pub cycle_groups: usize,
    pub areas: Vec<Area>,
    pub connections: Vec<Connection>,
}

pub fn scope(ev: &Evidence, within: Option<&str>) -> Option<Scope> {
    let atlas = archguard_engine::architecture_atlas(&ev.graph, within).ok()?;
    let by_name: HashMap<&str, usize> = atlas
        .areas
        .iter()
        .enumerate()
        .map(|(i, a)| (a.name.as_str(), i))
        .collect();
    let owners = owners(&ev.graph, &by_name);
    let mut areas: Vec<Area> = atlas
        .areas
        .iter()
        .map(|a| Area {
            name: a.name.clone(),
            module: a.kind == NodeKind::Module,
            cyclic: a.cyclic,
            findings: Vec::new(),
            entities: Vec::new(),
        })
        .collect();
    for (i, e) in ev.world.entities.iter().enumerate() {
        if let Some(&owner) = owners.get(e.node.as_str()) {
            areas[owner].entities.push(i);
        }
    }
    for area in &mut areas {
        let prefix = format!("{}/", area.name);
        area.findings = (0..ev.findings.len())
            .filter(|&f| {
                ev.findings[f]
                    .nodes
                    .iter()
                    .any(|n| *n == area.name || n.starts_with(&prefix))
            })
            .collect();
        area.entities
            .sort_by(|&a, &b| ev.world.entities[a].id.cmp(&ev.world.entities[b].id));
    }
    let connections = atlas
        .connections
        .iter()
        .filter_map(|c| {
            Some(Connection {
                from: *by_name.get(c.from.as_str())?,
                to: *by_name.get(c.to.as_str())?,
                cyclic: c.cyclic,
                references: c.evidence_count,
            })
        })
        .collect();
    Some(Scope {
        name: atlas.scope,
        cycle_groups: atlas.cycle_groups,
        areas,
        connections,
    })
}

/// Node name -> the area (by index) that contains it.
fn owners<'g>(graph: &'g Graph, areas: &HashMap<&str, usize>) -> HashMap<&'g str, usize> {
    let index: HashMap<_, _> = graph.nodes.iter().map(|n| (n.id, n)).collect();
    let mut out = HashMap::new();
    for node in &graph.nodes {
        let mut current = Some(node);
        while let Some(n) = current {
            if let Some(&area) = areas.get(n.name.as_str()) {
                out.insert(node.name.as_str(), area);
                break;
            }
            current = n.parent.and_then(|p| index.get(&p).copied());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{FindingKind, GraphBuilder, Ref, Severity};

    fn evidence() -> Evidence {
        let mut b = GraphBuilder::new();
        let a = b.ensure_path("org/app/search/A.ts", Some("ts"));
        let c = b.ensure_path("org/app/client/C.ts", Some("ts"));
        let t = b.ensure_path("org/app/search/Types.ts", Some("ts"));
        let r = |file: &str| Ref {
            file: file.into(),
            line: 1,
            raw: "import".into(),
        };
        b.edge(a, c, r("A.ts")).unwrap();
        b.edge(c, t, r("C.ts")).unwrap();
        let graph = b.build("/fixture".into());
        let calls = CallAnalysis::default();
        let coverage = archguard_engine::TestCoverageReport {
            schema_version: 1,
            status: "missing".into(),
            format: None,
            report_path: None,
            imported_at_unix: None,
            report_modified_unix: None,
            files: Vec::new(),
            unmatched_files: 0,
            diagnostics: Vec::new(),
        };
        let world = archguard_engine::architecture_world(&graph, &calls, &coverage);
        let finding = Finding {
            kind: FindingKind::MetricThreshold,
            id: "m-1".into(),
            severity: Severity::Warning,
            message: None,
            evidence: Vec::new(),
            nodes: vec!["org/app/search/A.ts".into()],
            edge_count: 0,
            cycles_sampled: 0,
            sampling: None,
            suggested_cuts: Vec::new(),
        };
        Evidence::new(graph, vec![finding], &calls, world)
    }

    /// The skipped `org/app` chain gives two package areas in one cycle;
    /// files without classes are entities of their package.
    #[test]
    fn scope_groups_entities_and_findings_by_area() {
        let ev = evidence();
        let s = scope(&ev, None).expect("root scope");
        assert_eq!(s.name.as_deref(), Some("org/app"));
        assert_eq!(s.cycle_groups, 1);
        let names: Vec<&str> = s.areas.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["org/app/client", "org/app/search"]);
        assert!(s.areas.iter().all(|a| a.module && a.cyclic));
        assert_eq!(s.areas[0].entities.len(), 1);
        assert_eq!(s.areas[1].entities.len(), 2);
        assert_eq!(s.areas[1].findings, vec![0]);
        assert!(s.areas[0].findings.is_empty());
        assert_eq!(s.connections.len(), 2);
        assert!(s.connections.iter().all(|c| c.cyclic));
        // A file scope holds the file itself.
        let file = scope(&ev, Some("org/app/search/A.ts")).unwrap();
        assert_eq!(file.areas.len(), 1);
        assert!(!file.areas[0].module);
        assert!(scope(&ev, Some("nope")).is_none());
    }
}
