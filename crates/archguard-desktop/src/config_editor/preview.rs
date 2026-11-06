//! What the edit changes in `archguard check` before it is saved: the
//! engine's check (cycles, layer and dependency rules, thresholds) over
//! the project with the saved and with the edited configuration.
//!
//! Not included: `--baseline` acceptance and the `[review]` key-figure
//! gates, which the CLI adds on top of the report (`check_cmd.rs`).

use std::collections::BTreeSet;
use std::path::PathBuf;

use archguard_engine::core::{Finding, Severity};
use archguard_engine::{Level, LoadedConfig, analyze_run_quiet};

use super::document::ConfigEditor;
use super::error::FieldError;
use super::validate::validate;

/// Everything a worker thread needs; [`run`] does the work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewRequest {
    pub root: PathBuf,
    /// The file's text (empty when there is no file).
    pub saved: String,
    pub edited: String,
}

/// The check result for one configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckSummary {
    /// No error finding: `archguard check` exits 0.
    pub pass: bool,
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    /// The saved configuration's result; `Err` when it does not load or
    /// analyse (the edit may be the fix).
    pub before: Result<CheckSummary, String>,
    pub after: CheckSummary,
    /// Findings (by id) only with the edit, and only without it.
    pub added: Vec<Finding>,
    pub removed: Vec<Finding>,
}

impl Preview {
    /// Whether the edit changes pass/fail or any finding.
    pub fn changed(&self) -> bool {
        match &self.before {
            Ok(before) => {
                before.pass != self.after.pass || !self.added.is_empty() || !self.removed.is_empty()
            }
            Err(_) => true,
        }
    }
}

impl ConfigEditor {
    pub fn preview_request(&self) -> PreviewRequest {
        PreviewRequest {
            root: self.root().to_path_buf(),
            saved: self.disk.clone().unwrap_or_default(),
            edited: self.text(),
        }
    }
}

/// Analyse the project twice. Blocking; run it on a worker thread. An
/// invalid edit is an error naming the field.
pub fn run(request: &PreviewRequest) -> Result<Preview, FieldError> {
    let edited = validate(&request.root, &request.edited)?;
    let after = check(request, &edited).map_err(|e| FieldError::new("", e))?;
    let before = validate(&request.root, &request.saved)
        .map_err(|e| e.to_string())
        .and_then(|saved| check(request, &saved));
    let (added, removed) = match &before {
        Ok(before) => (only_in(&after, before), only_in(before, &after)),
        Err(_) => (after.findings.clone(), Vec::new()),
    };
    Ok(Preview {
        before,
        after,
        added,
        removed,
    })
}

fn check(request: &PreviewRequest, loaded: &LoadedConfig) -> Result<CheckSummary, String> {
    let run = analyze_run_quiet(&request.root, loaded, Level::Module).map_err(|e| e.to_string())?;
    let findings = run.report.findings;
    let count = |s: Severity| findings.iter().filter(|f| f.severity == s).count();
    let errors = count(Severity::Error);
    Ok(CheckSummary {
        pass: errors == 0,
        errors,
        warnings: count(Severity::Warning),
        infos: count(Severity::Info),
        findings,
    })
}

fn only_in(a: &CheckSummary, b: &CheckSummary) -> Vec<Finding> {
    let ids: BTreeSet<&str> = b.findings.iter().map(|f| f.id.as_str()).collect();
    a.findings
        .iter()
        .filter(|f| !ids.contains(f.id.as_str()))
        .cloned()
        .collect()
}
