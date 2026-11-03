//! Explicit architecture intent is reported separately from graph observations.
use archguard_core::{
    ArchitectureContext, ArchitectureObserved, Config, Graph, LayerOrigin, NodeKind, Report,
};
use std::path::Path;

pub fn architecture_context(
    root: &Path,
    config: &Config,
    graph: &Graph,
    report: &Report,
) -> ArchitectureContext {
    let intent = &config.architecture;
    let mut questions = vec![];
    if intent.goals.is_empty() {
        questions.push("Which quality goals matter most, and which trade-offs are acceptable to this organization?".into());
    }
    for goal in &intent.goals {
        if goal.priority.trim().is_empty() {
            questions.push(format!(
                "What priority does {:?} have relative to the other declared goals?",
                goal.name
            ));
        }
        if goal.rationale.trim().is_empty() {
            questions.push(format!(
                "Why is {:?} important to this organization?",
                goal.name
            ));
        }
    }
    if intent.constraints.is_empty() {
        questions.push("Which constraints are non-negotiable, such as compatibility, deployment, data residency or team ownership?".into());
    }
    for constraint in &intent.constraints {
        if constraint.rationale.trim().is_empty() {
            questions.push(format!(
                "What evidence or obligation makes {:?} a constraint?",
                constraint.name
            ));
        }
    }
    if intent.decisions.is_empty() {
        questions.push("Which architecture decisions were explicitly accepted, and what alternatives and disadvantages were considered?".into());
    }
    for decision in &intent.decisions {
        if decision.status.trim().is_empty() {
            questions.push(format!(
                "What is the current status of decision {:?}?",
                decision.id
            ));
        }
        if decision.chosen_option.trim().is_empty() {
            questions.push(format!(
                "Which option was chosen in decision {:?}?",
                decision.id
            ));
        }
        if decision.alternatives.is_empty() {
            questions.push(format!(
                "Which alternatives were considered for decision {:?}?",
                decision.id
            ));
        }
        if decision.trade_offs.is_empty() {
            questions.push(format!(
                "Which trade-offs were accepted in decision {:?}?",
                decision.id
            ));
        }
    }
    ArchitectureContext {
        schema_version:1,project_root:root.to_string_lossy().into_owned(),intent:intent.clone(),
        observed:ArchitectureObserved {
            summary:report.summary.clone(),
            source_files:graph.nodes.iter().filter(|node|node.kind==NodeKind::File&&!node.name.ends_with("/*")).count() as u32,
            declared_layers:config.layers.iter().filter(|layer|layer.origin==LayerOrigin::Declared).cloned().collect(),
            dependency_rules:config.dependency_rules.clone(),ownership_rules:config.ownership_rules.clone(),rules:config.rules.clone(),orphan_policy:config.review.orphans,
            findings:report.findings.iter().take(50).cloned().collect(),findings_total:report.findings.len() as u32,
            target_model:crate::target_architecture::analyze_target_architecture(graph,config).expect("loaded architecture config was validated"),
        },
        questions,
        limitations:vec![
            "Intent is supplied by the organization; graph structure cannot determine its preferences or accepted trade-offs.".into(),
            "A passing import gate does not establish that every business constraint or architecture decision is satisfied.".into(),
            "Observed findings are limited to the first 50 in report order; findings_total reports the full count.".into(),
            "Decision links are untrusted references, not fetched evidence; priority and status retain the organization's own vocabulary.".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Status, Summary};
    fn report() -> Report {
        Report {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            summary: Summary {
                nodes: 0,
                edges: 0,
                sccs: 0,
                largest_scc: 0,
                layer_violations: 0,
                status: Status::Pass,
                ccd: 0,
                acd: 0.0,
                nccd: 0.0,
            },
            config: Default::default(),
            findings: vec![],
            metrics: Default::default(),
        }
    }
    #[test]
    fn unspecified_intent_produces_questions_instead_of_invented_preferences() {
        let graph = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/repo".into(),
            nodes: vec![],
            edges: vec![],
        };
        let context =
            architecture_context(Path::new("/repo"), &Config::default(), &graph, &report());
        assert!(context.intent.goals.is_empty());
        assert_eq!(
            context.questions.len(),
            3,
            "missing goals, constraints and decisions must be explicit"
        );
        assert!(
            context
                .questions
                .iter()
                .any(|question| question.contains("trade-offs"))
        );
        assert_eq!(context.observed.summary.status, Status::Pass);
    }

    #[test]
    fn declared_tradeoffs_remain_data_and_derived_layers_are_not_intent() {
        use archguard_core::{
            ArchitectureConstraint, ArchitectureDecision, ArchitectureGoal, LayerConfig, Node,
            NodeId,
        };
        let mut config = Config::default();
        config.architecture.goals = vec![ArchitectureGoal {
            name: "Availability".into(),
            priority: "above cost".into(),
            rationale: "Customers need continuous access".into(),
        }];
        config.architecture.constraints = vec![ArchitectureConstraint {
            name: "Data location".into(),
            rationale: "Contract requirement".into(),
        }];
        config.architecture.decisions = vec![ArchitectureDecision {
            id: "ADR-1".into(),
            title: "Storage deployment".into(),
            status: "accepted by platform team".into(),
            chosen_option: "Managed store".into(),
            alternatives: vec!["Self hosted store".into()],
            trade_offs: vec!["Higher operating cost".into()],
            links: vec!["untrusted://do-not-fetch".into()],
        }];
        config.layers = vec![
            LayerConfig {
                name: "declared".into(),
                origin: LayerOrigin::Declared,
                ..Default::default()
            },
            LayerConfig {
                name: "derived".into(),
                origin: LayerOrigin::Derived,
                ..Default::default()
            },
        ];
        let graph = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/nonexistent".into(),
            nodes: [
                ("file.ts", NodeKind::File),
                ("module", NodeKind::Module),
                ("external/*", NodeKind::File),
            ]
            .into_iter()
            .enumerate()
            .map(|(id, (name, kind))| Node {
                id: NodeId(id as u32),
                name: name.into(),
                kind,
                parent: None,
                lang: None,
            })
            .collect(),
            edges: vec![],
        };
        let context = architecture_context(Path::new("/nonexistent"), &config, &graph, &report());
        assert_eq!(context.intent, config.architecture);
        assert!(context.questions.is_empty());
        assert_eq!(context.observed.source_files, 1);
        assert_eq!(context.observed.declared_layers.len(), 1);
        assert_eq!(context.observed.declared_layers[0].name, "declared");
        assert!(
            context
                .limitations
                .iter()
                .any(|limitation| limitation.contains("not fetched evidence"))
        );
        config.architecture.decisions[0].trade_offs.clear();
        let context = architecture_context(Path::new("/nonexistent"), &config, &graph, &report());
        assert_eq!(context.questions.len(), 1);
        assert!(context.questions[0].contains("ADR-1"));
    }

    #[test]
    fn architecture_config_rejects_ambiguous_identifiers_and_preserves_freeform_vocabulary() {
        let dir = tempfile::tempdir().unwrap();
        let valid = r#"[[architecture.goals]]
name="Latency"
priority="P0 for checkout"
rationale="Business constraint"
[[architecture.constraints]]
name="Legacy compatibility"
rationale="Customer API"
[[architecture.decisions]]
id="ADR-1"
title="Deployment boundary"
status="accepted after trial"
chosen_option="Independent deployment"
alternatives=["Single deployment"]
trade_offs=["Extra operational work"]
links=["docs/adr-1.md"]
"#;
        std::fs::write(dir.path().join("archguard.toml"), valid).unwrap();
        let loaded = crate::config::load_config(None, dir.path()).unwrap();
        assert_eq!(
            loaded.config.architecture.goals[0].priority,
            "P0 for checkout"
        );
        assert_eq!(
            loaded.config.architecture.decisions[0].status,
            "accepted after trial"
        );
        for invalid in [
            valid.replace("name=\"Latency\"", "name=\"\""),
            valid.replace("id=\"ADR-1\"", "id=\"\""),
            valid.replace("title=\"Deployment boundary\"", "title=\"\""),
            format!("{valid}\n[[architecture.goals]]\nname='Latency'"),
            format!("{valid}\n[[architecture.constraints]]\nname='Legacy compatibility'"),
            format!("{valid}\n[[architecture.decisions]]\nid='ADR-1'\ntitle='duplicate'"),
            valid.replace("trade_offs=", "tradeoffs="),
        ] {
            std::fs::write(dir.path().join("archguard.toml"), invalid).unwrap();
            assert!(crate::config::load_config(None, dir.path()).is_err());
        }
    }
}
