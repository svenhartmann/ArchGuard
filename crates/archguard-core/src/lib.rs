//! `archguard-core` — L0: the shared vocabulary of archguard.
//!
//! This crate defines the canonical graph model (ARCHITECTURE.md Kap. 5):
//! a containment tree of `module`/`file` [`Node`]s plus [`Edge`]s that
//! connect only file nodes, each edge carrying its source-evidence [`Ref`]s.
//!
//! Every zoom level of an analysis is itself a [`Graph`], which is why all
//! analyses run unchanged at any depth. On top of the graph this crate
//! defines the analysis vocabulary: [`Finding`]s collected into a [`Report`]
//! with per-module [`ModuleMetrics`], the `archguard.toml` [`Config`] (Kap. 11)
//! and the shared [`ArchguardError`]. `GraphBuilder` arrives in T04.

/// Shared UI and MCP exploration routes.
pub const DEVELOPMENT_PATHS: &str = include_str!("development_paths.json");

mod architecture;
mod calls;
mod config;
mod diagrams;
pub use diagrams::{
    Arc42Config, Diagram, DiagramElement, DiagramFormat, DiagramKind, DiagramRelation, DocLang,
    ElementKind, RelationKind, SequenceStep, sequence_steps,
};
mod evaluation;
pub use evaluation::{
    Assessment, AssessmentMeta, ExpectedKind, ExpectedProblem, JudgedFinding, Verdict,
    finding_kind_name,
};
mod model;
mod quality_goal;
mod semantic_details;
mod settings;
pub use semantic_details::{
    ClassCohesion, ClassInheritance, ClassState, FieldAccess, SemanticDetails, SemanticTypeUse,
    SourceMethodMetrics, StateField,
};
mod graph;
mod performance;
mod review;
mod review_kpi;
mod review_session;
mod rules;
mod runtime;
pub use runtime::{
    RuntimeBreakdown, RuntimeCallSite, RuntimeConfig, RuntimeDiagnostic, RuntimeEndpoint,
    RuntimeEvidence, RuntimeFinding, RuntimeMethod, RuntimeProfile, RuntimeRequest, RuntimeSpan,
    RuntimeStatement, RuntimeStats, RuntimeTimePart,
};
mod services;
mod staged;
pub use staged::{
    ArchitectureDiffDetails, ArchitectureEdgeChange, BranchArchitectureDiff, StagedArchitectureDiff,
};
mod view;
mod workspace;

pub use architecture::{
    AmbiguousArchitectureNode, ArchitectureComponent, ArchitectureConfig, ArchitectureConstraint,
    ArchitectureContext, ArchitectureDecision, ArchitectureDependency, ArchitectureDirection,
    ArchitectureGoal, ArchitectureObserved, ArchitecturePrivateAccess, MappedArchitectureComponent,
    TargetArchitectureConfig, TargetArchitectureReport,
};
pub use calls::{
    CSharpSemanticConfig, CallAnalysis, CallAnalyzer, CallClass, CallMethod, CallSupertypeSite,
    JavaSemanticConfig, MethodCall, MethodComplexity, SemanticConfig, SemanticDependencyCoverage,
    TypeScriptSemanticConfig,
};
pub use config::Config;
pub use graph::GraphBuilder;
pub use model::{
    ArchguardError, ConfigSource, CycleSampling, Edge, Finding, FindingKind, Graph, ModuleMetrics,
    Node, NodeId, NodeKind, Ref, Report, Severity, Status, SuggestedCut, Summary,
};
pub use performance::{
    AbstractionMetric, ContractCandidate, CyclicBuildGroup, EntryClosure, PerformanceConfig,
    PerformanceReport, SourceCostFinding,
};
pub use quality_goal::{
    EvidenceSource, GoalAdvisory, GoalAssessment, GoalEvidenceSource, GoalKpi, GoalStatus,
    ImportedEvidence, QualityAttribute,
};
pub use review::{
    GitEvidence, GitStatus, ModuleReview, OrphanCandidate, OrphanPolicy, ReviewConfig,
    ReviewPriority, ReviewReport, ReviewScore, ReviewSources, ReviewSummary, SourceEvidence,
};
pub use review_kpi::{
    ApiSurface, DebtDensity, DebtMarker, DependencyInventory, ExternalDependency, HealthIndex,
    HealthLevel, HealthWeights, HubGini, LayerDrift, Leak, MarkerKind, PropagationCost, ReviewKpis,
    RiskCoverageGap, SdpViolation, Tangledness, Visibility, ZoneStats,
};
pub use review_session::{DecisionKind, FindingRef, ReviewDecision, ReviewRegister};
pub use rules::{DependencyRule, OwnershipRule};
pub use services::{
    Datastore, DatastoreUsage, RouteFailure, RouteStep, Service, ServiceCycle, ServiceDatastores,
    ServiceDependency, ServiceDependencyKind, ServiceDiagnostic, ServiceEvidence, ServiceProject,
    ServiceResilience, ServiceRoute, ServiceTopology, SharedDatastore, SharedDatastoreScope,
    TraceSequence, TraceStep,
};
pub use settings::{
    DenyCycles, LayerConfig, LayerOrigin, ModulesConfig, ProjectConfig, RulesConfig, StoreConfig,
};
pub use view::{Cut, aggregate};
pub use workspace::{WorkspaceDependency, WorkspaceReport, WorkspaceUnresolved};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Tiny hand-built tree mirroring the canonical example (Kap. 5):
    /// app → app/billing → app/billing/invoice.ts; domain → domain/money.ts,
    /// with one edge invoice.ts → money.ts.
    fn sample_graph() -> Graph {
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![
                Node {
                    id: NodeId(0),
                    name: "app".to_string(),
                    kind: NodeKind::Module,
                    parent: None,
                    lang: None,
                },
                Node {
                    id: NodeId(1),
                    name: "app/billing".to_string(),
                    kind: NodeKind::Module,
                    parent: Some(NodeId(0)),
                    lang: None,
                },
                Node {
                    id: NodeId(2),
                    name: "app/billing/invoice.ts".to_string(),
                    kind: NodeKind::File,
                    parent: Some(NodeId(1)),
                    lang: Some("ts".to_string()),
                },
                Node {
                    id: NodeId(3),
                    name: "domain".to_string(),
                    kind: NodeKind::Module,
                    parent: None,
                    lang: None,
                },
                Node {
                    id: NodeId(4),
                    name: "domain/money.ts".to_string(),
                    kind: NodeKind::File,
                    parent: Some(NodeId(3)),
                    lang: Some("ts".to_string()),
                },
            ],
            // Legacy shape (no `count`): weight() is the sample length.
            edges: vec![Edge {
                from: NodeId(2),
                to: NodeId(4),
                refs: vec![Ref {
                    file: "src/app/billing/invoice.ts".to_string(),
                    line: 4,
                    raw: "import { Money } from \"../../domain/money\";".to_string(),
                }],
                count: None,
            }],
        }
    }

    #[test]
    fn graph_round_trips_through_json() {
        let graph = sample_graph();
        let json = serde_json::to_string(&graph).expect("serialization");
        let back: Graph = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(graph, back);
    }

    #[test]
    fn node_id_is_transparent_in_json() {
        assert_eq!(serde_json::to_string(&NodeId(7)).unwrap(), "7");
        assert_eq!(serde_json::from_str::<NodeId>("7").unwrap(), NodeId(7));
    }

    #[test]
    fn node_kind_uses_lowercase_names() {
        assert_eq!(
            serde_json::to_string(&NodeKind::Module).unwrap(),
            "\"module\""
        );
        assert_eq!(serde_json::to_string(&NodeKind::File).unwrap(), "\"file\"");
        assert_eq!(
            serde_json::from_str::<NodeKind>("\"module\"").unwrap(),
            NodeKind::Module
        );
        assert_eq!(
            serde_json::from_str::<NodeKind>("\"file\"").unwrap(),
            NodeKind::File
        );
    }

    #[test]
    fn module_node_json_matches_canonical_shape() {
        // Kap. 5 example: module nodes carry no "lang" key, roots carry
        // "parent": null.
        let node = sample_graph().nodes[0].clone();
        assert_eq!(
            serde_json::to_string(&node).unwrap(),
            r#"{"id":0,"name":"app","kind":"module","parent":null}"#
        );
    }

    #[test]
    fn graph_deserializes_from_canonical_json() {
        let json = r#"{
            "schema_version": 1,
            "root": "/abs/projekt",
            "nodes": [
                { "id": 0, "name": "app", "kind": "module", "parent": null },
                { "id": 1, "name": "app/billing", "kind": "module", "parent": 0 },
                { "id": 2, "name": "app/billing/invoice.ts", "kind": "file",
                  "parent": 1, "lang": "ts" },
                { "id": 3, "name": "domain", "kind": "module", "parent": null },
                { "id": 4, "name": "domain/money.ts", "kind": "file",
                  "parent": 3, "lang": "ts" }
            ],
            "edges": [
                { "from": 2, "to": 4, "refs": [
                    { "file": "src/app/billing/invoice.ts", "line": 4,
                      "raw": "import { Money } from \"../../domain/money\";" }
                ] }
            ]
        }"#;
        let graph: Graph = serde_json::from_str(json).expect("canonical JSON");
        assert_eq!(graph, sample_graph());
    }

    #[test]
    fn ref_and_edge_round_trip() {
        let edge = sample_graph().edges[0].clone();
        let json = serde_json::to_string(&edge).expect("serialization");
        let back: Edge = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(edge, back);
    }

    #[test]
    fn edge_json_omits_count_when_none_and_accepts_legacy_json() {
        // F12: the field is invisible when absent — a legacy graph JSON
        // (no "count" key) parses to `None`, and a `None` edge
        // serializes without the key, keeping canonical JSON stable.
        let edge = sample_graph().edges[0].clone();
        assert!(edge.count.is_none());
        let json = serde_json::to_string(&edge).expect("serialization");
        assert!(!json.contains("count"), "{json}");
        let legacy = r#"{"from":2,"to":4,"refs":[{"file":"a.ts","line":1,"raw":"import"}]}"#;
        let back: Edge = serde_json::from_str(legacy).expect("legacy JSON");
        assert_eq!(back.count, None);
    }

    #[test]
    fn weight_is_exact_with_count_and_a_lower_bound_without() {
        let refs: Vec<Ref> = (0..3)
            .map(|k| Ref {
                file: "a.ts".to_string(),
                line: k + 1,
                raw: "import".to_string(),
            })
            .collect();
        let counted = Edge {
            from: NodeId(0),
            to: NodeId(1),
            refs: refs.clone(),
            count: Some(100),
        };
        // The total wins over the sample length; it is kept even when
        // it contradicts the (sampled) refs, never corrected away.
        assert_eq!(counted.weight(), 100);
        let legacy = Edge {
            from: NodeId(0),
            to: NodeId(1),
            refs,
            count: None,
        };
        // Without the field only the sample is known: lower bound.
        assert_eq!(legacy.weight(), 3);
    }

    #[test]
    fn node_name_resolves_known_and_unknown_ids() {
        let graph = sample_graph();
        assert_eq!(graph.node_name(NodeId(2)), Some("app/billing/invoice.ts"));
        assert_eq!(graph.node_name(NodeId(3)), Some("domain"));
        assert_eq!(graph.node_name(NodeId(99)), None);
    }

    #[test]
    fn out_edges_yields_only_edges_starting_at_id() {
        let graph = sample_graph();
        let outs: Vec<&Edge> = graph.out_edges(NodeId(2)).collect();
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].to, NodeId(4));
        assert_eq!(outs[0].refs.len(), 1);
        assert_eq!(graph.out_edges(NodeId(4)).count(), 0);
        assert_eq!(graph.out_edges(NodeId(99)).count(), 0);
    }

    #[test]
    fn children_of_returns_direct_children_sorted() {
        let graph = sample_graph();
        assert_eq!(graph.children_of(NodeId(0)), vec![NodeId(1)]);
        assert_eq!(graph.children_of(NodeId(1)), vec![NodeId(2)]);
        assert_eq!(graph.children_of(NodeId(3)), vec![NodeId(4)]);
        assert_eq!(graph.children_of(NodeId(2)), Vec::<NodeId>::new());
        assert_eq!(graph.children_of(NodeId(99)), Vec::<NodeId>::new());
    }

    #[test]
    fn ancestors_of_returns_nearest_first() {
        let graph = sample_graph();
        assert_eq!(graph.ancestors_of(NodeId(2)), vec![NodeId(1), NodeId(0)]);
        assert_eq!(graph.ancestors_of(NodeId(4)), vec![NodeId(3)]);
        assert_eq!(graph.ancestors_of(NodeId(0)), Vec::<NodeId>::new());
        assert_eq!(graph.ancestors_of(NodeId(99)), Vec::<NodeId>::new());
    }

    #[test]
    fn ancestors_of_terminates_on_parent_cycle() {
        // Malformed input must not loop forever: a -> b -> a.
        let graph = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/abs".to_string(),
            nodes: vec![
                Node {
                    id: NodeId(0),
                    name: "a".to_string(),
                    kind: NodeKind::Module,
                    parent: Some(NodeId(1)),
                    lang: None,
                },
                Node {
                    id: NodeId(1),
                    name: "b".to_string(),
                    kind: NodeKind::Module,
                    parent: Some(NodeId(0)),
                    lang: None,
                },
            ],
            edges: Vec::new(),
        };
        assert_eq!(graph.ancestors_of(NodeId(0)), vec![NodeId(1), NodeId(0)]);
    }

    /// The Kap. 5 example finding, used by the report tests.
    fn sample_finding() -> Finding {
        Finding {
            message: None,
            evidence: Vec::new(),
            kind: FindingKind::Cycle,
            id: "scc-3".to_string(),
            severity: Severity::Error,
            nodes: vec![
                "app/billing".to_string(),
                "app/tax".to_string(),
                "domain/money".to_string(),
            ],
            edge_count: 5,
            cycles_sampled: 4,
            sampling: None,
            suggested_cuts: vec![SuggestedCut {
                from: "domain/money".to_string(),
                to: "app/tax".to_string(),
                score: 0.91,
                reason: "Nur 1 Import-Stelle; Trennen bricht 4 von 4 Zyklen.".to_string(),
            }],
        }
    }

    /// The Kap. 5 example report.
    fn sample_report() -> Report {
        Report {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            summary: Summary {
                nodes: 142,
                edges: 519,
                sccs: 2,
                largest_scc: 7,
                layer_violations: 3,
                status: Status::Fail,
                ccd: 0,
                acd: 0.0,
                nccd: 0.0,
            },
            // Kap. 5 example: provenance per section.
            config: [
                ("project".to_string(), ConfigSource::Derived),
                ("modules".to_string(), ConfigSource::Derived),
                ("layers".to_string(), ConfigSource::Mixed),
                ("rules".to_string(), ConfigSource::Default),
                ("store".to_string(), ConfigSource::Default),
            ]
            .into_iter()
            .collect(),
            findings: vec![sample_finding()],
            metrics: [
                (
                    "app/billing".to_string(),
                    ModuleMetrics {
                        fan_in: 3,
                        fan_out: 2,
                        instability: 0.4,
                        in_cycle: true,
                    },
                ),
                (
                    "domain/money".to_string(),
                    ModuleMetrics {
                        fan_in: 5,
                        fan_out: 0,
                        instability: 0.0,
                        in_cycle: false,
                    },
                ),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn report_serde_round_trips() {
        let report = sample_report();
        let json = serde_json::to_string(&report).expect("serialization");
        let back: Report = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(report, back);
    }

    #[test]
    fn report_json_uses_architecture_names() {
        let value = serde_json::to_value(sample_report()).expect("serialization");
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["summary"]["status"], "fail");
        assert_eq!(value["summary"]["largest_scc"], 7);
        assert_eq!(value["config"]["project"], "derived");
        assert_eq!(value["config"]["layers"], "mixed");
        assert_eq!(value["config"]["rules"], "default");
        assert_eq!(value["findings"][0]["kind"], "cycle");
        assert_eq!(value["findings"][0]["id"], "scc-3");
        assert_eq!(value["findings"][0]["severity"], "error");
        assert_eq!(value["findings"][0]["edge_count"], 5);
        assert_eq!(value["findings"][0]["cycles_sampled"], 4);
        let cut = &value["findings"][0]["suggested_cuts"][0];
        assert_eq!(cut["from"], "domain/money");
        assert_eq!(cut["to"], "app/tax");
        assert_eq!(cut["score"].as_f64(), Some(f64::from(0.91f32)));
        assert_eq!(value["metrics"]["app/billing"]["in_cycle"], true);
        assert_eq!(value["metrics"]["app/billing"]["fan_in"], 3);
    }

    #[test]
    fn module_metrics_round_trip() {
        let metrics = ModuleMetrics {
            fan_in: 3,
            fan_out: 2,
            instability: 0.4,
            in_cycle: true,
        };
        let json = serde_json::to_string(&metrics).expect("serialization");
        assert_eq!(
            json,
            r#"{"fan_in":3,"fan_out":2,"instability":0.4,"in_cycle":true}"#
        );
        let back: ModuleMetrics = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(metrics, back);
    }

    #[test]
    fn finding_kind_uses_snake_case_names() {
        for (kind, name) in [
            (FindingKind::Cycle, "cycle"),
            (FindingKind::LayerViolation, "layer_violation"),
            (FindingKind::MetricThreshold, "metric_threshold"),
            (FindingKind::LayerUnranked, "layer_unranked"),
        ] {
            let json = serde_json::to_string(&kind).expect("serialization");
            assert_eq!(json, format!("\"{name}\""));
            let back: FindingKind = serde_json::from_str(&json).expect("deserialization");
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn severity_uses_lowercase_names_and_sorts_error_first() {
        assert_eq!(
            serde_json::to_string(&Severity::Error).unwrap(),
            "\"error\""
        );
        assert_eq!(
            serde_json::to_string(&Severity::Warning).unwrap(),
            "\"warning\""
        );
        assert_eq!(serde_json::to_string(&Severity::Info).unwrap(), "\"info\"");
        // Kap. 5: findings are sorted by severity first, so Error < Warning < Info.
        assert!(Severity::Error < Severity::Warning);
        assert!(Severity::Warning < Severity::Info);
    }

    #[test]
    fn config_default_round_trips() {
        let config = Config::default();
        let json = serde_json::to_string(&config).expect("serialization");
        let back: Config = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(config, back);
    }

    #[test]
    fn config_defaults_match_architecture_doc() {
        let config = Config::default();
        assert!(config.derive_layers);
        assert_eq!(config.project.source_roots, ["src"]);
        assert_eq!(
            config.project.exclude,
            ["**/tests/**", "**/*.spec.*", "**/generated/**"]
        );
        assert_eq!(config.project.languages, ["auto"]);
        assert_eq!(config.modules.depth, 2);
        assert!(config.layers.is_empty());
        assert_eq!(config.rules.deny_cycles, DenyCycles::Module);
        assert_eq!(config.rules.max_scc_size, 1);
        assert_eq!(config.rules.baseline, ".archguard/baseline.json");
        assert_eq!(config.rules.max_acd, 0.0);
        assert_eq!(config.rules.max_instability_core, 0.0);
        assert_eq!(config.store.db, ".archguard/graph.db");
        assert!(!config.store.sync_on_check);
    }

    #[test]
    fn config_missing_sections_fall_back_to_defaults() {
        // `#[serde(default)]` everywhere: one key in one section, everything
        // else defaulted.
        let config: Config = serde_json::from_str(r#"{ "rules": { "deny_cycles": "off" } }"#)
            .expect("partial config");
        assert_eq!(config.rules.deny_cycles, DenyCycles::Off);
        assert_eq!(config.rules.max_scc_size, 1);
        assert_eq!(config.project, ProjectConfig::default());
        let mut expected = Config::default();
        expected.rules.deny_cycles = DenyCycles::Off;
        assert_eq!(config, expected);
    }

    #[test]
    fn empty_config_is_default_config() {
        let config: Config = serde_json::from_str("{}").expect("empty config");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn config_derive_layers_defaults_true_and_can_be_overridden() {
        // Struct-level `#[serde(default)]` uses the manual `Default`, so an
        // omitted key means `true` (Kap. 11); an explicit key wins.
        let config: Config = serde_json::from_str("{}").expect("omitted derive_layers");
        assert!(config.derive_layers);
        let config: Config =
            serde_json::from_str(r#"{ "derive_layers": false }"#).expect("explicit derive_layers");
        assert!(!config.derive_layers);
    }

    #[test]
    fn config_source_uses_lowercase_names() {
        for (source, name) in [
            (ConfigSource::Declared, "declared"),
            (ConfigSource::Derived, "derived"),
            (ConfigSource::Default, "default"),
            (ConfigSource::Mixed, "mixed"),
        ] {
            let json = serde_json::to_string(&source).expect("serialization");
            assert_eq!(json, format!("\"{name}\""));
            let back: ConfigSource = serde_json::from_str(&json).expect("deserialization");
            assert_eq!(back, source);
        }
    }

    #[test]
    fn layer_origin_uses_lowercase_names_and_defaults_to_declared() {
        for (origin, name) in [
            (LayerOrigin::Declared, "declared"),
            (LayerOrigin::Derived, "derived"),
        ] {
            let json = serde_json::to_string(&origin).expect("serialization");
            assert_eq!(json, format!("\"{name}\""));
            let back: LayerOrigin = serde_json::from_str(&json).expect("deserialization");
            assert_eq!(back, origin);
        }
        assert_eq!(LayerOrigin::default(), LayerOrigin::Declared);
    }

    #[test]
    fn layer_config_toml_shape_is_unchanged_and_origin_is_engine_only() {
        // `origin` is skipped: never serialized, never parsed — the merge
        // sets it. A stray "origin" key in the input is rejected.
        let layer = LayerConfig {
            name: "domain".to_string(),
            contains: vec!["domain/**".to_string()],
            may_depend_on: Vec::new(),
            origin: LayerOrigin::Derived,
        };
        let json = serde_json::to_string(&layer).expect("serialization");
        assert_eq!(
            json,
            r#"{"name":"domain","contains":["domain/**"],"may_depend_on":[]}"#
        );
        assert!(
            serde_json::from_str::<LayerConfig>(r#"{"name":"domain","origin":"derived"}"#).is_err()
        );
        assert_eq!(LayerConfig::default().origin, LayerOrigin::Declared);
    }

    #[test]
    fn report_config_block_is_optional_and_uses_lowercase_names() {
        let mut report = sample_report();
        report.config = BTreeMap::new();
        let mut value = serde_json::to_value(&report).expect("serialization");
        assert_eq!(value["config"], serde_json::json!({}));
        let back: Report =
            serde_json::from_value(value.clone()).expect("deserialization with empty block");
        assert!(back.config.is_empty());
        // An absent block deserializes to empty via `#[serde(default)]`.
        value.as_object_mut().expect("object").remove("config");
        let back: Report = serde_json::from_value(value).expect("deserialization without block");
        assert!(back.config.is_empty());
        assert_eq!(
            serde_json::to_value(ConfigSource::Mixed).expect("serialization"),
            "mixed"
        );
    }

    #[test]
    fn archguard_error_converts_io_and_displays_variants() {
        let io: ArchguardError = std::io::Error::other("disk gone").into();
        assert!(matches!(io, ArchguardError::Io(_)));
        assert!(io.to_string().contains("disk gone"));
        assert_eq!(
            ArchguardError::Config("bad toml".to_string()).to_string(),
            "config error: bad toml"
        );
        assert_eq!(
            ArchguardError::Analysis("no such node".to_string()).to_string(),
            "analysis error: no such node"
        );
        let result: Result<(), ArchguardError> = Err(std::io::Error::other("boom").into());
        assert!(result.is_err());
    }
}
