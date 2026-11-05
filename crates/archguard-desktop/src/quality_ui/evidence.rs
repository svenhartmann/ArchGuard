//! Cards of the imported-evidence sections: `renderHistoryCard`,
//! `renderTestEvidenceCard`, `renderMutationCard`, `renderDuplicateCard`
//! of `viewer_engineering_quality.js`.

use std::collections::BTreeSet;

use archguard_engine::{DuplicateGroup, QualityHistoryReport};

use super::card::{Builder, Card, Kind, Line};
use super::lang::{Lang, number, percent};
use super::rows::{HistoryRow, MutationRow, TestRow};

fn opt<T: ToString>(v: Option<T>) -> String {
    v.map_or("—".into(), |v| v.to_string())
}

pub fn history(
    lang: Lang,
    nodes: &BTreeSet<String>,
    report: &QualityHistoryReport,
    row: &HistoryRow,
) -> Card {
    let p = row.snapshot;
    let title = match p {
        Some(_) => row.node.clone(),
        None => format!(
            "{} · {}",
            row.node,
            lang.t(
                "absent from latest snapshot",
                "im letzten Snapshot nicht vorhanden"
            )
        ),
    };
    let mut b = Builder::new(lang, nodes, title);
    let revision = report
        .latest
        .as_ref()
        .map_or("unverified", |l| l.producer_revision.as_str());
    let revision = if revision.is_empty() {
        "unverified"
    } else {
        revision
    };
    let files = opt(p.map(|p| p.files));
    let status = |s: Option<&str>| {
        lang.status(s.filter(|s| !s.is_empty()).unwrap_or("unavailable"))
            .to_string()
    };
    b.push(
        Kind::Muted,
        lang.f(
            "Latest snapshot · {files} files · producer revision: {revision}",
            "Letzter Snapshot · {files} Dateien · Producer-Revision: {revision}",
            &[("files", files.clone()), ("revision", revision.into())],
        ),
    )
    .push(
        Kind::Muted,
        lang.f(
            "Coverage: {status} · {known} / {total} files",
            "Abdeckung: {status} · {known} / {total} Dateien",
            &[
                ("status", status(p.map(|p| p.coverage_status.as_str()))),
                ("known", opt(p.map(|p| p.coverage_files))),
                ("total", files),
            ],
        ),
    )
    .push(
        Kind::Muted,
        lang.f(
            "Compiler analysis: {status} · {measured} / {declared} methods measured",
            "Compileranalyse: {status} · {measured} / {declared} Methoden gemessen",
            &[
                ("status", status(p.map(|p| p.semantic_status.as_str()))),
                ("measured", opt(p.map(|p| p.measured_methods))),
                ("declared", opt(p.map(|p| p.declared_methods))),
            ],
        ),
    )
    .push(
        Kind::Muted,
        lang.f(
            "Findings: {status}",
            "Befunde: {status}",
            &[("status", status(p.map(|p| p.findings_status.as_str())))],
        ),
    );
    history_metrics(&mut b, report, row);
    b.locate(Some(&row.node));
    b.done()
}

fn history_metrics(b: &mut Builder, report: &QualityHistoryReport, row: &HistoryRow) {
    let lang = b.lang;
    let p = row.snapshot;
    let delta = report
        .comparison
        .as_ref()
        .and_then(|c| c.modules.iter().find(|m| m.node == row.node));
    let coverage = p
        .and_then(|p| p.line_coverage.as_ref())
        .filter(|c| c.total > 0)
        .map(|c| 100.0 * c.covered as f64 / c.total as f64);
    let metrics = [
        (
            lang.t("Line coverage", "Zeilenabdeckung"),
            percent(coverage),
            delta.map(|d| &d.line_coverage),
        ),
        (
            lang.t("Mean complexity", "Mittlere Komplexität"),
            number(p.and_then(|p| p.mean_complexity)),
            delta.map(|d| &d.mean_complexity),
        ),
        (
            lang.t("Max. complexity", "Max. Komplexität"),
            number(p.and_then(|p| p.max_complexity).map(f64::from)),
            delta.map(|d| &d.max_complexity),
        ),
        (
            lang.t("Active findings", "Aktive Befunde"),
            number(p.and_then(|p| p.active_findings).map(|v| v as f64)),
            delta.map(|d| &d.active_findings),
        ),
    ];
    for (label, value, m) in metrics {
        let change = m.map_or(String::new(), |m| {
            format!(
                " · {} → {} · Δ {} · {}",
                number(m.before),
                number(m.after),
                number(m.delta),
                lang.status(&m.status)
            )
        });
        b.push(Kind::Text, format!("{label}: {value}{change}"));
        if let Some(reason) = m.and_then(|m| m.reason.as_ref()) {
            b.push(Kind::Small, reason);
        }
    }
    if delta.is_none() {
        b.push(
            Kind::Muted,
            lang.t(
                "No comparable previous module measurement.",
                "Keine vergleichbare vorherige Modulmessung.",
            ),
        );
    }
    if let Some(p) = p.filter(|p| !p.extended.is_empty()) {
        extended(b, p, delta);
    }
}

/// `p.extended`: class, method and package measurements with their deltas.
fn extended(
    b: &mut Builder,
    p: &archguard_engine::ModuleQualitySnapshot,
    delta: Option<&archguard_engine::ModuleQualityDelta>,
) {
    let lang = b.lang;
    {
        let lines = p
            .extended
            .iter()
            .map(|(key, m)| {
                let change = delta
                    .and_then(|d| d.extended.get(key))
                    .map_or(String::new(), |c| {
                        format!(" · Δ {} · {}", number(c.delta), lang.status(&c.status))
                    });
                Line::new(
                    Kind::Text,
                    format!(
                        "{key}: {} · {}/{}{change}",
                        number(m.value),
                        m.measured,
                        m.population
                    ),
                )
            })
            .collect();
        b.details(
            lang.t(
                "Class, method and package history",
                "Klassen-, Methoden- und Paketverlauf",
            ),
            lines,
        );
    }
}

pub fn test(lang: Lang, nodes: &BTreeSet<String>, row: &TestRow) -> Card {
    let t = row.test;
    let mut b = Builder::new(lang, nodes, format!("{} · {}", t.suite, t.name));
    b.push(
        Kind::Text,
        format!(
            "{} · {}",
            t.outcome,
            lang.f(
                "Duration: {duration} s",
                "Dauer: {duration} s",
                &[("duration", number(t.duration_seconds))]
            )
        ),
    );
    let verdict = if !row.matched.is_empty() {
        lang.t(
            "Conservative candidate: mapped code is affected by the selected dependency change.",
            "Konservativer Kandidat: zugeordneter Code ist von der gewählten Abhängigkeitsänderung betroffen.",
        )
    } else if row.unknown {
        lang.t(
            "Code mapping unknown or stale; no safe exclusion is possible.",
            "Codezuordnung unbekannt oder veraltet; kein sicherer Ausschluss möglich.",
        )
    } else {
        lang.t(
            "Explicit test-to-code mapping.",
            "Explizite Test-zu-Code-Zuordnung.",
        )
    };
    b.push(Kind::Muted, verdict);
    for m in t.mappings.iter().take(20) {
        let lines: Vec<String> = m.lines.iter().take(10).map(u32::to_string).collect();
        let at = if lines.is_empty() {
            String::new()
        } else {
            format!(":{}", lines.join(","))
        };
        b.push(Kind::Path, format!("{}{at} · {}", m.path, m.status));
        b.locate(m.node.as_deref());
    }
    if t.mappings.len() > 20 {
        b.push(
            Kind::Muted,
            lang.t(
                "Additional mappings remain in the export.",
                "Weitere Zuordnungen stehen im Export.",
            ),
        );
    }
    if let Some(message) = &t.message {
        b.push(Kind::Pre, message);
    }
    b.done()
}

pub fn mutation(lang: Lang, nodes: &BTreeSet<String>, row: &MutationRow) -> Card {
    let (file, m) = (row.file, row.mutant);
    let mut b = Builder::new(
        lang,
        nodes,
        format!("{}:{} · {}", file.path, m.location.start.line, m.status),
    );
    b.push(
        Kind::Text,
        format!(
            "{} · {}",
            m.mutator_name,
            lang.f(
                "{count} distinct incoming files",
                "{count} unterschiedliche eingehende Dateien",
                &[("count", row.incoming.to_string())]
            )
        ),
    )
    .push(Kind::Muted, &file.status)
    .push(
        Kind::Pre,
        format!(
            "{} → {}",
            m.original,
            m.replacement.as_deref().unwrap_or("")
        ),
    );
    if m.original_truncated {
        b.push(
            Kind::Small,
            lang.t(
                "Original source excerpt shortened.",
                "Ursprünglicher Quellausschnitt gekürzt.",
            ),
        );
    }
    if let Some(d) = &m.description {
        b.push(Kind::Text, d);
    }
    b.locate(file.node.as_deref());
    b.done()
}

pub fn duplicate(lang: Lang, nodes: &BTreeSet<String>, group: &DuplicateGroup) -> Card {
    let title = lang.f(
        "Duplicate block · {tokens} tokens",
        "Duplikatblock · {tokens} Tokens",
        &[("tokens", group.tokens.to_string())],
    );
    let mut b = Builder::new(lang, nodes, title);
    b.push(
        Kind::Text,
        format!(
            "{} · {}",
            lang.status(&group.status),
            lang.f(
                "{lines} reported lines · {count} locations",
                "{lines} gemeldete Zeilen · {count} Fundstellen",
                &[
                    ("lines", group.lines.to_string()),
                    ("count", group.occurrences.len().to_string())
                ]
            )
        ),
    );
    for o in &group.occurrences {
        b.push(
            Kind::Path,
            format!(
                "{}:{}–{} · {}",
                o.path,
                o.line,
                o.end_line,
                lang.status(&o.status)
            ),
        );
        b.locate(o.node.as_deref());
    }
    if !group.snippet.is_empty() {
        b.push(Kind::Pre, &group.snippet);
    }
    if group.snippet_truncated {
        b.push(Kind::Small, lang.t(
            "Producer excerpt shortened to 4096 characters during import; use the original CPD report for the full fragment.",
            "Producer-Ausschnitt beim Import auf 4096 Zeichen gekürzt; der vollständige Ausschnitt steht im ursprünglichen CPD-Bericht.",
        ));
    }
    b.done()
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::DuplicateOccurrence;

    #[test]
    fn duplicate_card_counts_tokens_lines_and_locations() {
        let at = |path: &str, line| DuplicateOccurrence {
            path: path.into(),
            node: Some(path.into()),
            line,
            end_line: line + 9,
            column: None,
            end_column: None,
            source_sha256: None,
            status: "unchanged_since_import".into(),
        };
        let group = DuplicateGroup {
            id: "g1".into(),
            lines: 10,
            tokens: 120,
            status: "unchanged_since_import".into(),
            occurrences: vec![at("a/x.rs", 3), at("b/z.rs", 40)],
            snippet: "fn same() {}".into(),
            snippet_truncated: false,
            fragment_sha256: String::new(),
        };
        let nodes = BTreeSet::from(["a/x.rs".to_string()]);
        let card = duplicate(Lang::default(), &nodes, &group);
        assert_eq!(card.title, "Duplicate block · 120 tokens");
        assert_eq!(
            card.items[0].text,
            "unchanged since import · 10 reported lines · 2 locations"
        );
        assert_eq!(card.items[1].text, "a/x.rs:3–12 · unchanged since import");
        assert_eq!(
            (card.items[2].kind, card.items[2].node.as_str()),
            (Kind::Locate, "a/x.rs")
        );
        assert_eq!(card.items[3].text, "b/z.rs:40–49 · unchanged since import");
        assert_eq!(card.items[4].text, "fn same() {}");
    }
}
