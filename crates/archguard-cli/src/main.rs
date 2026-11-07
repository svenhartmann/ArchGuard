//! The `archguard` binary — argument parsing, format selection and exit
//! codes (Kap. 9). T24 landed `scan` and `cycles`; T25 adds `check`,
//! `metrics`, `graph`, `suggest`, `baseline write|diff`, `init` and
//! `config`; T33 adds `db sync|query` (graph-DB mirror, Kap. 8 — its
//! handler lives in [`db`], real only with the `db` build feature).
//! T41 adds `snapshot` and `trend` (snapshots & trends, Kap. 9 — their
//! handler lives in [`trend_cmd`]).
//! [`serve_cmd`] serves the live HTML viewer on loopback with automatic updates.
//!
//! Exit codes (Kap. 9): `0` ok · `1` violations found (`check`,
//! `cycles --fail-on-found`) · `2` usage error (clap's own code for
//! parse failures) · `3` analysis, IO or config error.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use archguard_engine::analysis::{Baseline, cycle_hash};
use archguard_engine::core::{Finding, FindingKind, Graph, ModuleMetrics, Report};
use archguard_engine::report::{to_dot, to_gexf, to_graphml, to_json, to_mermaid, to_text};
use archguard_engine::{
    Level, LoadedConfig, analyze_run, default_config_string, effective_config_string, load_config,
};
use clap::Parser;
use serde_json::{Value, json};

mod args;
mod output;
use args::{BaselineAction, Cli, Command, DbAction, Format, LevelArg};
use check_cmd::read_baseline;
use cycle_view::{cycle_findings, cyclic_edges};
use output::{
    EXIT_FOUND, EXIT_OK, EXIT_USAGE, analysis_error, emit, format_label, to_pretty_json, usage,
};
mod architecture_cmd;
mod assessment_cmd;
mod check_cmd;
mod coverage_cmd;
mod cycle_view;
mod db;
mod doctor_cmd;
mod dsm_cmd;
mod engineering_cmd;
mod evaluate_cmd;
mod insights_cmd;
mod project_files;
mod quality_cmd;
mod review_cmd;
mod review_session_cmd;
mod sarif;
mod security_cmd;
mod serve_cmd;
mod service_cmd;
mod simulate_cmd;
mod trend_cmd;
mod update_cmd;
mod view_util;
mod workspace_cmd;

impl LevelArg {
    fn to_engine(self) -> Level {
        match self {
            LevelArg::Module => Level::Module,
            LevelArg::File => Level::File,
        }
    }
}

impl Command {
    fn path(&self) -> Option<&Path> {
        match self {
            Command::Scan { path, .. }
            | Command::Review { path, .. }
            | Command::Cycles { path, .. }
            | Command::Check { path, .. }
            | Command::Metrics { path, .. }
            | Command::Security { path, .. }
            | Command::Hotspots { path, .. }
            | Command::CoChange { path, .. }
            | Command::Ownership { path, .. }
            | Command::Performance { path, .. }
            | Command::Calls { path, .. }
            | Command::Cohesion { path, .. }
            | Command::QualityHistory { path, .. }
            | Command::TestEvidence { path, .. }
            | Command::Mutations { path, .. }
            | Command::Duplicates { path, .. }
            | Command::Contracts { path, .. }
            | Command::Coverage { path, .. }
            | Command::Quality { path, .. }
            | Command::Architecture { path, .. }
            | Command::Patterns { path, .. }
            | Command::Evaluate { path, .. }
            | Command::Services { path, .. }
            | Command::Workspace { path, .. }
            | Command::Doc { path, .. }
            | Command::Graph { path, .. }
            | Command::Diff { path, .. }
            | Command::Serve { path, .. }
            | Command::Suggest { path, .. }
            | Command::Dsm { path, .. }
            | Command::Simulate { path, .. }
            | Command::Snapshot { path, .. }
            | Command::Trend { path, .. }
            | Command::Init { path, .. }
            | Command::Config { path, .. } => path.as_deref(),
            Command::Baseline { action } => match action {
                BaselineAction::Write { path, .. } | BaselineAction::Diff { path, .. } => {
                    path.as_deref()
                }
            },
            Command::Db { action } => match action {
                DbAction::Sync { path, .. } => path.as_deref(),
                // `query` only opens the database file — no project root.
                DbAction::Query { .. } => None,
            },
            Command::Mcp { .. } => None,
            Command::Doctor | Command::Update { .. } => None,
            Command::ReviewDecision { .. } => None,
        }
    }

    /// `--measure-zones` on `scan`/`check`.
    fn measure_zones(&self) -> bool {
        matches!(
            self,
            Command::Scan {
                measure_zones: true,
                ..
            } | Command::Check {
                measure_zones: true,
                ..
            }
        )
    }

    fn level(&self) -> Level {
        match self {
            Command::Scan { level, .. }
            | Command::Cycles { level, .. }
            | Command::Hotspots { level, .. } => level.to_engine(),
            _ => Level::Module,
        }
    }
}

fn main() -> ExitCode {
    ExitCode::from(run(&Cli::parse()) as u8)
}

/// The shared analysis run. The pipeline leaves zones unmeasured; only the
/// explicit `--measure-zones` opt-in pays for the compiler-backed call
/// analysis that measures them.
fn analyze_with_zones(
    cli: &Cli,
    root: &Path,
    loaded: &LoadedConfig,
) -> Result<archguard_engine::AnalysisRun, archguard_engine::core::ArchguardError> {
    let mut run = analyze_run(root, loaded, cli.command.level())?;
    if cli.command.measure_zones()
        && let Some(kpis) = run.report.review_kpis.as_mut()
    {
        kpis.zones =
            archguard_engine::measure_review_zones(root, &run.config, &run.graph, &run.view)?;
    }
    Ok(run)
}

/// Parse-free core: everything after argument parsing, mapped to a Kap. 9
/// exit code.
fn run(cli: &Cli) -> i32 {
    if let Command::Mcp { root } = &cli.command {
        return mcp(root);
    }
    if matches!(cli.command, Command::Doctor) {
        return doctor_cmd::run(cli);
    }
    if let Command::Update { check } = &cli.command {
        return update_cmd::run(cli, *check);
    }
    let root = match resolve_root(cli.command.path()) {
        Ok(root) => root,
        Err(err) => return analysis_error(err),
    };
    if let Command::Diff {
        base, to, format, ..
    } = &cli.command
    {
        if !matches!(format, Format::Html | Format::Json | Format::Text) {
            eprintln!("diff supports html, json or text");
            return EXIT_USAGE;
        }
        if let Some(target) = to {
            return match archguard_engine::compare_branches(
                &root,
                base,
                target,
                cli.config.as_deref(),
            ) {
                Ok(diff) => {
                    let output = match format {
                        Format::Html => archguard_engine::report::branch_diff_html(&diff),
                        Format::Json => {
                            serde_json::to_string_pretty(&diff).expect("diff serializes")
                        }
                        _ => format!(
                            "{} → {}: +{} / -{} dependencies, {} changed evidence; {} new / {} resolved findings\n",
                            diff.base_ref,
                            diff.target_ref,
                            diff.details.added_edges.len(),
                            diff.details.removed_edges.len(),
                            diff.details.changed_edges.len(),
                            diff.details.new_findings.len(),
                            diff.details.resolved_findings.len()
                        ),
                    };
                    emit(&output);
                    EXIT_OK
                }
                Err(error) => analysis_error(error),
            };
        }
        return match archguard_engine::compare_staged(&root, base, cli.config.as_deref()) {
            Ok(diff) => {
                let output = match format {
                    Format::Html => archguard_engine::report::staged_diff_html(&diff),
                    Format::Json => serde_json::to_string_pretty(&diff).expect("diff serializes"),
                    _ => format!(
                        "{} → index: +{} / -{} dependencies, {} changed evidence; {} new / {} resolved findings\n",
                        diff.base_ref,
                        diff.added_edges.len(),
                        diff.removed_edges.len(),
                        diff.changed_edges.len(),
                        diff.new_findings.len(),
                        diff.resolved_findings.len()
                    ),
                };
                emit(&output);
                EXIT_OK
            }
            Err(error) => analysis_error(error),
        };
    }
    if let Command::Security {
        cached,
        offline,
        cache_dir,
        fail_on_found,
        ..
    } = &cli.command
    {
        return security_cmd::run(
            cli,
            &root,
            *cached,
            *fail_on_found,
            &archguard_engine::SecurityScanOptions {
                offline: *offline,
                cache_dir: cache_dir.clone(),
            },
        );
    }
    // Compose-only repositories do not need an import graph or archguard.toml.
    if let Command::Services { fail_on_found, .. } = &cli.command {
        return service_cmd::run(cli, &root, *fail_on_found);
    }
    if let Command::Workspace {
        sources, target, ..
    } = &cli.command
    {
        return workspace_cmd::run(cli, &root, *sources, target);
    }
    if let Command::Contracts {
        report,
        service,
        usage,
        ..
    } = &cli.command
    {
        return engineering_cmd::contracts(
            cli,
            &root,
            report.as_deref(),
            service.as_deref(),
            usage.as_deref(),
        );
    }
    if let Command::QualityHistory { record: false, .. } = &cli.command {
        return engineering_cmd::history(cli, &root);
    }
    if let Some(code) = review_command(cli, &root) {
        return code;
    }
    if let Command::Serve { port, depth, .. } = &cli.command {
        return match serve_cmd::run(&root, cli.config.as_deref(), *port, *depth) {
            Ok(()) => EXIT_OK,
            Err(err) => analysis_error(err),
        };
    }
    // `db query` never analyses; `db sync` loads the config and analyses
    // inside its handler. Without the `db` feature both are a usage error
    // with a build hint (Kap. 8/9, T33).
    if let Command::Db { action } = &cli.command {
        return db::dispatch(action, cli, &root);
    }
    // `init` only writes a config — no analysis, no config load.
    if let Command::Init { defaults, .. } = &cli.command {
        return init(cli, &root, *defaults);
    }
    // `trend` only reads `.archguard/history/` — no analysis, no config
    // load (T41); `snapshot` needs the analysis and runs below.
    if let Command::Trend { last, format, .. } = &cli.command {
        return trend_cmd::trend(cli, &root, *last, *format);
    }
    let loaded = match load_config(cli.config.as_deref(), &root) {
        Ok(loaded) => loaded,
        Err(err) => return analysis_error(err),
    };
    // Every command runs the effective pipeline with the LOADED config:
    // a declared `archguard.toml` section must keep its provenance and its
    // values (`analyze` would re-derive `project` and `modules` over the
    // user's word — see the contract note in `archguard_engine::pipeline`).
    let run = match analyze_with_zones(cli, &root, &loaded) {
        Ok(run) => run,
        Err(err) => return analysis_error(err),
    };
    let loaded = run.effective_config();
    let (graph, report) = (run.graph, run.report);
    match &cli.command {
        Command::Scan { out, .. } => scan(cli, &root, out.as_deref(), &graph, &report, &loaded),
        Command::Cycles {
            max_cycles,
            fail_on_found,
            ..
        } => cycles(
            cli,
            &root,
            *max_cycles,
            *fail_on_found,
            &graph,
            &report,
            &loaded,
        ),
        Command::Check {
            baseline,
            base,
            fail_on_acd_regression,
            ..
        } => check_cmd::run(
            cli,
            &root,
            &loaded,
            &graph,
            &report,
            baseline.as_deref(),
            base.as_deref(),
            *fail_on_acd_regression,
        ),
        Command::Metrics { extended: true, .. } => {
            engineering_cmd::run(cli, &root, &loaded, &graph)
        }
        Command::Metrics { module, top, .. } => {
            metrics(cli, &root, module.as_deref(), *top, &report)
        }
        Command::Hotspots { top, since, .. } => {
            review_cmd::run(cli, &root, &graph, &loaded, *top, since.as_deref())
        }
        Command::CoChange {
            since,
            min_shared,
            min_ratio,
            top,
            ..
        } => insights_cmd::co_change(
            cli,
            &root,
            &loaded,
            &graph,
            since.as_deref(),
            *min_shared,
            *min_ratio,
            *top,
        ),
        Command::Ownership { .. } => insights_cmd::ownership(cli, &root, &loaded, &graph),
        Command::Performance { .. } => insights_cmd::performance(cli, &root, &loaded, &graph),
        Command::Calls { .. } => insights_cmd::calls(cli, &root, &loaded),
        Command::Cohesion { .. }
        | Command::QualityHistory { .. }
        | Command::TestEvidence { .. }
        | Command::Duplicates { .. }
        | Command::Mutations { .. } => engineering_cmd::run(cli, &root, &loaded, &graph),
        Command::Coverage {
            report,
            source_root,
            ..
        } => coverage_cmd::run(
            cli,
            &root,
            &loaded,
            &graph,
            report.as_deref(),
            source_root.as_deref(),
        ),
        Command::Quality {
            report,
            source_root,
            base,
            min_changed_coverage,
            max_complexity,
            max_nesting,
            ..
        } => quality_cmd::run(
            cli,
            &root,
            &loaded,
            &graph,
            quality_cmd::QualityRequest {
                report: report.as_deref(),
                source_root: source_root.as_deref(),
                base: base.as_deref(),
                thresholds: archguard_engine::QualityGateOptions {
                    min_changed_coverage: *min_changed_coverage,
                    max_complexity: *max_complexity,
                    max_nesting: *max_nesting,
                },
            },
        ),
        Command::Architecture { .. } => architecture_cmd::run(cli, &loaded, &graph),
        Command::Patterns { .. } => {
            architecture_cmd::patterns(cli, &root, &loaded, &graph, &report)
        }
        Command::Doc {
            out,
            check,
            arc42,
            chapter,
            stdout,
            ..
        } => insights_cmd::doc(
            cli,
            &root,
            &loaded,
            &graph,
            &report,
            insights_cmd::DocOptions {
                out: out.as_deref(),
                check: *check,
                arc42: *arc42,
                chapter: *chapter,
                stdout: *stdout,
            },
        ),
        Command::Graph {
            format,
            focus,
            depth,
            expand,
            ..
        } => graph_export(
            cli,
            &root,
            *format,
            focus.as_deref(),
            *depth,
            expand,
            &loaded,
            &graph,
            &report,
        ),
        Command::Dsm {
            expand,
            within,
            format,
            ..
        } => dsm_cmd::run(cli, *format, expand, within.as_deref(), &loaded, &graph),
        Command::Simulate {
            cut,
            move_node,
            merge,
            ops,
            format,
            ..
        } => simulate_cmd::run(
            cli,
            *format,
            cut,
            move_node,
            merge,
            ops.as_deref(),
            &loaded,
            &graph,
        ),
        Command::Suggest { cycle, max, .. } => suggest(cli, cycle.as_deref(), *max, &report),
        Command::Baseline { action } => baseline(cli, action, &report),
        Command::Config { .. } => config(cli, &report, &loaded),
        Command::Snapshot { tag, .. } => {
            trend_cmd::snapshot(cli, &root, tag.as_deref(), &report, &loaded)
        }
        // Dispatched before the analysis above; reaching them is a bug.
        Command::Init { .. }
        | Command::Review { .. }
        | Command::Mcp { .. }
        | Command::Db { .. }
        | Command::Trend { .. }
        | Command::Services { .. }
        | Command::Workspace { .. }
        | Command::Contracts { .. }
        | Command::Security { .. }
        | Command::Diff { .. }
        | Command::Serve { .. }
        | Command::Doctor
        | Command::Update { .. }
        | Command::Evaluate { .. }
        | Command::ReviewDecision { .. } => {
            unreachable!(
                "init, mcp, db, trend, services, security, serve, evaluate, review-decision, doctor and update are handled before the analysis"
            )
        }
    }
}

/// Review, review decisions and expert evaluation: they run their own
/// analysis (or none) instead of the shared analysis path.
fn review_command(cli: &Cli, root: &Path) -> Option<i32> {
    Some(match &cli.command {
        Command::Review {
            base,
            decisions,
            save,
            out,
            ..
        } => assessment_cmd::run(
            cli,
            root,
            base.as_deref(),
            decisions.as_deref(),
            *save,
            out.as_deref(),
        ),
        Command::Evaluate {
            assessment, init, ..
        } => evaluate_cmd::run(cli, root, assessment.as_deref(), *init),
        // The register needs no analysis; run before the analysis path.
        Command::ReviewDecision { action } => review_session_cmd::run(cli, root, action),
        _ => return None,
    })
}

/// The project root to analyse: the `PATH` argument or the current
/// directory, canonicalised so an unreadable path fails early (exit 3).
fn resolve_root(path: Option<&Path>) -> Result<PathBuf, std::io::Error> {
    let root = path.unwrap_or_else(|| Path::new("."));
    root.canonicalize().map_err(|err| {
        std::io::Error::other(format!("path {} not readable: {err}", root.display()))
    })
}

/// The `mcp` subcommand: canonicalise `--root` (an unreadable path is a
/// Kap. 9 error 3) and hand over to the stdio MCP server (Kap. 10).
fn mcp(root: &Path) -> i32 {
    let root = match resolve_root(Some(root)) {
        Ok(root) => root,
        Err(err) => return analysis_error(err),
    };
    match archguard_mcp::run_stdio_server(root) {
        Ok(()) => EXIT_OK,
        Err(err) => analysis_error(err),
    }
}

fn scan(
    cli: &Cli,
    root: &Path,
    out: Option<&Path>,
    graph: &Graph,
    report: &Report,
    loaded: &LoadedConfig,
) -> i32 {
    if matches!(
        cli.format,
        Format::Mermaid
            | Format::Sarif
            | Format::Dot
            | Format::Html
            | Format::Graphml
            | Format::Gexf
            | Format::Table
    ) {
        return usage(format!(
            "invalid value '{}' for '--format': only text and json are supported here",
            cli.format_name()
        ));
    }
    let saved = match out {
        Some(out) => write_json_file(
            out,
            &serde_json::to_string_pretty(graph).expect("graph serializes"),
        ),
        None => project_files::write_project_graph(root, graph, None),
    };
    if let Err(err) = saved.and_then(|()| project_files::write_effective_config(root, loaded)) {
        return analysis_error(err);
    }
    if !cli.quiet {
        match cli.format {
            Format::Json => emit(&to_json(report, true)),
            _ => emit(&to_text(report)),
        }
        if cli.verbose {
            println!(
                "graph with {} nodes and {} edges written to {}",
                graph.nodes.len(),
                graph.edges.len(),
                out.map(Path::to_path_buf)
                    .unwrap_or_else(|| root.join(".archguard/graph.json"))
                    .display()
            );
        }
    }
    EXIT_OK
}

fn cycles(
    cli: &Cli,
    root: &Path,
    max_cycles: Option<usize>,
    fail_on_found: bool,
    graph: &Graph,
    report: &Report,
    loaded: &LoadedConfig,
) -> i32 {
    if matches!(
        cli.format,
        Format::Sarif | Format::Dot | Format::Graphml | Format::Gexf | Format::Table
    ) {
        return usage(format!(
            "invalid value '{}' for '--format': cycles supports text, json, mermaid and html (sarif comes from check)",
            cli.format_name()
        ));
    }
    let mut shown: Vec<&Finding> = cycle_findings(report).collect();
    let total = shown.len();
    if let Some(max) = max_cycles {
        shown.truncate(max);
    }
    let diagram = match cli.format {
        Format::Mermaid => Some((
            "cycles.mmd",
            to_mermaid(graph, None, 0, &cyclic_edges(graph, &shown)),
        )),
        // Same viewer as graph, initially showing the cycles panel.
        Format::Html => match review_cmd::html(
            graph,
            loaded,
            None,
            0,
            &cyclic_edges(graph, &shown),
            &report.findings,
            Some("cycles"),
            None,
        ) {
            Ok(html) => Some(("cycles.html", html)),
            Err(error) => return analysis_error(error),
        },
        _ => None,
    };
    if let Some((name, text)) = &diagram {
        let saved = project_files::write_project_graph(root, graph, None)
            .and_then(|()| project_files::write_report(root, name, text))
            .and_then(|_| project_files::write_effective_config(root, loaded));
        if let Err(error) = saved {
            return analysis_error(error);
        }
    }
    if !cli.quiet {
        match cli.format {
            Format::Json => emit(&to_json(&cycle_report(report, &shown), true)),
            Format::Mermaid | Format::Html => emit(&diagram.as_ref().expect("rendered diagram").1),
            _ => {
                emit(&to_text(&cycle_report(report, &shown)));
                if shown.len() < total {
                    println!("({} of {} cycles shown)", shown.len(), total);
                }
            }
        }
        if cli.verbose {
            println!("{total} cycle findings");
        }
    }
    if fail_on_found && total > 0 {
        EXIT_FOUND
    } else {
        EXIT_OK
    }
}

/// The CI command (Kap. 9): the full report plus the quality gate.
///
/// `metrics`: one row per module, sorted by instability (descending),
/// then by fan-in + fan-out, then by name. `--module` filters to a
/// single module (an unknown name is a usage error, exit 2); `--top N`
/// caps the table.
fn metrics(
    cli: &Cli,
    root: &Path,
    module: Option<&str>,
    top: Option<usize>,
    report: &Report,
) -> i32 {
    if matches!(
        cli.format,
        Format::Mermaid
            | Format::Sarif
            | Format::Dot
            | Format::Html
            | Format::Graphml
            | Format::Gexf
            | Format::Table
    ) {
        return usage(format!(
            "invalid value '{}' for '--format': metrics supports text and json",
            cli.format_name()
        ));
    }
    if let Some(name) = module
        && !report.metrics.contains_key(name)
    {
        return usage(format!("unknown module '{name}'"));
    }
    let mut rows: Vec<(&str, &ModuleMetrics)> = report
        .metrics
        .iter()
        .filter(|(name, _)| module.is_none_or(|wanted| name.as_str() == wanted))
        .map(|(name, metrics)| (name.as_str(), metrics))
        .collect();
    rows.sort_by(|a, b| {
        b.1.instability
            .partial_cmp(&a.1.instability)
            .unwrap_or(Ordering::Equal)
            .then((b.1.fan_in + b.1.fan_out).cmp(&(a.1.fan_in + a.1.fan_out)))
            .then(a.0.cmp(b.0))
    });
    if let Some(n) = top {
        rows.truncate(n);
    }
    if !cli.quiet {
        if cli.format == Format::Json {
            let rows: Vec<Value> = rows
                .iter()
                .map(|(name, m)| {
                    json!({
                        "module": name,
                        "fan_in": m.fan_in,
                        "fan_out": m.fan_out,
                        "instability": m.instability,
                        "in_cycle": m.in_cycle,
                    })
                })
                .collect();
            emit(&to_pretty_json(&json!(rows)));
        } else {
            let width = rows
                .iter()
                .map(|(name, _)| name.len())
                .max()
                .unwrap_or(6)
                .max(6);
            println!(
                "{:<width$} {:>6} {:>7} {:>11} {:>9}",
                "MODULE",
                "FAN-IN",
                "FAN-OUT",
                "INSTABILITY",
                "IN-CYCLE",
                width = width
            );
            for (name, m) in rows {
                println!(
                    "{name:<width$} {:>6} {:>7} {:>11.2} {:>9}",
                    m.fan_in,
                    m.fan_out,
                    m.instability,
                    if m.in_cycle { "yes" } else { "no" },
                    width = width
                );
            }
            // R28: the review KPI table rides below the module table,
            // with deltas against the KPIs of the newest snapshot.
            if let Some(kpis) = &report.review_kpis {
                let baseline = archguard_engine::latest_snapshot(root).and_then(|s| s.review_kpis);
                for line in insights_cmd::review_kpi_lines(kpis, baseline.as_ref()) {
                    println!("{line}");
                }
            }
        }
    }
    EXIT_OK
}

/// `graph`: dot/mermaid/graphml/gexf/html via the report renderers
/// (focus and depth pass straight through to their `focus_subset`),
/// json as the canonical graph. Cyclic edges are highlighted from every
/// cycle finding.
///
/// With `--expand a,b` (T40) the export renders the module view the
/// analysis checked — [`Cut::at_depth`] at the effective zoom — with the
/// named modules unfolded. Static renderers and JSON use the aggregate;
/// HTML retains the full graph and starts with that cut so navigation and
/// source-backed review evidence remain available.
#[expect(clippy::too_many_arguments, reason = "one argument per CLI flag")]
fn graph_export(
    cli: &Cli,
    root: &Path,
    format: Format,
    focus: Option<&str>,
    depth: Option<u32>,
    expand: &[String],
    loaded: &LoadedConfig,
    graph: &Graph,
    report: &Report,
) -> i32 {
    if !matches!(
        format,
        Format::Dot
            | Format::Mermaid
            | Format::Json
            | Format::Html
            | Format::Graphml
            | Format::Gexf
    ) {
        return usage(format!(
            "invalid value '{}' for '--format': graph supports dot, mermaid, json, html, graphml and gexf",
            format_label(format)
        ));
    }
    // Unknown module names are a usage error even under `--quiet`, so
    // validate before the quiet shortcut.
    let expanded_cut = if expand.is_empty() {
        None
    } else {
        let base = view_util::module_cut(graph, loaded);
        match view_util::expand_cut(graph, &base, expand) {
            Ok(cut) => Some(cut),
            Err(name) => return usage(format!("unknown module '{name}' for '--expand'")),
        }
    };
    let expanded = expanded_cut
        .as_ref()
        .map(|cut| archguard_engine::core::aggregate(graph, cut));
    let shown_graph = expanded.as_ref().unwrap_or(graph);
    let text = match format {
        Format::Json => serde_json::to_string_pretty(shown_graph)
            .expect("graph is plain data and always serializable"),
        _ => {
            let shown: Vec<&Finding> = cycle_findings(report).collect();
            let cyclic_edges = cyclic_edges(shown_graph, &shown);
            // Without --focus the whole graph renders; with --focus the
            // default radius is 1 (the neighbourhood).
            let depth = depth.unwrap_or(u32::from(focus.is_some()));
            match format {
                Format::Mermaid => to_mermaid(shown_graph, focus, depth, &cyclic_edges),
                Format::Html => match review_cmd::html(
                    graph,
                    loaded,
                    focus,
                    depth,
                    &cycle_view::cyclic_edges(graph, &shown),
                    &report.findings,
                    None,
                    expanded_cut.as_ref(),
                ) {
                    Ok(html) => html,
                    Err(error) => return analysis_error(error),
                },
                Format::Graphml => to_graphml(shown_graph, focus, depth, &cyclic_edges),
                Format::Gexf => to_gexf(shown_graph, focus, depth, &cyclic_edges),
                _ => to_dot(shown_graph, focus, depth, &cyclic_edges),
            }
        }
    };
    let extension = match format {
        Format::Dot => "dot",
        Format::Mermaid => "mmd",
        Format::Json => "json",
        Format::Html => "html",
        Format::Graphml => "graphml",
        Format::Gexf => "gexf",
        _ => unreachable!(),
    };
    let saved = project_files::write_project_graph(root, graph, None)
        .and_then(|()| project_files::write_report(root, &format!("graph.{extension}"), &text))
        .and_then(|_| project_files::write_effective_config(root, loaded));
    if let Err(err) = saved {
        return analysis_error(err);
    }
    if !cli.quiet {
        emit(&text);
    }
    EXIT_OK
}

/// `suggest`: the [`SuggestedCut`]s already carried by the cycle
/// findings (Kap. 7). `--cycle <ID>` selects one finding (an unknown id
/// is a usage error), `--max N` caps the cuts per cycle.
fn suggest(cli: &Cli, cycle: Option<&str>, max: Option<usize>, report: &Report) -> i32 {
    if matches!(
        cli.format,
        Format::Mermaid
            | Format::Sarif
            | Format::Dot
            | Format::Html
            | Format::Graphml
            | Format::Gexf
            | Format::Table
    ) {
        return usage(format!(
            "invalid value '{}' for '--format': suggest supports text and json",
            cli.format_name()
        ));
    }
    let findings: Vec<&Finding> = report
        .findings
        .iter()
        .filter(|finding| finding.kind == FindingKind::Cycle)
        .filter(|finding| cycle.is_none_or(|id| finding.id == id))
        .collect();
    if let Some(id) = cycle
        && findings.is_empty()
    {
        return usage(format!("unknown cycle '{id}'"));
    }
    if cli.quiet {
        return EXIT_OK;
    }
    if cli.format == Format::Json {
        let out: Vec<Value> = findings
            .iter()
            .map(|finding| {
                let cuts: Vec<Value> = capped_cuts(finding, max)
                    .iter()
                    .map(|cut| {
                        json!({
                            "from": cut.from,
                            "to": cut.to,
                            "score": cut.score,
                            "reason": cut.reason,
                        })
                    })
                    .collect();
                json!({ "id": finding.id, "cuts": cuts })
            })
            .collect();
        emit(&to_pretty_json(&json!(out)));
    } else {
        for finding in &findings {
            println!("{}:", finding.id);
            for cut in capped_cuts(finding, max) {
                println!(
                    "  {} -> {} (score {:.2}): {}",
                    cut.from, cut.to, cut.score, cut.reason
                );
            }
        }
        if findings.is_empty() {
            println!("no cycle findings");
        }
    }
    EXIT_OK
}

/// `baseline write|diff` (Kap. 7 ratchet). The identity of a cycle is
/// Kap. 7's [`cycle_hash`] over the sorted member names of its finding
/// — the very hashes [`Baseline`] stores — so write and `check
/// --baseline` agree by construction.
fn baseline(cli: &Cli, action: &BaselineAction, report: &Report) -> i32 {
    match action {
        BaselineAction::Write { file, .. } => {
            let hashes: BTreeSet<String> = cycle_hashes(report).into_values().collect();
            let baseline = Baseline {
                schema_version: 1,
                hashes,
            };
            let json = match serde_json::to_string_pretty(&baseline) {
                Ok(json) => json,
                Err(err) => return analysis_error(err),
            };
            if let Err(err) = write_json_file(file, &json) {
                return analysis_error(err);
            }
            if !cli.quiet {
                println!(
                    "wrote {} cycle hashes to {}",
                    cycle_hashes(report).len(),
                    file.display()
                );
            }
            EXIT_OK
        }
        BaselineAction::Diff { file, .. } => {
            let baseline = match read_baseline(file) {
                Ok(baseline) => baseline,
                Err(err) => return analysis_error(err),
            };
            let current = cycle_hashes(report);
            let current_hashes: BTreeSet<String> = current.values().cloned().collect();
            let added: Vec<(&String, &String)> = current
                .iter()
                .filter(|(_, hash)| !baseline.hashes.contains(*hash))
                .collect();
            let removed: Vec<&String> = baseline
                .hashes
                .iter()
                .filter(|hash| !current_hashes.contains(*hash))
                .collect();
            if !cli.quiet {
                for (id, hash) in &added {
                    println!("ADDED {id} {hash}");
                }
                for hash in &removed {
                    println!("REMOVED {hash}");
                }
                if added.is_empty() && removed.is_empty() {
                    println!("baseline matches: {} cycles accepted", current.len());
                }
            }
            // Kap. 9: only NEW cycles block; resolved ones are reported.
            if added.is_empty() {
                EXIT_OK
            } else {
                EXIT_FOUND
            }
        }
    }
}

/// `init`: write the derived `.archguard/archguard.toml` into the project.
/// Refuses to overwrite or shadow an existing config (exit 3 — an IO error,
/// not a usage error); `--defaults` writes the commented stock
/// defaults instead of the derived config.
fn init(cli: &Cli, root: &Path, defaults: bool) -> i32 {
    let target = root.join(archguard_engine::DEFAULT_CONFIG_PATH);
    for existing in [&target, &root.join("archguard.toml")] {
        if fs::symlink_metadata(existing).is_ok() {
            return analysis_error(format!(
                "refusing to overwrite or shadow existing {}",
                existing.display()
            ));
        }
    }
    let text = if defaults {
        default_config_string()
    } else {
        effective_config_string(root)
    };
    let text = if text.ends_with('\n') {
        text
    } else {
        format!("{text}\n")
    };
    let result = project_files::ensure_ignore_rules(root).and_then(|()| {
        archguard_engine::write_project_file(
            root,
            Path::new("archguard.toml"),
            text.as_bytes(),
            false,
        )
    });
    if let Err(err) = result {
        return analysis_error(err);
    }
    if !cli.quiet {
        println!("wrote {}", target.display());
    }
    EXIT_OK
}

/// `config` (Kap. 9/11): the per-section provenance of the effective
/// config, taken from [`Report::config`] produced by
/// [`analyze_run`] — so declared sections show `Declared`.
fn config(cli: &Cli, report: &Report, loaded: &LoadedConfig) -> i32 {
    if matches!(
        cli.format,
        Format::Mermaid
            | Format::Sarif
            | Format::Dot
            | Format::Html
            | Format::Graphml
            | Format::Gexf
            | Format::Table
    ) {
        return usage(format!(
            "invalid value '{}' for '--format': config supports text and json",
            cli.format_name()
        ));
    }
    if cli.quiet {
        return EXIT_OK;
    }
    if cli.format == Format::Json {
        emit(&to_pretty_json(
            &json!({ "config": &report.config, "effective": &loaded.config }),
        ));
    } else {
        println!("effective config provenance (declared ⇐ derived ⇐ default):");
        let width = report
            .config
            .keys()
            .map(String::len)
            .max()
            .unwrap_or(0)
            .max(7);
        for (section, source) in &report.config {
            println!("{section:<width$}  {}", source_name(*source));
        }
    }
    EXIT_OK
}

/// `scc-N` id → [`cycle_hash`] of the finding's member names, for every
/// cycle finding (the identity [`Baseline`] stores, Kap. 7).
fn cycle_hashes(report: &Report) -> BTreeMap<String, String> {
    cycle_findings(report)
        .map(|finding| (finding.id.clone(), cycle_hash(&finding.nodes)))
        .collect()
}

/// The first `max` suggested cuts of a finding (all of them without a cap).
fn capped_cuts(finding: &Finding, max: Option<usize>) -> &[archguard_engine::core::SuggestedCut] {
    match max {
        Some(max) => &finding.suggested_cuts[..finding.suggested_cuts.len().min(max)],
        None => &finding.suggested_cuts,
    }
}

/// Write `json` (plus trailing newline) to `path`, creating parent
/// directories; IO errors surface for the caller's exit-3 mapping.
fn write_json_file(path: &Path, json: &str) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{json}\n"))
}

/// A copy of `report` carrying exactly the given (already capped) cycle
/// findings, so the archguard-report renderers produce a cycles-only view.
fn cycle_report(report: &Report, shown: &[&Finding]) -> Report {
    Report {
        semantic: report.semantic.clone(),
        review_kpis: report.review_kpis.clone(),
        schema_version: report.schema_version,
        summary: report.summary.clone(),
        config: report.config.clone(),
        findings: shown.iter().map(|finding| (*finding).clone()).collect(),
        metrics: report.metrics.clone(),
    }
}

/// The lowercase name of a [`ConfigSource`](archguard_engine::core::ConfigSource)
/// (same spelling as its serde form, used in `config --format text`).
fn source_name(source: archguard_engine::core::ConfigSource) -> &'static str {
    use archguard_engine::core::ConfigSource;
    match source {
        ConfigSource::Declared => "declared",
        ConfigSource::Derived => "derived",
        ConfigSource::Default => "default",
        ConfigSource::Mixed => "mixed",
    }
}

impl Cli {
    /// The CLI spelling of the selected global format (for usage errors).
    fn format_name(&self) -> &'static str {
        format_label(self.format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measure_zones_is_an_opt_in_on_scan_and_check_only() {
        let parse = |args: &[&str]| Cli::try_parse_from(args).map(|cli| cli.command);
        assert!(!parse(&["archguard", "scan"]).unwrap().measure_zones());
        assert!(!parse(&["archguard", "check"]).unwrap().measure_zones());
        assert!(
            parse(&["archguard", "scan", "--measure-zones"])
                .unwrap()
                .measure_zones()
        );
        assert!(
            parse(&["archguard", "check", "--measure-zones"])
                .unwrap()
                .measure_zones()
        );
        assert!(parse(&["archguard", "metrics", "--measure-zones"]).is_err());
    }
}
