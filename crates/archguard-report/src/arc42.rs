//! Evidence-driven arc42 sections shared by CLI, MCP and the viewer.
//! Chapter structure adapted from arc42 by Gernot Starke and Peter Hruschka,
//! CC BY-SA 4.0. All explanatory text and projections below are ArchGuard's.
use archguard_core::{CallAnalysis, Config, Graph, Report, ServiceDependencyKind, ServiceTopology};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, fmt::Write};

const NOTICE: &str = "Kapitelstruktur nach [arc42](https://arc42.org/overview/) von Gernot Starke und Peter Hruschka, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). Anpassung: automatisch befüllte Belegabschnitte und offene Ergänzungen. Die Lizenz der Kapitelvorlage gilt nicht automatisch für die eingefügten Projektdaten.\n";
const TITLES: [&str; 12] = [
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
const LIMIT: usize = 30;

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
    pub scope: String,
    pub notice: &'static str,
    pub chapters: Vec<Arc42Chapter>,
}

pub struct Arc42Inputs<'a> {
    /// Original containment tree and source edges, before view aggregation.
    pub source_graph: &'a Graph,
    pub view: &'a Graph,
    pub report: &'a Report,
    pub config: &'a Config,
    pub calls: &'a CallAnalysis,
    pub services: &'a ServiceTopology,
    /// The same normalized evidence models that the viewer displays.
    pub insights: &'a Value,
    /// Logical compiler paths -> unambiguous physical project-relative paths.
    pub source_paths: &'a BTreeMap<String, String>,
}

/// Escapes only what changes Markdown structure inside a line, so paths and
/// sentences stay readable in the hand-edited file.
fn cell(text: &str) -> String {
    let short: String = text.chars().take(512).collect();
    let text = if text.chars().count() > 512 {
        short + " … [gekürzt]"
    } else {
        short
    };
    let mut out = String::new();
    for (i, ch) in text.chars().enumerate() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' | '\n' | '\t' => out.push(' '),
            '\\' | '`' | '*' | '_' | '[' | ']' | '|' | '~' | '!' => {
                out.push('\\');
                out.push(ch);
            }
            '#' | '+' | '-' | '=' if i == 0 => {
                out.push('\\');
                out.push(ch);
            }
            ch => out.push(ch),
        }
    }
    out
}
fn val(value: &Value) -> String {
    match value {
        Value::Null => "unbekannt".into(),
        Value::Bool(b) => yes(*b).into(),
        Value::String(s) => cell(s),
        other => cell(&other.to_string()),
    }
}
fn yes(value: bool) -> &'static str {
    if value { "ja" } else { "nein" }
}
/// Producer status codes in German; unknown codes stay visible as code.
fn status(value: &Value) -> String {
    let Some(code) = value.as_str() else {
        return val(value);
    };
    let label = match code {
        "ready" => "verfügbar",
        "partial" => "teilweise",
        "unavailable" => "nicht verfügbar",
        "not_imported" => "nicht importiert",
        "not_scanned" => "nicht gescannt",
        "not_applicable" => "nicht anwendbar",
        "not_recorded" => "nicht erfasst",
        "error" => "Fehler",
        "stale" => "veraltet",
        "current" => "aktuell",
        "changed_since_import" => "seit Import geändert",
        "unchanged_since_import" => "seit Import unverändert",
        "changed_since_report" => "seit Bericht geändert",
        "analyzed" => "analysiert",
        "measured" => "gemessen",
        "incomplete" => "unvollständig",
        "unmapped" => "nicht zugeordnet",
        "mixed" => "gemischt",
        "unverified" => "ungeprüft",
        _ => return format!("`{}`", code.replace('`', "'")),
    };
    label.into()
}
/// Writes header and alignment lines plus rows, or one sentence when there
/// is nothing to show (no output for an empty sentence).
fn table(out: &mut String, head: &str, rows: Vec<String>, empty: &str) {
    if rows.is_empty() {
        if !empty.is_empty() {
            let _ = writeln!(out, "{empty}\n");
        }
        return;
    }
    let _ = writeln!(out, "{head}");
    for row in rows {
        let _ = writeln!(out, "{row}");
    }
    out.push('\n');
}
fn rows(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}
fn note(out: &mut String, count: usize) {
    if count > LIMIT {
        let _ = writeln!(
            out,
            "\nAuszug: {LIMIT} von {count} Einträgen. Weitere Belege sind in den jeweiligen ArchGuard-Ansichten verfügbar.\n"
        );
    }
}
fn source(input: &Arc42Inputs<'_>, logical: &str, line: u32) -> String {
    input
        .source_paths
        .get(logical)
        .map(|p| format!("{}:{line}", cell(p)))
        .unwrap_or_else(|| {
            format!(
                "Logischer Quellpfad {}:{line}; physische Zuordnung unbekannt",
                cell(logical)
            )
        })
}
fn block(number: u8, content: &str) -> String {
    format!(
        "<!-- archguard:arc42:{number:02}:start -->\n{content}\n<!-- archguard:arc42:{number:02}:end -->"
    )
}
impl Arc42Document {
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
        let mut out = format!(
            "# arc42 – Architekturdokumentation\n\n{NOTICE}\nAnalysierter Umfang: **{}**. Statische Beobachtungen, deklarierte Absichten und importierte Messwerte bleiben getrennt. Keine simulierten Graphänderungen. Keine Aussage über Vollständigkeit der Architektur.\n\nListen enthalten höchstens {LIMIT} Einträge; lange Texte werden sichtbar gekürzt. Eigene Erläuterungen außerhalb der archguard-Blöcke bleiben bei Aktualisierungen erhalten.\n",
            // The header is written once and committed; a machine path means nothing to readers.
            cell(
                std::path::Path::new(&self.scope)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&self.scope)
            )
        );
        for c in &self.chapters {
            if chapter.is_none_or(|n| n == c.number) {
                let _ = writeln!(
                    out,
                    "\n## {}. {}\n\n{}\n\n<!-- Eigene Erläuterungen zu Kapitel {} hier ergänzen. -->",
                    c.number,
                    c.title,
                    block(c.number, &c.markdown),
                    c.number
                );
            }
        }
        out
    }
}

fn package_markdown(graph: &Graph) -> String {
    use archguard_core::{Node, NodeId, NodeKind};
    let by_id: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id, n)).collect();
    let mut children: BTreeMap<Option<NodeId>, Vec<&Node>> = BTreeMap::new();
    for node in &graph.nodes {
        children.entry(node.parent).or_default().push(node);
    }
    let mut frontier = Vec::new();
    for mut root in children.get(&None).cloned().unwrap_or_default() {
        if root.kind != NodeKind::Module {
            continue;
        }
        loop {
            let kids = children
                .get(&Some(root.id))
                .map(Vec::as_slice)
                .unwrap_or_default();
            if kids.len() == 1 && kids[0].kind == NodeKind::Module {
                root = kids[0];
            } else {
                break;
            }
        }
        let kids = children
            .get(&Some(root.id))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let packages: Vec<_> = kids
            .iter()
            .filter(|n| n.kind == NodeKind::Module)
            .copied()
            .collect();
        if packages.is_empty() {
            frontier.push(root);
        } else {
            frontier.extend(packages);
            if kids.iter().any(|n| n.kind == NodeKind::File) {
                frontier.push(root);
            }
        }
    }
    let mut names: Vec<_> = frontier.iter().map(|n| n.name.clone()).collect();
    let loose = names.len();
    if graph
        .nodes
        .iter()
        .any(|n| n.kind == NodeKind::File && n.parent.is_none())
    {
        names.push("Dateien ohne Paket".into());
    }
    let index: BTreeMap<_, _> = frontier
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    let mut owner = BTreeMap::new();
    let mut files = vec![0usize; names.len()];
    for node in &graph.nodes {
        let mut current = Some(node.id);
        let mut group = None;
        // Canonical graphs are trees; the bound also protects legacy hand-built reports.
        for _ in 0..graph.nodes.len() {
            let Some(id) = current else { break };
            if let Some(&i) = index.get(&id) {
                group = Some(i);
                break;
            }
            current = by_id.get(&id).and_then(|n| n.parent);
        }
        if node.kind == NodeKind::File && node.parent.is_none() {
            group = Some(loose);
        }
        if let Some(i) = group {
            owner.insert(node.id, i);
            if node.kind == NodeKind::File {
                files[i] += 1;
            }
        }
    }
    let mut order: Vec<_> = (0..names.len()).collect();
    order.sort_by(|&a, &b| files[b].cmp(&files[a]).then(names[a].cmp(&names[b])));
    let mut shown: Vec<_> = order.iter().take(8).map(|&i| names[i].clone()).collect();
    if names.len() > 8 {
        shown.push(format!("Weitere {} Pakete", names.len() - 8));
    }
    let display: BTreeMap<_, _> = order
        .iter()
        .enumerate()
        .map(|(position, &i)| (i, position.min(8)))
        .collect();
    let mut relations = BTreeMap::<(usize, usize), usize>::new();
    let mut incoming = vec![0usize; names.len()];
    let mut outgoing = vec![0usize; names.len()];
    let mut internal = 0;
    let mut covered = 0;
    for edge in &graph.edges {
        let (Some(&a), Some(&b)) = (owner.get(&edge.from), owner.get(&edge.to)) else {
            continue;
        };
        covered += 1;
        if a != b {
            outgoing[a] += 1;
            incoming[b] += 1;
        }
        let (a, b) = (display[&a], display[&b]);
        if a == b {
            internal += 1;
        } else {
            *relations.entry((a, b)).or_default() += 1;
        }
    }
    let mut out = format!(
        "### Paketübersicht\n\nQuelle: vollständiger ursprünglicher Quellgraph. {} Datei-/Symbolknoten, {} Paketgruppen, {} Quellverbindungen. Ein Pfeil bündelt gerichtete Quellverbindungen zwischen Paketen; {} Verbindungen liegen innerhalb dargestellter Gruppen. Ansichtfilter und Simulationen gehen nicht ein.\n\n",
        files.iter().sum::<usize>(),
        names.len(),
        covered,
        internal
    );
    if !shown.is_empty() {
        out.push_str("```mermaid\nflowchart LR\n");
        for (i, name) in shown.iter().enumerate() {
            let label: String = name
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || " /._-".contains(ch) {
                        ch.to_string()
                    } else {
                        format!("#{};", ch as u32)
                    }
                })
                .collect();
            let _ = writeln!(out, "  n{i}[\"{label}\"]");
        }
        for ((a, b), count) in relations {
            let _ = writeln!(out, "  n{a} -->|{count}| n{b}");
        }
        out.push_str("```\n\n");
    }
    out.push_str("Die Übersicht bündelt alle erfassten Quellen. Weitere Pakete, Unterpakete und einzelne Beziehungen sind im Viewer auswählbar. Paketzyklen beschreiben die aggregierte Paketsicht, nicht automatisch einen Zyklus zwischen einzelnen Dateien.\n\n");
    table(
        &mut out,
        "| Paket | Datei-/Symbolknoten | Quellverbindungen eingehend | Quellverbindungen ausgehend |\n| --- | ---: | ---: | ---: |",
        order
            .iter()
            .take(LIMIT)
            .map(|&i| {
                format!(
                    "| {} | {} | {} | {} |",
                    cell(&names[i]),
                    files[i],
                    incoming[i],
                    outgoing[i]
                )
            })
            .collect(),
        "Keine Pakete im Quellgraph erfasst.",
    );
    note(&mut out, names.len());
    out
}

pub fn render_arc42(input: Arc42Inputs<'_>) -> Arc42Document {
    let Arc42Inputs {
        view,
        report,
        config,
        calls,
        services,
        insights,
        ..
    } = &input;
    let mut chapters: Vec<_> = TITLES
        .iter()
        .enumerate()
        .map(|(i, title)| Arc42Chapter {
            number: (i + 1) as u8,
            title,
            markdown: String::new(),
            questions: vec![],
        })
        .collect();
    let c = &mut chapters[0];
    c.markdown.push_str("### Deklarierte Qualitätsziele\n\nQuelle: `architecture.goals` in der wirksamen Konfiguration.\n\n");
    table(
        &mut c.markdown,
        "| Ziel | Priorität | Begründung |\n| --- | --- | --- |",
        config
            .architecture
            .goals
            .iter()
            .take(LIMIT)
            .map(|g| {
                format!(
                    "| {} | {} | {} |",
                    cell(&g.name),
                    cell(&g.priority),
                    cell(&g.rationale)
                )
            })
            .collect(),
        "Keine Qualitätsziele deklariert.",
    );
    note(&mut c.markdown, config.architecture.goals.len());
    if config.architecture.goals.is_empty() {
        c.questions
            .push("Qualitätsziele und Prioritäten sind nicht deklariert.".into());
    }
    c.questions.push("Fachlicher Zweck, wesentliche Anforderungen und Stakeholder mit ihren Erwartungen ergänzen.".into());

    let c = &mut chapters[1];
    c.markdown.push_str("Quelle: `architecture.constraints`. Technisch beobachtete Sprachen und Werkzeuge sind allein keine verbindlichen Randbedingungen.\n\n");
    for r in config.architecture.constraints.iter().take(LIMIT) {
        let _ = writeln!(
            c.markdown,
            "- **{}**: {}",
            cell(&r.name),
            cell(&r.rationale)
        );
    }
    note(&mut c.markdown, config.architecture.constraints.len());
    if config.architecture.constraints.is_empty() {
        c.markdown.push_str("Keine Randbedingungen deklariert.\n");
    }
    c.questions.push(
        "Nicht erfasste organisatorische, regulatorische und technische Vorgaben ergänzen.".into(),
    );

    let c = &mut chapters[2];
    c.markdown.push_str("### Konfigurierte technische Beziehungen\n\nDiese Beziehungen stammen aus Deployment-Konfiguration. Sie belegen weder tatsächlich ausgeführte Kommunikation noch eine vollständige Systemgrenze.\n\n");
    let http: Vec<_> = services
        .dependencies
        .iter()
        .filter(|d| d.kind == ServiceDependencyKind::ConfiguredHttp)
        .collect();
    table(
        &mut c.markdown,
        "| Quelle | Ziel | Beleg |\n| --- | --- | --- |",
        http.iter()
            .take(LIMIT)
            .map(|d| {
                format!(
                    "| {} | {} | {} |",
                    cell(&d.from),
                    cell(&d.to),
                    d.evidence
                        .first()
                        .map(|e| format!("{} · {}", cell(&e.file), cell(&e.pointer)))
                        .unwrap_or_else(|| "unbekannt".into())
                )
            })
            .collect(),
        "Keine konfigurierten HTTP-Beziehungen erfasst.",
    );
    note(&mut c.markdown, http.len());
    let _ = writeln!(
        c.markdown,
        "API-Vertragsimport: **{}**. Verträge sind deklarierte Schnittstellen; Nutzung und Erreichbarkeit sind damit nicht nachgewiesen.\n",
        status(&insights["contracts"]["status"])
    );
    for contract in rows(&insights["contracts"]["contracts"]).iter().take(LIMIT) {
        let _ = writeln!(
            c.markdown,
            "- Provider {} · Quelle {} · Quellstatus {} · {} deklarierte Operationen.",
            val(&contract["provider"]),
            val(&contract["source_path"]),
            status(&contract["source_status"]),
            rows(&contract["operations"]).len()
        );
    }
    note(
        &mut c.markdown,
        rows(&insights["contracts"]["contracts"]).len(),
    );
    c.questions.push("Systemgrenze, Benutzerrollen, externe Nachbarsysteme und fachliche Schnittstellen bestätigen.".into());

    let c = &mut chapters[3];
    if let Some(model) = &config.architecture.model {
        let _ = writeln!(
            c.markdown,
            "Deklariertes Zielmodell: **{}**. Quelle: `architecture.model`.\n",
            cell(&model.name)
        );
        for component in model.components.iter().take(LIMIT) {
            let _ = writeln!(
                c.markdown,
                "- Komponente {}: {}",
                cell(&component.name),
                cell(&component.include.join(", "))
            );
        }
        note(&mut c.markdown, model.components.len());
    }
    if config.architecture.model.is_none() {
        c.markdown.push_str("Eine Lösungsstrategie ist nicht deklariert. Aus Abhängigkeiten wird kein Architekturstil abgeleitet.\n");
    }
    c.questions.push("Tragende Entwurfsentscheidungen und ihre Zuordnung zu Qualitätszielen erläutern. Erfasste Entscheidungen stehen in Kapitel 9.".into());

    let c = &mut chapters[4];
    c.markdown.push_str(&package_markdown(input.source_graph));
    c.questions.push("Fachliche Verantwortlichkeiten, Schnittstellen und bei Bedarf eine feinere Zerlegung der Bausteine erläutern.".into());

    let c = &mut chapters[5];
    let _ = writeln!(
        c.markdown,
        "### Statische Aufrufbelege\n\nCompilerstatus: **{}**. {} erfasste Aufrufstellen. Dies ist kein Laufzeittrace und keine Ausführungsreihenfolge.\n",
        status(&Value::from(calls.status.as_str())),
        calls.calls.len()
    );
    let mut sites: Vec<_> = calls.calls.iter().collect();
    sites.sort_by_key(|s| (&s.file, s.line, s.column, &s.target_label));
    table(
        &mut c.markdown,
        "| Aufrufer | Ziel | Auflösung | Quellbeleg |\n| --- | --- | --- | --- |",
        sites
            .iter()
            .take(LIMIT)
            .map(|s| {
                format!(
                    "| {} | {} | {} | {} |",
                    cell(s.from_method.as_deref().unwrap_or(&s.from_class)),
                    cell(&s.target_label),
                    cell(&s.resolution),
                    source(&input, &s.file, s.line)
                )
            })
            .collect(),
        "Keine Aufrufstellen erfasst.",
    );
    note(&mut c.markdown, sites.len());
    c.questions.push("Relevante Ablauf- und Fehlerszenarien mit Auslöser, Reihenfolge, Nebenläufigkeit und Ergebnis beschreiben; mit Laufzeitbelegen prüfen.".into());

    let c = &mut chapters[6];
    let _ = writeln!(
        c.markdown,
        "### Gelesene Deployment-Konfiguration\n\nKonfigurationsleser vollständig: **{}**. Keine Aussage über tatsächlichen Betrieb.\n",
        yes(services.complete)
    );
    let deployments: Vec<_> = services
        .services
        .iter()
        .filter(|s| !s.compose_file.is_empty())
        .collect();
    let list = |items: &[String]| {
        if items.is_empty() {
            "–".into()
        } else {
            cell(&items.join(", "))
        }
    };
    table(
        &mut c.markdown,
        "| Dienst | Quelle | Build-Kontext | Netzwerke | Profile |\n| --- | --- | --- | --- | --- |",
        deployments
            .iter()
            .take(LIMIT)
            .map(|s| {
                format!(
                    "| {} | {} | {} | {} | {} |",
                    cell(&s.name),
                    cell(&s.compose_file),
                    s.build_context
                        .as_deref()
                        .map(cell)
                        .unwrap_or_else(|| "–".into()),
                    list(&s.networks),
                    list(&s.profiles)
                )
            })
            .collect(),
        "Keine Compose-Dienstdeklarationen erfasst. Manifest-Projekte folgen separat.",
    );
    note(&mut c.markdown, deployments.len());
    c.markdown.push_str("### Build- und Projekteinteilung\n\nManifest-Einheiten sind nicht automatisch eigenständige Laufzeitdienste.\n\n");
    for p in services.projects.iter().take(LIMIT) {
        let _ = writeln!(
            c.markdown,
            "- {} · {} · {} · Manifest {}",
            cell(&p.name),
            cell(&p.kind),
            cell(&p.ecosystem),
            cell(&p.manifest)
        );
    }
    note(&mut c.markdown, services.projects.len());
    c.questions.push("Produktionsumgebungen, Hosts, Skalierung, Ressourcengrenzen und Betriebsverantwortung ergänzen.".into());

    let c = &mut chapters[7];
    c.markdown.push_str("### Deklarierte Abhängigkeitskonventionen\n\nQuelle: wirksame `layers`, `dependency_rules` und `ownership_rules`. Regeln beschreiben Vorgaben; ihre bloße Existenz beweist keine Einhaltung.\n\n");
    let (declared, derived): (Vec<_>, Vec<_>) = config
        .layers
        .iter()
        .partition(|l| l.origin == archguard_core::LayerOrigin::Declared);
    let layers = |out: &mut String, layers: &[&archguard_core::LayerConfig]| {
        for l in layers.iter().take(LIMIT) {
            let _ = writeln!(
                out,
                "- Schicht {} · enthält {} · darf abhängen von {}",
                cell(&l.name),
                cell(&l.contains.join(", ")),
                if l.may_depend_on.is_empty() {
                    "keiner anderen Schicht".into()
                } else {
                    cell(&l.may_depend_on.join(", "))
                }
            );
        }
        note(out, layers.len());
    };
    layers(&mut c.markdown, &declared);
    let rules = config.dependency_rules.len() + config.ownership_rules.len();
    if declared.is_empty() && rules == 0 {
        c.markdown
            .push_str("Keine Schichten oder Abhängigkeitsregeln deklariert.\n");
    }
    for r in config.dependency_rules.iter().take(LIMIT) {
        let _ = writeln!(
            c.markdown,
            "- Verbot {}: {} → {}",
            cell(&r.name),
            cell(&r.from.join(", ")),
            cell(&r.to.join(", "))
        );
    }
    note(&mut c.markdown, config.dependency_rules.len());
    for r in config.ownership_rules.iter().take(LIMIT) {
        let _ = writeln!(
            c.markdown,
            "- Verbotene Eigentümerrichtung: {} → {}",
            cell(&r.from),
            cell(&r.to)
        );
    }
    note(&mut c.markdown, config.ownership_rules.len());
    if !derived.is_empty() {
        c.markdown.push_str("\n### Abgeleitete Schichten\n\nVon ArchGuard aus den beobachteten Abhängigkeiten berechnet. Das ist eine Beobachtung, keine Vorgabe; Verstöße gegen diese Schichten sind nur Hinweise.\n\n");
        layers(&mut c.markdown, &derived);
    }
    c.questions.push("Sicherheits-, Fehlerbehandlungs-, Persistenz-, Transaktions- und Observability-Konzepte anhand zusätzlicher Belege beschreiben.".into());

    let c = &mut chapters[8];
    c.markdown.push_str("Quelle: `architecture.decisions`. Status, Alternativen und Abwägungen werden ausschließlich aus deklarierten Angaben übernommen.\n\n");
    for d in config.architecture.decisions.iter().take(LIMIT) {
        let _ = writeln!(
            c.markdown,
            "### {}: {}\n\n- Status: {}\n- Gewählte Option: {}\n- Alternativen: {}\n- Akzeptierte Nachteile: {}\n- Ungeprüfte Referenzen: {}\n",
            cell(&d.id),
            cell(&d.title),
            cell(&d.status),
            cell(&d.chosen_option),
            cell(&d.alternatives.join("; ")),
            cell(&d.trade_offs.join("; ")),
            cell(&d.links.join("; "))
        );
    }
    note(&mut c.markdown, config.architecture.decisions.len());
    if config.architecture.decisions.is_empty() {
        c.markdown
            .push_str("Keine Architekturentscheidungen deklariert.\n");
    }
    c.questions.push("Fehlende Entscheidungen, Begründungen und Konsequenzen ergänzen; erkannte Muster sind keine automatisch akzeptierten ADRs.".into());

    let c = &mut chapters[9];
    let s = &report.summary;
    let _ = writeln!(
        c.markdown,
        "Qualitätsziele siehe Kapitel 1. Messwerte sind Beobachtungen, keine automatisch festgelegten Anforderungen.\n\n### Abhängigkeitskennzahlen\n\nQuelle: Analysebericht des in Kapitel 5 genannten Umfangs.\n\n| Kennzahl | Wert |\n| --- | ---: |\n| CCD | {} |\n| ACD | {:.3} |\n| NCCD | {:.3} |\n| Zyklische Gruppen | {} |\n| Größte zyklische Gruppe | {} |\n",
        s.ccd, s.acd, s.nccd, s.sccs, s.largest_scc
    );
    let _ = writeln!(
        c.markdown,
        "Konfigurierte Prüfkriterien: Zyklen verboten {}, maximale SCC-Größe {}, ACD-Grenze {} (0 = deaktiviert). Quelle: `rules`.\n",
        match config.rules.deny_cycles {
            archguard_core::DenyCycles::Module => "zwischen Modulen",
            archguard_core::DenyCycles::File => "auch zwischen Dateien",
            archguard_core::DenyCycles::Off => "nicht",
        },
        config.rules.max_scc_size,
        config.rules.max_acd
    );
    let code = &insights["code_metrics"];
    let _ = writeln!(
        c.markdown,
        "### Quellcodemessung und Abdeckung\n\nStatus: **{}** · Definition: {}. Nicht anwendbare oder unbekannte Werte werden nicht als 0 interpretiert.\n",
        status(&code["status"]),
        val(&code["definition"])
    );
    // Without any declaration there is nothing a metric could have measured.
    let coverage: Vec<_> = code["coverage"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, counts)| counts[1].as_u64().is_some_and(|total| total > 0))
        .map(|(metric, counts)| {
            format!(
                "| {} | {} | {} |",
                cell(metric),
                val(&counts[0]),
                val(&counts[1])
            )
        })
        .collect();
    table(
        &mut c.markdown,
        "| Metrik | Gemessen | Deklarationen insgesamt |\n| --- | ---: | ---: |",
        coverage,
        "Keine Deklarationen gemessen.",
    );
    c.markdown.push_str("### Gemessene Klassenwerte\n\n");
    table(
        &mut c.markdown,
        "| Klasse | CBO | RFC | DIT | NOC | WMC | LCOM4 |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |",
        rows(&code["classes"])
            .iter()
            .take(LIMIT)
            .map(|row| {
                format!(
                    "| {} | {} | {} | {} | {} | {} | {} |",
                    val(&row["class_id"]),
                    val(&row["cbo"]["value"]),
                    val(&row["rfc"]["value"]),
                    val(&row["dit"]["value"]),
                    val(&row["noc"]["value"]),
                    val(&row["wmc"]["value"]),
                    val(&row["lcom4"])
                )
            })
            .collect(),
        "Keine Klassenwerte gemessen.",
    );
    note(&mut c.markdown, rows(&code["classes"]).len());
    c.markdown.push_str("### Methodenwartbarkeit\n\nNach Cognitive-Wert absteigend. Cognitive und Halstead verwenden die versionierten ArchGuard-Quellcode-/AST-Zählregeln; MI verwendet dieses Halstead-Volumen.\n\n");
    let mut methods: Vec<_> = rows(&code["methods"]).iter().collect();
    methods.sort_by(|a, b| {
        b["cognitive"]
            .as_u64()
            .cmp(&a["cognitive"].as_u64())
            .then_with(|| a["method_id"].as_str().cmp(&b["method_id"].as_str()))
    });
    table(
        &mut c.markdown,
        "| Methode | Cognitive | Halstead-Volumen | MI | Quellbeleg |\n| --- | ---: | ---: | ---: | --- |",
        methods
            .iter()
            .take(LIMIT)
            .map(|row| {
                format!(
                    "| {} | {} | {} | {} | {} |",
                    val(&row["method_id"]),
                    val(&row["cognitive"]),
                    val(&row["halstead"]["volume"]),
                    val(&row["maintainability_index"]),
                    source(
                        &input,
                        row["file"].as_str().unwrap_or(""),
                        row["line"].as_u64().unwrap_or(0) as u32
                    )
                )
            })
            .collect(),
        "Keine Methodenwerte gemessen.",
    );
    note(&mut c.markdown, methods.len());
    c.markdown.push_str("### Importierte Berichte\n\nAktualität und Messumfang der gespeicherten Berichte sind separat zu prüfen.\n\n");
    for (key, label) in [
        ("test_coverage", "Testabdeckung"),
        ("measurement_runs", "Build-/Laufzeit-/Testmessungen"),
        ("mutations", "Mutationstests"),
        ("duplicates", "Duplikate"),
    ] {
        let _ = writeln!(
            c.markdown,
            "- {label}: **{}** (`{key}`)",
            status(&insights[key]["status"])
        );
    }
    c.markdown.push('\n');
    let files = rows(&insights["test_coverage"]["files"]);
    if !files.is_empty() {
        c.markdown.push_str("### Testabdeckung je Datei\n\n");
    }
    table(
        &mut c.markdown,
        "| Coverage-Datei | Quellstatus | Berichtete gedeckte Zeilen | Berichtete ausführbare Zeilen |\n| --- | --- | ---: | ---: |",
        files
            .iter()
            .take(LIMIT)
            .map(|row| {
                format!(
                    "| {} | {} | {} | {} |",
                    val(&row["path"]),
                    status(&row["status"]),
                    val(&row["lines"]["covered"]),
                    val(&row["lines"]["total"])
                )
            })
            .collect(),
        "",
    );
    note(&mut c.markdown, files.len());
    let groups = rows(&insights["measurement_runs"]["groups"]);
    if !groups.is_empty() {
        c.markdown.push_str("### Importierte Laufmessungen\n\nNur gleichartige Revision, Umgebung und Producer bilden eine Messgruppe. Median/p95 sind Sekunden erfolgreicher Läufe; wechselnde Testergebnisse sind ein Hinweis, keine Ursachenanalyse.\n\n");
    }
    table(
        &mut c.markdown,
        "| Messung | Revision / Umgebung / Producer | Erfolgreiche Zeitmessungen | Median | p95 | Wechselnde Ergebnisse |\n| --- | --- | ---: | ---: | ---: | --- |",
        groups
            .iter()
            .take(LIMIT)
            .map(|row| {
                format!(
                    "| {} · {} | {} / {} / {} | {} | {} | {} | {} |",
                    val(&row["kind"]),
                    val(&row["name"]),
                    val(&row["revision"]),
                    val(&row["environment"]),
                    val(&row["producer"]),
                    val(&row["duration_samples"]),
                    val(&row["median_seconds"]),
                    val(&row["p95_seconds"]),
                    val(&row["inconsistent"])
                )
            })
            .collect(),
        "",
    );
    note(&mut c.markdown, groups.len());
    c.questions.push("Messbare Qualitätsszenarien mit Kontext, Auslöser, Reaktion und Akzeptanzgrenze festlegen. Laufzeitziele nicht aus statischen Kennzahlen ableiten.".into());

    let c = &mut chapters[10];
    c.markdown.push_str("### Befunde zur Prüfung\n\nQuelle: Analysebericht. Statische Befunde sind keine automatisch akzeptierten technischen Schulden. Fehlende Befunde beweisen keine Fehlerfreiheit; Compilerabdeckung siehe Kapitel 6 und 10.\n\n");
    let mut findings: Vec<_> = report.findings.iter().collect();
    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let ident = |value: Value| value.as_str().map(str::to_owned).unwrap_or_default();
    for f in findings.iter().take(LIMIT) {
        let _ = writeln!(
            c.markdown,
            "- {} · `{}` · {} · {} · {}",
            cell(&f.id),
            ident(serde_json::to_value(f.kind).unwrap_or_default()),
            match f.severity {
                archguard_core::Severity::Error => "Fehler",
                archguard_core::Severity::Warning => "Warnung",
                archguard_core::Severity::Info => "Hinweis",
            },
            cell(&f.nodes.join(", ")),
            cell(f.message.as_deref().unwrap_or(""))
        );
        for e in f.evidence.iter().take(2) {
            let _ = writeln!(c.markdown, "  - Quelle {}:{}", cell(&e.file), e.line);
        }
    }
    note(&mut c.markdown, findings.len());
    if findings.is_empty() {
        c.markdown
            .push_str("Die Analyse hat keine Befunde gemeldet.\n");
    }
    for (key, label) in [
        ("quality", "Importierte Qualitätsbefunde"),
        ("security", "Sicherheitsbefunde"),
    ] {
        let model = &insights[key];
        let _ = writeln!(
            c.markdown,
            "\n### {label}\n\nImportstatus: **{}**.",
            status(&model["status"])
        );
        let found = rows(&model["findings"]);
        if !found.is_empty() {
            c.markdown.push_str("Aufgelistete Einträge behalten ihren ursprünglichen Status; veraltete oder unterdrückte Einträge sind nicht automatisch aktive Probleme.\n\n");
        }
        for f in found.iter().take(LIMIT) {
            let _ = writeln!(
                c.markdown,
                "- {} · {} · {} · Quellstatus {} · unterdrückt {} · Baseline {}",
                val(&f["id"]),
                val(f.get("severity").unwrap_or(&f["level"])),
                val(f.get("message").unwrap_or(&f["title"])),
                status(&f["status"]),
                val(&f["suppressed"]),
                status(&f["baseline_state"])
            );
        }
        note(&mut c.markdown, found.len());
    }
    c.questions.push("Risikoauswirkung, Verantwortliche, Priorität, akzeptierte Schulden und Maßnahmen mit dem Team bewerten.".into());

    let c = &mut chapters[11];
    c.markdown.push_str("| Begriff | Bedeutung in diesem Dokument |\n| --- | --- |\n| SCC | Gruppe gegenseitig erreichbarer Knoten; eine zyklische Gruppe ist keine Liste aller Einzelzyklen. |\n| CCD / ACD | Summe / Durchschnitt transitiv erreichbarer Knoten einschließlich des Ausgangsknotens. |\n| NCCD | CCD im Verhältnis zur verwendeten balancierten Baumreferenz. |\n| CBO / RFC | Erfasste Klassenzielkopplung / eigene Methoden plus direkte Aufrufziele. |\n| DIT / NOC | Compilerbekannte Basisklassentiefe / direkte Unterklassen im Quellumfang. |\n| WMC / LCOM4 | Summierte Methodenkomplexität / Gruppen expliziter Instanzmethoden anhand Zustand und Aufrufen. |\n| Cognitive / Halstead / MI | Versionierte Quellcode- bzw. AST-Messungen; keine Laufzeitmessung oder zugesicherte Werkzeuggleichheit. |\n| Unbekannt | Keine ausreichende Messgrundlage; ausdrücklich kein Nullwert. |\n");
    c.questions.push(
        "Fachbegriffe und Abkürzungen des Projekts mit abgestimmten Definitionen ergänzen.".into(),
    );
    for c in &mut chapters {
        c.markdown.push_str("\n### Offen zu ergänzen\n\n");
        // Questions are fixed prose from this file, not project data.
        for q in &c.questions {
            let _ = writeln!(c.markdown, "- {q}");
        }
    }
    Arc42Document {
        schema_version: 1,
        scope: view.root.clone(),
        notice: NOTICE,
        chapters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            scope: "test".into(),
            notice: NOTICE,
            chapters: TITLES
                .iter()
                .enumerate()
                .map(|(i, title)| Arc42Chapter {
                    number: (i + 1) as u8,
                    title,
                    markdown: format!("Chapter {}", i + 1),
                    questions: vec![],
                })
                .collect(),
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
