//! Deterministic architecture documentation from the supplied analysis scope.
use archguard_core::{
    Config, FindingKind, GoalAssessment, GoalStatus, Graph, LayerOrigin, Report, ReviewReport,
    Severity, Status,
};
use std::collections::BTreeMap;
use std::fmt::Write;

/// Reproducible from the supplied inputs. The DSM is limited to 30 named
/// nodes, finding sections to 50 items, and optional review rows to ten.
/// Source/config text is escaped as data rather than executable Markdown/HTML.
#[tracing::instrument(skip_all)]
pub fn render_architecture_doc(
    view: &Graph,
    report: &Report,
    config: &Config,
    review: Option<&ReviewReport>,
    goals: &[GoalAssessment],
) -> String {
    let mut out = String::from("# Architecture\n\n");
    let _ = writeln!(
        out,
        "Generated from static dependency analysis. Scope and configuration determine these observations; runtime behavior and organization priorities require separate evidence.\n"
    );
    goal_section(&mut out, config, goals);
    out.push_str("\n### Constraints\n\n");
    if config.architecture.constraints.is_empty() {
        out.push_str("No declared constraints.\n");
    }
    for constraint in &config.architecture.constraints {
        let _ = writeln!(
            out,
            "- **{}** — {}",
            md(&constraint.name),
            specified(&constraint.rationale)
        );
    }
    out.push_str("\n### Decisions\n\n");
    if config.architecture.decisions.is_empty() {
        out.push_str("No recorded decisions.\n");
    }
    for decision in &config.architecture.decisions {
        let _ = writeln!(
            out,
            "- **{}: {}** — status: {}\n  - Chosen option: {}\n  - Alternatives: {}\n  - Accepted trade-offs: {}",
            md(&decision.id),
            md(&decision.title),
            specified(&decision.status),
            specified(&decision.chosen_option),
            list(&decision.alternatives),
            list(&decision.trade_offs)
        );
        if !decision.links.is_empty() {
            let _ = writeln!(out, "  - Unverified references: {}", list(&decision.links));
        }
    }
    if let Some(model) = &config.architecture.model {
        let _ = writeln!(
            out,
            "\n### Logical target model: {}\n\nGate severity: {}. Unlisted cross-component directions are forbidden; dependencies within a component are allowed. Assignment is {}.\n",
            md(&model.name),
            severity(model.severity),
            if model.require_assignment {
                "required"
            } else {
                "optional; unmatched sources still limit coverage"
            }
        );
        for component in &model.components {
            let _ = writeln!(
                out,
                "- Component **{}** maps source names matching {}.",
                md(&component.name),
                list(&component.include)
            );
            if let Some(public) = &component.public {
                let _ = writeln!(
                    out,
                    "  Public interface: {}. Internal component access remains allowed.",
                    if public.is_empty() {
                        "closed to other components".into()
                    } else {
                        list(public)
                    }
                );
            }
            if let Some(limit) = component.max_incoming_components {
                let _ = writeln!(out, "  Incoming component limit: {limit}.");
            }
            if let Some(limit) = component.max_outgoing_components {
                let _ = writeln!(out, "  Outgoing component limit: {limit}.");
            }
        }
        if model.allowed_dependencies.is_empty() {
            out.push_str("\nNo cross-component directions are allowed.\n");
        }
        for direction in &model.allowed_dependencies {
            let _ = writeln!(
                out,
                "- Allowed: {} → {}.",
                md(&direction.from),
                md(&direction.to)
            );
        }
        out.push_str("\nThese are declared boundaries. The architecture command reports source mapping coverage and observed logical dependencies independently of display depth.\n");
    }
    let summary = &report.summary;
    let _ = writeln!(
        out,
        "\n## Dependency scope\n\nGate status: **{}**. A pass applies to the configured dependency checks, not to every business constraint.\n\n| Metric | Value |\n| --- | ---: |\n| Scope nodes | {} |\n| Dependencies | {} |\n| Cyclic groups | {} |\n| Largest cyclic group | {} |\n| CCD | {} |\n| ACD | {:.3} |\n| NCCD | {:.3} |\n",
        match summary.status {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
        },
        summary.nodes,
        summary.edges,
        summary.sccs,
        summary.largest_scc,
        summary.ccd,
        summary.acd,
        summary.nccd
    );
    if let Some(coverage) = &report.semantic {
        let _ = writeln!(
            out,
            "Semantic dependency coverage: **{}**. Original source graph: {} resolved call sites, {} inheritance references, {} unmapped project calls.\n",
            md(&coverage.status),
            coverage.projected_calls,
            coverage.projected_supertypes,
            coverage.skipped_project_calls
        );
        out.push_str("Includes imports, compiler-resolved calls, inheritance and available declaration types; runtime wiring is not covered. No detected cycles is not proof of an acyclic codebase.\n\n");
    } else {
        out.push_str("Semantic coverage was not recorded for this graph. No detected cycles is not proof of an acyclic codebase.\n\n");
    }
    out.push_str("CCD sums distinct transitively reachable nodes including each source itself. ACD is CCD divided by scope nodes. NCCD compares CCD with a balanced-tree reference; none of these values measures runtime latency.\n\n## Layers and prohibitions\n\n");
    if config.layers.is_empty() {
        out.push_str("No configured layers.\n");
    }
    for layer in &config.layers {
        let _ = writeln!(
            out,
            "- Layer **{}** ({}): contains {}; may depend on {}.",
            md(&layer.name),
            if layer.origin == LayerOrigin::Declared {
                "declared"
            } else {
                "derived hypothesis"
            },
            list(&layer.contains),
            if layer.may_depend_on.is_empty() {
                "no other layer".into()
            } else {
                list(&layer.may_depend_on)
            }
        );
    }
    for rule in &config.dependency_rules {
        let _ = writeln!(
            out,
            "- Forbidden dependencies **{}** ({}): {} → {}. {}",
            md(&rule.name),
            severity(rule.severity),
            list(&rule.from),
            list(&rule.to),
            specified(&rule.message)
        );
    }
    for rule in &config.ownership_rules {
        let _ = writeln!(
            out,
            "- Forbidden owner direction ({}): {} → {}. {}",
            severity(rule.severity),
            md(&rule.from),
            md(&rule.to),
            specified(&rule.message)
        );
    }

    out.push_str("\n## Dependency structure matrix\n\nRows depend on columns. Counts are dependency references; `≥` marks a sample lower bound, `·` means no edge.\n\n");
    let mut nodes: Vec<_> = view.nodes.iter().collect();
    nodes.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.id.cmp(&right.id))
    });
    let total = nodes.len();
    nodes.truncate(30);
    let mut weights = BTreeMap::new();
    for edge in &view.edges {
        let entry = weights.entry((edge.from, edge.to)).or_insert((0u64, true));
        entry.0 = entry.0.saturating_add(edge.weight());
        entry.1 &= edge.count.is_some();
    }
    if nodes.is_empty() {
        out.push_str("No nodes in this scope.\n");
    } else {
        out.push_str("| Module |");
        for index in 1..=nodes.len() {
            let _ = write!(out, " {index} |");
        }
        out.push_str("\n| --- |");
        for _ in &nodes {
            out.push_str(" ---: |");
        }
        out.push('\n');
        for (index, node) in nodes.iter().enumerate() {
            let _ = write!(out, "| {}. {} |", index + 1, md(&node.name));
            for target in &nodes {
                if let Some((count, exact)) = weights.get(&(node.id, target.id)) {
                    let _ = write!(out, " {}{count} |", if *exact { "" } else { "≥" });
                } else {
                    out.push_str(" · |");
                }
            }
            out.push('\n');
        }
    }
    if total > nodes.len() {
        let _ = writeln!(
            out,
            "\nMatrix truncated: {} of {total} nodes shown in name order. Metrics and findings still describe the full analysis scope.",
            nodes.len()
        );
    }
    out.push_str("\n## Reported cycles\n\n");
    let mut cycles: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.kind == FindingKind::Cycle)
        .collect();
    cycles.sort_by(|left, right| left.id.cmp(&right.id));
    if cycles.is_empty() {
        out.push_str("No cycle findings in the supplied report.\n");
    }
    for finding in cycles.iter().take(50) {
        let _ = writeln!(
            out,
            "- **{}** ({}): {}; {} directed edges.",
            md(&finding.id),
            severity(finding.severity),
            bounded_names(&finding.nodes),
            finding.edge_count
        );
    }
    if cycles.len() > 50 {
        let _ = writeln!(
            out,
            "\nCycle list truncated: 50 of {} findings shown.",
            cycles.len()
        );
    }
    out.push_str("\n## Other findings\n\n");
    let mut findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.kind != FindingKind::Cycle)
        .collect();
    findings.sort_by(|left, right| left.id.cmp(&right.id));
    if findings.is_empty() {
        out.push_str("No other findings in the supplied report.\n");
    }
    for finding in findings.iter().take(50) {
        let _ = writeln!(
            out,
            "- **{}** ({}): {}. {}",
            md(&finding.id),
            severity(finding.severity),
            bounded_names(&finding.nodes),
            finding.message.as_deref().map(md).unwrap_or_default()
        );
        for evidence in finding.evidence.iter().take(3) {
            let _ = writeln!(
                out,
                "  - Evidence: {}:{} — {}",
                md(&evidence.file),
                evidence.line,
                md(&evidence.raw)
            );
        }
    }
    if findings.len() > 50 {
        let _ = writeln!(
            out,
            "\nFinding list truncated: 50 of {} findings shown.",
            findings.len()
        );
    }
    if let Some(review) = review {
        out.push_str("\n## Review priorities\n\nThese scores order manual review; they are not defect probabilities or measured performance. Git-based values describe the supplied history window.\n\n| Rank | Module | Score | Reasons |\n| ---: | --- | ---: | --- |\n");
        for node in review.nodes.iter().take(10) {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} |",
                node.rank,
                md(&node.name),
                node.score,
                list(&node.reasons)
            );
        }
    }
    out.push_str("\nRegenerate this document from the same source and configuration to compare it in CI. No generation timestamp is embedded.\n");
    out
}

/// Declared goals with characteristic, scope and assessment status (ADR 0003).
fn goal_section(out: &mut String, config: &Config, goals: &[GoalAssessment]) {
    let _ = writeln!(out, "## Declared intent\n\n### Quality goals\n");
    if config.architecture.goals.is_empty() {
        out.push_str("No declared quality goals.\n");
    }
    for goal in &config.architecture.goals {
        let scope = if goal.scope.is_empty() {
            "whole system".into()
        } else {
            list(&goal.scope)
        };
        let status = match goals.iter().find(|a| a.goal == goal.name) {
            None => "unknown".into(),
            Some(a) if a.findings.is_empty() => goal_status(a.status).to_string(),
            Some(a) => format!("{} ({})", goal_status(a.status), bounded_names(&a.findings)),
        };
        let _ = writeln!(
            out,
            "- **{}** — priority: {}; rationale: {}; characteristic: {}; scope: {scope}; status: {status}",
            md(&goal.name),
            specified(&goal.priority),
            specified(&goal.rationale),
            goal.attribute.map_or("unspecified", |a| a.as_str()),
        );
        if !goal.scenario.trim().is_empty() {
            let _ = writeln!(out, "  - Scenario: {}", md(&goal.scenario));
        }
    }
    if !goals.is_empty() {
        out.push_str("\nA status describes the available evidence (static analysis, review KPIs and saved reports under `.archguard/`), not whether a goal is reached; see ADR 0003 for the fixed mapping.\n");
    }
}
fn goal_status(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Unclassified => "unclassified",
        GoalStatus::NotAssessable => "not assessable by ArchGuard",
        GoalStatus::NoEvidence => "no evidence (needed source absent)",
        GoalStatus::Findings => "findings",
        GoalStatus::NoFindings => "no findings from the available sources",
    }
}
fn severity(value: Severity) -> &'static str {
    match value {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}
fn specified(value: &str) -> String {
    if value.trim().is_empty() {
        "unspecified".into()
    } else {
        md(value)
    }
}
fn list(values: &[String]) -> String {
    if values.is_empty() {
        "unspecified".into()
    } else {
        values
            .iter()
            .map(|value| md(value))
            .collect::<Vec<_>>()
            .join("; ")
    }
}
fn bounded_names(names: &[String]) -> String {
    let mut value = list(&names.iter().take(20).cloned().collect::<Vec<_>>());
    if names.len() > 20 {
        let _ = write!(value, "; … {} more", names.len() - 20);
    }
    value
}
pub(crate) fn md(value: &str) -> String {
    let mut out = String::new();
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' | '\n' | '\t' => out.push(' '),
            character if character.is_ascii_punctuation() => {
                out.push('\\');
                out.push(character);
            }
            character => out.push(character),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Edge, Node, NodeId, NodeKind, Status, Summary};
    fn report() -> Report {
        Report {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            summary: Summary {
                nodes: 2,
                edges: 1,
                sccs: 0,
                largest_scc: 0,
                layer_violations: 0,
                status: Status::Pass,
                ccd: 3,
                acd: 1.5,
                nccd: 1.0,
            },
            config: Default::default(),
            findings: vec![],
            metrics: Default::default(),
        }
    }
    fn graph() -> Graph {
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/repo".into(),
            nodes: [(0, "a"), (1, "b")]
                .into_iter()
                .map(|(id, name)| Node {
                    id: NodeId(id),
                    name: name.into(),
                    kind: NodeKind::Module,
                    parent: None,
                    lang: None,
                })
                .collect(),
            edges: vec![Edge {
                from: NodeId(0),
                to: NodeId(1),
                refs: vec![],
                count: Some(3),
            }],
        }
    }

    #[test]
    fn goals_show_characteristic_scope_status_and_linked_findings() {
        use archguard_core::{ArchitectureGoal, GoalAssessment, GoalStatus, QualityAttribute};
        let mut config = Config::default();
        let goal = |name: &str, attribute, scope: &[&str]| ArchitectureGoal {
            name: name.into(),
            attribute,
            scope: scope.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        config.architecture.goals = vec![
            goal(
                "Changeable",
                Some(QualityAttribute::Maintainability),
                &["app/**"],
            ),
            goal("Quiet", Some(QualityAttribute::Maintainability), &[]),
            goal("Fast delivery", None, &[]),
        ];
        let assessed = |goal: &ArchitectureGoal, status, findings: &[&str]| GoalAssessment {
            goal: goal.name.clone(),
            priority: String::new(),
            attribute: goal.attribute,
            scope: goal.scope.clone(),
            status,
            findings: findings.iter().map(|s| s.to_string()).collect(),
            finding_count_by_severity: Default::default(),
            kpis: vec![],
            evidence_sources: vec![],
            gaps: vec![],
        };
        let goals = &config.architecture.goals;
        let assessments = vec![
            assessed(&goals[0], GoalStatus::Findings, &["scc-1"]),
            assessed(&goals[1], GoalStatus::NoFindings, &[]),
            assessed(&goals[2], GoalStatus::Unclassified, &[]),
        ];
        let doc = render_architecture_doc(&graph(), &report(), &config, None, &assessments);
        assert!(
            doc.contains(
                "characteristic: maintainability; scope: app\\/\\*\\*; status: findings (scc\\-1)"
            ),
            "{doc}"
        );
        assert!(
            doc.contains("scope: whole system; status: no findings from the available sources")
        );
        assert!(
            doc.contains("characteristic: unspecified; scope: whole system; status: unclassified")
        );
        assert!(!doc.to_lowercase().contains("goal met"));
    }

    #[test]
    fn zero_cycle_document_explains_semantic_scope_and_partial_coverage() {
        let mut report = report();
        report.semantic = Some(archguard_core::SemanticDependencyCoverage {
            status: "partial".into(),
            projected_calls: 7,
            projected_supertypes: 3,
            skipped_project_calls: 2,
            ..Default::default()
        });
        let doc = render_architecture_doc(&graph(), &report, &Config::default(), None, &[]);
        assert!(doc.contains("## Dependency scope"));
        assert!(doc.contains("Semantic dependency coverage: **partial**"));
        assert!(doc.contains("7 resolved call sites"));
        assert!(doc.contains("3 inheritance references"));
        assert!(doc.contains("2 unmapped project calls"));
        assert!(doc.contains("No detected cycles is not proof of an acyclic codebase"));
        assert!(doc.contains("Counts are dependency references"));
        assert!(!doc.contains("static import analysis"));
    }
    #[test]
    fn documentation_includes_declared_logical_boundaries_and_escapes_names() {
        let mut config = Config::default();
        config.architecture.model = Some(archguard_core::TargetArchitectureConfig {
            name: "Logical <system>".into(),
            components: vec![
                archguard_core::ArchitectureComponent {
                    name: "UI".into(),
                    include: vec!["screen/**".into()],
                    public: None,
                    max_incoming_components: None,
                    max_outgoing_components: None,
                },
                archguard_core::ArchitectureComponent {
                    name: "Domain".into(),
                    include: vec!["core/**".into()],
                    public: Some(vec!["core/api/**".into()]),
                    max_incoming_components: Some(2),
                    max_outgoing_components: Some(0),
                },
            ],
            allowed_dependencies: vec![archguard_core::ArchitectureDirection {
                from: "UI".into(),
                to: "Domain".into(),
            }],
            ..Default::default()
        });
        let doc = render_architecture_doc(&graph(), &report(), &config, None, &[]);
        assert!(doc.contains("Logical &lt;system&gt;"));
        assert!(doc.contains("UI → Domain"));
        assert!(doc.contains("screen\\/\\*\\*"));
        assert!(doc.contains("Unlisted cross-component directions are forbidden"));
        assert!(doc.contains("Public interface"));
        assert!(doc.contains("core\\/api\\/\\*\\*"));
        assert!(doc.contains("Incoming component limit: 2"));
        assert!(doc.contains("Outgoing component limit: 0"));
    }

    #[test]
    fn documentation_contains_scope_metrics_and_directed_dsm() {
        let doc = render_architecture_doc(&graph(), &report(), &Config::default(), None, &[]);
        assert!(doc.starts_with("# Architecture\n"));
        assert!(doc.contains("| ACD | 1.500 |"));
        assert!(doc.contains("| 1. a | · | 3 |"));
        assert!(doc.contains("| 2. b | · | · |"));
        assert!(doc.contains("No declared quality goals."));
    }

    #[test]
    fn documentation_is_deterministic_and_escapes_untrusted_source_and_intent() {
        let mut graph = graph();
        let mut config = Config::default();
        graph.nodes[0].name = "<script>alert(1)</script>|[run](javascript:bad)".into();
        config
            .architecture
            .goals
            .push(archguard_core::ArchitectureGoal {
                name: "[run](javascript:bad)".into(),
                priority: "\n# forged".into(),
                rationale: "<img src=x onerror=bad>".into(),
                ..Default::default()
            });
        let first = render_architecture_doc(&graph, &report(), &config, None, &[]);
        graph.nodes.reverse();
        graph.edges.reverse();
        assert_eq!(
            render_architecture_doc(&graph, &report(), &config, None, &[]),
            first
        );
        assert!(!first.contains("<script>"));
        assert!(!first.contains("<img"));
        assert!(!first.contains("[run](javascript:bad)"));
        assert!(first.contains("&lt;script&gt;"));
        assert!(!first.contains("\n# forged"));
        assert!(first.contains("\\|"));
    }

    #[test]
    fn large_matrix_is_explicitly_bounded_without_hiding_full_scope_metrics() {
        let mut graph = graph();
        graph.nodes = (0..100)
            .map(|id| Node {
                id: NodeId(id),
                name: format!("module-{id:03}"),
                kind: NodeKind::Module,
                parent: None,
                lang: None,
            })
            .collect();
        graph.edges.clear();
        let mut report = report();
        report.summary.nodes = 100;
        let doc = render_architecture_doc(&graph, &report, &Config::default(), None, &[]);
        assert!(doc.contains("| Scope nodes | 100 |"));
        assert!(doc.contains("Matrix truncated: 30 of 100 nodes shown"));
        assert!(!doc.contains("module\\-030"));
        assert!(doc.lines().count() < 100);
    }

    #[test]
    fn legacy_import_counts_remain_lower_bounds_in_the_matrix() {
        let mut graph = graph();
        graph.edges[0].count = None;
        graph.edges[0].refs = vec![archguard_core::Ref {
            file: "a.ts".into(),
            line: 1,
            raw: "import b".into(),
        }];
        let doc = render_architecture_doc(&graph, &report(), &Config::default(), None, &[]);
        assert!(doc.contains("| 1. a | · | ≥1 |"));
    }
}
