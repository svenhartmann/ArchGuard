//! Explicit dependency-advisory scans; cached reads never invoke the scanner.
use std::path::Path;

use archguard_engine::{
    SecurityReport, SecurityScanOptions, load_config, read_security, scan_security,
};

use crate::args::{Cli, Format};
use crate::output::{EXIT_ERROR, EXIT_FOUND, EXIT_OK, analysis_error, emit, usage};

pub(crate) fn run(
    cli: &Cli,
    root: &Path,
    cached: bool,
    fail_on_found: bool,
    options: &SecurityScanOptions,
) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("security supports text and json");
    }
    let loaded = match load_config(cli.config.as_deref(), root) {
        Ok(loaded) => loaded,
        Err(error) => return analysis_error(error),
    };
    let report = if cached {
        read_security(root, &loaded.config)
    } else {
        let report = match scan_security(root, &loaded.config, options) {
            Ok(report) => report,
            Err(error) => return analysis_error(error),
        };
        if let Err(error) = crate::project_files::ensure_ignore_rules(root) {
            return analysis_error(error);
        }
        report
    };
    if !cli.quiet {
        if cli.format == Format::Json {
            emit(&serde_json::to_string_pretty(&report).expect("security report serializes"));
        } else {
            print_report(&report);
        }
    }
    if matches!(report.status.as_str(), "not_scanned" | "error")
        || (fail_on_found && report.status != "ready")
    {
        EXIT_ERROR
    } else if fail_on_found && !report.findings.is_empty() {
        EXIT_FOUND
    } else {
        EXIT_OK
    }
}

fn print_report(report: &SecurityReport) {
    outln!(
        "Dependency security: {} · {} packages · {} advisories",
        report.status,
        report.packages_scanned,
        report.findings.len()
    );
    outln!(
        "Scanner: {} {} · scanned: {} · database: {}",
        report.scanner,
        report.scanner_version.as_deref().unwrap_or("unknown"),
        report.scanned_at.as_deref().unwrap_or("not available"),
        report
            .database_updated_at
            .as_deref()
            .unwrap_or("not available")
    );
    outln!(
        "{}",
        ["critical", "high", "medium", "low", "unknown"]
            .map(|severity| format!(
                "{severity}: {}",
                report
                    .findings
                    .iter()
                    .filter(|f| f.severity == severity)
                    .count()
            ))
            .join(" · ")
    );
    for target in &report.targets {
        outln!(
            "  {} ({}, {} packages)",
            target.path, target.ecosystem, target.packages
        );
    }
    for finding in &report.findings {
        outln!(
            "{} {} · {} {} · fix: {}",
            finding.severity.to_uppercase(),
            finding.id,
            finding.package,
            finding.installed_version,
            finding.fixed_version.as_deref().unwrap_or("not reported")
        );
        outln!("  {}", finding.title);
        outln!("  {}", finding.targets.join(", "));
        if let Some(url) = &finding.url {
            outln!("  {url}");
        }
    }
    for diagnostic in &report.diagnostics {
        outln!("Note: {diagnostic}");
    }
    outln!(
        "Known dependency advisories at the recorded database date; runtime exploitability and application code are not assessed."
    );
}
