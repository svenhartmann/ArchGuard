//! What an empty Quality section shows: what is missing and the CLI
//! command that imports or creates it ([`Section::command`]), or, when
//! the section has records, that none matches the search and scope.

use super::inputs::Inputs;
use super::notes::Section;
use crate::tab_support::Texts;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Empty {
    pub title: String,
    pub reason: String,
    /// Empty when no command creates the data.
    pub command: String,
}

/// The empty state of `section`, whose search and scope list no record.
pub fn of(lang: &Texts, inputs: &Inputs, section: Section) -> Empty {
    if has_data(inputs, section) {
        return Empty {
            title: lang
                .t("No matching measurements. Missing data is not evidence of good quality.")
                .into(),
            ..Empty::default()
        };
    }
    let (title, reason) = missing(lang, section);
    Empty {
        title: title.into(),
        reason: reason.into(),
        command: section.command().unwrap_or_default().into(),
    }
}

/// Whether the section's source holds any record, before search and scope.
fn has_data(inputs: &Inputs, section: Section) -> bool {
    match section {
        Section::Heatmap | Section::Classes => !inputs.code.classes.is_empty(),
        Section::Methods => !inputs.code.methods.is_empty(),
        Section::Knowledge => inputs
            .knowledge
            .as_ref()
            .is_some_and(|k| !k.files.is_empty()),
        Section::Measurements => !inputs.measurements.groups.is_empty(),
        Section::Cohesion => !inputs.cohesion.packages.is_empty(),
        Section::History => inputs.history.latest.is_some() || !inputs.history.records.is_empty(),
        Section::Tests => !inputs.tests.tests.is_empty(),
        Section::Mutations => !inputs.mutations.files.is_empty(),
        Section::Duplicates => !inputs.duplicates.groups.is_empty(),
    }
}

/// What is missing, and how the command below gets it.
fn missing(lang: &Texts, section: Section) -> (&str, &str) {
    match section {
        Section::Heatmap | Section::Classes | Section::Methods => (
            lang.t("No compiler-backed class or method measurements."),
            lang.t("They come from the compiler-backed call analysis. List the toolchains it is missing:"),
        ),
        Section::Knowledge => (
            lang.t("No Git history in the configured window."),
            lang.t("Knowledge concentration reads Git within review.git_since. Check that Git is available:"),
        ),
        Section::Measurements => (
            lang.t("No build, runtime or repeated-test runs imported."),
            lang.t("Import revision-labelled observations as JSON:"),
        ),
        Section::Cohesion => (
            lang.t("No package with source files in this scope."),
            lang.t("Package cohesion is computed from the scanned sources; there is nothing to import."),
        ),
        Section::History => (
            lang.t("No quality snapshot recorded."),
            lang.t("Record the current per-package measurements; deltas need two snapshots:"),
        ),
        Section::Tests => (
            lang.t("No test results imported."),
            lang.t("Import a JUnit XML report and optionally an exact test-to-source mapping:"),
        ),
        Section::Mutations => (
            lang.t("No mutation report imported."),
            lang.t("Import a mutation-testing-elements JSON report; ArchGuard runs no tests:"),
        ),
        Section::Duplicates => (
            lang.t("No duplicate groups saved."),
            lang.t("Run the built-in token clone scan, or import CPD XML with --report:"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CLI's argument definitions, read as text: the desktop cannot
    /// link the CLI binary crate.
    fn args_rs() -> String {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../archguard-cli/src/args.rs");
        std::fs::read_to_string(path).expect("archguard-cli/src/args.rs")
    }

    /// The field names of subcommand `variant` (`TestEvidence`), or
    /// `None` when `args.rs` declares no such variant.
    fn fields(args: &str, variant: &str) -> Option<Vec<String>> {
        let mut lines = args.lines().map(str::trim);
        if lines.clone().any(|l| l == format!("{variant},")) {
            return Some(Vec::new());
        }
        lines.find(|l| *l == format!("{variant} {{"))?;
        let names = lines
            .take_while(|l| *l != "},")
            .filter_map(|l| l.split_once(':').map(|(name, _)| name.to_string()))
            .filter(|n| n.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .collect();
        Some(names)
    }

    /// `test-evidence` → `TestEvidence`.
    fn variant(subcommand: &str) -> String {
        subcommand
            .split('-')
            .map(|w| w[..1].to_uppercase() + &w[1..])
            .collect()
    }

    #[test]
    fn every_command_is_a_cli_subcommand_with_its_flags() {
        let args = args_rs();
        for section in Section::ALL {
            let Some(command) = section.command() else {
                assert_eq!(
                    section,
                    Section::Cohesion,
                    "only cohesion has nothing to import"
                );
                continue;
            };
            let words: Vec<&str> = command.split_whitespace().collect();
            assert_eq!(words[0], "archguard", "{command}");
            let known = fields(&args, &variant(words[1]))
                .unwrap_or_else(|| panic!("no subcommand {} in args.rs", words[1]));
            for flag in words.iter().filter_map(|w| w.strip_prefix("--")) {
                let field = flag.replace('-', "_");
                assert!(known.contains(&field), "{command}: no --{flag}");
            }
        }
    }

    #[test]
    fn the_check_reads_args_rs_as_intended() {
        let args = args_rs();
        assert_eq!(fields(&args, "Doctor"), Some(Vec::new()));
        let mutations = fields(&args, "Mutations").expect("Mutations");
        assert_eq!(mutations, ["path", "report", "source_root"]);
        assert_eq!(fields(&args, "NoSuchCommand"), None);
    }

    fn inputs() -> Inputs {
        let root = std::path::Path::new("/nonexistent-archguard-root");
        let graph = archguard_engine::core::GraphBuilder::new().build(root.display().to_string());
        let config = archguard_engine::core::Config::default();
        let sources = archguard_engine::core::ReviewSources::default();
        let calls = archguard_engine::analyze_calls(root, &config);
        Inputs::compute(root, &graph, &config, &sources, &calls)
    }

    /// Without data every section says what is missing in German, and
    /// all but cohesion name the command that creates it.
    #[test]
    fn a_section_without_data_names_what_is_missing_and_the_command() {
        let (en, de, inputs) = (Texts::default(), Texts::german(), inputs());
        for section in Section::ALL {
            let e = of(&de, &inputs, section);
            let (title, reason) = missing(&en, section);
            assert!(!title.is_empty() && !reason.is_empty());
            assert_ne!(e.title, title, "German title of {section:?}");
            assert_ne!(e.reason, reason, "German reason of {section:?}");
            assert_eq!(e.command, section.command().unwrap_or_default());
            assert_eq!(e.command.is_empty(), section == Section::Cohesion);
        }
    }
}
