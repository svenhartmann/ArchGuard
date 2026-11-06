//! Cards of the code-metric and Git sections: `renderClassMetricsCard`,
//! `renderMethodMetricsCard`, `renderKnowledgeCard`,
//! `renderMeasurementCard` of `viewer_engineering_quality.js`.

use std::collections::BTreeSet;

use archguard_engine::{ClassMetrics, KnowledgeMetric, MeasurementGroup, MethodMetrics};

use super::card::{Builder, Card, Kind, Line};
use super::heatmap::{METRICS, not_applicable, value};
use super::lang::{number, percent};
use crate::tab_support::Texts;

/// Evidence per metric is listed up to this many references.
const EVIDENCE: usize = 100;

fn evidence<'a>(row: &'a ClassMetrics, key: &str) -> &'a [String] {
    match key {
        "cbo" => &row.cbo.evidence,
        "rfc" => &row.rfc.evidence,
        "dit" => &row.dit.evidence,
        "noc" => &row.noc.evidence,
        "wmc" => &row.wmc.evidence,
        _ => &[],
    }
}

pub fn class(lang: &Texts, nodes: &BTreeSet<String>, row: &ClassMetrics) -> Card {
    let mut b = Builder::new(lang, nodes, &row.class_id);
    b.card.key = row.class_id.clone();
    b.card.metrics = METRICS
        .iter()
        .map(|k| {
            let v = if not_applicable(row, k) {
                lang.t("n/a").to_string()
            } else {
                number(lang, value(row, k))
            };
            format!("{}: {v}", k.to_uppercase())
        })
        .collect();
    b.push(Kind::Small, &row.file).push(Kind::Metrics, "");
    b.locate(Some(&row.node));
    let mut lines = Vec::new();
    for key in METRICS.iter().filter(|&&k| k != "lcom4") {
        lines.push(Line::new(Kind::Heading, key.to_uppercase()));
        let refs = evidence(row, key);
        lines.extend(refs.iter().take(EVIDENCE).map(|r| Line::new(Kind::Path, r)));
        if refs.len() > EVIDENCE {
            let note = lang.t("First 100; export contains all evidence.");
            lines.push(Line::new(Kind::Text, note));
        }
    }
    b.details(lang.t("Dependency evidence"), lines);
    b.done()
}

/// A method id (`rust:a/x.rs::Store#open@12`) as a title: owner and
/// name, without language, path and line; module-level functions by name.
pub fn method_title(id: &str) -> String {
    let Some((owner, rest)) = id.rsplit_once('#') else {
        return id.to_string();
    };
    let name = rest.split('@').next().unwrap_or(rest);
    let (owner, sep) = match owner.rsplit_once("::") {
        Some((_, last)) => (last, "::"),
        None => (owner.rsplit('.').next().unwrap_or(owner), "."),
    };
    let owner = owner.rsplit(':').next().unwrap_or(owner);
    if owner.is_empty() || owner == "(module)" {
        return name.to_string();
    }
    format!("{owner}{sep}{name}")
}

pub fn method(lang: &Texts, nodes: &BTreeSet<String>, row: &MethodMetrics) -> Card {
    let mut b = Builder::new(lang, nodes, method_title(&row.method_id));
    b.card.full = row.method_id.clone();
    b.push(Kind::Small, format!("{}:{}", row.file, row.line))
        .push(
            Kind::Text,
            format!("Cognitive: {}", number(lang, row.cognitive.map(f64::from))),
        )
        .push(
            Kind::Text,
            format!("MI: {}", number(lang, row.maintainability_index)),
        );
    let h = row.halstead.as_ref();
    let lines = vec![
        Line::new(Kind::Text, format!("{}: {}", lang.t("Volume"), number(lang, h.map(|h| h.volume)))),
        Line::new(
            Kind::Text,
            format!("{}: {}", lang.t("Difficulty"), number(lang, h.and_then(|h| h.difficulty))),
        ),
        Line::new(Kind::Text, format!("{}: {}", lang.t("Effort"), number(lang, h.and_then(|h| h.effort)))),
        Line::new(
            Kind::Text,
            lang.f("Source lines: {lines} · source-maintainability/v1. Compare the same language and measurement definition.",
                &[("lines", row.source_lines.map_or("—".into(), |l| l.to_string()))],
            ),
        ),
    ];
    b.details(lang.t("Halstead and measurement rules"), lines);
    b.done()
}

pub fn knowledge(lang: &Texts, nodes: &BTreeSet<String>, row: &KnowledgeMetric) -> Card {
    let mut b = Builder::new(lang, nodes, &row.node);
    let share = row.top_share.map(|s| s * 100.0);
    b.push(Kind::Small, lang.t("Contribution concentration"))
        .push(
            Kind::Text,
            lang.f(
                "Bus factor 50: {factor} · largest share {share}",
                &[
                    ("factor", number(lang, row.bus_factor_50.map(|f| f as f64))),
                    ("share", percent(lang, share)),
                ],
            ),
        )
        .bar(row.top_share.filter(|s| s.is_finite()).unwrap_or(0.0))
        .push(
            Kind::Text,
            lang.f(
                "{people} contributors · {touches} file changes",
                &[
                    ("people", row.contributors.to_string()),
                    ("touches", row.touches.to_string()),
                ],
            ),
        );
    let mut lines = vec![Line::new(Kind::Text, lang.t("Smallest number of authors responsible for at least 50% of file/commit touches in the selected Git window. A concentration heuristic; no claim about actual expertise or availability. Shallow history leaves the factor unknown.",
    ))];
    lines.extend(
        row.authors
            .iter()
            .take(50)
            .map(|a| Line::new(Kind::Text, format!("{} · {}", a.author, a.touches))),
    );
    b.details(lang.t("Contributors and interpretation"), lines);
    b.locate(Some(&row.node));
    b.done()
}

fn kind_label(lang: &Texts, kind: &str) -> String {
    match kind {
        "test" => lang.t("Repeated test").into(),
        "build" => lang.t("Build duration").into(),
        "runtime" => lang.t("Runtime measurement").into(),
        _ => kind.into(),
    }
}

pub fn measurement(lang: &Texts, nodes: &BTreeSet<String>, row: &MeasurementGroup) -> Card {
    let mut b = Builder::new(lang, nodes, &row.name);
    b.push(Kind::Small, kind_label(lang, &row.kind));
    if row.kind == "test" {
        let verdict = match row.inconsistent {
            Some(true) => lang.t("Mixed results in the same context"),
            Some(false) => lang.t("Consistent observed results"),
            None => lang.t("Too few runs to assess stability"),
        };
        b.push(Kind::Text, verdict);
    }
    b.push(
        Kind::Text,
        format!(
            "Median: {} s · p95: {} s · n={}",
            number(lang, row.median_seconds),
            number(lang, row.p95_seconds),
            row.duration_samples
        ),
    )
    .push(
        Kind::Text,
        lang.f(
            "{passed} passed · {failed} failed · {skipped} skipped",
            &[
                ("passed", row.passed.to_string()),
                ("failed", row.failed.to_string()),
                ("skipped", row.skipped.to_string()),
            ],
        ),
    );
    let mut lines: Vec<Line> = [&row.revision, &row.environment, &row.producer]
        .iter()
        .map(|v| Line::new(Kind::Path, v))
        .collect();
    lines.push(Line::new(Kind::Text, lang.t("Producer-declared context, unverified. Durations use successful runs. Different revisions and environments remain separate; mixed results indicate instability, not its cause.",
    )));
    b.details(lang.t("Measurement context"), lines);
    b.done()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality_ui::heatmap::tests::{class as class_row, count};

    fn texts(card: &Card) -> Vec<String> {
        card.items.iter().map(|i| i.text.clone()).collect()
    }

    #[test]
    fn class_card_shows_values_unknowns_and_evidence() {
        let nodes = BTreeSet::from(["a/x.rs".to_string()]);
        let mut row = class_row("rust:a/x.rs::A", "a/x.rs", None, Some(2));
        row.wmc = count(Some(11), "ready");
        row.dit = count(None, "not_applicable");
        row.cbo.evidence = vec!["a/x.rs:3 B".into()];
        let card = class(&Texts::default(), &nodes, &row);
        assert_eq!(card.title, "rust:a/x.rs::A");
        assert_eq!(
            card.metrics,
            [
                "CBO: —", "RFC: —", "DIT: n/a", "NOC: —", "WMC: 11", "LCOM4: 2"
            ]
        );
        assert_eq!(texts(&card)[0], "a/x.rs");
        assert!(
            card.items
                .iter()
                .any(|i| i.kind == Kind::Locate && i.node == "a/x.rs")
        );
        let evidence = card
            .items
            .iter()
            .find(|i| i.kind == Kind::Details)
            .expect("evidence");
        assert_eq!(evidence.text, "Dependency evidence");
        assert_eq!(evidence.lines[1].text, "a/x.rs:3 B");
        let de = class(&Texts::german(), &nodes, &row);
        assert_eq!(de.metrics[2], "DIT: n. a.");
    }

    #[test]
    fn knowledge_card_reads_as_in_the_web() {
        let row = KnowledgeMetric {
            node: "a/x.rs".into(),
            touches: 4,
            contributors: 2,
            top_share: Some(0.75),
            bus_factor_50: Some(1),
            authors: vec![archguard_engine::Contributor {
                author: "dev@example.org".into(),
                touches: 3,
            }],
            status: "ready".into(),
        };
        let nodes = BTreeSet::new();
        let card = knowledge(&Texts::default(), &nodes, &row);
        assert_eq!(
            texts(&card)[..4],
            [
                "Contribution concentration",
                "Bus factor 50: 1 · largest share 75.0%",
                "",
                "2 contributors · 4 file changes"
            ]
        );
        assert_eq!(card.items[2].value, 0.75);
        assert!(
            !card.items.iter().any(|i| i.kind == Kind::Locate),
            "not in the graph"
        );
    }

    #[test]
    fn method_card_lists_cognitive_and_mi() {
        let row = MethodMetrics {
            method_id: "A#run".into(),
            node: None,
            class_id: "A".into(),
            file: "a/x.rs".into(),
            line: 7,
            cognitive: Some(4),
            halstead: None,
            source_lines: Some(12),
            maintainability_index: Some(61.234),
        };
        let card = method(&Texts::default(), &BTreeSet::new(), &row);
        assert_eq!(texts(&card)[..3], ["a/x.rs:7", "Cognitive: 4", "MI: 61.23"]);
    }

    #[test]
    fn method_titles_read_as_owner_and_name() {
        assert_eq!(
            method_title("rust:crates/a/src/lib.rs::(module)#collect_messaging@300"),
            "collect_messaging"
        );
        assert_eq!(
            method_title("rust:crates/a/src/lib.rs::Store#open@12"),
            "Store::open"
        );
        assert_eq!(
            method_title("java:com.shop.OrderService#place@40"),
            "OrderService.place"
        );
        assert_eq!(method_title("A#run"), "A.run");
        assert_eq!(method_title("no-marker"), "no-marker");
        let row = MethodMetrics {
            method_id: "rust:a/x.rs::(module)#run@7".into(),
            node: None,
            class_id: "A".into(),
            file: "a/x.rs".into(),
            line: 7,
            cognitive: None,
            halstead: None,
            source_lines: None,
            maintainability_index: None,
        };
        let card = method(&Texts::default(), &BTreeSet::new(), &row);
        assert_eq!(card.title, "run");
        assert_eq!(card.full, "rust:a/x.rs::(module)#run@7");
    }
}
