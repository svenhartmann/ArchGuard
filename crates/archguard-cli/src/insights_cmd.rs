//! Evidence-based analysis commands. The engine owns their semantics.
use crate::args::{Cli, Format};
use crate::output::{EXIT_FOUND, EXIT_OK, analysis_error, emit, usage};
use archguard_engine::core::{ArchguardError, Cut, Graph, aggregate};
use archguard_engine::{CoChangeOptions, LoadedConfig, collect_review_sources};
use std::path::Path;
fn output(cli: &Cli, value: serde_json::Value, label: &str) -> i32 {
    if !matches!(cli.format, Format::Json | Format::Text) {
        return usage(format!("{label} supports text and json"));
    }
    if !cli.quiet {
        if cli.format == Format::Text {
            println!("{label} · evidence and coverage");
        }
        emit(&serde_json::to_string_pretty(&value).unwrap());
    }
    EXIT_OK
}
pub(crate) fn calls(cli: &Cli, root: &Path, loaded: &LoadedConfig) -> i32 {
    if !matches!(cli.format, Format::Json | Format::Text) {
        return usage("calls supports text and json");
    }
    let calls = archguard_engine::analyze_calls(root, &loaded.config);
    if cli.quiet {
        return EXIT_OK;
    }
    if cli.format == Format::Json {
        emit(&serde_json::to_string_pretty(&calls).unwrap());
    } else {
        println!(
            "Semantic analysis: {} · {} files · {} types · {} methods · {} call sites",
            calls.status,
            calls.files,
            calls.classes.len(),
            calls.methods.len(),
            calls.calls.len()
        );
        for analyzer in calls.analyzers {
            println!(
                "{} / {}: {} · {} files{}",
                analyzer.language,
                analyzer.backend,
                analyzer.status,
                analyzer.files,
                analyzer
                    .project
                    .as_ref()
                    .map(|project| format!(" · {project}"))
                    .unwrap_or_default()
            );
            for diagnostic in analyzer.diagnostics {
                println!("  {diagnostic}");
            }
        }
        println!(
            "Static declaration targets; runtime dispatch may differ. Use --format json for source evidence."
        );
    }
    EXIT_OK
}
#[expect(clippy::too_many_arguments, reason = "CLI flags")]
pub(crate) fn co_change(
    cli: &Cli,
    root: &Path,
    loaded: &LoadedConfig,
    graph: &Graph,
    since: Option<&str>,
    min_shared: u32,
    min_ratio: f64,
    top: usize,
) -> i32 {
    let options = CoChangeOptions {
        since: since.unwrap_or(&loaded.config.review.git_since).into(),
        min_shared,
        min_ratio,
        max_results: top,
    };
    match archguard_engine::co_change(root, &loaded.config, graph, &options) {
        Ok(report) => output(cli, serde_json::to_value(report).unwrap(), "co-change"),
        Err(error) => analysis_error(error),
    }
}
pub(crate) fn ownership(cli: &Cli, root: &Path, loaded: &LoadedConfig, graph: &Graph) -> i32 {
    let result =
        collect_review_sources(root, &loaded.config, graph, &loaded.config.review.git_since)
            .and_then(|sources| {
                archguard_engine::analyze_ownership(
                    root,
                    graph,
                    &sources,
                    &loaded.config.ownership_rules,
                )
            });
    match result {
        Ok(report) => {
            let fails = report
                .findings
                .iter()
                .any(|f| f.severity == archguard_engine::core::Severity::Error);
            let code = output(cli, serde_json::to_value(report).unwrap(), "ownership");
            if code == EXIT_OK && fails {
                EXIT_FOUND
            } else {
                code
            }
        }
        Err(error) => analysis_error(error),
    }
}
pub(crate) fn performance(cli: &Cli, root: &Path, loaded: &LoadedConfig, graph: &Graph) -> i32 {
    let result = (|| -> Result<_, ArchguardError> {
        let sources =
            collect_review_sources(root, &loaded.config, graph, &loaded.config.review.git_since)?;
        let entries = archguard_engine::analysis::match_review_entry_points(
            graph,
            &sources,
            &loaded.config.review.entry_points,
        )?;
        let calls = archguard_engine::analyze_calls(root, &loaded.config);
        let view = aggregate(
            graph,
            &Cut::at_depth(graph, loaded.config.modules.depth as usize),
        );
        archguard_engine::analyze_performance(
            root,
            &loaded.config,
            graph,
            &view,
            &sources,
            &calls,
            &entries,
        )
    })();
    match result {
        Ok(report) => output(cli, serde_json::to_value(report).unwrap(), "performance"),
        Err(error) => analysis_error(error),
    }
}

pub(crate) struct DocOptions<'a> {
    pub out: Option<&'a Path>,
    pub check: bool,
    pub arc42: bool,
    pub chapter: Option<u8>,
    pub stdout: bool,
}
pub(crate) fn doc(
    cli: &Cli,
    root: &Path,
    loaded: &LoadedConfig,
    graph: &Graph,
    report: &archguard_engine::core::Report,
    options: DocOptions<'_>,
) -> i32 {
    if cli.format != Format::Text {
        return usage("doc writes Markdown; use the default text format");
    }
    if options.arc42 {
        return arc42_doc(cli, root, loaded, graph, report, options);
    }
    let out = options.out.unwrap_or_else(|| Path::new("architecture.md"));
    let check = options.check;
    let view = aggregate(
        graph,
        &Cut::at_depth(graph, loaded.config.modules.depth as usize),
    );
    let goals = archguard_engine::quality_goal_assessment(
        root,
        &loaded.config,
        graph,
        &report.findings,
        report.review_kpis.as_ref(),
    );
    let document = archguard_engine::report::render_architecture_doc(
        &view,
        report,
        &loaded.config,
        None,
        &goals,
    );
    if options.stdout {
        if !cli.quiet {
            emit(&document);
        }
        return EXIT_OK;
    }
    let target = if out.is_absolute() {
        out.to_path_buf()
    } else {
        root.join(out)
    };
    if check {
        return match std::fs::read_to_string(&target) {
            Ok(saved) if saved == document => {
                if !cli.quiet {
                    println!("Architecture document is current: {}", target.display());
                }
                EXIT_OK
            }
            Ok(_) => {
                if !cli.quiet {
                    println!("Architecture document is outdated: {}", target.display());
                }
                EXIT_FOUND
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !cli.quiet {
                    println!("Architecture document is missing: {}", target.display());
                }
                EXIT_FOUND
            }
            Err(error) => analysis_error(error),
        };
    }
    match std::fs::write(&target, document) {
        Ok(()) => {
            if !cli.quiet {
                println!("Architecture document written: {}", target.display());
            }
            EXIT_OK
        }
        Err(error) => analysis_error(error),
    }
}

fn arc42_doc(
    cli: &Cli,
    root: &Path,
    loaded: &LoadedConfig,
    graph: &Graph,
    report: &archguard_engine::core::Report,
    options: DocOptions<'_>,
) -> i32 {
    let document = match archguard_engine::arc42_document(root, &loaded.config, graph, report) {
        Ok(d) => d,
        Err(e) => return analysis_error(e),
    };
    if options.stdout {
        if !cli.quiet {
            emit(&document.markdown(options.chapter));
        }
        return EXIT_OK;
    }
    let out = options.out.unwrap_or_else(|| Path::new("arc42.md"));
    let target = if out.is_absolute() {
        out.to_path_buf()
    } else {
        root.join(out)
    };
    // Reading/writing a documentation file is explicit; never follow a final
    // symlink or replace a non-regular file supplied as a document.
    match std::fs::symlink_metadata(&target) {
        Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
            return usage("arc42 output must be a regular file, not a symlink");
        }
        Ok(m) if m.len() > 8 * 1024 * 1024 => return usage("arc42 document exceeds 8 MiB"),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return analysis_error(e),
        _ => {}
    }
    let saved = match std::fs::read_to_string(&target) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return analysis_error(e),
    };
    let next = match saved.as_deref() {
        Some(s) => match document.merge(s, options.chapter) {
            Ok(n) => n,
            Err(e) => return usage(&e),
        },
        None => document.markdown(options.chapter),
    };
    if options.check {
        let current = saved.as_deref() == Some(next.as_str());
        if !cli.quiet {
            println!(
                "arc42 document {}: {}",
                if current {
                    "is current"
                } else {
                    "is missing or outdated"
                },
                target.display()
            );
        }
        return if current { EXIT_OK } else { EXIT_FOUND };
    }
    if saved.as_deref() == Some(next.as_str()) {
        return EXIT_OK;
    }
    if let Some(parent) = target.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        return analysis_error(e);
    }
    // Publish a complete update so interruption cannot leave half a document.
    let temp = target.with_file_name(format!(".archguard-arc42-{}.tmp", std::process::id()));
    let mut created = false;
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        created = true;
        if let Ok(metadata) = std::fs::metadata(&target) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(next.as_bytes())?;
        file.sync_all()?;
        if saved
            != match std::fs::read_to_string(&target) {
                Ok(s) => Some(s),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e),
            }
        {
            return Err(std::io::Error::other(
                "arc42 document changed during generation; retry without losing manual edits",
            ));
        }
        std::fs::rename(&temp, &target)
    })();
    if created && temp.exists() {
        let _ = std::fs::remove_file(&temp);
    }
    match result {
        Ok(()) => {
            if !cli.quiet {
                println!("arc42 document updated: {}", target.display());
            }
            EXIT_OK
        }
        Err(e) => analysis_error(e),
    }
}

// ----------------------------------------------------- review KPI table (R28)

/// One `KPI | Wert | Delta | Ampel` row; same column style as the text
/// report so both outputs read as one system.
fn kpi_row(name: &str, value: &str, delta: &str, ampel: &str) -> String {
    format!("  {name:<22} {value:>24}  delta {delta:>6}  ampel {ampel}")
}

/// Signed delta against the newest snapshot; `-` without a baseline —
/// absence is never shown as zero.
fn delta(cur: f64, prev: Option<f64>) -> String {
    prev.map_or_else(|| "-".to_string(), |p| format!("{:+.2}", cur - p))
}

fn delta_count(cur: usize, prev: Option<usize>) -> String {
    prev.map_or_else(
        || "-".to_string(),
        |p| format!("{:+}", cur as i64 - p as i64),
    )
}

/// `risk_coverage_gaps`: "not measured" without coverage, a delta only
/// where both sides measured.
fn risk_gap_row(
    current: &archguard_engine::core::ReviewKpis,
    previous: Option<&archguard_engine::core::ReviewKpis>,
) -> String {
    let count = current.risk_gap_count();
    let before = previous.and_then(archguard_engine::core::ReviewKpis::risk_gap_count);
    kpi_row(
        "risk_coverage_gaps",
        &count.map_or_else(|| "not measured".to_string(), |n| n.to_string()),
        &count.map_or_else(|| "-".to_string(), |n| delta_count(n, before)),
        "-",
    )
}

fn opt_f64(v: Option<f64>) -> String {
    v.map_or_else(|| "not measured".to_string(), |v| format!("{v:.2}"))
}

/// The `metrics` KPI table (R28): the current payload values with
/// deltas against the KPIs stored in the newest snapshot. A KPI only
/// gets a delta where both sides measured it; the ampel is the health
/// level, everything else has no configured gate (R31 adds those).
pub(crate) fn review_kpi_lines(
    current: &archguard_engine::core::ReviewKpis,
    previous: Option<&archguard_engine::core::ReviewKpis>,
) -> Vec<String> {
    use archguard_engine::core::HealthLevel;
    let max_pc = |k: &archguard_engine::core::ReviewKpis| {
        k.propagation_cost
            .iter()
            .map(|p| p.pc)
            .fold(0.0_f64, f64::max)
    };
    let value_pc = if current.propagation_cost.is_empty() {
        "0 cycles".to_string()
    } else {
        format!(
            "{} cycles, max {:.2}",
            current.propagation_cost.len(),
            max_pc(current)
        )
    };
    let delta_pc = if current.propagation_cost.is_empty() {
        "-".to_string()
    } else {
        delta(max_pc(current), previous.map(max_pc))
    };
    let mut lines = vec![
        "Review KPIs (delta vs. newest snapshot):".to_string(),
        kpi_row("propagation_cost", &value_pc, &delta_pc, "-"),
        kpi_row(
            "sdp_violations",
            &current.sdp_violations.len().to_string(),
            &delta_count(
                current.sdp_violations.len(),
                previous.map(|p| p.sdp_violations.len()),
            ),
            "-",
        ),
        kpi_row(
            "tangledness",
            &opt_f64(current.tangledness.as_ref().map(|t| t.quote)),
            &delta(
                current.tangledness.as_ref().map_or(0.0, |t| t.quote),
                previous.and_then(|p| p.tangledness.as_ref().map(|t| t.quote)),
            ),
            "-",
        ),
        kpi_row(
            "hub_gini",
            &opt_f64(current.hub_gini.as_ref().map(|h| h.gini)),
            &delta(
                current.hub_gini.as_ref().map_or(0.0, |h| h.gini),
                previous.and_then(|p| p.hub_gini.as_ref().map(|h| h.gini)),
            ),
            "-",
        ),
        kpi_row(
            "layer_drift_agreement",
            &opt_f64(current.layer_drift.as_ref().and_then(|d| d.agreement)),
            "-",
            "-",
        ),
    ];
    let debt_value = current.debt.as_ref().map_or_else(
        || "not measured".to_string(),
        |d| format!("{} markers", d.markers),
    );
    let debt_delta = current.debt.as_ref().map_or_else(
        || "-".to_string(),
        |d| {
            delta_count(
                d.markers,
                previous.and_then(|p| p.debt.as_ref().map(|p| p.markers)),
            )
        },
    );
    lines.push(kpi_row("debt_markers", &debt_value, &debt_delta, "-"));
    let exported = current
        .api_surface
        .iter()
        .map(|a| a.exported_symbols)
        .sum::<usize>();
    lines.push(kpi_row(
        "api_surface",
        &format!("{exported} symbols"),
        &delta_count(
            exported,
            previous.map(|p| p.api_surface.iter().map(|a| a.exported_symbols).sum()),
        ),
        "-",
    ));
    lines.push(kpi_row(
        "leaks",
        &current.leaks.len().to_string(),
        &delta_count(current.leaks.len(), previous.map(|p| p.leaks.len())),
        "-",
    ));
    lines.push(risk_gap_row(current, previous));
    let deps = current.dependencies.as_ref().map_or_else(
        || "not measured".to_string(),
        |d| format!("{} deps", d.total),
    );
    let deps_delta = current.dependencies.as_ref().map_or_else(
        || "-".to_string(),
        |d| {
            delta_count(
                d.total,
                previous.and_then(|p| p.dependencies.as_ref().map(|p| p.total)),
            )
        },
    );
    lines.push(kpi_row("dependencies", &deps, &deps_delta, "-"));
    let (score, score_delta, ampel) = match &current.health {
        Some(h) => (
            h.score
                .map_or_else(|| "not measured".to_string(), |s| format!("{s:.1}")),
            h.score.map_or_else(
                || "-".to_string(),
                |s| {
                    delta(
                        s,
                        previous.and_then(|p| p.health.as_ref().and_then(|h| h.score)),
                    )
                },
            ),
            match h.level {
                Some(HealthLevel::Green) => "green",
                Some(HealthLevel::Yellow) => "yellow",
                Some(HealthLevel::Red) => "red",
                None => "-",
            }
            .to_string(),
        ),
        None => ("not measured".to_string(), "-".to_string(), "-".to_string()),
    };
    lines.push(kpi_row("health", &score, &score_delta, &ampel));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{
        DebtDensity, HealthIndex, HealthLevel, HubGini, ReviewKpis, Tangledness,
    };
    use std::collections::BTreeMap;

    fn kpis(quote: f64, markers: usize, score: f64) -> ReviewKpis {
        let level = if score >= 80.0 {
            HealthLevel::Green
        } else if score >= 50.0 {
            HealthLevel::Yellow
        } else {
            HealthLevel::Red
        };
        ReviewKpis {
            tangledness: Some(Tangledness {
                tangled_nodes: 1,
                total: 10,
                quote,
            }),
            hub_gini: Some(HubGini {
                gini: 0.2,
                top_share: 0.5,
            }),
            debt: Some(DebtDensity {
                markers,
                loc: 100,
                per_kloc: Some(markers as f64 * 10.0),
                by_kind: BTreeMap::new(),
            }),
            health: Some(HealthIndex {
                score: Some(score),
                level: Some(level),
                trend: Some(5.0),
                parts: BTreeMap::from([("tangledness".to_string(), 1.0 - quote)]),
            }),
            ..ReviewKpis::default()
        }
    }

    #[test]
    fn deltas_are_signed_and_absent_without_baseline() {
        let lines = review_kpi_lines(&kpis(0.4, 3, 80.0), Some(&kpis(0.2, 5, 70.0)));
        let joined = lines.join("\n");
        assert!(joined.starts_with("Review KPIs (delta vs. newest snapshot):"));
        let tangled = lines
            .iter()
            .find(|l| l.contains("tangledness"))
            .expect("row");
        assert!(tangled.contains("+0.20"), "{tangled}"); // tangledness grew
        let debt = lines
            .iter()
            .find(|l| l.contains("debt_markers"))
            .expect("row");
        assert!(debt.contains("-2"), "{debt}"); // debt markers fell
        assert!(joined.contains("ampel"), "{joined}");
        let no_base = review_kpi_lines(&kpis(0.4, 3, 80.0), None).join("\n");
        assert!(no_base.contains("delta      -"), "{no_base}");
    }

    #[test]
    fn unmeasured_kpis_stay_not_measured() {
        let lines = review_kpi_lines(&ReviewKpis::default(), None).join("\n");
        assert!(lines.contains("not measured"));
        assert!(!lines.contains("  0.00\n")); // no invented zeros in the value column
    }
}
