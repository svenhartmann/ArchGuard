//! The shareable single-file report (desktop File → Export → Report…,
//! `archguard report`): one Markdown file a colleague reads without
//! ArchGuard. Diagrams are Mermaid blocks (rendered by GitHub and GitLab)
//! with the terminal drawing of [`crate::ascii`] in a collapsed section as
//! fallback. Markdown, not HTML: the workspace has no layout-and-SVG
//! renderer for catalogue diagrams. A static snapshot: no interactivity,
//! no links back into ArchGuard.

use std::fmt::Write as _;

use archguard_core::{
    Diagram, DiagramFormat, Finding, FindingKind, Report, ReviewReport, ServiceTopology,
};
use serde::Serialize;

use crate::arc42_diagrams::section;
use crate::ascii::{self, Style};

/// Everything the report shows, gathered by the engine from one analysis.
pub struct ShareReport<'a> {
    pub project: &'a str,
    pub revision: Option<&'a str>,
    /// When the report was made (ISO 8601).
    pub date: &'a str,
    pub scope: &'a str,
    pub report: &'a Report,
    pub review: Option<&'a ReviewReport>,
    pub services: Option<&'a ServiceTopology>,
    /// Summaries of imported scans (quality, security): heading and lines.
    pub scans: &'a [(String, Vec<String>)],
    /// The arc42 catalogue; only the shown diagrams are written.
    pub diagrams: &'a [Diagram],
}

/// Review priorities listed in the report.
const TOP_REVIEW: usize = 10;

/// The report as Markdown.
pub fn render_share_report(r: &ShareReport<'_>) -> String {
    let mut out = header(r);
    out += &health(r.report);
    out += &gate_findings(&r.report.findings);
    out += &cycles(&r.report.findings);
    out += &review(r.review);
    out += &services(r.services);
    for (title, lines) in r.scans {
        out += &format!("## {title}\n\n{}\n\n", bullets(lines));
    }
    out += &diagrams(r.diagrams);
    out
}

fn header(r: &ShareReport<'_>) -> String {
    let rows = [
        ("Project", r.project),
        ("Revision", r.revision.unwrap_or("not a git repository")),
        ("Date", r.date),
        ("Scope", r.scope),
    ];
    let mut out = format!("# ArchGuard report: {}\n\n", r.project);
    out += &table(
        &["Item", "Value"],
        rows.iter().map(|(k, v)| vec![k.to_string(), v.to_string()]),
    );
    out += "\nA static snapshot of one analysis: no interactivity and no links back into \
            ArchGuard. Diagrams are Mermaid blocks; each has a text drawing in a collapsed \
            section for viewers without Mermaid.\n\n";
    out
}

fn health(report: &Report) -> String {
    let s = &report.summary;
    let mut lines = vec![
        format!("Gate status: **{}**", name(&s.status)),
        format!("{} modules, {} dependencies", s.nodes, s.edges),
        format!("{} cycles, largest {} modules", s.sccs, s.largest_scc),
        format!("{} layer violations", s.layer_violations),
        format!("ACD {:.2}, NCCD {:.2}", s.acd, s.nccd),
    ];
    if let Some(k) = &report.review_kpis {
        if let Some(score) = k.health.as_ref().and_then(|h| h.score) {
            let level = k.health.as_ref().and_then(|h| h.level.as_ref());
            lines.push(format!("Health index {score:.0}/100 ({})", name(&level)));
        }
        if let Some(t) = &k.tangledness {
            lines.push(format!(
                "Tangledness {:.0} % ({} of {} modules)",
                t.quote * 100.0,
                t.tangled_nodes,
                t.total
            ));
        }
        if let Some(h) = &k.hub_gini {
            lines.push(format!("Hub concentration (Gini) {:.2}", h.gini));
        }
        lines.push(format!(
            "{} stable-dependency violations",
            k.sdp_violations.len()
        ));
        if let Some(z) = &k.zones {
            lines.push(format!(
                "Zone of pain {}, zone of uselessness {}, main sequence {}",
                z.zone_of_pain, z.zone_of_uselessness, z.main_sequence
            ));
        }
        if let Some(per_kloc) = k.debt.as_ref().and_then(|d| d.per_kloc) {
            lines.push(format!("Debt markers {per_kloc:.1} per 1000 lines"));
        }
    }
    format!("## Health and KPIs\n\n{}\n\n", bullets(&lines))
}

fn gate_findings(findings: &[Finding]) -> String {
    let rows: Vec<Vec<String>> = findings
        .iter()
        .filter(|f| f.kind != FindingKind::Cycle)
        .map(|f| {
            let what = f.message.clone().unwrap_or_else(|| f.nodes.join(", "));
            vec![name(&f.severity), name(&f.kind), f.id.clone(), what]
        })
        .collect();
    if rows.is_empty() {
        return "## Gate findings\n\nNo findings besides cycles.\n\n".into();
    }
    let header = ["Severity", "Kind", "Id", "Finding"];
    format!("## Gate findings\n\n{}\n", table(&header, rows))
}

fn cycles(findings: &[Finding]) -> String {
    let mut out = "## Cycles and suggested cuts\n\n".to_string();
    let cycles: Vec<_> = findings
        .iter()
        .filter(|f| f.kind == FindingKind::Cycle)
        .collect();
    if cycles.is_empty() {
        return out + "No dependency cycles.\n\n";
    }
    for f in cycles {
        let _ = write!(
            out,
            "### {}\n\n{} modules, {} dependencies: {}\n\n",
            f.id,
            f.nodes.len(),
            f.edge_count,
            f.nodes
                .iter()
                .map(|n| format!("`{n}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let cuts = f.suggested_cuts.iter().map(|c| {
            vec![
                format!("`{}` → `{}`", c.from, c.to),
                format!("{:.2}", c.score),
                c.reason.clone(),
            ]
        });
        if !f.suggested_cuts.is_empty() {
            out += &table(&["Suggested cut", "Score", "Reason"], cuts);
            out += "\n";
        }
    }
    out
}

fn review(review: Option<&ReviewReport>) -> String {
    let mut out = "## Review priorities\n\n".to_string();
    let Some(r) = review else {
        return out + "Not available for this project.\n\n";
    };
    out += "The score is a heuristic for where to look first, not a defect probability.\n\n";
    let rows = r.nodes.iter().take(TOP_REVIEW).map(|n| {
        vec![
            n.rank.to_string(),
            n.score.to_string(),
            name(&n.priority),
            format!("`{}`", n.name),
            n.reasons.join("; "),
        ]
    });
    out += &table(&["#", "Score", "Priority", "Module", "Reasons"], rows);
    for note in r.git.message.iter().chain(&r.limitations) {
        let _ = writeln!(out, "\n> {note}");
    }
    out + "\n"
}

fn services(topology: Option<&ServiceTopology>) -> String {
    let mut out = "## Services\n\n".to_string();
    let Some(t) = topology.filter(|t| !t.services.is_empty() || !t.projects.is_empty()) else {
        return out + "No services or deployable projects found.\n\n";
    };
    let _ = writeln!(
        out,
        "{} services, {} projects, {} dependencies, {} service cycles.\n",
        t.services.len(),
        t.projects.len(),
        t.dependencies.len(),
        t.cycles.len()
    );
    let rows = t.services.iter().map(|s| {
        vec![
            format!("`{}`", s.name),
            s.compose_file.clone(),
            s.networks.join(", "),
        ]
    });
    out += &table(&["Service", "Defined in", "Networks"], rows);
    if !t.dependencies.is_empty() {
        let deps = t
            .dependencies
            .iter()
            .map(|d| vec![format!("`{}` → `{}`", d.from, d.to), name(&d.kind)]);
        out += "\n";
        out += &table(&["Dependency", "Kind"], deps);
    }
    out + "\n"
}

fn diagrams(all: &[Diagram]) -> String {
    let mut out = "## Diagrams\n\n".to_string();
    let shown: Vec<_> = all.iter().filter(|d| d.shown).collect();
    if shown.is_empty() {
        return out + "No diagram of the arc42 catalogue is shown.\n\n";
    }
    for d in shown {
        out += &section(d, DiagramFormat::Mermaid);
        let drawing = ascii::draw(d, Style::default());
        let _ = write!(
            out,
            "<details>\n<summary>Text drawing</summary>\n\n```text\n{}\n```\n\n</details>\n\n",
            drawing.trim_end()
        );
    }
    out
}

/// A Markdown table; cells are escaped.
fn table(header: &[&str], rows: impl IntoIterator<Item = Vec<String>>) -> String {
    let line = |cells: Vec<String>| format!("| {} |\n", cells.join(" | "));
    let mut out = line(header.iter().map(|h| h.to_string()).collect());
    out += &line(header.iter().map(|_| "---".to_string()).collect());
    for row in rows {
        out += &line(row.iter().map(|c| cell(c)).collect());
    }
    out
}

/// `text` safe inside a table cell: no pipes, no line breaks.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn bullets(lines: &[String]) -> String {
    lines
        .iter()
        .map(|l| format!("- {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The serde name of an enum value (`"error"`, `"layer_violation"`).
fn name(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(serde_json::Value::Null) => "unknown".into(),
        Ok(other) => other.to_string(),
        Err(_) => "unknown".into(),
    }
}

#[cfg(test)]
mod tests {
    use archguard_core::{Diagram, Report};
    use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Parser, Tag};
    use serde_json::json;

    use super::*;

    fn report() -> Report {
        serde_json::from_value(json!({
            "schema_version": 1,
            "summary": {"nodes": 3, "edges": 3, "sccs": 1, "largest_scc": 2,
                        "layer_violations": 1, "status": "fail"},
            "findings": [
                {"kind": "cycle", "id": "cycle-1", "severity": "error",
                 "nodes": ["app/a", "app/b"], "edge_count": 2, "cycles_sampled": 1,
                 "suggested_cuts": [{"from": "app/b", "to": "app/a", "score": 0.9,
                                     "reason": "one import | weak"}]},
                {"kind": "layer_violation", "id": "layer-1", "severity": "error",
                 "message": "domain imports\nweb", "nodes": ["domain", "web"], "edge_count": 1, "cycles_sampled": 0, "suggested_cuts": []}
            ],
            "metrics": {}
        }))
        .expect("report")
    }

    fn diagram(id: &str, shown: bool) -> Diagram {
        serde_json::from_value(json!({
            "id": id, "chapter": 5, "title": format!("Diagram {id}"), "kind": "packages",
            "note": "From the imports.", "unit": "source connections", "shown": shown,
            "elements": [{"id": "a", "name": "a", "kind": "package"},
                         {"id": "b", "name": "b", "kind": "package"}],
            "relations": [{"from": "a", "to": "b", "kind": "dependency", "count": 2}]
        }))
        .expect("diagram")
    }

    fn render(diagrams: &[Diagram]) -> String {
        let report = report();
        render_share_report(&ShareReport {
            project: "demo",
            revision: Some("abc1234"),
            date: "2026-11-19T10:00:00Z",
            scope: "modules at depth 2",
            report: &report,
            review: None,
            services: None,
            scans: &[("Security".into(), vec!["2 vulnerabilities".into()])],
            diagrams,
        })
    }

    /// The second-level headings and the languages of the code blocks, as a
    /// Markdown parser reads them.
    fn outline(markdown: &str) -> (Vec<String>, Vec<String>) {
        let (mut headings, mut blocks) = (vec![], vec![]);
        let mut in_h2 = false;
        for event in Parser::new(markdown) {
            match event {
                Event::Start(Tag::Heading {
                    level: HeadingLevel::H2,
                    ..
                }) => in_h2 = true,
                Event::Text(t) if in_h2 => {
                    headings.push(t.to_string());
                    in_h2 = false;
                }
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                    blocks.push(lang.to_string())
                }
                _ => {}
            }
        }
        (headings, blocks)
    }

    #[test]
    fn every_section_is_a_heading_of_the_parsed_markdown() {
        let text = render(&[diagram("one", true)]);
        let (headings, _) = outline(&text);
        assert_eq!(
            headings,
            [
                "Health and KPIs",
                "Gate findings",
                "Cycles and suggested cuts",
                "Review priorities",
                "Services",
                "Security",
                "Diagrams"
            ],
            "{text}"
        );
        assert!(text.starts_with("# ArchGuard report: demo\n"), "{text}");
        for fact in [
            "abc1234",
            "2026-11-19T10:00:00Z",
            "Gate status: **fail**",
            "`app/b` → `app/a`",
            "layer-1",
        ] {
            assert!(text.contains(fact), "{fact}: {text}");
        }
        assert!(!text.contains("<script"), "no script");
    }

    /// Only shown diagrams, each as Mermaid with a text drawing in a
    /// collapsed section.
    #[test]
    fn shown_diagrams_are_mermaid_with_a_text_fallback() {
        let text = render(&[diagram("one", true), diagram("two", false)]);
        let (_, blocks) = outline(&text);
        assert_eq!(blocks, ["mermaid", "text"], "{text}");
        assert!(
            text.contains("### Diagram one") && !text.contains("Diagram two"),
            "{text}"
        );
        assert!(
            text.contains("<details>\n<summary>Text drawing</summary>"),
            "{text}"
        );
        assert!(render(&[]).contains("No diagram of the arc42 catalogue is shown."));
    }

    /// A pipe or line break in a cell keeps the table intact.
    #[test]
    fn table_cells_are_escaped() {
        let text = render(&[]);
        assert!(text.contains("one import \\| weak"), "{text}");
        assert!(text.contains("domain imports web"), "{text}");
    }
}
