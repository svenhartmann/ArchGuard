//! Evidence-based analysis commands. The engine owns their semantics.
use crate::args::{Cli, Format};
use crate::output::{EXIT_FOUND, EXIT_OK, analysis_error, emit, usage};
use archguard_engine::arc42_file::{self, Arc42Files, Target};
use archguard_engine::core::{ArchguardError, Cut, Graph, aggregate};
use archguard_engine::{CoChangeOptions, LoadedConfig, collect_review_sources};
use std::path::Path;
fn output(cli: &Cli, value: serde_json::Value, label: &str) -> i32 {
    if !matches!(cli.format, Format::Json | Format::Text) {
        return usage(format!("{label} supports text and json"));
    }
    if !cli.quiet {
        if cli.format == Format::Text {
            outln!("{label} · evidence and coverage");
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
        outln!(
            "Semantic analysis: {} · {} files · {} types · {} methods · {} call sites",
            calls.status,
            calls.files,
            calls.classes.len(),
            calls.methods.len(),
            calls.calls.len()
        );
        for analyzer in calls.analyzers {
            outln!(
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
                outln!("  {diagnostic}");
            }
        }
        outln!(
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
                    outln!("Architecture document is current: {}", target.display());
                }
                EXIT_OK
            }
            Ok(_) => {
                if !cli.quiet {
                    outln!("Architecture document is outdated: {}", target.display());
                }
                EXIT_FOUND
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !cli.quiet {
                    outln!("Architecture document is missing: {}", target.display());
                }
                EXIT_FOUND
            }
            Err(error) => analysis_error(error),
        };
    }
    match write_document(&target, &document) {
        Ok(()) => {
            if !cli.quiet {
                outln!("Architecture document written: {}", target.display());
            }
            EXIT_OK
        }
        Err(error) => analysis_error(error),
    }
}

/// `doc --arc42 --check`: exit 1 and name the chapters a refresh would
/// change (`Arc42Files::stale`). ADR 0008: the desktop may have written
/// German blocks while the CLI writes English, so `german` is tried as well.
fn arc42_check(
    cli: &Cli,
    files: &Arc42Files,
    chapter: Option<u8>,
    document: &archguard_engine::report::Arc42Document,
    german: impl FnOnce() -> Result<
        archguard_engine::report::Arc42Document,
        archguard_engine::core::ArchguardError,
    >,
) -> i32 {
    let (Target::File(target) | Target::Folder(target)) = &files.target;
    if !files.exists() {
        if !cli.quiet {
            outln!("arc42 document is missing: {}", target.display());
        }
        return EXIT_FOUND;
    }
    let mut stale = match files.stale(document, chapter) {
        Ok(s) => s,
        Err(e) => return usage(&e),
    };
    // Each chapter may be in either language (one window saves one chapter).
    if !stale.is_empty() {
        let german = match german() {
            Ok(g) => files.stale(&g, chapter).unwrap_or_default(),
            Err(e) => return analysis_error(e),
        };
        stale.retain(|n| german.contains(n));
    }
    if !cli.quiet {
        if stale.is_empty() {
            outln!("arc42 document is current: {}", target.display());
        } else {
            let list: Vec<_> = stale.iter().map(|n| format!("chapter {n}")).collect();
            outln!(
                "arc42 document is outdated: {} ({})",
                target.display(),
                list.join(", ")
            );
        }
    }
    if stale.is_empty() {
        EXIT_OK
    } else {
        EXIT_FOUND
    }
}

/// Replace a generated document as a whole. A final symlink or other
/// non-regular file is refused, as for arc42, so a committed
/// `architecture.md -> ~/victim.txt` cannot redirect the write. The
/// directory is the user's choice: `--out` may name any path.
pub(crate) fn write_document(target: &Path, text: &str) -> std::io::Result<()> {
    use std::io::{ErrorKind, Write};
    let existing = regular_or_missing(target)?;
    let parent = target.parent().ok_or(ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(parent)?;
    let temp = target.with_file_name(format!(".archguard-doc-{}.tmp", std::process::id()));
    let result = (|| {
        // create_new: the temporary name can never be a planted symlink.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        if let Some(m) = existing {
            file.set_permissions(m.permissions())?;
        }
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        // rename replaces the entry; it never writes through a link.
        std::fs::rename(&temp, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Metadata of a regular file, `None` if it does not exist; an error for a
/// symlink or any other kind of entry.
fn regular_or_missing(target: &Path) -> std::io::Result<Option<std::fs::Metadata>> {
    use std::io::{Error, ErrorKind};
    match std::fs::symlink_metadata(target) {
        Ok(m) if !m.is_file() || m.file_type().is_symlink() => Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{} must be a regular file, not a symlink", target.display()),
        )),
        Ok(m) => Ok(Some(m)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
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
    let format = loaded.config.arc42.diagram_format;
    // `--out` names the document; else it is where the desktop finds it.
    let files = match options.out {
        Some(out) => Arc42Files::load(Target::File(root.join(out))),
        None => Arc42Files::open(root, format),
    };
    let mut files = match files {
        Ok(f) => f,
        Err(e) => return usage(&e),
    };
    files.format = format;
    if options.check {
        let german = || {
            let mut config = loaded.config.clone();
            config.arc42.lang = archguard_engine::core::DocLang::De;
            archguard_engine::arc42_document(root, &config, graph, report)
        };
        return arc42_check(cli, &files, options.chapter, &document, german);
    }
    let changes = match files.update(&document, options.chapter) {
        Ok(c) => c,
        Err(e) => return usage(&e),
    };
    for change in changes {
        // Publish complete files, and none over a change made meanwhile.
        if let Err(e) = arc42_file::write(&change.path, change.old.as_deref(), &change.new) {
            return analysis_error(e);
        }
        if !cli.quiet {
            outln!("arc42 document updated: {}", change.path.display());
        }
    }
    EXIT_OK
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
