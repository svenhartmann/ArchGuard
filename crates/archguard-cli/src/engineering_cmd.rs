//! Explicit evidence imports and read-only structural/quality exploration.
use crate::args::{Cli, Command, Format};
use crate::output::{EXIT_ERROR, EXIT_OK, analysis_error, emit, usage};
use archguard_engine::{LoadedConfig, core::Graph};
use serde_json::{Value, json};
use std::path::Path;

fn format_allowed(cli: &Cli) -> bool {
    matches!(cli.format, Format::Text | Format::Json)
}
fn output(cli: &Cli, label: &str, value: Value) -> i32 {
    if !cli.quiet {
        if cli.format == Format::Text {
            println!("{label} · measured evidence and limitations");
        }
        emit(&serde_json::to_string_pretty(&value).expect("engineering evidence serializes"));
    }
    if value["status"] == "error" || value["report"]["status"] == "error" {
        EXIT_ERROR
    } else {
        EXIT_OK
    }
}
fn ignore(root: &Path) -> Result<(), std::io::Error> {
    crate::project_files::ensure_ignore_rules(root)
}

pub(crate) fn history(cli: &Cli, root: &Path) -> i32 {
    if !format_allowed(cli) {
        return usage("quality-history supports text and json");
    }
    output(
        cli,
        "Quality history",
        json!(archguard_engine::read_quality_history(root)),
    )
}

pub(crate) fn contracts(
    cli: &Cli,
    root: &Path,
    report: Option<&Path>,
    service: Option<&str>,
    usage_file: Option<&Path>,
) -> i32 {
    if !format_allowed(cli) {
        return usage("contracts supports text and json");
    }
    if report.is_some() != service.is_some() || (usage_file.is_some() && report.is_none()) {
        return usage("contracts requires --report with --service; --usage requires an import");
    }
    let topology = match archguard_engine::analyze_services(root) {
        Ok(v) => v,
        Err(e) => return analysis_error(e),
    };
    let report = if let Some(path) = report {
        match archguard_engine::import_api_contract(
            root,
            &topology,
            path,
            service.expect("validated service"),
            usage_file,
        ) {
            Ok(v) => {
                if let Err(e) = ignore(root) {
                    return analysis_error(e);
                }
                v
            }
            Err(e) => return analysis_error(e),
        }
    } else {
        archguard_engine::read_api_contracts(root, &topology)
    };
    output(cli, "API contracts", json!(report))
}

pub(crate) fn run(cli: &Cli, root: &Path, loaded: &LoadedConfig, graph: &Graph) -> i32 {
    if !format_allowed(cli) {
        return usage("quality exploration supports text and json");
    }
    let config = &loaded.config;
    match &cli.command {
        Command::Metrics { report, .. } => {
            let measurements = if let Some(path) = report {
                match archguard_engine::import_measurement_runs(root, path) {
                    Ok(value) => {
                        if let Err(e) = ignore(root) {
                            return analysis_error(e);
                        }
                        value
                    }
                    Err(e) => return analysis_error(e),
                }
            } else {
                archguard_engine::read_measurement_runs(root)
            };
            let knowledge = match archguard_engine::knowledge_metrics(root, config, graph) {
                Ok(value) => value,
                Err(e) => return analysis_error(e),
            };
            let calls = archguard_engine::analyze_calls(root, config);
            output(
                cli,
                "Extended quality metrics",
                json!({"status":if measurements.status=="error"{"error"}else{calls.status.as_str()},"code":archguard_engine::code_metrics(&calls),"knowledge":knowledge,"measurements":measurements}),
            )
        }
        Command::Cohesion { classes: true, .. } => {
            let calls = archguard_engine::analyze_calls(root, config);
            output(
                cli,
                "Class cohesion",
                json!({"definition":"lcom4-source/v1","status":calls.status,"classes":archguard_engine::class_cohesion(&calls),"analyzers":calls.analyzers,"diagnostics":calls.diagnostics}),
            )
        }
        Command::Cohesion { .. } => {
            let options = archguard_engine::CoChangeOptions {
                since: config.review.git_since.clone(),
                ..Default::default()
            };
            let changes = match archguard_engine::co_change(root, config, graph, &options) {
                Ok(v) => v,
                Err(e) => return analysis_error(e),
            };
            output(
                cli,
                "Package cohesion",
                json!(archguard_engine::analyze_cohesion(graph, Some(&changes))),
            )
        }
        Command::QualityHistory { record, tag, .. } => {
            if *record {
                let calls = archguard_engine::analyze_calls(root, config);
                let coverage = archguard_engine::read_test_coverage(root, config, graph);
                let findings = archguard_engine::read_quality_findings(root, config, graph);
                let snapshot = match archguard_engine::build_quality_snapshot(
                    root, config, graph, &calls, &coverage, &findings,
                ) {
                    Ok(v) => v,
                    Err(e) => return analysis_error(e),
                };
                if let Err(e) =
                    archguard_engine::record_quality_snapshot(root, &snapshot, tag.as_deref())
                {
                    return analysis_error(e);
                }
                if let Err(e) = ignore(root) {
                    return analysis_error(e);
                }
            }
            history(cli, root)
        }
        Command::TestEvidence {
            report,
            mapping,
            source_root,
            target,
            ..
        } => {
            if target.len() > 256
                || target
                    .iter()
                    .any(|t| t.trim().is_empty() || t.contains('\0'))
            {
                return usage("test-evidence accepts at most 256 nonempty --target values");
            }
            let report = if let Some(path) = report {
                match archguard_engine::import_test_evidence(
                    root,
                    config,
                    graph,
                    path,
                    mapping.as_deref(),
                    source_root.as_deref(),
                ) {
                    Ok(v) => {
                        if let Err(e) = ignore(root) {
                            return analysis_error(e);
                        }
                        v
                    }
                    Err(e) => return analysis_error(e),
                }
            } else {
                archguard_engine::read_test_evidence(root, config, graph)
            };
            let impact = archguard_engine::test_impact(graph, &report, target);
            output(
                cli,
                "Test evidence",
                json!({"report":report,"impact":impact}),
            )
        }
        Command::Mutations {
            report,
            source_root,
            ..
        } => {
            let report = if let Some(path) = report {
                match archguard_engine::import_mutations(
                    root,
                    config,
                    graph,
                    path,
                    source_root.as_deref(),
                ) {
                    Ok(v) => {
                        if let Err(e) = ignore(root) {
                            return analysis_error(e);
                        }
                        v
                    }
                    Err(e) => return analysis_error(e),
                }
            } else {
                archguard_engine::read_mutations(root, config, graph)
            };
            output(cli, "Mutation evidence", json!(report))
        }
        Command::Duplicates { .. } => duplicates(cli, root, config, graph),
        _ => unreachable!("engineering handler requires an engineering command"),
    }
}

fn duplicates(
    cli: &Cli,
    root: &Path,
    config: &archguard_engine::core::Config,
    graph: &Graph,
) -> i32 {
    let Command::Duplicates {
        report,
        source_root,
        scan,
        min_tokens,
        normalize,
        replace,
        ..
    } = &cli.command
    else {
        unreachable!("duplicates handler requires the duplicates command")
    };
    let saved = if *scan {
        let options = archguard_engine::DuplicateScanOptions {
            min_tokens: *min_tokens as usize,
            normalize: *normalize,
            replace: *replace,
            ..Default::default()
        };
        archguard_engine::scan_duplicates(root, config, graph, &options)
    } else if let Some(path) = report {
        archguard_engine::import_duplicates(root, config, graph, path, source_root.as_deref())
    } else {
        return output(
            cli,
            "Duplication evidence",
            json!(archguard_engine::read_duplicates(root, config, graph)),
        );
    };
    match saved {
        Ok(v) => match ignore(root) {
            Ok(()) => output(cli, "Duplication evidence", json!(v)),
            Err(e) => analysis_error(e),
        },
        Err(e) => analysis_error(e),
    }
}
