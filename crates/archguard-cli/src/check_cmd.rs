//! Optional Git diff and relative ACD gate around the existing check command.
use crate::args::{Cli, Format};
use crate::output::{EXIT_FOUND, EXIT_OK, analysis_error, emit, usage};
use crate::sarif::to_sarif;
use archguard_engine::core::{Finding, FindingKind, Report, Severity};
/// The baseline reader `baseline diff` shares with the gate.
pub(crate) use archguard_engine::read_baseline;
use archguard_engine::report::{to_json, to_text};
use archguard_engine::{LoadedConfig, check_gate_findings, compare_to_ref, gate_fails, read_trend};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
#[expect(
    clippy::too_many_arguments,
    reason = "explicit check flags and shared run"
)]
pub(crate) fn run(
    cli: &Cli,
    root: &Path,
    loaded: &LoadedConfig,
    graph: &archguard_engine::core::Graph,
    report: &Report,
    baseline: Option<Option<&Path>>,
    base: Option<&str>,
    acd_limit: Option<f64>,
) -> i32 {
    if let Some(code) = precondition_error(root, graph, acd_limit) {
        return code;
    }
    let comparison = match base
        .map(|base| compare_to_ref(root, loaded, graph, report, base))
        .transpose()
    {
        Ok(value) => value,
        Err(error) => return analysis_error(error),
    };
    let mut checked = report.clone();
    if let Some(comparison) = &comparison {
        checked.findings = comparison.new_findings.clone();
    }
    if let Some(limit) = acd_limit {
        let history = read_trend(root, 1);
        if comparison.is_none()
            && history.last().is_some_and(|row| {
                row.analysis_scope.as_deref()
                    != Some(&archguard_engine::snapshot_scope(&loaded.config))
            })
        {
            return analysis_error(
                "ACD snapshot scope is missing or differs from current configuration; use --base or create a new snapshot with this scope",
            );
        }
        let previous = match comparison
            .as_ref()
            .map(|c| c.base_summary.acd)
            .or_else(|| history.last().map(|row| row.acd))
        {
            Some(value) => value,
            None => {
                return analysis_error(
                    "ACD regression gate needs --base or a saved snapshot; run archguard snapshot first",
                );
            }
        };
        if report.summary.acd > previous * (1.0 + limit / 100.0) {
            checked.findings.push(Finding {kind:FindingKind::MetricThreshold,id:"metric-acd-regression".into(),severity:Severity::Error,nodes:vec![],edge_count:0,cycles_sampled:0,sampling:None,suggested_cuts:vec![],evidence:vec![],message:Some(format!("ACD rose from {previous:.4} to {:.4}; allowed growth is {limit}%. This measures structural coupling, not elapsed build or runtime cost.",report.summary.acd))});
        }
    }
    // R31/ADR 0016: the `[review]` KPI gates and the `[licenses]` policy
    // join the finding list, so text, json and sarif all carry them. Like
    // the ACD regression gate they speak about the current structure, so
    // they are evaluated even when a comparison replaced the list.
    checked
        .findings
        .extend(check_gate_findings(root, &loaded.config, report));
    let paths = if cli.format == crate::args::Format::Sarif {
        match archguard_engine::source_reference_paths(root, &loaded.config) {
            Ok(paths) => paths,
            Err(error) => return analysis_error(error),
        }
    } else {
        Default::default()
    };
    let baseline = baseline.map(|file| load_baseline(root, loaded, file));
    check(cli, baseline, &checked, comparison.as_ref(), &paths)
}

/// The baseline file: `file` as given (relative to the working directory),
/// or `[rules].baseline` under the project `root` — the file MCP reads too.
/// The configured path is repository state: it must stay relative without
/// `..`, and a symlink on its way is refused. An operator `file` is not.
pub(crate) fn baseline_path(
    root: &Path,
    loaded: &LoadedConfig,
    file: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(file) = file {
        return Ok(file.to_path_buf());
    }
    let configured = Path::new(&loaded.config.rules.baseline);
    let mut path = root.to_path_buf();
    for part in configured.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return Err(format!(
                "[rules].baseline {:?} must be a relative path inside the project without `..`",
                configured.display().to_string()
            ));
        }
        path.push(part);
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(format!(
                "[rules].baseline: {} is a symlink and is not followed",
                path.display()
            ));
        }
    }
    Ok(path)
}

/// The baseline [`baseline_path`] names; the configured one is read
/// descriptor-relative, so a symlink swapped in after the check is not followed.
pub(crate) fn load_baseline(
    root: &Path,
    loaded: &LoadedConfig,
    file: Option<&Path>,
) -> Result<archguard_engine::analysis::Baseline, String> {
    let path = baseline_path(root, loaded, file)?;
    match file {
        Some(_) => read_baseline(&path),
        None => {
            archguard_engine::read_repo_baseline(root, Path::new(&loaded.config.rules.baseline))
        }
    }
}

/// `baseline write`: an operator `file` is written as given; the
/// repository-controlled `[rules].baseline` only below `.archguard`, through
/// the descriptor-relative writer that never follows a symlink.
pub(crate) fn write_baseline(
    root: &Path,
    loaded: &LoadedConfig,
    file: Option<&Path>,
    baseline: &archguard_engine::analysis::Baseline,
) -> Result<PathBuf, String> {
    let path = baseline_path(root, loaded, file)?;
    let json = serde_json::to_string_pretty(baseline).map_err(|err| err.to_string())?;
    let written = match file {
        Some(_) => crate::insights_cmd::write_document(&path, &format!("{json}\n")),
        None => Path::new(&loaded.config.rules.baseline)
            .strip_prefix(".archguard")
            .map_err(|_| {
                std::io::Error::other(
                    "[rules].baseline must lie below .archguard to be written; pass a file argument",
                )
            })
            .and_then(|relative| {
                archguard_engine::write_project_file(root, relative, format!("{json}\n").as_bytes(), true)
            })
            .map(drop),
    };
    written.map(|()| path).map_err(|err| err.to_string())
}

/// The exit code of a check that must not run: an invalid ACD limit
/// (usage) or an analysis without any source file (error).
fn precondition_error(
    root: &Path,
    graph: &archguard_engine::core::Graph,
    acd_limit: Option<f64>,
) -> Option<i32> {
    if acd_limit.is_some_and(|n| !n.is_finite() || n < 0.0) {
        return Some(usage(
            "--fail-on-acd-regression must be finite and nonnegative",
        ));
    }
    archguard_engine::empty_analysis_error(root, graph).map(analysis_error)
}

/// Gate semantics ([`gate_fails`]): `check` fails (exit 1) as soon as one [`Finding`]
/// has severity `Error` — the same rule that sets
/// [`Report::summary`] to `Fail` (Kap. 5). With `--baseline <DATEI>`
/// (default: [`baseline_path`]), **cycle** findings whose
/// membership hash — Kap. 7's [`archguard_engine::analysis::cycle_hash`] over the finding's sorted
/// member names, the same identity [`archguard_engine::analysis::Baseline`] stores — is present in
/// the baseline are accepted and no longer fail the gate; every other
/// Error finding (declared layer violations, metric thresholds) fails
/// regardless. The full report is still printed either way; a missing
/// or unparsable baseline file is an IO error (exit 3).
pub(crate) fn check(
    cli: &Cli,
    baseline: Option<Result<archguard_engine::analysis::Baseline, String>>,
    report: &Report,
    comparison: Option<&archguard_engine::Comparison>,
    source_paths: &BTreeMap<String, String>,
) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json | Format::Sarif) {
        return usage(format!(
            "invalid value '{}' for '--format': check supports text, json and sarif",
            cli.format_name()
        ));
    }
    let baseline = match baseline {
        None => None,
        Some(loaded) => match loaded {
            Ok(baseline) => Some(baseline),
            Err(err) => return analysis_error(err),
        },
    };
    let fails = gate_fails(&report.findings, baseline.as_ref());
    let mut report = report.clone();
    report.summary.status = if fails {
        archguard_engine::core::Status::Fail
    } else {
        archguard_engine::core::Status::Pass
    };
    if !cli.quiet {
        match cli.format {
            Format::Json => {
                let json = to_json(&report, true);
                if let Some(comparison) = comparison {
                    // Preserve the canonical report's f32 number representation.
                    let mut value: Value = serde_json::from_str(&json).expect("valid report JSON");
                    value["comparison"] =
                        serde_json::to_value(comparison).expect("comparison serializes");
                    emit(&serde_json::to_string_pretty(&value).unwrap());
                } else {
                    emit(&json);
                }
            }
            Format::Sarif => emit(&to_sarif(&report, source_paths, baseline.as_ref())),
            _ => emit(&to_text(&report)),
        }
    }
    if fails { EXIT_FOUND } else { EXIT_OK }
}
