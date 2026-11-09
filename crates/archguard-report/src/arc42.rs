//! Evidence-driven arc42 sections shared by CLI, MCP and the viewer.
//! Chapter structure adapted from arc42 by Gernot Starke and Peter Hruschka,
//! CC BY-SA 4.0. All explanatory text and projections below are ArchGuard's.
use archguard_core::{Diagram, DocLang};
use serde::Serialize;
use std::fmt::Write;

use crate::arc42_text::{LIMIT, cell, fill};
use crate::{arc42_chapters as chapters, arc42_quality as quality};

const NOTICE_EN: &str = "Chapter structure after [arc42](https://arc42.org/overview/) by Gernot Starke and Peter Hruschka, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). Adaptation: automatically filled evidence sections and open additions. The licence of the chapter template does not automatically apply to the inserted project data.\n";
const NOTICE_DE: &str = "Kapitelstruktur nach [arc42](https://arc42.org/overview/) von Gernot Starke und Peter Hruschka, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). Anpassung: automatisch befüllte Belegabschnitte und offene Ergänzungen. Die Lizenz der Kapitelvorlage gilt nicht automatisch für die eingefügten Projektdaten.\n";
const TITLES_EN: [&str; 12] = [
    "Introduction and Goals",
    "Constraints",
    "Context and Scope",
    "Solution Strategy",
    "Building Block View",
    "Runtime View",
    "Deployment View",
    "Crosscutting Concepts",
    "Architecture Decisions",
    "Quality Requirements",
    "Risks and Technical Debt",
    "Glossary",
];
const TITLES_DE: [&str; 12] = [
    "Einführung und Ziele",
    "Randbedingungen",
    "Kontextabgrenzung",
    "Lösungsstrategie",
    "Bausteinsicht",
    "Laufzeitsicht",
    "Verteilungssicht",
    "Querschnittliche Konzepte",
    "Architekturentscheidungen",
    "Qualitätsanforderungen",
    "Risiken und technische Schulden",
    "Glossar",
];

#[derive(Debug, Clone, Serialize)]
pub struct Arc42Chapter {
    pub number: u8,
    pub title: &'static str,
    pub markdown: String,
    pub questions: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Arc42Document {
    pub schema_version: u32,
    /// The language of the generated text (ADR 0008).
    #[serde(skip)]
    pub lang: DocLang,
    pub scope: String,
    pub notice: &'static str,
    pub chapters: Vec<Arc42Chapter>,
    /// The diagram catalogue (ADR 0006); `shown` ones are in the chapters.
    pub diagrams: Vec<Diagram>,
}

pub use crate::arc42_blocks::{Arc42Blocks, Arc42Context, Arc42Inputs};

fn block(number: u8, content: &str) -> String {
    format!(
        "<!-- archguard:arc42:{number:02}:start -->\n{content}\n<!-- archguard:arc42:{number:02}:end -->"
    )
}
impl Arc42Document {
    /// The document of `chapters` with the diagram catalogue; shown
    /// diagrams open their chapters (ADR 0006).
    fn new(scope: String, mut chapters: Vec<Arc42Chapter>, input: &Arc42Inputs<'_>) -> Self {
        let text = chapters.iter_mut().map(|c| (c.number, &mut c.markdown));
        let diagrams = crate::arc42_diagrams::add(text, input);
        Self {
            schema_version: 1,
            lang: input.lang,
            scope,
            notice: input.lang.pick(NOTICE_EN, NOTICE_DE),
            chapters,
            diagrams,
        }
    }

    /// Regeneration is limited to explicit generated blocks. Free prose and
    /// headings remain byte-for-byte unchanged, including a file's CRLF style.
    pub fn merge(&self, saved: &str, chapter: Option<u8>) -> Result<String, String> {
        let mut spans = Vec::new();
        for c in &self.chapters {
            let start = format!("<!-- archguard:arc42:{:02}:start -->", c.number);
            let end = format!("<!-- archguard:arc42:{:02}:end -->", c.number);
            let starts: Vec<_> = saved.match_indices(&start).map(|(i, _)| i).collect();
            let ends: Vec<_> = saved.match_indices(&end).map(|(i, _)| i).collect();
            if starts.is_empty() && ends.is_empty() {
                continue;
            }
            if starts.len() != 1 || ends.len() != 1 || starts[0] >= ends[0] {
                return Err(format!(
                    "arc42 chapter {} needs exactly one intact generated block; no file was changed. Use --stdout to obtain blocks for an existing document.",
                    c.number
                ));
            }
            spans.push((starts[0], ends[0] + end.len(), c));
        }
        if spans.is_empty()
            || chapter.is_some_and(|number| !spans.iter().any(|s| s.2.number == number))
        {
            return Err("arc42 document has no generated block for this selection; no file was changed. Use --stdout to obtain blocks for an existing document.".into());
        }
        spans.sort_by_key(|span| span.0);
        if spans.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err("arc42 generated blocks overlap; no file was changed".into());
        }
        let mut result = saved.to_owned();
        for (from, to, c) in spans.into_iter().rev() {
            if chapter.is_none_or(|n| n == c.number) {
                let mut generated = block(c.number, &c.markdown);
                if saved[from..to].contains("\r\n") {
                    generated = generated.replace('\n', "\r\n");
                }
                result.replace_range(from..to, &generated);
            }
        }
        Ok(result)
    }
    pub fn markdown(&self, chapter: Option<u8>) -> String {
        let header = self.lang.pick(
            "# arc42 – Architecture Documentation\n\n{}\nAnalysed scope: **{}**. Static observations, declared intentions and imported measurements stay separate. No simulated graph changes. No statement about the completeness of the architecture.\n\nLists hold at most {} entries; long texts are visibly shortened. Your own explanations outside the archguard blocks are kept on updates.\n",
            "# arc42 – Architekturdokumentation\n\n{}\nAnalysierter Umfang: **{}**. Statische Beobachtungen, deklarierte Absichten und importierte Messwerte bleiben getrennt. Keine simulierten Graphänderungen. Keine Aussage über Vollständigkeit der Architektur.\n\nListen enthalten höchstens {} Einträge; lange Texte werden sichtbar gekürzt. Eigene Erläuterungen außerhalb der archguard-Blöcke bleiben bei Aktualisierungen erhalten.\n",
        );
        // The header is written once and committed; a machine path means nothing to readers.
        let scope = cell(
            std::path::Path::new(&self.scope)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&self.scope),
        );
        let mut out = fill(header, &[&self.notice, &scope, &LIMIT]);
        let own = self.lang.pick(
            "<!-- Add your own explanations for chapter {} here. -->",
            "<!-- Eigene Erläuterungen zu Kapitel {} hier ergänzen. -->",
        );
        for c in &self.chapters {
            if chapter.is_none_or(|n| n == c.number) {
                let _ = writeln!(
                    out,
                    "\n## {}. {}\n\n{}\n\n{}",
                    c.number,
                    c.title,
                    block(c.number, &c.markdown),
                    fill(own, &[&c.number])
                );
            }
        }
        out
    }
}

#[tracing::instrument(skip_all)]
pub fn render_arc42(input: Arc42Inputs<'_>) -> Arc42Document {
    let l = input.lang;
    let mut chapters: Vec<_> = l
        .pick(TITLES_EN, TITLES_DE)
        .iter()
        .enumerate()
        .map(|(i, title)| Arc42Chapter {
            number: (i + 1) as u8,
            title,
            markdown: String::new(),
            questions: vec![],
        })
        .collect();
    let write: [Chapter; 12] = [
        chapters::goals,
        chapters::constraints,
        chapters::context,
        chapters::strategy,
        |input, md, q| md.push_str(&chapters::package_markdown(input, q)),
        chapters::calls,
        chapters::deployment,
        chapters::conventions,
        chapters::decisions,
        quality::requirements,
        quality::risks,
        quality::glossary,
    ];
    let open = l.pick("\n### Still to add\n\n", "\n### Offen zu ergänzen\n\n");
    for (c, write) in chapters.iter_mut().zip(write) {
        write(&input, &mut c.markdown, &mut c.questions);
        c.markdown.push_str(open);
        // Questions are fixed prose of this crate, not project data.
        for q in &c.questions {
            let _ = writeln!(c.markdown, "- {q}");
        }
    }
    Arc42Document::new(input.view.root.clone(), chapters, &input)
}

/// Writes one chapter's text and its open questions.
type Chapter = fn(&Arc42Inputs<'_>, &mut String, &mut Vec<String>);

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{CallAnalysis, Config, Report, ServiceTopology};
    use std::collections::BTreeMap;

    /// The default language is English throughout: chapter titles, the
    /// generated blocks, diagrams and the open questions (ADR 0008).
    #[test]
    fn an_english_document_has_no_german_text() {
        let mut builder = archguard_core::GraphBuilder::new();
        let a = builder.ensure_path("org/demo/a/A.java", Some("java"));
        let b = builder.ensure_path("org/demo/b/B.java", Some("java"));
        builder.ensure_path("Loose.java", Some("java"));
        for (from, to) in [(a, b), (b, a)] {
            let r = archguard_core::Ref {
                file: "x".into(),
                line: 1,
                raw: "import".into(),
            };
            builder.edge(from, to, r).unwrap();
        }
        let graph = builder.build("shop".into());
        let report: Report = serde_json::from_value(serde_json::json!({
            "schema_version":1,"summary":{"nodes":0,"edges":0,"sccs":1,"largest_scc":2,"layer_violations":0,"status":"pass"},
            "findings":[],"metrics":{}
        }))
        .unwrap();
        let services: ServiceTopology = serde_json::from_value(serde_json::json!({
            "schema_version":1,"root":"shop","monorepo":false,"monorepo_reason":"","compose_files":["compose.yaml"],
            "services":[{"id":"web","name":"web","compose_file":"compose.yaml","build_context":".","networks":[],"profiles":[]}],
            "dependencies":[],"cycles":[],"diagnostics":[],"complete":true
        }))
        .unwrap();
        let doc = render_arc42(Arc42Inputs {
            source_graph: &graph,
            view: &graph,
            report: &report,
            config: &Config::default(),
            calls: &CallAnalysis::default(),
            services: &services,
            insights: &serde_json::json!({}),
            source_paths: &BTreeMap::new(),
            blocks: &Arc42Blocks::default(),
            goals: &[],
            lang: DocLang::default(),
        });
        assert_eq!(doc.chapters[5].title, "Runtime View");
        let text = doc.markdown(None);
        assert!(
            text.starts_with("# arc42 – Architecture Documentation"),
            "{text}"
        );
        assert!(text.contains("### Still to add") && text.contains("Files without a package"));
        assert!(!doc.diagrams.is_empty(), "the catalogue is written too");
        for d in &doc.diagrams {
            assert!(
                !d.title.contains("Paket") && !d.note.contains("Quelle"),
                "{d:?}"
            );
        }
        let german = [
            "ä", "ö", "ü", "ß", "Quelle", "Kapitel", " und ", " nicht ", "Keine ",
        ];
        let found: Vec<_> = german.iter().filter(|w| text.contains(*w)).collect();
        assert!(
            found.is_empty(),
            "German in the English document: {found:?}\n{text}"
        );
    }
    #[test]
    fn imported_lists_show_omissions_and_manifest_projects_are_not_deployments() {
        let graph = archguard_core::GraphBuilder::new().build("test".into());
        let report: Report = serde_json::from_value(serde_json::json!({
            "schema_version":1,"summary":{"nodes":0,"edges":0,"sccs":0,"largest_scc":0,"layer_violations":0,"status":"pass"},
            "findings":[],"metrics":{}
        })).unwrap();
        let services: ServiceTopology = serde_json::from_value(serde_json::json!({
            "schema_version":1,"root":"test","monorepo":false,"monorepo_reason":"","compose_files":["compose.yaml"],
            "services":[
                {"id":"app","name":"ManifestApp","compose_file":"","build_context":".","networks":[],"profiles":[],"project":"app"},
                {"id":"worker","name":"ComposeWorker","compose_file":"compose.yaml","build_context":"worker","networks":[],"profiles":[]}
            ],
            "projects":[{"id":"app","name":"ManifestApp","path":".","manifest":"pom.xml","ecosystem":"maven","kind":"application","runtime":null,"evidence":[]}],
            "dependencies":[],"cycles":[],"diagnostics":[],"complete":true
        })).unwrap();
        let contracts: Vec<_> = (0..32).map(|i| serde_json::json!({"provider":format!("provider-{i}"),"source_path":"api.yaml","source_status":"current","operations":[]})).collect();
        let doc = render_arc42(Arc42Inputs {
            source_graph: &graph,
            view: &graph,
            report: &report,
            config: &Config::default(),
            calls: &CallAnalysis::default(),
            services: &services,
            insights: &serde_json::json!({"contracts":{"status":"ready","contracts":contracts},"patterns":{"assessments":[{"pattern":"layered","status":"candidate","scope":"."}]}}),
            source_paths: &BTreeMap::new(),
            blocks: &Arc42Blocks::default(),
            goals: &[],
            lang: archguard_core::DocLang::De,
        });
        assert!(!doc.chapters[3].markdown.contains("Musterhypothesen"));
        assert!(!doc.chapters[3].markdown.contains("layered"));
        let deployment = &doc.chapters[6].markdown;
        let (configured, projects) = deployment
            .split_once("### Build- und Projekteinteilung")
            .unwrap();
        assert!(configured.contains("ComposeWorker"));
        assert!(
            !configured.contains("ManifestApp"),
            "manifest-only projects must not appear as deployment declarations"
        );
        assert!(projects.contains("ManifestApp") && projects.contains("pom.xml"));
        assert!(
            doc.chapters[2]
                .markdown
                .contains("Auszug: 30 von 32 Einträgen")
        );
        assert!(!doc.chapters[2].markdown.contains("provider-30"));
    }
    #[test]
    fn package_overview_retains_all_sources_without_file_nodes_in_the_diagram() {
        let mut builder = archguard_core::GraphBuilder::new();
        let files: Vec<_> = (0..40)
            .map(|i| builder.ensure_path(&format!("org/demo/p{i:02}/Source.java"), Some("java")))
            .collect();
        for i in 0..40 {
            builder
                .edge(
                    files[i],
                    files[(i + 1) % 40],
                    archguard_core::Ref {
                        file: format!("p{i}"),
                        line: 1,
                        raw: "import target".into(),
                    },
                )
                .unwrap();
        }
        let graph = builder.build("test".into());
        let report: Report = serde_json::from_value(serde_json::json!({
            "schema_version":1,"summary":{"nodes":0,"edges":0,"sccs":0,"largest_scc":0,"layer_violations":0,"status":"pass"},
            "findings":[],"metrics":{}
        })).unwrap();
        let services: ServiceTopology = serde_json::from_value(serde_json::json!({
            "schema_version":1,"root":"test","monorepo":false,"monorepo_reason":"",
            "compose_files":[],"services":[],"dependencies":[],"cycles":[],"diagnostics":[],"complete":true
        })).unwrap();
        let doc = render_arc42(Arc42Inputs {
            source_graph: &graph,
            view: &graph,
            report: &report,
            config: &Config::default(),
            calls: &CallAnalysis::default(),
            services: &services,
            insights: &serde_json::json!({}),
            source_paths: &BTreeMap::new(),
            blocks: &Arc42Blocks::default(),
            goals: &[],
            lang: archguard_core::DocLang::De,
        });
        let chapter = &doc.chapters[4].markdown;
        let fence = char::from(96).to_string().repeat(3);
        let diagram = chapter
            .split(&format!("{fence}mermaid"))
            .nth(1)
            .unwrap()
            .split(&fence)
            .next()
            .unwrap();
        assert!(
            !diagram.contains(".java"),
            "package diagrams must not mix in arbitrary source files"
        );
        assert!(chapter.contains("40 Paketgruppen"));
        assert!(chapter.contains("40 Quellverbindungen"));
        assert!(diagram.contains("Weitere 32 Pakete"));
    }

    #[test]
    fn a_single_remaining_package_is_drawn_instead_of_a_remainder_box() {
        let mut builder = archguard_core::GraphBuilder::new();
        for i in 0..9 {
            builder.ensure_path(&format!("org/demo/p{i}/Source.java"), Some("java"));
        }
        let graph = builder.build("test".into());
        let report: Report = serde_json::from_value(serde_json::json!({
            "schema_version":1,"summary":{"nodes":0,"edges":0,"sccs":0,"largest_scc":0,"layer_violations":0,"status":"pass"},
            "findings":[],"metrics":{}
        })).unwrap();
        let services: ServiceTopology = serde_json::from_value(serde_json::json!({
            "schema_version":1,"root":"test","monorepo":false,"monorepo_reason":"",
            "compose_files":[],"services":[],"dependencies":[],"cycles":[],"diagnostics":[],"complete":true
        })).unwrap();
        let doc = render_arc42(Arc42Inputs {
            source_graph: &graph,
            view: &graph,
            report: &report,
            config: &Config::default(),
            calls: &CallAnalysis::default(),
            services: &services,
            insights: &serde_json::json!({}),
            source_paths: &BTreeMap::new(),
            blocks: &Arc42Blocks::default(),
            goals: &[],
            lang: archguard_core::DocLang::De,
        });
        let md = &doc.chapters[4].markdown;
        assert!(!md.contains("Weitere 1 Pakete"), "{md}");
        assert!(md.contains("p8\"]"), "the ninth package is drawn: {md}");
    }

    #[test]
    fn building_blocks_name_their_project_responsibility_layer_and_dependencies() {
        let mut builder = archguard_core::GraphBuilder::new();
        let cli = builder.ensure_path("cli/src/main.rs", Some("rust"));
        let engine = builder.ensure_path("engine/src/lib.rs", Some("rust"));
        let core = builder.ensure_path("core/src/lib.rs", Some("rust"));
        let tool = builder.ensure_path("tool/src/lib.rs", Some("rust"));
        let reference = |file: &str| archguard_core::Ref {
            file: file.into(),
            line: 1,
            raw: "use".into(),
        };
        for (from, to) in [(cli, engine), (engine, core), (cli, core), (tool, core)] {
            builder.edge(from, to, reference("x")).unwrap();
        }
        let graph = builder.build("repo".into());
        let report: Report = serde_json::from_value(serde_json::json!({
            "schema_version":1,"summary":{"nodes":0,"edges":0,"sccs":0,"largest_scc":0,"layer_violations":0,"status":"pass"},
            "findings":[],"metrics":{}
        })).unwrap();
        let project = |id: &str| serde_json::json!({"id":id,"name":format!("app-{id}"),"path":format!("crates/{id}"),"manifest":format!("crates/{id}/Cargo.toml"),"ecosystem":"cargo","kind":"library","runtime":null,"evidence":[]});
        let services: ServiceTopology = serde_json::from_value(serde_json::json!({
            "schema_version":1,"root":"repo","monorepo":true,"monorepo_reason":"","compose_files":[],"services":[],
            "projects":[{"id":"root","name":"workspace","path":".","manifest":"Cargo.toml","ecosystem":"cargo","kind":"workspace","runtime":null,"evidence":[]},project("cli"),project("engine"),project("core"),project("tool")],
            "dependencies":[],"cycles":[],"diagnostics":[],"complete":true
        })).unwrap();
        let layer = |name: &str, may: &[&str]| archguard_core::LayerConfig {
            name: name.into(),
            contains: vec![format!("{name}/**")],
            may_depend_on: may.iter().map(|s| s.to_string()).collect(),
            origin: archguard_core::LayerOrigin::Declared,
        };
        let config = Config {
            layers: vec![
                layer("core", &[]),
                layer("engine", &["core"]),
                layer("cli", &["engine", "core"]),
            ],
            ..Config::default()
        };
        let source_paths: BTreeMap<_, _> = [
            "cli/src/main.rs",
            "engine/src/lib.rs",
            "core/src/lib.rs",
            "tool/src/lib.rs",
        ]
        .into_iter()
        .map(|p| (p.to_string(), format!("crates/{p}")))
        .collect();
        let layer_of = |name: &str| graph.nodes.iter().find(|n| n.name == name).unwrap().id;
        let blocks = Arc42Blocks {
            layers: [("core/src", 0), ("engine/src", 1), ("cli/src", 2)]
                .into_iter()
                .map(|(name, i)| (layer_of(name), i))
                .collect(),
            descriptions: [
                ("cli", "the command-line interface"),
                ("engine", "pipeline orchestration"),
                ("core", "shared types"),
            ]
            .into_iter()
            .map(|(id, text)| (id.to_string(), text.to_string()))
            .collect(),
            ..Default::default()
        };
        let doc = render_arc42(Arc42Inputs {
            source_graph: &graph,
            view: &graph,
            report: &report,
            config: &config,
            calls: &CallAnalysis::default(),
            services: &services,
            insights: &serde_json::json!({}),
            source_paths: &source_paths,
            blocks: &blocks,
            goals: &[],
            lang: archguard_core::DocLang::De,
        });
        let chapter = &doc.chapters[4];
        let md = &chapter.markdown;
        assert!(
            md.contains("| app-engine | pipeline orchestration | engine | 1 | app-core |"),
            "{md}"
        );
        assert!(
            md.contains(
                "| app-cli | the command-line interface | cli | 1 | app-core, app-engine |"
            ),
            "{md}"
        );
        assert!(md.contains("| app-tool | – | – | 1 | app-core |"), "{md}");
        let cli_row = md.find("| app-cli |").unwrap();
        assert!(
            cli_row < md.find("| app-engine |").unwrap()
                && md.find("| app-engine |").unwrap() < md.find("| app-core |").unwrap(),
            "consumers before providers"
        );
        assert!(
            md.contains("subgraph g0[\"Schicht cli\"]"),
            "the diagram groups blocks by layer: {md}"
        );
        assert!(
            !md.contains("cli/src"),
            "project names replace source-root paths"
        );
        assert!(
            chapter
                .questions
                .iter()
                .any(|q| q.contains("app-tool") && !q.contains("app-core")),
            "only blocks without a responsibility are asked for: {:?}",
            chapter.questions
        );
    }

    #[test]
    fn missing_evidence_reads_as_german_prose_instead_of_empty_tables() {
        let graph = archguard_core::GraphBuilder::new().build("/home/someone/shop".into());
        let report: Report = serde_json::from_value(serde_json::json!({
            "schema_version":1,"summary":{"nodes":0,"edges":0,"sccs":0,"largest_scc":0,"layer_violations":0,"status":"pass"},
            "findings":[],"metrics":{}
        })).unwrap();
        let services: ServiceTopology = serde_json::from_value(serde_json::json!({
            "schema_version":1,"root":"test","monorepo":false,"monorepo_reason":"","compose_files":["stacks/a/compose.yaml"],
            "services":[{"id":"w","name":"worker|x","compose_file":"stacks/a/compose.yaml","build_context":"services/worker","networks":["data"],"profiles":[]}],
            "projects":[],"dependencies":[],"cycles":[],"diagnostics":[],"complete":true
        })).unwrap();
        let layer = |name: &str, origin| archguard_core::LayerConfig {
            name: name.into(),
            contains: vec![format!("{name}/**")],
            may_depend_on: vec![],
            origin,
        };
        let config = Config {
            layers: vec![
                layer("core", archguard_core::LayerOrigin::Derived),
                layer("app", archguard_core::LayerOrigin::Declared),
            ],
            ..Default::default()
        };
        let calls = CallAnalysis {
            status: "unavailable".into(),
            ..Default::default()
        };
        let doc = render_arc42(Arc42Inputs {
            source_graph: &graph,
            view: &graph,
            report: &report,
            config: &config,
            calls: &calls,
            services: &services,
            insights: &serde_json::json!({
                "contracts":{"status":"not_imported","contracts":[]},
                "code_metrics":{"status":"unavailable","definition":"source-metrics/v1","coverage":{"cbo":[0,0]},"classes":[],"methods":[]},
                "test_coverage":{"status":"not_imported","files":[]},
                "measurement_runs":{"status":"not_imported","groups":[]},
                "mutations":{"status":"not_imported"},"duplicates":{"status":"not_imported"},
                "quality":{"status":"not_imported","findings":[]},"security":{"status":"not_scanned","findings":[]}
            }),
            source_paths: &BTreeMap::new(),
            blocks: &Arc42Blocks::default(),
            goals: &[],
            lang: archguard_core::DocLang::De,
        });
        let all = doc.markdown(None);
        let lines: Vec<_> = all.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.starts_with("| ---") {
                assert!(
                    lines.get(i + 1).is_some_and(|next| next.starts_with('|')),
                    "table without rows before line {}: {}",
                    i + 1,
                    lines[i - 1]
                );
            }
        }
        for raw in [
            "not_imported",
            "not\\_imported",
            "unavailable",
            "not\\_scanned",
            "Derived",
            "Module,",
            ": **true**",
        ] {
            assert!(
                !all.contains(raw),
                "raw identifier {raw:?} leaked into the document"
            );
        }
        assert!(all.contains("nicht importiert") && all.contains("nicht verfügbar"));
        assert!(
            !all.contains("| cbo | 0 | 0 |"),
            "unmeasured metrics must not read as zero"
        );
        assert!(
            all.contains("Fachlicher Zweck, wesentliche Anforderungen"),
            "fixed prose is not escaped"
        );
        assert!(
            all.contains("| stacks/a/compose.yaml | services/worker |"),
            "paths stay readable"
        );
        assert!(
            all.contains("worker\\|x"),
            "table separators in data stay escaped"
        );
        let rules = &doc.chapters[7].markdown;
        let (declared, derived) = rules.split_once("### Abgeleitete Schichten").unwrap();
        assert!(declared.contains("Schicht app") && !declared.contains("Schicht core"));
        assert!(derived.contains("Schicht core"));
        assert!(!all.contains("/home/someone") && all.contains("Analysierter Umfang: **shop**"));
    }

    fn document() -> Arc42Document {
        Arc42Document {
            schema_version: 1,
            lang: DocLang::De,
            scope: "test".into(),
            notice: NOTICE_DE,
            chapters: TITLES_DE
                .iter()
                .enumerate()
                .map(|(i, title)| Arc42Chapter {
                    number: (i + 1) as u8,
                    title,
                    markdown: format!("Chapter {}", i + 1),
                    questions: vec![],
                })
                .collect(),
            diagrams: vec![],
        }
    }
    #[test]
    fn chapter_updates_preserve_other_blocks_manual_text_and_crlf() {
        let old = document();
        let mut new = old.clone();
        new.chapters[4].markdown = "New evidence".into();
        new.chapters[8].markdown = "Unselected evidence".into();
        let saved = (old.markdown(None) + "\nOwn notes: Ää \n").replace('\n', "\r\n");
        let merged = new.merge(&saved, Some(5)).unwrap();
        assert!(merged.contains("New evidence"));
        assert!(!merged.contains("Unselected evidence"));
        assert!(merged.ends_with("\r\nOwn notes: Ää \r\n"));
        assert!(!merged.replace("\r\n", "").contains('\n'));
        assert!(
            new.merge(
                &saved.replace(
                    "<!-- archguard:arc42:05:end -->",
                    "<!-- archguard:arc42:05:start -->"
                ),
                None
            )
            .is_err()
        );
        let nested = format!(
            "{}{}{}{}",
            "<!-- archguard:arc42:01:start -->",
            "<!-- archguard:arc42:02:start -->",
            "<!-- archguard:arc42:01:end -->",
            "<!-- archguard:arc42:02:end -->"
        );
        assert!(old.merge(&nested, None).is_err());
    }
}
