//! Review priorities and shared source evidence for HTML exports.
use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};
use archguard_engine::analysis::{analyze_review, match_review_entry_points};
use archguard_engine::core::{ArchguardError, Cut, Finding, Graph, NodeId, aggregate};
use archguard_engine::report::{HtmlContext, HtmlPage, to_html_page, to_html_with_context};
use archguard_engine::{LoadedConfig, collect_review_sources};
use std::path::Path;

pub(crate) fn run(
    cli: &Cli,
    root: &Path,
    graph: &Graph,
    loaded: &LoadedConfig,
    top: usize,
    since: Option<&str>,
) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("hotspots supports text and json");
    }
    let depth = if matches!(cli.command.level(), archguard_engine::Level::File) {
        usize::MAX
    } else {
        loaded.config.modules.depth as usize
    };
    let view = aggregate(graph, &Cut::at_depth(graph, depth));
    let result = collect_review_sources(
        root,
        &loaded.config,
        graph,
        since.unwrap_or(&loaded.config.review.git_since),
    )
    .and_then(|sources| analyze_review(graph, &view, &sources, &loaded.config.review.entry_points));
    let mut report = match result {
        Ok(report) => report,
        Err(error) => return analysis_error(error),
    };
    let total = report.nodes.len();
    report.nodes.truncate(top);
    if !cli.quiet {
        if cli.format == Format::Json {
            let mut value = serde_json::to_value(&report).expect("review is serializable");
            value["total"] = total.into();
            value["truncated"] = (top < total).into();
            emit(&serde_json::to_string_pretty(&value).unwrap());
        } else {
            println!(
                "Review priorities · {} of {total} candidates · score is a heuristic, not defect probability",
                report.nodes.len()
            );
            for row in &report.nodes {
                println!(
                    "{:>3}. {:>3}/100 {:?} {}",
                    row.rank, row.score, row.priority, row.name
                );
                for reason in &row.reasons {
                    println!("     {reason}");
                }
            }
            if let Some(message) = &report.git.message {
                println!("Git: {message}");
            }
            for limitation in &report.limitations {
                println!("Note: {limitation}");
            }
        }
    }
    EXIT_OK
}

/// `archguard review --format pr-comment` (R27): the Markdown comment
/// comparing `--base` against the working tree, annotated with the
/// decision register. Records nothing — the register only decorates.
pub(crate) fn pr_comment(cli: &Cli, root: &Path, base: Option<&str>, out: Option<&Path>) -> i32 {
    let Some(base) = base else {
        return usage("pr-comment needs --base <git-ref> to compare against");
    };
    let result = || -> Result<String, ArchguardError> {
        let root = root.canonicalize()?;
        let loaded = archguard_engine::load_config(cli.config.as_deref(), &root)?;
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)?;
        let view = aggregate(
            &run.graph,
            &Cut::at_depth(&run.graph, run.config.modules.depth as usize),
        );
        let sources =
            collect_review_sources(&root, &run.config, &run.graph, &run.config.review.git_since)?;
        let current = analyze_review(&run.graph, &view, &sources, &run.config.review.entry_points)?;
        // Same effective-policy comparison the review session uses; the
        // base priority report rides along in the comparison (R27).
        let comparison = archguard_engine::compare_to_ref(
            &root,
            &run.effective_config(),
            &run.graph,
            &run.report,
            base,
        )?;
        let base_report = comparison.base_review.ok_or_else(|| {
            ArchguardError::Analysis("comparison returned no base review report".to_string())
        })?;
        let register = archguard_engine::load_register(&root)?;
        Ok(archguard_engine::report::render_pr_comment(
            &base_report,
            &current,
            &register,
        ))
    };
    match result() {
        Ok(markdown) => match out {
            Some(path) => {
                let target = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    root.join(path)
                };
                if let Err(error) = write_pr_comment(&target, &markdown) {
                    return analysis_error(error);
                }
                if !cli.quiet {
                    println!("Wrote {}", target.display());
                }
                EXIT_OK
            }
            None => {
                emit(&markdown);
                EXIT_OK
            }
        },
        Err(error) => analysis_error(error),
    }
}

fn write_pr_comment(path: &Path, markdown: &str) -> Result<(), ArchguardError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, markdown)?;
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "same explicit view inputs as the HTML renderer"
)]
pub(crate) fn html(
    graph: &Graph,
    loaded: &LoadedConfig,
    focus: Option<&str>,
    depth: u32,
    cyclic: &std::collections::HashSet<(NodeId, NodeId)>,
    findings: &[Finding],
    initial_tab: Option<&str>,
    initial_cut: Option<&Cut>,
) -> Result<String, ArchguardError> {
    let services = archguard_engine::analyze_services(Path::new(&graph.root))?;
    let security = archguard_engine::read_security(Path::new(&graph.root), &loaded.config);
    html_page_with_services(
        graph,
        loaded,
        focus,
        depth,
        cyclic,
        findings,
        initial_tab,
        initial_cut,
        &services,
        &security,
        None,
        None,
    )
    .map(|page| page.html)
}

#[expect(
    clippy::too_many_arguments,
    reason = "same view inputs plus the live probe's precomputed service evidence"
)]
/// `defer_above` lets `serve` move payload sections above that many bytes out
/// of the page (see [`archguard_engine::report::to_html_page`]).
pub(crate) fn html_page_with_services(
    graph: &Graph,
    loaded: &LoadedConfig,
    focus: Option<&str>,
    depth: u32,
    cyclic: &std::collections::HashSet<(NodeId, NodeId)>,
    findings: &[Finding],
    initial_tab: Option<&str>,
    initial_cut: Option<&Cut>,
    services: &archguard_engine::core::ServiceTopology,
    security: &archguard_engine::SecurityReport,
    repository: Option<&serde_json::Value>,
    defer_above: Option<usize>,
) -> Result<HtmlPage, ArchguardError> {
    let root = Path::new(&graph.root);
    let sources =
        collect_review_sources(root, &loaded.config, graph, &loaded.config.review.git_since)?;
    let ids = match_review_entry_points(graph, &sources, &loaded.config.review.entry_points)?;
    let calls = archguard_engine::analyze_calls(root, &loaded.config);
    let default_cut = Cut::at_depth(graph, depth as usize);
    let view = aggregate(graph, initial_cut.unwrap_or(&default_cut));
    let performance = archguard_engine::analyze_performance(
        root,
        &loaded.config,
        graph,
        &view,
        &sources,
        &calls,
        &ids,
    )?;
    // The pipeline runs no compiler, so its KPI block leaves zones unmeasured;
    // the abstractness computed here for the viewer fills them in.
    let review_kpis = graph.review_kpis.clone().map(|mut kpis| {
        kpis.zones = archguard_engine::review_zones(&view, &performance);
        kpis
    });
    let ownership =
        archguard_engine::analyze_ownership(root, graph, &sources, &loaded.config.ownership_rules)?;
    let co_change = archguard_engine::co_change(
        root,
        &loaded.config,
        graph,
        &archguard_engine::CoChangeOptions {
            since: loaded.config.review.git_since.clone(),
            ..Default::default()
        },
    )?;
    let report = archguard_engine::analysis::summarize(
        &view,
        &archguard_engine::analysis::find_sccs(&view),
        findings,
    );
    let context_report = archguard_engine::core::Report {
        semantic: graph.semantic.clone(),
        review_kpis: graph.review_kpis.clone(),
        schema_version: 1,
        summary: report,
        config: loaded.sources.clone(),
        findings: findings.to_vec(),
        metrics: Default::default(),
    };
    let architecture =
        archguard_engine::architecture_context(root, &loaded.config, graph, &context_report);
    let patterns = archguard_engine::analyze_architecture_patterns(
        root,
        &loaded.config,
        graph,
        &calls,
        services,
    );
    let test_coverage = archguard_engine::read_test_coverage(root, &loaded.config, graph);
    let coverage_review = archguard_engine::test_coverage_review(graph, &test_coverage);
    let world = archguard_engine::architecture_world(graph, &calls, &test_coverage);
    let quality = archguard_engine::read_quality_findings(root, &loaded.config, graph);
    let cohesion = archguard_engine::analyze_cohesion(graph, Some(&co_change));
    let quality_history = archguard_engine::read_quality_history(root);
    let code_metrics = archguard_engine::code_metrics(&calls);
    let knowledge_metrics = archguard_engine::knowledge_metrics(root, &loaded.config, graph)?;
    let measurement_runs = archguard_engine::read_measurement_runs(root);
    let test_evidence = archguard_engine::read_test_evidence(root, &loaded.config, graph);
    let mutations = archguard_engine::read_mutations(root, &loaded.config, graph);
    let duplicates = archguard_engine::read_duplicates(root, &loaded.config, graph);
    let contracts = archguard_engine::read_api_contracts(root, services);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| ArchguardError::Analysis(e.to_string()))?
        .as_secs();
    let review_session = archguard_engine::read_saved_review(root, loaded, now);
    // Review Dashboard inputs (R40/R39/R42). An absent register or PR
    // comment stays None — the panels state the absence, never an empty
    // list. The PR Markdown is the last file the CLI or MCP wrote, so the
    // panel labels its provenance; the history rows come from the same
    // `.archguard/history/` files `archguard trend` reads (R09).
    let review_register = archguard_engine::register_path(root)
        .exists()
        .then(|| archguard_engine::load_register(root).ok())
        .flatten();
    // Health series for the R39 sparkline (viewer_dash_health.js): one row
    // per snapshot in `read_snapshots` order (oldest first), carrying the
    // composite health score. Pre-R28 history files have no review_kpis at
    // all, so their row keeps score null — "no baseline", never a zero.
    let health_rows = archguard_engine::read_snapshots(root)
        .iter()
        .map(|snapshot| {
            serde_json::json!({
                "timestamp": snapshot.timestamp,
                "tag": snapshot.tag,
                "score": snapshot
                    .review_kpis
                    .as_ref()
                    .and_then(|kpis| kpis.health.as_ref())
                    .and_then(|health| health.score),
            })
        })
        .collect::<Vec<_>>();
    let review_history = serde_json::to_value(health_rows).unwrap_or(serde_json::Value::Null);
    let pr_comment_markdown = std::fs::read_to_string(
        root.join(".archguard")
            .join("reports")
            .join("pr-comment.md"),
    )
    .ok();
    let workspace_root = repository
        .and_then(|r| r["root"].as_str())
        .map(Path::new)
        .unwrap_or(root);
    let workspace = archguard_engine::analyze_workspace(workspace_root)?;
    let mut insights = serde_json::json!({"code_metrics":code_metrics,"knowledge_metrics":knowledge_metrics,"measurement_runs":measurement_runs,"patterns":patterns,"ownership":ownership,"co_change":co_change,"architecture":architecture,"security":security,"repository":repository,"test_coverage":test_coverage,"coverage_review":coverage_review,"quality":quality,"cohesion":cohesion,"quality_history":quality_history,"test_evidence":test_evidence,"mutations":mutations,"duplicates":duplicates,"contracts":contracts,"workspace":workspace,"review_session":review_session});
    insights["world"] = serde_json::to_value(world).expect("serializable world");
    let arc42 = archguard_engine::arc42_from_evidence(
        graph,
        &loaded.config,
        findings,
        &calls,
        services,
        &insights,
    )?;
    insights["arc42"] = serde_json::json!({"schema_version":arc42.schema_version,"scope":arc42.scope,"markdown":arc42.markdown(None),"chapters":arc42.chapters.iter().map(|c|serde_json::json!({"number":c.number,"title":c.title,"questions":c.questions,"markdown":arc42.markdown(Some(c.number))})).collect::<Vec<_>>()});
    Ok(to_html_page(
        graph,
        focus,
        depth,
        cyclic,
        findings,
        initial_tab,
        &HtmlContext {
            calls: Some(&calls),
            review_sources: Some(&sources),
            entry_point_ids: &ids,
            entry_points: &loaded.config.review.entry_points,
            performance: Some(&performance),
            insights: Some(&insights),
            services: Some(services),
            initial_cut,
            review_register: review_register.as_ref(),
            review_history: Some(&review_history),
            pr_comment_markdown: pr_comment_markdown.as_deref(),
            review_kpis: review_kpis.as_ref(),
        },
        defer_above,
    ))
}

/// Repository landing page: source analysis is deferred until a project is chosen.
/// Independent applications may declare identical classes and must not share a compiler scope.
pub(crate) fn repository_html(
    root: &Path,
    _loaded: &LoadedConfig,
    services: &archguard_engine::core::ServiceTopology,
    security: &archguard_engine::SecurityReport,
    repository: &serde_json::Value,
    workspace: &archguard_engine::core::WorkspaceReport,
) -> Result<String, ArchguardError> {
    let graph =
        archguard_engine::core::GraphBuilder::new().build(root.to_string_lossy().into_owned());
    let contracts = archguard_engine::read_api_contracts(root, services);
    let patterns = archguard_engine::analyze_architecture_patterns(
        root,
        &_loaded.config,
        &graph,
        &Default::default(),
        services,
    );
    let insights = serde_json::json!({"patterns":patterns,"security":security,"repository":repository,"contracts":contracts,"workspace":workspace});
    Ok(to_html_with_context(
        &graph,
        None,
        0,
        &Default::default(),
        &[],
        Some("services"),
        &HtmlContext {
            insights: Some(&insights),
            services: Some(services),
            ..Default::default()
        },
    ))
}
