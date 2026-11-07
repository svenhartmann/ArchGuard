//! The arc42 diagram catalogue (ADR 0006): every diagram the document can
//! show, with a stable id, and the shown ones written into their chapter
//! as Mermaid or PlantUML (`[arc42]` in `archguard.toml`).

mod cyclic;
mod render;
mod scoped;
mod sequences;
mod services;

use archguard_core::{Diagram, DiagramFormat, DiagramKind, DiagramRelation, ElementKind};

use crate::arc42_blocks::{Arc42Inputs, Blocks};

pub use render::{mermaid, plantuml};

/// The catalogue of `input`; shown diagrams are put at the start of
/// their chapter's text (`(number, markdown)`).
pub(crate) fn add<'c>(
    chapters: impl Iterator<Item = (u8, &'c mut String)>,
    input: &Arc42Inputs<'_>,
) -> Vec<Diagram> {
    let choice = &input.config.arc42;
    let mut diagrams = catalogue(input);
    for d in &mut diagrams {
        d.shown = choice.shows(&d.id);
    }
    for (number, markdown) in chapters {
        let text: String = diagrams
            .iter()
            .filter(|d| d.shown && d.chapter == number)
            .map(|d| section(d, choice.diagram_format))
            .collect();
        markdown.insert_str(0, &text);
    }
    diagrams
}

/// One diagram as a Markdown section: heading, note, fenced source.
pub fn section(d: &Diagram, format: DiagramFormat) -> String {
    let (lang, source) = match format {
        DiagramFormat::Mermaid => ("mermaid", mermaid(d)),
        DiagramFormat::Plantuml => ("plantuml", plantuml(d)),
    };
    format!(
        "### {}\n\n{}\n\n```{lang}\n{source}```\n\n",
        d.title, d.note
    )
}

/// All diagrams of the project, in chapter order.
fn catalogue(input: &Arc42Inputs<'_>) -> Vec<Diagram> {
    let blocks = Blocks::new(
        input.source_graph,
        input.config,
        input.services,
        input.source_paths,
        input.blocks,
    );
    let mut out = Vec::new();
    let elements = blocks.elements();
    if !elements.is_empty() {
        let relations = blocks.relations();
        out.push(packages(
            "building-blocks",
            5,
            "Paketübersicht",
            "Quelle: vollständiger Quellgraph; Pakete als Bausteine, höchstens neun Kästen. Pfeile: Quellverbindungen von der nutzenden zur genutzten Seite.",
            &elements,
            &relations,
        ));
        let calls = blocks.call_relations(input.source_graph, input.calls);
        if !calls.is_empty() {
            out.push(packages(
                "runtime-calls",
                6,
                "Statische Aufrufe zwischen Bausteinen",
                "Quelle: compileraufgelöste Aufrufstellen. Statisch erfasst; keine Laufzeitreihenfolge und keine Häufigkeit zur Laufzeit.",
                &elements,
                &calls,
            ));
        }
        out.extend(cycles(&elements, &relations));
    }
    out.extend(services::diagrams(input.services));
    out.extend(sequences::diagrams(input.services));
    out.extend(scoped::diagrams(input));
    out.sort_by_key(|d| d.chapter);
    out
}

fn packages(
    id: &str,
    chapter: u8,
    title: &str,
    note: &str,
    elements: &[archguard_core::DiagramElement],
    relations: &[DiagramRelation],
) -> Diagram {
    let mut d = Diagram {
        id: id.into(),
        chapter,
        title: title.into(),
        kind: DiagramKind::Packages,
        note: note.into(),
        unit: if relations
            .first()
            .is_some_and(|r| r.kind == archguard_core::RelationKind::Call)
        {
            "Aufrufstellen".into()
        } else {
            "Quellverbindungen".into()
        },
        elements: elements.to_vec(),
        relations: relations.to_vec(),
        shown: true,
    };
    cyclic::mark(&mut d);
    d
}

/// Chapter 11: the boxes and relations that lie on a cycle.
fn cycles(
    elements: &[archguard_core::DiagramElement],
    relations: &[DiagramRelation],
) -> Option<Diagram> {
    let mut d = packages(
        "cycles",
        11,
        "Zyklische Paketbeziehungen",
        "Quelle: Quellverbindungen zwischen den Kästen der Paketübersicht; gezeigt sind nur Beziehungen in Zyklen.",
        elements,
        relations,
    );
    d.relations.retain(|r| r.cycle);
    d.elements
        .retain(|e| e.cycle && e.kind != ElementKind::Remainder);
    (!d.relations.is_empty()).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arc42_blocks::Arc42Blocks;
    use archguard_core::{CallAnalysis, Config, Report, ServiceTopology};
    use std::collections::BTreeMap;

    /// p0 ↔ p1 (a cycle) and p1 → p2.
    /// The catalogue and the 12 chapter texts it opens.
    fn render(config: &Config) -> (Vec<Diagram>, Vec<String>) {
        let mut builder = archguard_core::GraphBuilder::new();
        let f: Vec<_> = (0..3)
            .map(|i| builder.ensure_path(&format!("org/demo/p{i}/S.java"), Some("java")))
            .collect();
        for (a, b) in [(0, 1), (1, 0), (1, 2)] {
            let r = archguard_core::Ref {
                file: "x".into(),
                line: 1,
                raw: "import".into(),
            };
            builder.edge(f[a], f[b], r).unwrap();
        }
        let graph = builder.build("test".into());
        let report: Report = serde_json::from_value(serde_json::json!({
            "schema_version":1,"summary":{"nodes":0,"edges":0,"sccs":0,"largest_scc":0,"layer_violations":0,"status":"pass"},
            "findings":[],"metrics":{}
        }))
        .unwrap();
        let mut chapters = vec![String::new(); 12];
        let text = chapters
            .iter_mut()
            .enumerate()
            .map(|(i, m)| (i as u8 + 1, m));
        let diagrams = add(
            text,
            &Arc42Inputs {
                source_graph: &graph,
                view: &graph,
                report: &report,
                config,
                calls: &CallAnalysis::default(),
                services: &ServiceTopology::default(),
                insights: &serde_json::json!({}),
                source_paths: &BTreeMap::new(),
                blocks: &Arc42Blocks::default(),
                goals: &[],
            },
        );
        (diagrams, chapters)
    }

    #[test]
    fn shown_diagrams_open_their_chapter_and_cycles_reach_chapter_11() {
        let (diagrams, chapters) = render(&Config::default());
        let ids: Vec<&str> = diagrams.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["building-blocks", "cycles"]);
        assert!(chapters[4].starts_with("### Paketübersicht\n"));
        let cycles = &diagrams[1];
        assert_eq!(cycles.relations.len(), 2, "only the cycle p0 ↔ p1");
        assert!(chapters[10].contains("```mermaid\nflowchart LR"));
        assert!(chapters[10].contains("class e"));
    }

    #[test]
    fn hidden_ids_stay_in_the_catalogue_but_leave_the_document() {
        let mut config = Config::default();
        config.arc42.hidden_diagrams = vec!["building-blocks".into(), "unknown".into()];
        let (diagrams, chapters) = render(&config);
        assert!(!diagrams[0].shown && diagrams[1].shown);
        assert!(!chapters[4].contains("```mermaid"));
        assert!(chapters[10].contains("```mermaid"));
    }

    #[test]
    fn listed_package_map_levels_join_their_chapter() {
        let mut config = Config::default();
        config.arc42.shown_diagrams = vec![
            "building-blocks@org/demo".into(),
            "cycles@org/demo".into(),
            "cycles@nope".into(),
        ];
        let (diagrams, chapters) = render(&config);
        let ids: Vec<&str> = diagrams.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "building-blocks",
                "building-blocks@org/demo",
                "cycles",
                "cycles@org/demo"
            ]
        );
        let level = &diagrams[1];
        let names: Vec<&str> = level.elements.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["p0", "p1", "p2"], "the packages inside org/demo");
        assert_eq!(level.relations.len(), 3);
        assert!(level.shown && level.title.ends_with("org/demo"));
        assert_eq!(diagrams[3].relations.len(), 2, "only the cycle p0 ↔ p1");
        assert!(chapters[4].contains(&section(level, DiagramFormat::Mermaid)));
    }

    #[test]
    fn plantuml_format_writes_plantuml_blocks() {
        let mut config = Config::default();
        config.arc42.diagram_format = DiagramFormat::Plantuml;
        let md = &render(&config).1[4];
        assert!(md.contains("```plantuml\n@startuml\n"), "{md}");
        assert!(md.contains("-[#d33]-> e1 : 1 Quellverbindung"), "{md}");
        assert!(!md.contains("```mermaid"));
    }
}
