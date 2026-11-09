//! arc42 chapters 10 to 12, one function per section, in the document's
//! language (ADR 0008).

use serde_json::Value;
use std::fmt::Write;

use crate::arc42_blocks::Arc42Inputs;
use crate::arc42_chapters::source;
use crate::arc42_text::{LIMIT, cell, fill, note, rows, status, table, val};

/// Chapter 10: metrics, goal assessment, measurements and imports.
pub(crate) fn requirements(input: &Arc42Inputs<'_>, md: &mut String, q: &mut Vec<String>) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    dependency_metrics(input, md);
    code_coverage(input, md);
    class_values(input, md);
    method_values(input, md);
    imported_reports(input, md);
    run_measurements(input, md);
    q.push(
        t(
            "Set measurable quality scenarios with context, trigger, response and acceptance limit. Do not derive runtime goals from static metrics.",
            "Messbare Qualitätsszenarien mit Kontext, Auslöser, Reaktion und Akzeptanzgrenze festlegen. Laufzeitziele nicht aus statischen Kennzahlen ableiten.",
        )
        .into(),
    );
}

/// Chapter 10: Lakos metrics, goal assessment and check criteria.
pub(crate) fn dependency_metrics(input: &Arc42Inputs<'_>, md: &mut String) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { report, config, .. } = input;
    let s = &report.summary;
    let text = t(
        "Quality goals: see chapter 1. Measurements are observations, not requirements set automatically.\n\n### Dependency metrics\n\nSource: analysis report of the scope named in chapter 5.\n\n| Metric | Value |\n| --- | ---: |\n| CCD | {} |\n| ACD | {} |\n| NCCD | {} |\n| Cyclic groups | {} |\n| Largest cyclic group | {} |\n",
        "Qualitätsziele siehe Kapitel 1. Messwerte sind Beobachtungen, keine automatisch festgelegten Anforderungen.\n\n### Abhängigkeitskennzahlen\n\nQuelle: Analysebericht des in Kapitel 5 genannten Umfangs.\n\n| Kennzahl | Wert |\n| --- | ---: |\n| CCD | {} |\n| ACD | {} |\n| NCCD | {} |\n| Zyklische Gruppen | {} |\n| Größte zyklische Gruppe | {} |\n",
    );
    let (acd, nccd) = (format!("{:.3}", s.acd), format!("{:.3}", s.nccd));
    let _ = writeln!(
        md,
        "{}",
        fill(text, &[&s.ccd, &acd, &nccd, &s.sccs, &s.largest_scc])
    );
    md.push_str(&crate::arc42_goals::assessment_section(
        l,
        config,
        input.goals,
        LIMIT,
        cell,
    ));
    let text = t(
        "Configured check criteria: cycles forbidden {}, maximum SCC size {}, ACD limit {} (0 = off). Source: `rules`.\n",
        "Konfigurierte Prüfkriterien: Zyklen verboten {}, maximale SCC-Größe {}, ACD-Grenze {} (0 = deaktiviert). Quelle: `rules`.\n",
    );
    let deny = match config.rules.deny_cycles {
        archguard_core::DenyCycles::Module => t("between modules", "zwischen Modulen"),
        archguard_core::DenyCycles::File => t("also between files", "auch zwischen Dateien"),
        archguard_core::DenyCycles::Off => t("no", "nicht"),
    };
    let _ = writeln!(
        md,
        "{}",
        fill(
            text,
            &[&deny, &config.rules.max_scc_size, &config.rules.max_acd]
        )
    );
}

/// Chapter 10: what the source code measurement covers.
pub(crate) fn code_coverage(input: &Arc42Inputs<'_>, md: &mut String) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { insights, .. } = input;
    let code = &insights["code_metrics"];
    let text = t(
        "### Source code measurement and coverage\n\nStatus: **{}** · definition: {}. Values that do not apply or are unknown are not read as 0.\n",
        "### Quellcodemessung und Abdeckung\n\nStatus: **{}** · Definition: {}. Nicht anwendbare oder unbekannte Werte werden nicht als 0 interpretiert.\n",
    );
    let _ = writeln!(
        md,
        "{}",
        fill(
            text,
            &[&status(l, &code["status"]), &val(l, &code["definition"])]
        )
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
                val(l, &counts[0]),
                val(l, &counts[1])
            )
        })
        .collect();
    table(
        md,
        t(
            "| Metric | Measured | Declarations in total |\n| --- | ---: | ---: |",
            "| Metrik | Gemessen | Deklarationen insgesamt |\n| --- | ---: | ---: |",
        ),
        coverage,
        t("No declarations measured.", "Keine Deklarationen gemessen."),
    );
}

/// Chapter 10: measured class values.
pub(crate) fn class_values(input: &Arc42Inputs<'_>, md: &mut String) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { insights, .. } = input;
    let code = &insights["code_metrics"];
    md.push_str(t(
        "### Measured class values\n\n",
        "### Gemessene Klassenwerte\n\n",
    ));
    table(
        md,
        t(
            "| Class | CBO | RFC | DIT | NOC | WMC | LCOM4 |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |",
            "| Klasse | CBO | RFC | DIT | NOC | WMC | LCOM4 |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |",
        ),
        rows(&code["classes"])
            .iter()
            .take(LIMIT)
            .map(|row| {
                format!(
                    "| {} | {} | {} | {} | {} | {} | {} |",
                    val(l, &row["class_id"]),
                    val(l, &row["cbo"]["value"]),
                    val(l, &row["rfc"]["value"]),
                    val(l, &row["dit"]["value"]),
                    val(l, &row["noc"]["value"]),
                    val(l, &row["wmc"]["value"]),
                    val(l, &row["lcom4"])
                )
            })
            .collect(),
        t("No class values measured.", "Keine Klassenwerte gemessen."),
    );
    note(l, md, rows(&code["classes"]).len());
}

/// Chapter 10: method maintainability.
pub(crate) fn method_values(input: &Arc42Inputs<'_>, md: &mut String) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { insights, .. } = input;
    let code = &insights["code_metrics"];
    md.push_str(t(
        "### Method maintainability\n\nBy cognitive value, highest first. Cognitive and Halstead use the versioned ArchGuard source/AST counting rules; MI uses this Halstead volume.\n\n",
        "### Methodenwartbarkeit\n\nNach Cognitive-Wert absteigend. Cognitive und Halstead verwenden die versionierten ArchGuard-Quellcode-/AST-Zählregeln; MI verwendet dieses Halstead-Volumen.\n\n",
    ));
    let mut methods: Vec<_> = rows(&code["methods"]).iter().collect();
    methods.sort_by(|a, b| {
        b["cognitive"]
            .as_u64()
            .cmp(&a["cognitive"].as_u64())
            .then_with(|| a["method_id"].as_str().cmp(&b["method_id"].as_str()))
    });
    table(
        md,
        t(
            "| Method | Cognitive | Halstead volume | MI | Source evidence |\n| --- | ---: | ---: | ---: | --- |",
            "| Methode | Cognitive | Halstead-Volumen | MI | Quellbeleg |\n| --- | ---: | ---: | ---: | --- |",
        ),
        methods
            .iter()
            .take(LIMIT)
            .map(|row| {
                format!(
                    "| {} | {} | {} | {} | {} |",
                    val(l, &row["method_id"]),
                    val(l, &row["cognitive"]),
                    val(l, &row["halstead"]["volume"]),
                    val(l, &row["maintainability_index"]),
                    source(
                        input,
                        row["file"].as_str().unwrap_or(""),
                        row["line"].as_u64().unwrap_or(0) as u32
                    )
                )
            })
            .collect(),
        t(
            "No method values measured.",
            "Keine Methodenwerte gemessen.",
        ),
    );
    note(l, md, methods.len());
}

/// Chapter 10: imported reports and test coverage per file.
pub(crate) fn imported_reports(input: &Arc42Inputs<'_>, md: &mut String) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { insights, .. } = input;
    md.push_str(t(
        "### Imported reports\n\nCheck the freshness and measured scope of the saved reports separately.\n\n",
        "### Importierte Berichte\n\nAktualität und Messumfang der gespeicherten Berichte sind separat zu prüfen.\n\n",
    ));
    for (key, label) in [
        ("test_coverage", t("Test coverage", "Testabdeckung")),
        (
            "measurement_runs",
            t(
                "Build/runtime/test measurements",
                "Build-/Laufzeit-/Testmessungen",
            ),
        ),
        ("mutations", t("Mutation tests", "Mutationstests")),
        ("duplicates", t("Duplicates", "Duplikate")),
    ] {
        let _ = writeln!(
            md,
            "- {label}: **{}** (`{key}`)",
            status(l, &insights[key]["status"])
        );
    }
    md.push('\n');
    let files = rows(&insights["test_coverage"]["files"]);
    if !files.is_empty() {
        md.push_str(t(
            "### Test coverage per file\n\n",
            "### Testabdeckung je Datei\n\n",
        ));
    }
    table(
        md,
        t(
            "| Coverage file | Source status | Reported covered lines | Reported executable lines |\n| --- | --- | ---: | ---: |",
            "| Coverage-Datei | Quellstatus | Berichtete gedeckte Zeilen | Berichtete ausführbare Zeilen |\n| --- | --- | ---: | ---: |",
        ),
        files
            .iter()
            .take(LIMIT)
            .map(|row| {
                format!(
                    "| {} | {} | {} | {} |",
                    val(l, &row["path"]),
                    status(l, &row["status"]),
                    val(l, &row["lines"]["covered"]),
                    val(l, &row["lines"]["total"])
                )
            })
            .collect(),
        "",
    );
    note(l, md, files.len());
}

/// Chapter 10: imported run measurements.
pub(crate) fn run_measurements(input: &Arc42Inputs<'_>, md: &mut String) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { insights, .. } = input;
    let groups = rows(&insights["measurement_runs"]["groups"]);
    if !groups.is_empty() {
        md.push_str(t(
            "### Imported run measurements\n\nOnly the same revision, environment and producer form a measurement group. Median/p95 are seconds of successful runs; changing test results are a hint, not a root cause analysis.\n\n",
            "### Importierte Laufmessungen\n\nNur gleichartige Revision, Umgebung und Producer bilden eine Messgruppe. Median/p95 sind Sekunden erfolgreicher Läufe; wechselnde Testergebnisse sind ein Hinweis, keine Ursachenanalyse.\n\n",
        ));
    }
    table(
        md,
        t(
            "| Measurement | Revision / environment / producer | Successful timings | Median | p95 | Changing results |\n| --- | --- | ---: | ---: | ---: | --- |",
            "| Messung | Revision / Umgebung / Producer | Erfolgreiche Zeitmessungen | Median | p95 | Wechselnde Ergebnisse |\n| --- | --- | ---: | ---: | ---: | --- |",
        ),
        groups
            .iter()
            .take(LIMIT)
            .map(|row| {
                format!(
                    "| {} · {} | {} / {} / {} | {} | {} | {} | {} |",
                    val(l, &row["kind"]),
                    val(l, &row["name"]),
                    val(l, &row["revision"]),
                    val(l, &row["environment"]),
                    val(l, &row["producer"]),
                    val(l, &row["duration_samples"]),
                    val(l, &row["median_seconds"]),
                    val(l, &row["p95_seconds"]),
                    val(l, &row["inconsistent"])
                )
            })
            .collect(),
        "",
    );
    note(l, md, groups.len());
}

/// Chapter 11: findings to review.
pub(crate) fn risks(input: &Arc42Inputs<'_>, md: &mut String, q: &mut Vec<String>) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    findings(input, md);
    imported_findings(input, md);
    q.push(
        t(
            "Assess risk impact, owners, priority, accepted debt and measures with the team.",
            "Risikoauswirkung, Verantwortliche, Priorität, akzeptierte Schulden und Maßnahmen mit dem Team bewerten.",
        )
        .into(),
    );
}

/// Chapter 11: the analysis report's findings.
pub(crate) fn findings(input: &Arc42Inputs<'_>, md: &mut String) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { report, .. } = input;
    md.push_str(t(
        "### Findings to review\n\nSource: analysis report. Static findings are not automatically accepted technical debt. Missing findings do not prove the absence of defects; for compiler coverage see chapters 6 and 10.\n\n",
        "### Befunde zur Prüfung\n\nQuelle: Analysebericht. Statische Befunde sind keine automatisch akzeptierten technischen Schulden. Fehlende Befunde beweisen keine Fehlerfreiheit; Compilerabdeckung siehe Kapitel 6 und 10.\n\n",
    ));
    let mut findings: Vec<_> = report.findings.iter().collect();
    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let ident = |value: Value| value.as_str().map(str::to_owned).unwrap_or_default();
    for f in findings.iter().take(LIMIT) {
        let _ = writeln!(
            md,
            "- {} · `{}` · {} · {} · {}",
            cell(&f.id),
            ident(serde_json::to_value(f.kind).unwrap_or_default()),
            match f.severity {
                archguard_core::Severity::Error => t("Error", "Fehler"),
                archguard_core::Severity::Warning => t("Warning", "Warnung"),
                archguard_core::Severity::Info => t("Info", "Hinweis"),
            },
            cell(&f.nodes.join(", ")),
            cell(f.message.as_deref().unwrap_or(""))
        );
        for e in f.evidence.iter().take(2) {
            let _ = writeln!(
                md,
                "  - {} {}:{}",
                t("Source", "Quelle"),
                cell(&e.file),
                e.line
            );
        }
    }
    note(l, md, findings.len());
    if findings.is_empty() {
        md.push_str(t(
            "The analysis reported no findings.\n",
            "Die Analyse hat keine Befunde gemeldet.\n",
        ));
    }
}

/// Chapter 11: imported quality and security findings.
pub(crate) fn imported_findings(input: &Arc42Inputs<'_>, md: &mut String) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    let Arc42Inputs { insights, .. } = input;
    let line = t(
        "- {} · {} · {} · source status {} · suppressed {} · baseline {}",
        "- {} · {} · {} · Quellstatus {} · unterdrückt {} · Baseline {}",
    );
    for (key, label) in [
        (
            "quality",
            t("Imported quality findings", "Importierte Qualitätsbefunde"),
        ),
        ("security", t("Security findings", "Sicherheitsbefunde")),
    ] {
        let model = &insights[key];
        let _ = writeln!(
            md,
            "\n### {label}\n\n{}: **{}**.",
            t("Import status", "Importstatus"),
            status(l, &model["status"])
        );
        let found = rows(&model["findings"]);
        if !found.is_empty() {
            md.push_str(t(
                "Listed entries keep their original status; stale or suppressed entries are not automatically active problems.\n\n",
                "Aufgelistete Einträge behalten ihren ursprünglichen Status; veraltete oder unterdrückte Einträge sind nicht automatisch aktive Probleme.\n\n",
            ));
        }
        for f in found.iter().take(LIMIT) {
            let _ = writeln!(
                md,
                "{}",
                fill(
                    line,
                    &[
                        &val(l, &f["id"]),
                        &val(l, f.get("severity").unwrap_or(&f["level"])),
                        &val(l, f.get("message").unwrap_or(&f["title"])),
                        &status(l, &f["status"]),
                        &val(l, &f["suppressed"]),
                        &status(l, &f["baseline_state"]),
                    ]
                )
            );
        }
        note(l, md, found.len());
    }
}

/// Chapter 12: the terms of this document.
pub(crate) fn glossary(input: &Arc42Inputs<'_>, md: &mut String, q: &mut Vec<String>) {
    let l = input.lang;
    let t = |en: &'static str, de: &'static str| l.pick(en, de);
    md.push_str(t(
        "| Term | Meaning in this document |\n| --- | --- |\n| SCC | Group of mutually reachable nodes; a cyclic group is not a list of all single cycles. |\n| CCD / ACD | Sum / average of transitively reachable nodes including the start node. |\n| NCCD | CCD relative to the balanced tree reference used. |\n| CBO / RFC | Recorded class target coupling / own methods plus direct call targets. |\n| DIT / NOC | Compiler-known base class depth / direct subclasses within the source scope. |\n| WMC / LCOM4 | Summed method complexity / groups of explicit instance methods by state and calls. |\n| Cognitive / Halstead / MI | Versioned source or AST measurements; no runtime measurement and no promised tool equivalence. |\n| Unknown | No sufficient basis for a measurement; explicitly not zero. |\n",
        "| Begriff | Bedeutung in diesem Dokument |\n| --- | --- |\n| SCC | Gruppe gegenseitig erreichbarer Knoten; eine zyklische Gruppe ist keine Liste aller Einzelzyklen. |\n| CCD / ACD | Summe / Durchschnitt transitiv erreichbarer Knoten einschließlich des Ausgangsknotens. |\n| NCCD | CCD im Verhältnis zur verwendeten balancierten Baumreferenz. |\n| CBO / RFC | Erfasste Klassenzielkopplung / eigene Methoden plus direkte Aufrufziele. |\n| DIT / NOC | Compilerbekannte Basisklassentiefe / direkte Unterklassen im Quellumfang. |\n| WMC / LCOM4 | Summierte Methodenkomplexität / Gruppen expliziter Instanzmethoden anhand Zustand und Aufrufen. |\n| Cognitive / Halstead / MI | Versionierte Quellcode- bzw. AST-Messungen; keine Laufzeitmessung oder zugesicherte Werkzeuggleichheit. |\n| Unbekannt | Keine ausreichende Messgrundlage; ausdrücklich kein Nullwert. |\n",
    ));
    q.push(
        t(
            "Add the project's technical terms and abbreviations with agreed definitions.",
            "Fachbegriffe und Abkürzungen des Projekts mit abgestimmten Definitionen ergänzen.",
        )
        .into(),
    );
}
