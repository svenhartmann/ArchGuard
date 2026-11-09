//! arc42 chapters 1 to 9 (without 5, see `package_markdown`), one
//! function per chapter or section, in the document's language (ADR 0008).

use archguard_core::{DocLang, ServiceDependencyKind};
use serde_json::Value;
use std::fmt::Write;

use crate::arc42_blocks::{Arc42Inputs, Blocks};
use crate::arc42_text::{LIMIT, cell, fill, note, rows, status, table, val, yes};

/// Chapter 1: declared quality goals.
pub(crate) fn goals(input: &Arc42Inputs<'_>, md: &mut String, q: &mut Vec<String>) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { config, .. } = input;
    md.push_str(t(
        "### Declared quality goals\n\nSource: `architecture.goals` in the effective configuration.\n\n",
        "### Deklarierte Qualitätsziele\n\nQuelle: `architecture.goals` in der wirksamen Konfiguration.\n\n",
    ));
    table(
        md,
        t(
            "| Goal | Priority | Rationale | Characteristic (ISO/IEC 25010) | Scope | Status |\n| --- | --- | --- | --- | --- | --- |",
            "| Ziel | Priorität | Begründung | Merkmal (ISO/IEC 25010) | Umfang | Status |\n| --- | --- | --- | --- | --- | --- |",
        ),
        crate::arc42_goals::goal_rows(l, config, input.goals, LIMIT, cell),
        t(
            "No quality goals declared.",
            "Keine Qualitätsziele deklariert.",
        ),
    );
    note(l, md, config.architecture.goals.len());
    if config.architecture.goals.is_empty() {
        q.push(
            t(
                "Quality goals and priorities are not declared.",
                "Qualitätsziele und Prioritäten sind nicht deklariert.",
            )
            .into(),
        );
    }
    q.extend(crate::arc42_goals::goal_questions(l, config, cell));
    q.push(
        t(
            "Add the business purpose, essential requirements and stakeholders with their expectations.",
            "Fachlicher Zweck, wesentliche Anforderungen und Stakeholder mit ihren Erwartungen ergänzen.",
        )
        .into(),
    );
}

/// Chapter 2: declared constraints.
pub(crate) fn constraints(input: &Arc42Inputs<'_>, md: &mut String, q: &mut Vec<String>) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { config, .. } = input;
    md.push_str(t(
        "Source: `architecture.constraints`. Languages and tools observed in the code are not binding constraints by themselves.\n\n",
        "Quelle: `architecture.constraints`. Technisch beobachtete Sprachen und Werkzeuge sind allein keine verbindlichen Randbedingungen.\n\n",
    ));
    for r in config.architecture.constraints.iter().take(LIMIT) {
        let _ = writeln!(md, "- **{}**: {}", cell(&r.name), cell(&r.rationale));
    }
    note(l, md, config.architecture.constraints.len());
    if config.architecture.constraints.is_empty() {
        md.push_str(t(
            "No constraints declared.\n",
            "Keine Randbedingungen deklariert.\n",
        ));
    }
    q.push(
        t(
            "Add organisational, regulatory and technical requirements not yet recorded.",
            "Nicht erfasste organisatorische, regulatorische und technische Vorgaben ergänzen.",
        )
        .into(),
    );
}

/// Chapter 3: configured technical relations and API contracts.
pub(crate) fn context(input: &Arc42Inputs<'_>, md: &mut String, q: &mut Vec<String>) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { services, .. } = input;
    md.push_str(t(
        "### Configured technical relations\n\nThese relations come from deployment configuration. They prove neither communication that actually runs nor a complete system boundary.\n\n",
        "### Konfigurierte technische Beziehungen\n\nDiese Beziehungen stammen aus Deployment-Konfiguration. Sie belegen weder tatsächlich ausgeführte Kommunikation noch eine vollständige Systemgrenze.\n\n",
    ));
    let http: Vec<_> = services
        .dependencies
        .iter()
        .filter(|d| d.kind == ServiceDependencyKind::ConfiguredHttp)
        .collect();
    table(
        md,
        t(
            "| Source | Target | Evidence |\n| --- | --- | --- |",
            "| Quelle | Ziel | Beleg |\n| --- | --- | --- |",
        ),
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
                        .unwrap_or_else(|| t("unknown", "unbekannt").into())
                )
            })
            .collect(),
        t(
            "No configured HTTP relations recorded.",
            "Keine konfigurierten HTTP-Beziehungen erfasst.",
        ),
    );
    note(l, md, http.len());
    contracts(input, md);
    q.push(
        t(
            "Confirm the system boundary, user roles, neighbouring external systems and business interfaces.",
            "Systemgrenze, Benutzerrollen, externe Nachbarsysteme und fachliche Schnittstellen bestätigen.",
        )
        .into(),
    );
}

/// Chapter 3: imported API contracts.
pub(crate) fn contracts(input: &Arc42Inputs<'_>, md: &mut String) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { insights, .. } = input;
    let text = t(
        "API contract import: **{}**. Contracts are declared interfaces; they do not prove use or reachability.\n",
        "API-Vertragsimport: **{}**. Verträge sind deklarierte Schnittstellen; Nutzung und Erreichbarkeit sind damit nicht nachgewiesen.\n",
    );
    let _ = writeln!(
        md,
        "{}",
        fill(text, &[&status(l, &insights["contracts"]["status"])])
    );
    let line = t(
        "- Provider {} · source {} · source status {} · {} declared operations.",
        "- Provider {} · Quelle {} · Quellstatus {} · {} deklarierte Operationen.",
    );
    for contract in rows(&insights["contracts"]["contracts"]).iter().take(LIMIT) {
        let _ = writeln!(
            md,
            "{}",
            fill(
                line,
                &[
                    &val(l, &contract["provider"]),
                    &val(l, &contract["source_path"]),
                    &status(l, &contract["source_status"]),
                    &rows(&contract["operations"]).len(),
                ]
            )
        );
    }
    note(l, md, rows(&insights["contracts"]["contracts"]).len());
}

/// Chapter 4: the declared target model.
pub(crate) fn strategy(input: &Arc42Inputs<'_>, md: &mut String, q: &mut Vec<String>) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { config, .. } = input;
    if let Some(model) = &config.architecture.model {
        let text = t(
            "Declared target model: **{}**. Source: `architecture.model`.\n",
            "Deklariertes Zielmodell: **{}**. Quelle: `architecture.model`.\n",
        );
        let _ = writeln!(md, "{}", fill(text, &[&cell(&model.name)]));
        let line = t("- Component {}: {}", "- Komponente {}: {}");
        for component in model.components.iter().take(LIMIT) {
            let _ = writeln!(
                md,
                "{}",
                fill(
                    line,
                    &[&cell(&component.name), &cell(&component.include.join(", "))]
                )
            );
        }
        note(l, md, model.components.len());
    }
    if config.architecture.model.is_none() {
        md.push_str(t(
            "No solution strategy is declared. No architecture style is derived from dependencies.\n",
            "Eine Lösungsstrategie ist nicht deklariert. Aus Abhängigkeiten wird kein Architekturstil abgeleitet.\n",
        ));
    }
    q.push(
        t(
            "Explain the fundamental design decisions and how they serve the quality goals. Recorded decisions are in chapter 9.",
            "Tragende Entwurfsentscheidungen und ihre Zuordnung zu Qualitätszielen erläutern. Erfasste Entscheidungen stehen in Kapitel 9.",
        )
        .into(),
    );
}

/// Chapter 6: static call evidence.
pub(crate) fn calls(input: &Arc42Inputs<'_>, md: &mut String, q: &mut Vec<String>) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { calls, .. } = input;
    let text = t(
        "### Static call evidence\n\nCompiler status: **{}**. {} call sites recorded. This is no runtime trace and no execution order.\n",
        "### Statische Aufrufbelege\n\nCompilerstatus: **{}**. {} erfasste Aufrufstellen. Dies ist kein Laufzeittrace und keine Ausführungsreihenfolge.\n",
    );
    let _ = writeln!(
        md,
        "{}",
        fill(
            text,
            &[
                &status(l, &Value::from(calls.status.as_str())),
                &calls.calls.len()
            ]
        )
    );
    let mut sites: Vec<_> = calls.calls.iter().collect();
    sites.sort_by_key(|s| (&s.file, s.line, s.column, &s.target_label));
    table(
        md,
        t(
            "| Caller | Target | Resolution | Source evidence |\n| --- | --- | --- | --- |",
            "| Aufrufer | Ziel | Auflösung | Quellbeleg |\n| --- | --- | --- | --- |",
        ),
        sites
            .iter()
            .take(LIMIT)
            .map(|s| {
                format!(
                    "| {} | {} | {} | {} |",
                    cell(s.from_method.as_deref().unwrap_or(&s.from_class)),
                    cell(&s.target_label),
                    cell(&s.resolution),
                    source(input, &s.file, s.line)
                )
            })
            .collect(),
        t("No call sites recorded.", "Keine Aufrufstellen erfasst."),
    );
    note(l, md, sites.len());
    q.push(
        t(
            "Describe the relevant flow and error scenarios with trigger, order, concurrency and result; check them against runtime evidence.",
            "Relevante Ablauf- und Fehlerszenarien mit Auslöser, Reihenfolge, Nebenläufigkeit und Ergebnis beschreiben; mit Laufzeitbelegen prüfen.",
        )
        .into(),
    );
}

/// Chapter 7: deployment configuration read.
pub(crate) fn deployment(input: &Arc42Inputs<'_>, md: &mut String, q: &mut Vec<String>) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { services, .. } = input;
    let text = t(
        "### Deployment configuration read\n\nConfiguration reader complete: **{}**. No statement about actual operation.\n",
        "### Gelesene Deployment-Konfiguration\n\nKonfigurationsleser vollständig: **{}**. Keine Aussage über tatsächlichen Betrieb.\n",
    );
    let _ = writeln!(md, "{}", fill(text, &[&yes(l, services.complete)]));
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
        md,
        t(
            "| Service | Source | Build context | Networks | Profiles |\n| --- | --- | --- | --- | --- |",
            "| Dienst | Quelle | Build-Kontext | Netzwerke | Profile |\n| --- | --- | --- | --- | --- |",
        ),
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
        t(
            "No Compose service declarations recorded. Manifest projects follow separately.",
            "Keine Compose-Dienstdeklarationen erfasst. Manifest-Projekte folgen separat.",
        ),
    );
    note(l, md, deployments.len());
    projects(input, md);
    q.push(
        t(
            "Add production environments, hosts, scaling, resource limits and operational responsibility.",
            "Produktionsumgebungen, Hosts, Skalierung, Ressourcengrenzen und Betriebsverantwortung ergänzen.",
        )
        .into(),
    );
}

/// Chapter 7: build units from the manifests.
pub(crate) fn projects(input: &Arc42Inputs<'_>, md: &mut String) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { services, .. } = input;
    md.push_str(t(
        "### Build and project structure\n\nManifest units are not automatically services of their own at runtime.\n\n",
        "### Build- und Projekteinteilung\n\nManifest-Einheiten sind nicht automatisch eigenständige Laufzeitdienste.\n\n",
    ));
    for p in services.projects.iter().take(LIMIT) {
        let _ = writeln!(
            md,
            "- {} · {} · {} · Manifest {}",
            cell(&p.name),
            cell(&p.kind),
            cell(&p.ecosystem),
            cell(&p.manifest)
        );
    }
    note(l, md, services.projects.len());
}

/// Chapter 8: declared conventions, derived layers and error handling.
pub(crate) fn conventions(input: &Arc42Inputs<'_>, md: &mut String, q: &mut Vec<String>) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs {
        config, services, ..
    } = input;
    md.push_str(t(
        "### Declared dependency conventions\n\nSource: effective `layers`, `dependency_rules` and `ownership_rules`. Rules describe requirements; their mere existence does not prove compliance.\n\n",
        "### Deklarierte Abhängigkeitskonventionen\n\nQuelle: wirksame `layers`, `dependency_rules` und `ownership_rules`. Regeln beschreiben Vorgaben; ihre bloße Existenz beweist keine Einhaltung.\n\n",
    ));
    let (declared, derived): (Vec<_>, Vec<_>) = config
        .layers
        .iter()
        .partition(|l| l.origin == archguard_core::LayerOrigin::Declared);
    layer_lines(l, md, &declared);
    rules(input, md, declared.is_empty());
    if !derived.is_empty() {
        md.push_str(t(
            "\n### Derived layers\n\nComputed by ArchGuard from the observed dependencies. This is an observation, not a requirement; violations of these layers are hints only.\n\n",
            "\n### Abgeleitete Schichten\n\nVon ArchGuard aus den beobachteten Abhängigkeiten berechnet. Das ist eine Beobachtung, keine Vorgabe; Verstöße gegen diese Schichten sind nur Hinweise.\n\n",
        ));
        layer_lines(l, md, &derived);
    }
    crate::arc42_errors::add(l, md, q, services, cell);
}

/// Chapter 9: declared architecture decisions.
pub(crate) fn decisions(input: &Arc42Inputs<'_>, md: &mut String, q: &mut Vec<String>) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { config, .. } = input;
    md.push_str(t(
        "Source: `architecture.decisions`. Status, alternatives and trade-offs are taken only from declared entries.\n\n",
        "Quelle: `architecture.decisions`. Status, Alternativen und Abwägungen werden ausschließlich aus deklarierten Angaben übernommen.\n\n",
    ));
    let text = t(
        "### {}: {}\n\n- Status: {}\n- Chosen option: {}\n- Alternatives: {}\n- Accepted drawbacks: {}\n- Unchecked references: {}\n",
        "### {}: {}\n\n- Status: {}\n- Gewählte Option: {}\n- Alternativen: {}\n- Akzeptierte Nachteile: {}\n- Ungeprüfte Referenzen: {}\n",
    );
    for d in config.architecture.decisions.iter().take(LIMIT) {
        let _ = writeln!(
            md,
            "{}",
            fill(
                text,
                &[
                    &cell(&d.id),
                    &cell(&d.title),
                    &cell(&d.status),
                    &cell(&d.chosen_option),
                    &cell(&d.alternatives.join("; ")),
                    &cell(&d.trade_offs.join("; ")),
                    &cell(&d.links.join("; ")),
                ]
            )
        );
    }
    note(l, md, config.architecture.decisions.len());
    if config.architecture.decisions.is_empty() {
        md.push_str(t(
            "No architecture decisions declared.\n",
            "Keine Architekturentscheidungen deklariert.\n",
        ));
    }
    q.push(
        t(
            "Add missing decisions, rationales and consequences; detected patterns are not automatically accepted ADRs.",
            "Fehlende Entscheidungen, Begründungen und Konsequenzen ergänzen; erkannte Muster sind keine automatisch akzeptierten ADRs.",
        )
        .into(),
    );
}

/// Lines of `layers`: what each contains and may depend on.
fn layer_lines(l: DocLang, out: &mut String, layers: &[&archguard_core::LayerConfig]) {
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let line = t(
        "- Layer {} · contains {} · may depend on {}",
        "- Schicht {} · enthält {} · darf abhängen von {}",
    );
    for layer in layers.iter().take(LIMIT) {
        let to = if layer.may_depend_on.is_empty() {
            t("no other layer", "keiner anderen Schicht").into()
        } else {
            cell(&layer.may_depend_on.join(", "))
        };
        let _ = writeln!(
            out,
            "{}",
            fill(
                line,
                &[&cell(&layer.name), &cell(&layer.contains.join(", ")), &to]
            )
        );
    }
    note(l, out, layers.len());
}

/// Chapter 8: forbidden dependencies and ownership directions.
fn rules(input: &Arc42Inputs<'_>, md: &mut String, none_declared: bool) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let config = input.config;
    let rules = config.dependency_rules.len() + config.ownership_rules.len();
    if none_declared && rules == 0 {
        md.push_str(t(
            "No layers or dependency rules declared.\n",
            "Keine Schichten oder Abhängigkeitsregeln deklariert.\n",
        ));
    }
    let line = t("- Forbidden {}: {} → {}", "- Verbot {}: {} → {}");
    for r in config.dependency_rules.iter().take(LIMIT) {
        let _ = writeln!(
            md,
            "{}",
            fill(
                line,
                &[
                    &cell(&r.name),
                    &cell(&r.from.join(", ")),
                    &cell(&r.to.join(", "))
                ]
            )
        );
    }
    note(l, md, config.dependency_rules.len());
    let line = t(
        "- Forbidden ownership direction: {} → {}",
        "- Verbotene Eigentümerrichtung: {} → {}",
    );
    for r in config.ownership_rules.iter().take(LIMIT) {
        let _ = writeln!(md, "{}", fill(line, &[&cell(&r.from), &cell(&r.to)]));
    }
    note(l, md, config.ownership_rules.len());
}

/// `file:line`, or the logical path when no physical file is known.
pub(crate) fn source(input: &Arc42Inputs<'_>, logical: &str, line: u32) -> String {
    input
        .source_paths
        .get(logical)
        .map(|p| format!("{}:{line}", cell(p)))
        .unwrap_or_else(|| {
            let text = input.lang.pick(
                "Logical source path {}:{}; physical location unknown",
                "Logischer Quellpfad {}:{}; physische Zuordnung unbekannt",
            );
            fill(text, &[&cell(logical), &line])
        })
}

/// Chapter 5: packages become building blocks with the responsibility their
/// manifest declares, the layer they belong to and the blocks they use.
pub(crate) fn package_markdown(input: &Arc42Inputs<'_>, questions: &mut Vec<String>) -> String {
    let blocks = Blocks::new(
        input.source_graph,
        input.config,
        input.services,
        input.source_paths,
        input.blocks,
        input.lang,
    );
    let l = input.lang;
    let text = l.pick(
        "### Building blocks\n\nSource: complete original source graph. {} file/symbol nodes, {} package groups, {} source connections; {} of them lie inside the blocks shown. Responsibility: `description` of the related manifest. Layer: first matching layer from `layers`. “Uses” names blocks with source connections to them, the most frequent first.\n\n",
        "### Bausteine\n\nQuelle: vollständiger ursprünglicher Quellgraph. {} Datei-/Symbolknoten, {} Paketgruppen, {} Quellverbindungen; {} davon liegen innerhalb dargestellter Bausteine. Verantwortung: `description` des zugehörigen Manifests. Schicht: erste passende Schicht aus `layers`. „Nutzt“ nennt Bausteine mit Quellverbindungen dorthin, die häufigsten zuerst.\n\n",
    );
    let mut out = fill(
        text,
        &[
            &blocks.files.iter().sum::<usize>(),
            &blocks.labels.len(),
            &blocks.covered,
            &blocks.internal,
        ],
    );
    table(
        &mut out,
        l.pick(
            "| Building block | Responsibility | Layer | Files | Uses |\n| --- | --- | --- | ---: | --- |",
            "| Baustein | Verantwortung | Schicht | Dateien | Nutzt |\n| --- | --- | --- | ---: | --- |",
        ),
        blocks
            .order
            .iter()
            .take(LIMIT)
            .map(|&i| block_row(&blocks, i))
            .collect(),
        l.pick(
            "No packages recorded in the source graph.",
            "Keine Pakete im Quellgraph erfasst.",
        ),
    );
    note(l, &mut out, blocks.labels.len());
    questions.extend(blocks.missing_question(l));
    questions.push(
        l.pick(
            "Explain the interfaces of the building blocks and, if needed, a finer decomposition.",
            "Schnittstellen der Bausteine und bei Bedarf eine feinere Zerlegung erläutern.",
        )
        .into(),
    );
    out
}

fn block_row(blocks: &Blocks<'_>, i: usize) -> String {
    let used: Vec<_> = blocks.used(i).into_iter().map(cell).collect();
    format!(
        "| {} | {} | {} | {} | {} |",
        cell(&blocks.labels[i]),
        blocks.describe(i).map_or("–".into(), cell),
        blocks.layer_name(i).map_or("–".into(), cell),
        blocks.files[i],
        if used.is_empty() {
            "–".into()
        } else {
            used.join(", ")
        }
    )
}
