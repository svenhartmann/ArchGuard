//! `archguard sbom`: third-party dependencies from the lockfiles, offline (ADR 0016).
use std::path::Path;

use archguard_engine::{Sbom, load_config, sbom, to_cyclonedx};

use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};

pub(crate) fn run(cli: &Cli, root: &Path) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json | Format::Cyclonedx) {
        return usage("sbom supports text, json and cyclonedx");
    }
    let loaded = match load_config(cli.config.as_deref(), root) {
        Ok(loaded) => loaded,
        Err(error) => return analysis_error(error),
    };
    let policy = &loaded.config.licenses;
    let sbom = sbom(root, policy);
    if !cli.quiet {
        match cli.format {
            Format::Cyclonedx => {
                let project = root
                    .canonicalize()
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| "project".into());
                emit(&serde_json::to_string_pretty(&to_cyclonedx(&sbom, &project)).unwrap());
            }
            Format::Json => emit(&serde_json::to_string_pretty(&sbom).unwrap()),
            _ => print_summary(&sbom, policy.is_active()),
        }
    }
    EXIT_OK
}

fn print_summary(sbom: &Sbom, policy_active: bool) {
    println!(
        "SBOM: {} third-party components from {} manifests (offline, from the files in the repository)",
        sbom.components.len(),
        sbom.manifests.len()
    );
    let join = |pairs: Vec<String>| pairs.join(" · ");
    println!(
        "{}",
        join(
            sbom.by_ecosystem
                .iter()
                .map(|(eco, n)| format!("{eco}: {n}"))
                .collect()
        )
    );
    let states = serde_json::to_value(&sbom.license_states).expect("states serialize");
    println!(
        "Licences: {}",
        join(
            states
                .as_object()
                .into_iter()
                .flatten()
                .map(|(state, n)| format!("{state}: {n}"))
                .collect()
        )
    );
    if policy_active {
        println!(
            "Licence policy violations: {} (gate: archguard check)",
            sbom.violations
        );
    } else {
        println!("Licence policy: not configured ([licenses] allow/deny/unknown)");
    }
    for manifest in &sbom.manifests {
        println!(
            "  {} ({}, {} components)",
            manifest.path, manifest.ecosystem, manifest.components
        );
    }
    for note in &sbom.notes {
        println!("Note: {note}");
    }
    println!("Unknown licences are not in the lockfiles; ArchGuard never fetches or guesses them.");
}
