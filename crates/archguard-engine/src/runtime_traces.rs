//! Runtime profile from local OTLP JSON Lines (ADR 0004): requests per endpoint,
//! database calls per request and per statement, and the code that issued them.
//! Reads files only; no collector, application or network is started.
mod aggregate;
mod findings;
mod index;
mod input;
mod requests;
mod span;

use archguard_core::{RuntimeConfig, RuntimeDiagnostic, RuntimeProfile};
use std::{collections::HashSet, path::Path};

const NOTE: &str = "Runtime numbers come from imported OTLP traces: a sample of the recorded runs, not complete traffic. Durations are client-side span durations (execution, network and pool wait). A pattern missing from the sample is not proven absent. Query texts keep placeholders; literal values are replaced by ?.";

/// The runtime profile of the project at `root`, or `None` when it has no
/// runtime input (`.archguard/runtime/**/*.jsonl`, `.archguard/otel-traces.json`).
/// Finding thresholds come from `[runtime]` of `config` (else the project's
/// archguard.toml); an unusable configuration falls back to the defaults.
pub fn runtime_profile(root: &Path, config: Option<&Path>) -> Option<RuntimeProfile> {
    let inputs = input::discover(root)?;
    let mut reader = input::Reader::new();
    for (i, file) in inputs.iter().enumerate() {
        if reader.is_full() {
            break;
        }
        reader.read(i, file);
    }
    let mut profile = RuntimeProfile {
        schema_version: 1,
        inputs: inputs.iter().map(|i| i.rel.clone()).collect(),
        sampling: reader.sampling,
        diagnostics: reader.diagnostics,
        complete: reader.complete,
        note: NOTE.into(),
        ..RuntimeProfile::default()
    };
    let spans = dedupe(reader.spans, &mut profile);
    aggregate::fill(&spans, &mut profile);
    let thresholds = thresholds(root, config, &mut profile);
    profile.findings = findings::findings(&profile, &thresholds);
    Some(profile)
}

/// Keeps the first copy of each (trace, span) pair, as repeated exports duplicate spans.
fn dedupe(spans: Vec<span::Span>, profile: &mut RuntimeProfile) -> Vec<span::Span> {
    let before = spans.len();
    let mut seen = HashSet::new();
    let unique: Vec<_> = spans
        .into_iter()
        .filter(|s| seen.insert((s.trace.clone(), s.id.clone())))
        .collect();
    if unique.len() < before {
        profile.diagnostics.push(RuntimeDiagnostic {
            file: String::new(),
            line: None,
            message: format!(
                "{} duplicate spans (same trace and span id) ignored.",
                before - unique.len()
            ),
        });
    }
    unique
}

fn thresholds(root: &Path, config: Option<&Path>, profile: &mut RuntimeProfile) -> RuntimeConfig {
    let loaded = crate::config::load_config(config, root).and_then(|loaded| {
        crate::config::validate_runtime(&loaded.config.runtime)?;
        Ok(loaded.config.runtime)
    });
    loaded.unwrap_or_else(|error| {
        profile.diagnostics.push(RuntimeDiagnostic {
            file: String::new(),
            line: None,
            message: format!(
                "Configuration not usable ({error}); default runtime thresholds used."
            ),
        });
        RuntimeConfig::default()
    })
}
