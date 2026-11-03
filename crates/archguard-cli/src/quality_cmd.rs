//! Explicit SARIF import and opt-in changed-code quality checks.
use crate::args::{Cli, Format};
use crate::output::{EXIT_ERROR, EXIT_FOUND, EXIT_OK, analysis_error, emit, usage};
use archguard_engine::{
    LoadedConfig, QualityGateOptions, core::Graph, evaluate_quality_gate, import_quality_findings,
    read_quality_findings,
};
use std::path::Path;

pub(crate) struct QualityRequest<'a> {
    pub report: Option<&'a Path>,
    pub source_root: Option<&'a Path>,
    pub base: Option<&'a str>,
    pub thresholds: QualityGateOptions,
}

pub(crate) fn run(
    cli: &Cli,
    root: &Path,
    loaded: &LoadedConfig,
    graph: &Graph,
    request: QualityRequest<'_>,
) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("quality supports text and json");
    }
    let options = &request.thresholds;
    let gate_requested = options.min_changed_coverage.is_some()
        || options.max_complexity.is_some()
        || options.max_nesting.is_some();
    if request.base.is_some() != gate_requested {
        return usage(
            "quality gates require --base and at least one of --min-changed-coverage, --max-complexity or --max-nesting",
        );
    }
    if options
        .min_changed_coverage
        .is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value))
    {
        return usage("--min-changed-coverage must be a finite percentage from 0 to 100");
    }
    if options.max_complexity == Some(0) {
        return usage("--max-complexity must be at least 1");
    }
    if request
        .base
        .is_some_and(|base| base.trim().is_empty() || base.contains('\0'))
    {
        return usage("--base must be a nonempty Git ref without NUL");
    }
    let report = if let Some(path) = request.report {
        match import_quality_findings(root, &loaded.config, graph, path, request.source_root) {
            Ok(report) => {
                if let Err(error) = crate::project_files::ensure_ignore_rules(root) {
                    return analysis_error(error);
                }
                report
            }
            Err(error) => return analysis_error(error),
        }
    } else {
        read_quality_findings(root, &loaded.config, graph)
    };
    let gate = if let Some(base) = request.base {
        match evaluate_quality_gate(root, &loaded.config, graph, base, options) {
            Ok(report) => Some(report),
            Err(error) => return analysis_error(error),
        }
    } else {
        None
    };
    if !cli.quiet {
        if cli.format == Format::Json {
            let mut value = serde_json::json!({"report":report});
            if let Some(gate) = &gate {
                value["gate"] = serde_json::to_value(gate).expect("quality gate serializes");
            }
            emit(&serde_json::to_string_pretty(&value).expect("quality result serializes"));
        } else {
            println!(
                "Imported quality: {} · {} findings",
                report.status,
                report.findings.len()
            );
            for finding in &report.findings {
                println!(
                    "{} / {} · {}{} · {}{} · {}",
                    finding.tool,
                    finding.rule_id,
                    finding.level,
                    if finding.suppressed {
                        " (suppressed)"
                    } else {
                        ""
                    },
                    finding.path.as_deref().unwrap_or("unmapped"),
                    finding
                        .line
                        .map(|line| format!(":{line}"))
                        .unwrap_or_default(),
                    finding.message
                );
                println!("  Source status: {}", finding.status);
                if let Some(state) = &finding.baseline_state {
                    println!("  Producer baseline state: {state}");
                }
            }
            for diagnostic in &report.diagnostics {
                println!("Note: {diagnostic}");
            }
            if report.status == "not_imported" {
                println!(
                    "Import: archguard quality <project> --report <results.sarif> [--source-root <path>]"
                );
            }
            if let Some(gate) = &gate {
                println!(
                    "Changed-code quality: {} · base {} ({}) · {} changed source files / {} changed lines",
                    gate.status,
                    gate.base_ref,
                    gate.base_commit,
                    gate.changed_files,
                    gate.changed_lines
                );
                if let Some(coverage) = &gate.coverage {
                    println!(
                        "Changed executable lines: {}/{} covered · {} · {} unknown files",
                        coverage.covered_lines,
                        coverage.total_lines,
                        coverage
                            .percent
                            .map(|value| format!("{value:.2}%"))
                            .unwrap_or_else(|| "not applicable / unknown".into()),
                        coverage.unknown_files
                    );
                }
                for finding in &gate.findings {
                    println!("{}: {}", finding.kind, finding.message);
                }
                for diagnostic in &gate.diagnostics {
                    println!("Note: {diagnostic}");
                }
            }
        }
    }
    if report.status == "error"
        || gate
            .as_ref()
            .is_some_and(|gate| gate.status == "incomplete")
    {
        EXIT_ERROR
    } else if gate.as_ref().is_some_and(|gate| gate.status == "fail") {
        EXIT_FOUND
    } else {
        EXIT_OK
    }
}
