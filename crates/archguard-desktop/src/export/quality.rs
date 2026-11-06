//! Quality snapshot (`engineeringExport`) and `engineeringMarkdown` of
//! `viewer_engineering_quality.js`: every loaded record, independent of
//! the tab's filters.

use archguard_engine::{
    CodeMetricsReport, CohesionReport, DuplicationReport, KnowledgeReport, MeasurementReport,
    MutationReport, QualityHistoryReport, TestEvidenceReport,
};
use serde_json::{Value, json};

use super::cells::{Lines, cell, text, vcell};
use super::snapshot::{Snapshot, add_filters, base, extend};

/// The Quality tab's reports (`quality_ui::inputs::Inputs`).
pub struct QualityInput<'a> {
    pub code: &'a CodeMetricsReport,
    /// `None` when Git could not be read.
    pub knowledge: Option<&'a KnowledgeReport>,
    pub measurements: &'a MeasurementReport,
    pub cohesion: &'a CohesionReport,
    pub history: &'a QualityHistoryReport,
    pub tests: &'a TestEvidenceReport,
    pub mutations: &'a MutationReport,
    pub duplicates: &'a DuplicationReport,
}

/// The tab's settings under the web's names (`engineeringUi`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QualityFilters {
    pub section: String,
    /// Selected graph node, if any.
    pub target: Option<String>,
    pub query: String,
}

pub fn quality_snapshot(project: &str, q: &QualityInput, f: &QualityFilters) -> Snapshot {
    let mut map = base(project, "quality");
    extend(
        &mut map,
        json!({"kind": "quality", "coverage": "Original source measurements; all loaded records, independent of filters and simulation."}),
    );
    add_filters(
        &mut map,
        json!({"section": f.section, "target": f.target, "query": f.query}),
    );
    extend(
        &mut map,
        json!({
            "code_metrics": q.code, "knowledge_metrics": q.knowledge, "measurement_runs": q.measurements,
            "cohesion": q.cohesion, "quality_history": q.history, "test_evidence": q.tests,
            "mutations": q.mutations, "duplicates": q.duplicates,
        }),
    );
    Snapshot(Value::Object(map))
}

/// `engineeringNumber`: two decimals at most, "—" for no number.
fn number(v: &Value) -> String {
    match v.as_f64() {
        Some(f) if f.is_finite() => {
            let s = format!("{f:.2}");
            let s = s.trim_end_matches('0').trim_end_matches('.');
            if s == "-0" { "0".into() } else { s.into() }
        }
        _ => "—".into(),
    }
}

/// `engineeringPercent`.
fn percent(v: Option<f64>) -> String {
    v.filter(|f| f.is_finite())
        .map_or("—".into(), |f| format!("{f:.1}%"))
}

fn numbers(values: &[&Value]) -> String {
    values
        .iter()
        .map(|v| number(v))
        .collect::<Vec<_>>()
        .join(" / ")
}

fn items<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v[key].as_array().into_iter().flatten()
}

fn push(lines: &mut Lines, parts: &[String]) {
    lines.extend(parts.iter().cloned());
}

/// A section header with status, source paths and diagnostics.
fn section<'a>(
    lines: &mut Lines,
    s: &'a Value,
    key: &str,
    title: &str,
    missing: &str,
) -> Option<&'a Value> {
    let source = &s[key];
    let status = source["status"]
        .as_str()
        .filter(|t| !t.is_empty())
        .unwrap_or(missing);
    push(
        lines,
        &[
            String::new(),
            format!("## {title}"),
            String::new(),
            format!("Status: {}", cell(status)),
        ],
    );
    for (k, label) in [
        ("report_path", "Report"),
        ("mapping_path", "Source mapping"),
        ("imported_at_unix", "Imported at (Unix seconds)"),
    ] {
        if !source[k].is_null() {
            lines.push(format!("{label}: {}", vcell(&source[k])));
        }
    }
    for note in items(source, "diagnostics") {
        push(lines, &[String::new(), vcell(note)]);
    }
    (!source.is_null()).then_some(source)
}

/// `engineeringMarkdown`.
pub(super) fn markdown(s: &Value) -> Lines {
    let mut lines = vec![
        format!("# Software quality: {}", vcell(&s["project"])),
        String::new(),
        "Original analysis and imported evidence. Graph simulations do not change measurements."
            .into(),
    ];
    if let Some(c) = s["coverage"].as_str() {
        push(&mut lines, &[String::new(), cell(c)]);
    }
    code(&mut lines, s);
    measurements(&mut lines, s);
    history(&mut lines, s);
    evidence(&mut lines, s);
    duplicates(&mut lines, s);
    lines
}

fn code(lines: &mut Lines, s: &Value) {
    if let Some(code) = section(
        lines,
        s,
        "code_metrics",
        "Class and method metrics",
        "unavailable",
    ) {
        for r in items(code, "classes") {
            let metrics = numbers(&[
                &r["cbo"]["value"],
                &r["rfc"]["value"],
                &r["dit"]["value"],
                &r["noc"]["value"],
                &r["wmc"]["value"],
                &r["lcom4"],
            ]);
            push(
                lines,
                &[
                    String::new(),
                    format!("{} · {}", vcell(&r["class_id"]), vcell(&r["file"])),
                    format!("CBO / RFC / DIT / NOC / WMC / LCOM4: {metrics}"),
                ],
            );
        }
        for r in items(code, "methods") {
            let h = &r["halstead"];
            push(
                lines,
                &[
                    String::new(),
                    format!(
                        "{} · {}",
                        vcell(&r["method_id"]),
                        cell(&format!("{}:{}", text(&r["file"]), text(&r["line"])))
                    ),
                    format!(
                        "Cognitive: {} · MI: {} · Halstead volume / difficulty / effort: {}",
                        number(&r["cognitive"]),
                        number(&r["maintainability_index"]),
                        numbers(&[&h["volume"], &h["difficulty"], &h["effort"]])
                    ),
                ],
            );
        }
    }
    if let Some(k) = section(
        lines,
        s,
        "knowledge_metrics",
        "Git contribution concentration",
        "unavailable",
    ) {
        for r in items(k, "files") {
            let share = percent(r["top_share"].as_f64().map(|x| x * 100.0));
            push(
                lines,
                &[
                    String::new(),
                    format!(
                        "{} · Bus factor 50: {} · largest share: {share} · {} file/commit touches",
                        vcell(&r["node"]),
                        number(&r["bus_factor_50"]),
                        text(&r["touches"])
                    ),
                ],
            );
        }
    }
}

fn measurements(lines: &mut Lines, s: &Value) {
    if let Some(m) = section(
        lines,
        s,
        "measurement_runs",
        "Build, runtime and repeated tests",
        "not_imported",
    ) {
        for r in items(m, "groups") {
            let head = ["kind", "name", "revision", "environment", "producer"]
                .map(|k| vcell(&r[k]))
                .join(" · ");
            let mixed = if r["inconsistent"].is_null() {
                "unknown".into()
            } else {
                vcell(&r["inconsistent"])
            };
            push(
                lines,
                &[
                    String::new(),
                    head,
                    format!(
                        "Median / p95: {} / {} s · samples: {} · mixed test outcomes: {mixed}",
                        number(&r["median_seconds"]),
                        number(&r["p95_seconds"]),
                        text(&r["duration_samples"])
                    ),
                ],
            );
        }
    }
    if let Some(c) = section(lines, s, "cohesion", "Package cohesion", "not_imported") {
        for p in items(c, "packages") {
            push(
                lines,
                &[
                    String::new(),
                    format!("### Package {}", vcell(&p["node"])),
                    String::new(),
                    vcell(&p["reason"]),
                    String::new(),
                    format!(
                        "Internal share: {}.",
                        percent(p["internal_share"].as_f64().map(|x| x * 100.0))
                    ),
                ],
            );
            for g in items(p, "groups") {
                lines.push(format!(
                    "- {}",
                    items(g, "files").map(vcell).collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }
}

fn history(lines: &mut Lines, s: &Value) {
    let Some(h) = section(
        lines,
        s,
        "quality_history",
        "Quality history",
        "not_recorded",
    ) else {
        return;
    };
    let latest = &h["latest"];
    let revision = latest["producer_revision"].as_str().unwrap_or("unverified");
    push(
        lines,
        &[
            String::new(),
            format!(
                "Producer revision: {}. Deltas retain comparability status.",
                cell(revision)
            ),
        ],
    );
    for (k, label) in [
        ("source_sha256", "Source SHA256"),
        ("config_sha256", "Config SHA256"),
        ("timestamp", "Recorded at (Unix seconds)"),
    ] {
        if !latest[k].is_null() {
            lines.push(format!("{label}: {}", vcell(&latest[k])));
        }
    }
    for a in items(latest, "analyzers") {
        let version = if a["version"].is_null() {
            json!("version unknown")
        } else {
            a["version"].clone()
        };
        lines.push(format!(
            "Analyzer: {}",
            [&a["language"], &a["backend"], &version, &a["project"]]
                .map(vcell)
                .join(" · ")
        ));
    }
    history_modules(lines, h);
}

/// Per-module figures of the latest record and the comparison.
fn history_modules(lines: &mut Lines, h: &Value) {
    for p in items(&h["latest"], "modules") {
        let lc = &p["line_coverage"];
        let cov = lc["total"]
            .as_f64()
            .filter(|t| *t != 0.0)
            .and_then(|t| lc["covered"].as_f64().map(|c| 100.0 * c / t));
        push(
            lines,
            &[
                String::new(),
                vcell(&p["node"]),
                format!(
                    "Coverage: {} · {} · {} / {} files",
                    vcell(&p["coverage_status"]),
                    percent(cov),
                    vcell(&p["coverage_files"]),
                    vcell(&p["files"])
                ),
                format!(
                    "Methods: {} · {} / {} · complexity mean/max {} / {}",
                    vcell(&p["semantic_status"]),
                    vcell(&p["measured_methods"]),
                    vcell(&p["declared_methods"]),
                    number(&p["mean_complexity"]),
                    number(&p["max_complexity"])
                ),
                format!(
                    "Findings: {} · {}",
                    vcell(&p["findings_status"]),
                    number(&p["active_findings"])
                ),
            ],
        );
    }
    for m in items(&h["comparison"], "modules") {
        lines.extend([String::new(), vcell(&m["node"])]);
        for k in [
            "line_coverage",
            "mean_complexity",
            "max_complexity",
            "active_findings",
        ] {
            lines.push(format!("{}: {}", cell(k), cell(&m[k].to_string())));
        }
        for (k, v) in m["extended"].as_object().into_iter().flatten() {
            lines.push(format!("{}: {}", cell(k), cell(&v.to_string())));
        }
    }
}

fn evidence(lines: &mut Lines, s: &Value) {
    if let Some(t) = section(lines, s, "test_evidence", "Test evidence", "not_imported") {
        for test in items(t, "tests") {
            push(
                lines,
                &[
                    String::new(),
                    format!(
                        "{} · {} · {} · {} s",
                        vcell(&test["id"]),
                        vcell(&test["name"]),
                        vcell(&test["outcome"]),
                        number(&test["duration_seconds"])
                    ),
                ],
            );
            lines.extend(
                items(test, "mappings")
                    .map(|m| format!("{} · {}", vcell(&m["path"]), vcell(&m["status"]))),
            );
        }
    }
    let Some(m) = section(lines, s, "mutations", "Mutation evidence", "not_imported") else {
        return;
    };
    let or = |v: &Value, d: &str| if v.is_null() { d.to_string() } else { vcell(v) };
    push(
        lines,
        &[
            String::new(),
            format!(
                "Producer: {} · {}",
                or(&m["producer"], "unknown"),
                or(&m["producer_version"], "version unknown")
            ),
            format!(
                "Killed ratio: {}.",
                percent(m["summary"]["killed_percent"].as_f64())
            ),
        ],
    );
    for file in items(m, "files") {
        for mutant in items(file, "mutants") {
            let at = format!(
                "{}:{}",
                text(&file["path"]),
                text(&mutant["location"]["start"]["line"])
            );
            push(
                lines,
                &[
                    String::new(),
                    format!(
                        "{} · {} · {} · {}",
                        cell(&at),
                        vcell(&file["status"]),
                        vcell(&mutant["status"]),
                        vcell(&mutant["mutator_name"])
                    ),
                    format!(
                        "{} → {}",
                        vcell(&mutant["original"]),
                        vcell(&mutant["replacement"])
                    ),
                ],
            );
        }
    }
}

fn duplicates(lines: &mut Lines, s: &Value) {
    let Some(d) = section(lines, s, "duplicates", "Code duplication", "not_imported") else {
        return;
    };
    let or = |v: &Value, def: &str| {
        if v.is_null() {
            def.to_string()
        } else {
            vcell(v)
        }
    };
    push(
        lines,
        &[
            String::new(),
            format!(
                "CPD: {} · {}",
                or(&d["producer_version"], "version unknown"),
                or(&d["producer_timestamp"], "timestamp unknown")
            ),
            format!(
                "Producer revision: unverified · reported scan errors: {}",
                vcell(&d["producer_errors"])
            ),
        ],
    );
    for g in items(d, "groups") {
        push(
            lines,
            &[
                String::new(),
                format!("### Duplicate block {}", vcell(&g["id"])),
                String::new(),
                format!(
                    "{} tokens · {} reported lines · {}",
                    vcell(&g["tokens"]),
                    vcell(&g["lines"]),
                    vcell(&g["status"])
                ),
            ],
        );
        for o in items(g, "occurrences") {
            let at = format!(
                "{}:{}–{}",
                text(&o["path"]),
                text(&o["line"]),
                text(&o["end_line"])
            );
            lines.push(format!(
                "- {} · {} · {}",
                cell(&at),
                vcell(&o["status"]),
                or(&o["source_sha256"], "hash unavailable")
            ));
        }
        if g["snippet"].as_str().is_some_and(|t| !t.is_empty()) {
            push(lines, &[String::new(), vcell(&g["snippet"])]);
        }
        if g["snippet_truncated"] == true {
            lines.push("Producer excerpt shortened during import; full fragment remains in the original CPD report.".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn quality_exports_every_report_of_the_fixture() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ts-cyclic");
        let loaded = archguard_engine::load_config(None, &root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        let calls = archguard_engine::analyze_calls(&root, &run.config);
        let code = archguard_engine::code_metrics(&calls);
        let measurements = archguard_engine::read_measurement_runs(&root);
        let cohesion = archguard_engine::analyze_cohesion(&run.graph, None);
        let history = archguard_engine::read_quality_history(&root);
        let tests = archguard_engine::read_test_evidence(&root, &run.config, &run.graph);
        let mutations = archguard_engine::read_mutations(&root, &run.config, &run.graph);
        let duplicates = archguard_engine::read_duplicates(&root, &run.config, &run.graph);
        let input = QualityInput {
            code: &code,
            knowledge: None,
            measurements: &measurements,
            cohesion: &cohesion,
            history: &history,
            tests: &tests,
            mutations: &mutations,
            duplicates: &duplicates,
        };
        let s = quality_snapshot("ts-cyclic", &input, &QualityFilters::default());
        let v: Value = serde_json::from_str(&s.json()).expect("valid JSON");
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["kind"], "quality");
        for key in [
            "code_metrics",
            "measurement_runs",
            "cohesion",
            "quality_history",
            "test_evidence",
            "mutations",
            "duplicates",
        ] {
            assert!(v[key].is_object(), "{key}");
        }
        assert!(v["knowledge_metrics"].is_null());
        let md = s.markdown();
        assert!(md.starts_with("# Software quality: ts-cyclic\n"), "{md}");
        assert!(
            md.contains("## Git contribution concentration\n\nStatus: unavailable"),
            "{md}"
        );
        assert!(md.contains("## Code duplication"), "{md}");
    }

    #[test]
    fn numbers_follow_the_web_format() {
        assert_eq!(number(&json!(3)), "3");
        assert_eq!(number(&json!(1.234)), "1.23");
        assert_eq!(number(&json!(2.5)), "2.5");
        assert_eq!(number(&Value::Null), "—");
        assert_eq!(percent(Some(12.345)), "12.3%");
    }
}
