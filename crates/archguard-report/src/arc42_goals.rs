//! arc42 chapters 1 and 10: declared quality goals and their assessment
//! (docs/adr/0003-quality-goal-assessment.md). The escape function comes
//! from `arc42.rs`, which calls this module; importing it back would close
//! a file cycle.

use archguard_core::{ArchitectureGoal, Config, GoalAssessment, GoalStatus};
use std::fmt::Write;

type Cell = fn(&str) -> String;

const FINDING_IDS: usize = 30;

fn status_de(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Unclassified => "Kein Merkmal angegeben",
        GoalStatus::NotAssessable => "Nicht bewertbar",
        GoalStatus::NoEvidence => "Keine Belege",
        GoalStatus::Findings => "Befunde",
        GoalStatus::NoFindings => "Keine Befunde aus den verfügbaren Quellen",
    }
}

fn attribute_de(goal: &ArchitectureGoal) -> &'static str {
    goal.attribute.map_or("nicht angegeben", |a| a.as_str())
}

fn scope_de(goal: &ArchitectureGoal, cell: Cell) -> String {
    if goal.scope.is_empty() {
        "Gesamtsystem".into()
    } else {
        cell(&goal.scope.join(", "))
    }
}

fn assessment<'a>(goals: &'a [GoalAssessment], name: &str) -> Option<&'a GoalAssessment> {
    goals.iter().find(|a| a.goal == name)
}

/// Chapter 1 rows: goal, priority, rationale, characteristic, scope, status.
pub(crate) fn goal_rows(
    config: &Config,
    goals: &[GoalAssessment],
    limit: usize,
    cell: Cell,
) -> Vec<String> {
    config
        .architecture
        .goals
        .iter()
        .take(limit)
        .map(|g| {
            format!(
                "| {} | {} | {} | {} | {} | {} |",
                cell(&g.name),
                cell(&g.priority),
                cell(&g.rationale),
                attribute_de(g),
                scope_de(g, cell),
                assessment(goals, &g.name).map_or("unbekannt", |a| status_de(a.status))
            )
        })
        .collect()
}

/// Chapter 10: one block per goal with scenario, linked findings and evidence.
pub(crate) fn assessment_section(
    config: &Config,
    goals: &[GoalAssessment],
    limit: usize,
    cell: Cell,
) -> String {
    let mut out = String::from(
        "### Bewertung der Qualitätsziele\n\nQuelle: `architecture.goals` mit Merkmal nach ISO/IEC 25010 und feste Zuordnung von Befunden und Belegquellen (ADR 0003). Der Status beschreibt die verfügbaren Belege, nicht die Erfüllung eines Ziels. Gründe und Lücken je Ziel: MCP `architecture_context` (`goal_assessment`).\n\n",
    );
    if config.architecture.goals.is_empty() {
        out.push_str("Keine Qualitätsziele deklariert.\n\n");
    }
    for goal in config.architecture.goals.iter().take(limit) {
        let _ = writeln!(
            out,
            "#### {}\n\n- Merkmal: {} · Umfang: {} · Priorität: {}",
            cell(&goal.name),
            attribute_de(goal),
            scope_de(goal, cell),
            if goal.priority.trim().is_empty() {
                "nicht angegeben".into()
            } else {
                cell(&goal.priority)
            }
        );
        let scenario = if goal.scenario.trim().is_empty() {
            "nicht angegeben".into()
        } else {
            cell(&goal.scenario)
        };
        let _ = writeln!(out, "- Szenario: {scenario}");
        if let Some(a) = assessment(goals, &goal.name) {
            evidence_lines(&mut out, a);
        }
        out.push('\n');
    }
    out
}

/// Chapter 1 questions for goals without a characteristic.
pub(crate) fn goal_questions(config: &Config, cell: Cell) -> Vec<String> {
    config
        .architecture
        .goals
        .iter()
        .filter(|g| g.attribute.is_none())
        .map(|g| {
            format!(
                "Merkmal nach ISO/IEC 25010 für das Qualitätsziel „{}“ angeben (`attribute`).",
                cell(&g.name)
            )
        })
        .collect()
}

fn evidence_lines(out: &mut String, a: &GoalAssessment) {
    let _ = writeln!(out, "- Status: **{}**", status_de(a.status));
    if !a.findings.is_empty() {
        let ids: Vec<_> = a
            .findings
            .iter()
            .take(FINDING_IDS)
            .map(|id| format!("`{}`", id.replace('`', "'")))
            .collect();
        let more = a.findings.len().saturating_sub(FINDING_IDS);
        let rest = if more > 0 {
            format!(" und {more} weitere")
        } else {
            String::new()
        };
        let by_severity: Vec<_> = a
            .finding_count_by_severity
            .iter()
            .map(|(severity, n)| format!("{severity} {n}"))
            .collect();
        let _ = writeln!(
            out,
            "- Befunde ({}; {}): {}{rest}",
            a.findings.len(),
            by_severity.join(", "),
            ids.join(", ")
        );
    }
    if !a.kpis.is_empty() {
        let kpis: Vec<_> = a
            .kpis
            .iter()
            .map(|k| format!("{} {}", k.name, k.value))
            .collect();
        let _ = writeln!(out, "- Kennzahlen (projektweit): {}", kpis.join(", "));
    }
    if !a.evidence_sources.is_empty() {
        let sources: Vec<_> = a
            .evidence_sources
            .iter()
            .map(|s| {
                let state = if s.present { "vorhanden" } else { "fehlt" };
                format!("`{}` {state}", s.source.as_str())
            })
            .collect();
        let _ = writeln!(out, "- Belegquellen: {}", sources.join(", "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{EvidenceSource, GoalEvidenceSource, QualityAttribute};

    fn cell(text: &str) -> String {
        text.replace('*', "\\*")
    }

    #[test]
    fn chapter_ten_links_findings_and_names_missing_scenarios_and_sources() {
        let mut config = Config::default();
        config.architecture.goals = vec![
            ArchitectureGoal {
                name: "Changeable".into(),
                attribute: Some(QualityAttribute::Maintainability),
                scope: vec!["app/**".into()],
                scenario: "A price rule ships in one day".into(),
                ..Default::default()
            },
            ArchitectureGoal {
                name: "Fast delivery".into(),
                ..Default::default()
            },
        ];
        let assessment = GoalAssessment {
            goal: "Changeable".into(),
            priority: String::new(),
            attribute: Some(QualityAttribute::Maintainability),
            scope: vec!["app/**".into()],
            status: GoalStatus::Findings,
            findings: (0..35).map(|i| format!("scc-{i}")).collect(),
            finding_count_by_severity: [("error".to_string(), 35)].into(),
            kpis: vec![],
            evidence_sources: vec![GoalEvidenceSource {
                source: EvidenceSource::ReviewKpis,
                present: false,
            }],
            gaps: vec![],
        };
        let section = assessment_section(&config, &[assessment], 30, cell);
        assert!(section.contains("- Szenario: A price rule ships in one day"));
        assert!(section.contains("- Befunde (35; error 35): `scc-0`"));
        assert!(section.contains("`scc-29` und 5 weitere"));
        assert!(!section.contains("`scc-30`"));
        assert!(section.contains("`review_kpis` fehlt"));
        assert!(section.contains("Umfang: app/\\*\\*"));
        assert!(section.contains("#### Fast delivery\n\n- Merkmal: nicht angegeben · Umfang: Gesamtsystem · Priorität: nicht angegeben\n- Szenario: nicht angegeben\n"));
        assert_eq!(
            goal_questions(&config, cell),
            vec![
                "Merkmal nach ISO/IEC 25010 für das Qualitätsziel „Fast delivery“ angeben (`attribute`)."
            ]
        );
        let rows = goal_rows(&config, &[], 30, cell);
        assert!(rows[1].ends_with("| nicht angegeben | Gesamtsystem | unbekannt |"));
    }
}
