//! Test-report import is explicit; ordinary reads never execute project tests.
use crate::args::{Cli, Format};
use crate::output::{EXIT_ERROR, EXIT_OK, analysis_error, emit, usage};
use archguard_engine::{
    LoadedConfig, core::Graph, import_test_coverage, read_test_coverage, test_coverage_review,
};
use std::path::Path;

pub(crate) fn run(
    cli: &Cli,
    root: &Path,
    loaded: &LoadedConfig,
    graph: &Graph,
    report: Option<&Path>,
    source_root: Option<&Path>,
) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("coverage supports text and json");
    }
    let coverage = if let Some(report) = report {
        match import_test_coverage(root, &loaded.config, graph, report, source_root) {
            Ok(coverage) => {
                if let Err(error) = crate::project_files::ensure_ignore_rules(root) {
                    return analysis_error(error);
                }
                coverage
            }
            Err(error) => return analysis_error(error),
        }
    } else {
        read_test_coverage(root, &loaded.config, graph)
    };
    let review = test_coverage_review(graph, &coverage);
    if !cli.quiet {
        if cli.format == Format::Json {
            let mut value = serde_json::to_value(&coverage).expect("coverage serializes");
            value["review"] = serde_json::to_value(&review).expect("coverage review serializes");
            emit(&serde_json::to_string_pretty(&value).unwrap());
        } else {
            println!(
                "Test coverage: {} · {} mapped files · {} unmatched",
                coverage.status,
                coverage.files.len(),
                coverage.unmatched_files
            );
            println!(
                "Review index = uncovered line fraction × (1 + transitive file dependents); heuristic, not defect probability."
            );
            for row in &review.files {
                let file = coverage
                    .files
                    .iter()
                    .find(|file| file.node == row.node)
                    .expect("ranked coverage file");
                let counter = |value: &Option<archguard_engine::CoverageCounter>| {
                    value
                        .as_ref()
                        .filter(|v| v.total > 0)
                        .map(|v| {
                            format!(
                                "{}/{} ({:.1}%)",
                                v.covered,
                                v.total,
                                100.0 * v.covered as f64 / v.total as f64
                            )
                        })
                        .unwrap_or_else(|| "unknown".into())
                };
                println!(
                    "{} · lines {} · branches {} · dependents {} · index {} · {}",
                    row.node,
                    counter(&file.lines),
                    counter(&file.branches),
                    row.dependents
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".into()),
                    row.score
                        .map(|v| format!("{v:.2}"))
                        .unwrap_or_else(|| "unknown".into()),
                    file.status
                );
            }
            for diagnostic in &coverage.diagnostics {
                println!("Note: {diagnostic}");
            }
            if coverage.status == "not_imported" {
                println!(
                    "Import: archguard coverage <project> --report <jacoco.xml|cobertura.xml|clover.xml|lcov.info> [--source-root src/main/java]"
                );
            }
        }
    }
    if coverage.status == "error" {
        EXIT_ERROR
    } else {
        EXIT_OK
    }
}
