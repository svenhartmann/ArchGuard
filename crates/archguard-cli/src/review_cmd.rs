//! Review priorities (`hotspots`) and the PR comment.
use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};
use archguard_engine::analysis::analyze_review;
use archguard_engine::core::{ArchguardError, Cut, Graph, aggregate};
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
            outln!(
                "Review priorities · {} of {total} candidates · score is a heuristic, not defect probability",
                report.nodes.len()
            );
            for row in &report.nodes {
                outln!(
                    "{:>3}. {:>3}/100 {:?} {}",
                    row.rank, row.score, row.priority, row.name
                );
                for reason in &row.reasons {
                    outln!("     {reason}");
                }
            }
            if let Some(message) = &report.git.message {
                outln!("Git: {message}");
            }
            for limitation in &report.limitations {
                outln!("Note: {limitation}");
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
        let named = (&run.graph, &run.source_names);
        let sources =
            collect_review_sources(&root, &run.config, named, &run.config.review.git_since)?;
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
                if let Err(error) = write_pr_comment(root, &target, &markdown) {
                    return analysis_error(error);
                }
                if !cli.quiet {
                    outln!("Wrote {}", target.display());
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

/// Below `<root>/.archguard` no component may be a symlink (the default
/// path); any other `--out` the user chose gets the document refusal.
fn write_pr_comment(root: &Path, path: &Path, markdown: &str) -> Result<(), ArchguardError> {
    match path.strip_prefix(root.join(".archguard")) {
        Ok(relative) => {
            archguard_engine::write_project_file(root, relative, markdown.as_bytes(), true)?;
        }
        Err(_) => crate::insights_cmd::write_document(path, markdown)?,
    }
    Ok(())
}
