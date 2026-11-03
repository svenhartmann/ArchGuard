//! Explicit immutable run observations. Never executes project or benchmark commands.
use crate::evidence_import as evidence;
use archguard_core::ArchguardError;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
const FILE: &str = ".archguard/measurement-runs.json";
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementRun {
    pub run_id: String,
    pub kind: String,
    pub name: String,
    pub revision: String,
    pub environment: String,
    pub producer: String,
    pub timestamp: u64,
    pub outcome: String,
    pub duration_seconds: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema_version: u32,
    runs: Vec<MeasurementRun>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasurementGroup {
    pub kind: String,
    pub name: String,
    pub revision: String,
    pub environment: String,
    pub producer: String,
    pub samples: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub duration_samples: usize,
    pub median_seconds: Option<f64>,
    pub p95_seconds: Option<f64>,
    pub inconsistent: Option<bool>,
    pub failure_percent: Option<f64>,
    pub run_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasurementReport {
    pub definition: String,
    pub status: String,
    pub runs: usize,
    pub groups: Vec<MeasurementGroup>,
    pub eligible_test_groups: usize,
    pub inconsistent_test_groups: usize,
    pub flaky_test_percent: Option<f64>,
    pub diagnostics: Vec<String>,
}
fn error(v: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("measurement runs: {v}"))
}
fn identity(r: &MeasurementRun) -> (&str, &str, &str) {
    (&r.run_id, &r.kind, &r.name)
}
fn validate(input: &Input) -> Result<(), ArchguardError> {
    if input.schema_version != 1 || input.runs.len() > 100_000 {
        return Err(error(
            "expected schema_version 1 and at most 100000 observations",
        ));
    }
    let mut seen = BTreeSet::new();
    for r in &input.runs {
        if !matches!(r.kind.as_str(), "build" | "runtime" | "test")
            || !matches!(
                r.outcome.as_str(),
                "passed" | "failed" | "error" | "skipped"
            )
            || !matches!(r.revision.len(), 40 | 64)
            || !r.revision.bytes().all(|b| b.is_ascii_hexdigit())
            || [&r.run_id, &r.name, &r.environment, &r.producer]
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 1024 || s.chars().any(char::is_control))
            || r.duration_seconds
                .is_some_and(|n| !n.is_finite() || n < 0.0)
            || !seen.insert(identity(r))
        {
            return Err(error(
                "invalid or duplicate run observation; identifiers, revision, context, outcome and nonnegative finite durations are required",
            ));
        }
    }
    Ok(())
}
fn load(root: &Path) -> Result<Input, ArchguardError> {
    match crate::security::read_repo_file(root, Path::new(FILE), evidence::MAX_BYTES) {
        Ok(bytes) => {
            let input = serde_json::from_slice(&bytes).map_err(error)?;
            validate(&input)?;
            Ok(input)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Input {
            schema_version: 1,
            runs: vec![],
        }),
        Err(e) => Err(error(e)),
    }
}
pub fn measurement_runs_fingerprint(root: &Path) -> String {
    evidence::fingerprint(root, FILE)
}
pub fn import_measurement_runs(
    root: &Path,
    path: &Path,
) -> Result<MeasurementReport, ArchguardError> {
    let input: Input = serde_json::from_slice(&evidence::read_input(root, path)?).map_err(error)?;
    validate(&input)?;
    let mut saved = load(root)?;
    let existing: BTreeMap<_, _> = saved
        .runs
        .iter()
        .map(|r| {
            (
                (r.run_id.clone(), r.kind.clone(), r.name.clone()),
                r.clone(),
            )
        })
        .collect();
    for r in input.runs {
        if let Some(old) = existing.get(&(r.run_id.clone(), r.kind.clone(), r.name.clone())) {
            if old != &r {
                return Err(error(
                    "run identity already exists with different evidence; history was not changed",
                ));
            }
        } else {
            saved.runs.push(r);
        }
    }
    validate(&saved)?;
    saved.runs.sort_by(|a, b| identity(a).cmp(&identity(b)));
    let bytes = serde_json::to_vec_pretty(&saved).map_err(error)?;
    if bytes.len() as u64 > evidence::MAX_BYTES {
        return Err(error("stored run history exceeds 32 MiB"));
    }
    crate::project_files::write_project_file(
        root,
        Path::new("measurement-runs.json"),
        &bytes,
        true,
    )?;
    Ok(summarize(&saved))
}
pub fn read_measurement_runs(root: &Path) -> MeasurementReport {
    match load(root) {
        Ok(input) => summarize(&input),
        Err(e) => {
            let mut r = summarize(&Input {
                schema_version: 1,
                runs: vec![],
            });
            r.status = "error".into();
            r.diagnostics.push(e.to_string());
            r
        }
    }
}
fn summarize(input: &Input) -> MeasurementReport {
    let mut grouped = BTreeMap::<_, Vec<_>>::new();
    for r in &input.runs {
        grouped
            .entry((&r.kind, &r.name, &r.revision, &r.environment, &r.producer))
            .or_default()
            .push(r);
    }
    let mut groups = vec![];
    for ((kind, name, revision, environment, producer), rows) in grouped {
        let passed = rows.iter().filter(|r| r.outcome == "passed").count();
        let failed = rows
            .iter()
            .filter(|r| matches!(r.outcome.as_str(), "failed" | "error"))
            .count();
        let evaluated = passed + failed;
        let mut durations: Vec<_> = rows
            .iter()
            .filter(|r| r.outcome == "passed")
            .filter_map(|r| r.duration_seconds)
            .collect();
        durations.sort_by(f64::total_cmp);
        let n = durations.len();
        groups.push(MeasurementGroup {
            kind: kind.clone(),
            name: name.clone(),
            revision: revision.clone(),
            environment: environment.clone(),
            producer: producer.clone(),
            samples: rows.len(),
            passed,
            failed,
            skipped: rows.len() - evaluated,
            duration_samples: n,
            median_seconds: (n > 0).then(|| {
                if n % 2 == 1 {
                    durations[n / 2]
                } else {
                    durations[n / 2 - 1] / 2.0 + durations[n / 2] / 2.0
                }
            }),
            p95_seconds: (n > 0).then(|| durations[(n * 95).div_ceil(100) - 1]),
            inconsistent: (kind == "test" && evaluated >= 2).then_some(passed > 0 && failed > 0),
            failure_percent: (evaluated > 0).then(|| 100.0 * failed as f64 / evaluated as f64),
            run_ids: rows.iter().map(|r| r.run_id.clone()).collect(),
        });
    }
    let eligible = groups.iter().filter(|g| g.inconsistent.is_some()).count();
    let inconsistent = groups
        .iter()
        .filter(|g| g.inconsistent == Some(true))
        .count();
    MeasurementReport{definition:"measurement-runs/v1".into(),status:if input.runs.is_empty(){"not_imported"}else{"ready"}.into(),runs:input.runs.len(),groups,eligible_test_groups:eligible,inconsistent_test_groups:inconsistent,flaky_test_percent:(eligible>0).then(||100.0*inconsistent as f64/eligible as f64),diagnostics:vec!["Producer-declared revisions and environments are unverified. Compare only matching kind, name, revision, environment and producer. Durations describe successful runs; p95 uses nearest rank and small samples have limited meaning. Test instability is mixed passed/failed/error results in at least two runs of the same declared context, not proof of a flaky cause. Skipped tests do not enter its denominator. Different revisions are never blended. Duplicate run identities are idempotent; conflicting observations are rejected.".into()]}
}
