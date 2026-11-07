//! `.archguard/archguard.toml` loading and provenance (Kap. 11).
//!
//! The file is the correcting delta: it may be absent entirely, and every
//! section not written stays at its documented default. `load_config`
//! therefore parses once into a raw [`toml::Table`] to record which
//! sections the *user* actually wrote (`sources`), then into [`Config`]
//! (which fills `Default` values for every missing field).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use archguard_core::{ArchguardError, Config, ConfigSource};

/// Canonical configuration location relative to the analyzed project root.
pub const DEFAULT_CONFIG_PATH: &str = ".archguard/archguard.toml";

/// Legacy configuration location, consulted when the canonical file is absent.
const CONFIG_FILE_NAME: &str = "archguard.toml";

/// The configuration keys whose provenance [`load_config`] records: the
/// five sections of Kap. 11 plus the top-level `derive_layers` switch.
const SECTION_KEYS: [&str; 13] = [
    "project",
    "modules",
    "layers",
    "dependency_rules",
    "ownership_rules",
    "rules",
    "review",
    "performance",
    "architecture",
    "semantic",
    "store",
    "arc42",
    "derive_layers",
];

/// The commented stock-default `archguard.toml` of Kap. 11, written by
/// `archguard init --defaults`.
///
/// Every active value equals the corresponding [`Config`] default; the
/// commented-out gate suggestions (`max_acd`, `max_instability_core`) are
/// the documented placeholders `archguard init` would fill with measured
/// values.
const DEFAULT_CONFIG: &str = r#"# archguard.toml — Stock defaults (ARCHITECTURE.md, Kap. 11).
# ArchGuard runs fine without this file: everything not written here stays
# at its default or is derived from the project. The file records only
# the intent delta.

derive_layers = true        # layer derivation on/off

[project]                   # derived (manifest sniffing + extension statistics)
source_roots = ["src"]
exclude = ["**/tests/**", "**/*.spec.*", "**/generated/**"]   # Default
languages = ["auto"]        # "auto" or explicit language ids, e.g. ["ts", "py"]

[modules]
depth = 2                   # zoom: app/billing/x.ts is seen as app/billing

# [[layers]]                # derived: dependency ranking + name affinity
# name = "domain"           # declared entries override derived ones per name
# contains = ["domain/**"]
# may_depend_on = []        # bottom layer (no outgoing edges)

[rules]
deny_cycles = "module"      # intent (Default): "module" | "file" | "off"
max_scc_size = 1            # intent (Default)
baseline = ".archguard/baseline.json"
# max_acd = 3.42                  # measured: 3.42 — gate suggestion from init (0 = off)
# max_instability_core = 0.05     # measured core instability — gate suggestion (0 = off)

[store]                     # Default
db = ".archguard/graph.db"
sync_on_check = false

[review]
git_since = "180.days"       # commit touches, not lines changed
orphans = "off"             # opt-in candidates: off | info | warning | error
# entry_points = ["**/main.*", "**/index.*", "**/cli/**"]
# health_weights = { tangledness = 2.0, hubs = 1.0, sdp = 1.0, coverage = 1.0 }
# max_sdp_violations = 0          # gate: SDP violations (absent = off)
# max_tangledness = 0.5           # gate: tangledness quote 0..1 (absent = off)
# max_propagation_cost = 0.5      # gate: propagation cost 0..1 (absent = off)
# adr_required_on_accept_risk = false

# [[dependency_rules]]
# name = "ui-no-persistence"
# from = ["ui/**"]
# to = ["persistence/**"]
# severity = "error"
# message = "UI must call the application API."

# [[ownership_rules]]
# from = "@example/frontend"
# to = "@example/storage"
# severity = "warning"
# message = "Review the team boundary."

# [performance]
# heavy_imports = ["pandas", "numpy", "tensorflow", "torch", "scipy", "@tensorflow/tfjs"]
# min_barrel_exports = 10
# min_barrel_ratio = 0.8

# [semantic.java]
# release = 21              # otherwise: supported build metadata, then running JDK
# classpath = ["lib/contracts.jar"] # existing jars or class directories
# [semantic.csharp]
# project = "src/App/App.csproj" # optional project and its references
# configuration = "Debug"
# target_framework = "net8.0" # optional target for multi-target projects

# [[architecture.goals]]
# name = "Availability"
# priority = "above infrastructure cost"
# rationale = "State the organization's reason."
# # Optional (docs/adr/0003-quality-goal-assessment.md): ISO/IEC 25010
# # characteristic functional_suitability, performance_efficiency,
# # compatibility, usability, reliability, security, maintainability or
# # portability; scope = node-name globs (empty = whole system); scenario =
# # stimulus, response and measure. Without attribute the goal stays unclassified.
# attribute = "reliability"
# scope = ["checkout/**"]
# scenario = "A failed payment provider call is retried; the order completes within 5 s."
# [[architecture.constraints]]
# name = "Compatibility"
# rationale = "State the contract or other evidence."
# [[architecture.decisions]]
# id = "ADR-1"
# title = "Record an actual decision"
# status = "accepted"
# chosen_option = "Chosen alternative"
# alternatives = ["Another considered option"]
# trade_offs = ["An explicitly accepted disadvantage"]
# links = ["docs/adr-1.md"]

[arc42]                     # diagrams of arc42.md (ADR 0006)
diagram_format = "mermaid"  # mermaid | plantuml
hidden_diagrams = []        # catalogue ids left out, e.g. "runtime-calls"
"#;

/// A parsed `archguard.toml` plus the provenance of its sections.
///
/// `config` carries `Default` values for everything the user did not
/// write; `sources` records what the user *actually* wrote, so a section
/// can still be reported as `Default` in the [`archguard_core::Report`] even
/// though `config` holds a (defaulted) value for it.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedConfig {
    pub config: Config,
    /// Section key → provenance; always contains every [`SECTION_KEYS`]
    /// entry (`BTreeMap` keeps the order deterministic).
    pub sources: BTreeMap<String, ConfigSource>,
}

/// Load the configuration for `project_root`.
///
/// Uses `explicit` if given (an error if that path is missing), otherwise
/// `<project_root>/.archguard/archguard.toml`, then the legacy
/// `<project_root>/archguard.toml`, then defaults. Only a missing file permits
/// fallback: unreadable or invalid files are configuration errors. A discovered
/// path is repository content and must not be a symlink, because a parse error
/// quotes the file; `explicit` is the operator's choice and may be one. Relative
/// paths inside the configuration remain relative to `project_root`.
/// The raw table decides provenance: a section key present in the file is
/// `Declared`, an absent one is `Default` — `Config`'s own `Default`
/// filling is irrelevant for that. Parse/type errors become
/// [`ArchguardError::Config`] naming the file.
pub fn load_config(
    explicit: Option<&Path>,
    project_root: &Path,
) -> Result<LoadedConfig, ArchguardError> {
    let candidates = match explicit {
        Some(path) => vec![path.to_path_buf()],
        None => vec![
            project_root.join(DEFAULT_CONFIG_PATH),
            project_root.join(CONFIG_FILE_NAME),
        ],
    };
    let mut selected = None;
    for path in candidates {
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                let linked_directory = path == project_root.join(DEFAULT_CONFIG_PATH)
                    && path.parent().is_some_and(|parent| {
                        fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink())
                    });
                if explicit.is_none() && (metadata.file_type().is_symlink() || linked_directory) {
                    return Err(config_error(
                        &path,
                        &"a discovered configuration must not be a symlink; pass it with --config",
                    ));
                }
                selected = Some(path);
                break;
            }
            Err(err) if explicit.is_none() && err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(config_error(&path, &err)),
        }
    }

    let Some(path) = selected else {
        return Ok(LoadedConfig {
            config: Config::default(),
            sources: all_default_sources(),
        });
    };
    if !fs::metadata(&path)
        .map_err(|err| config_error(&path, &err))?
        .is_file()
    {
        return Err(config_error(&path, &"not a regular configuration file"));
    }

    let content = fs::read_to_string(&path).map_err(|err| config_error(&path, &err))?;
    let table = content
        .parse::<toml::Table>()
        .map_err(|err| config_error(&path, &err))?;
    let sources = SECTION_KEYS
        .iter()
        .map(|key| {
            let source = if table.get(*key).is_some_and(declares_value) {
                ConfigSource::Declared
            } else {
                ConfigSource::Default
            };
            ((*key).to_string(), source)
        })
        .collect();
    let config = toml::Value::Table(table)
        .try_into::<Config>()
        .map_err(|err| config_error(&path, &err))?;
    validate_config(&config).map_err(|err| config_error(&path, &err))?;
    Ok(LoadedConfig { config, sources })
}

/// Validate syntax and scalar values before any extraction, including
/// programmatic inputs. Derived layer targets are resolved after merging.
pub fn validate_config(config: &Config) -> Result<(), ArchguardError> {
    let invalid =
        |field: &str, message: String| ArchguardError::Config(format!("{field}: {message}"));
    if config
        .semantic
        .java
        .release
        .is_some_and(|release| release < 6)
    {
        return Err(invalid(
            "semantic.java.release",
            "must be a Java release number of at least 6".into(),
        ));
    }
    for (index, path) in config.semantic.java.classpath.iter().enumerate() {
        if path.trim().is_empty() || path.contains('\0') {
            return Err(invalid(
                &format!("semantic.java.classpath[{index}]"),
                "must be a nonempty path without NUL".into(),
            ));
        }
    }
    for (name, value) in [
        (
            "configuration",
            Some(config.semantic.csharp.configuration.as_str()),
        ),
        ("project", config.semantic.csharp.project.as_deref()),
        (
            "target_framework",
            config.semantic.csharp.target_framework.as_deref(),
        ),
    ] {
        if value.is_some_and(|value| value.trim().is_empty() || value.contains('\0')) {
            return Err(invalid(
                &format!("semantic.csharp.{name}"),
                "must not be empty or contain NUL".into(),
            ));
        }
    }
    for (name, value) in [
        ("project", config.semantic.typescript.project.as_deref()),
        ("compiler", config.semantic.typescript.compiler.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty() || value.contains('\0')) {
            return Err(invalid(
                &format!("semantic.typescript.{name}"),
                "must not be empty or contain NUL".into(),
            ));
        }
    }
    for (index, pattern) in config.project.exclude.iter().enumerate() {
        globset::Glob::new(pattern)
            .map_err(|err| invalid(&format!("project.exclude[{index}]"), err.to_string()))?;
    }
    for (index, language) in config.project.languages.iter().enumerate() {
        if !matches!(
            language.as_str(),
            "auto" | "rs" | "ts" | "js" | "py" | "java" | "cs" | "php" | "go" | "kt"
        ) {
            return Err(invalid(
                &format!("project.languages[{index}]"),
                format!("unsupported language {language:?}"),
            ));
        }
    }
    for (field, value, upper) in [
        ("rules.max_acd", config.rules.max_acd, f32::MAX),
        (
            "rules.max_instability_core",
            config.rules.max_instability_core,
            1.0,
        ),
    ] {
        if !value.is_finite() || value < 0.0 || value > upper {
            return Err(invalid(
                field,
                format!("must be finite and between 0 and {upper}"),
            ));
        }
    }
    let mut names = BTreeSet::new();
    for (index, pattern) in config.review.entry_points.iter().enumerate() {
        globset::Glob::new(pattern)
            .map_err(|err| invalid(&format!("review.entry_points[{index}]"), err.to_string()))?;
    }
    if let Some(weights) = &config.review.health_weights {
        for (part, weight) in weights {
            if !weight.is_finite() || *weight < 0.0 {
                return Err(invalid(
                    &format!("review.health_weights[{part}]"),
                    "must be finite and not negative".into(),
                ));
            }
        }
    }
    // The two normalized KPI gates share the 0..=1 bound; the count gate
    // (max_sdp_violations) is a plain u32 and needs no range check.
    for (field, bound) in [
        ("review.max_tangledness", config.review.max_tangledness),
        (
            "review.max_propagation_cost",
            config.review.max_propagation_cost,
        ),
    ] {
        if bound.is_some_and(|bound| !bound.is_finite() || !(0.0..=1.0).contains(&bound)) {
            return Err(invalid(field, "must be finite and between 0 and 1".into()));
        }
    }
    archguard_analysis::validate_dependency_rules(&config.dependency_rules)?;
    crate::target_architecture::validate(config)?;
    validate_review_since(&config.review.git_since)?;
    if config.performance.min_barrel_exports == 0 {
        return Err(invalid(
            "performance.min_barrel_exports",
            "must be greater than zero".into(),
        ));
    }
    if !config.performance.min_barrel_ratio.is_finite()
        || !(0.0..=1.0).contains(&config.performance.min_barrel_ratio)
    {
        return Err(invalid(
            "performance.min_barrel_ratio",
            "must be finite and between 0 and 1".into(),
        ));
    }
    for (index, prefix) in config.performance.heavy_imports.iter().enumerate() {
        if prefix.trim().is_empty() {
            return Err(invalid(
                &format!("performance.heavy_imports[{index}]"),
                "module prefix must not be empty".into(),
            ));
        }
    }
    let mut owner_directions = BTreeSet::new();
    for (index, rule) in config.ownership_rules.iter().enumerate() {
        if rule.from.trim().is_empty() || rule.to.trim().is_empty() {
            return Err(invalid(
                &format!("ownership_rules[{index}]"),
                "from and to must be nonempty owner tokens".into(),
            ));
        }
        if !owner_directions.insert((&rule.from, &rule.to)) {
            return Err(invalid(
                &format!("ownership_rules[{index}]"),
                "duplicate forbidden owner direction".into(),
            ));
        }
    }
    validate_goals(config)?;
    let mut constraints = BTreeSet::new();
    for (index, constraint) in config.architecture.constraints.iter().enumerate() {
        if constraint.name.trim().is_empty() || !constraints.insert(&constraint.name) {
            return Err(invalid(
                &format!("architecture.constraints[{index}].name"),
                "must be nonempty and unique".into(),
            ));
        }
    }
    let mut decisions = BTreeSet::new();
    for (index, decision) in config.architecture.decisions.iter().enumerate() {
        if decision.id.trim().is_empty()
            || !decisions.insert(&decision.id)
            || decision.title.trim().is_empty()
        {
            return Err(invalid(
                &format!("architecture.decisions[{index}]"),
                "id must be nonempty and unique; title must be nonempty".into(),
            ));
        }
    }
    for (index, layer) in config.layers.iter().enumerate() {
        if layer.name.trim().is_empty() || !names.insert(&layer.name) {
            return Err(invalid(
                &format!("layers[{index}].name"),
                "must be nonempty and unique".into(),
            ));
        }
        for (glob_index, pattern) in layer.contains.iter().enumerate() {
            globset::Glob::new(pattern).map_err(|err| {
                invalid(
                    &format!("layers[{index}].contains[{glob_index}]"),
                    err.to_string(),
                )
            })?;
        }
    }
    if !config.derive_layers {
        validate_layer_targets(config)?;
    }
    Ok(())
}

/// Goal names are unique; scope globs use the matcher of `layers[].contains`.
/// `attribute` values are checked by serde against the ISO 25010 enum.
fn validate_goals(config: &Config) -> Result<(), ArchguardError> {
    let mut goals = BTreeSet::new();
    for (index, goal) in config.architecture.goals.iter().enumerate() {
        if goal.name.trim().is_empty() || !goals.insert(&goal.name) {
            return Err(ArchguardError::Config(format!(
                "architecture.goals[{index}].name: must be nonempty and unique"
            )));
        }
        for (glob_index, pattern) in goal.scope.iter().enumerate() {
            globset::Glob::new(pattern).map_err(|err| {
                ArchguardError::Config(format!(
                    "architecture.goals[{index}].scope[{glob_index}]: {err}"
                ))
            })?;
        }
    }
    Ok(())
}

/// Target names may refer to a derived layer, so check the final namespace.
pub(crate) fn validate_layer_targets(config: &Config) -> Result<(), ArchguardError> {
    let names: BTreeSet<_> = config.layers.iter().map(|layer| &layer.name).collect();
    for (index, layer) in config.layers.iter().enumerate() {
        for target in &layer.may_depend_on {
            if !names.contains(target) {
                return Err(ArchguardError::Config(format!(
                    "layers[{index}].may_depend_on: unknown layer {target:?}"
                )));
            }
        }
    }
    Ok(())
}

/// The commented stock-default `archguard.toml` of Kap. 11, used by
/// `archguard init --defaults`.
pub fn default_config_string() -> String {
    DEFAULT_CONFIG.to_string()
}

/// Provenance map with every [`SECTION_KEYS`] entry at
/// [`ConfigSource::Default`] — the shape of a config nothing of which
/// was written by the user (the input of [`analyze`](crate::effective::analyze)
/// and of `archguard init`).
pub(crate) fn all_default_sources() -> BTreeMap<String, ConfigSource> {
    SECTION_KEYS
        .iter()
        .map(|key| ((*key).to_string(), ConfigSource::Default))
        .collect()
}

/// Whether `value` writes at least one value: a table header without keys
/// (`[modules]`, also nested like `[semantic.java]`) declares nothing, so its
/// section stays derived or default (`declared ⇐ derived ⇐ default`).
fn declares_value(value: &toml::Value) -> bool {
    match value {
        toml::Value::Table(table) => table.values().any(declares_value),
        _ => true,
    }
}

/// A [`ArchguardError::Config`] prefixed with the offending file.
fn config_error(path: &Path, err: &dyn std::fmt::Display) -> ArchguardError {
    ArchguardError::Config(format!("{}: {err}", path.display()))
}

/// Deliberately narrow date values: calendar dates or positive relative units.
/// Dates are always passed as one `--since=...` argument, never as revisions.
pub(crate) fn validate_review_since(since: &str) -> Result<(), ArchguardError> {
    let relative = since.strip_suffix(".ago").unwrap_or(since);
    let valid_relative = relative.split_once('.').is_some_and(|(number, unit)| {
        !number.is_empty()
            && number.bytes().all(|byte| byte.is_ascii_digit())
            && number.parse::<u32>().is_ok_and(|count| count > 0)
            && matches!(
                unit,
                "day" | "days" | "week" | "weeks" | "month" | "months" | "year" | "years"
            )
    });
    let date: Vec<_> = since.split('-').collect();
    let valid_date = if date.len() == 3
        && date[0].len() == 4
        && date[1].len() == 2
        && date[2].len() == 2
        && date
            .iter()
            .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        let year: u32 = date[0].parse().unwrap_or(0);
        let month: u32 = date[1].parse().unwrap_or(0);
        let day: u32 = date[2].parse().unwrap_or(0);
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if leap {
                    29
                } else {
                    28
                }
            }
            _ => 0,
        };
        year > 0 && day > 0 && day <= days
    } else {
        false
    };
    if since.len() <= 64 && (valid_relative || valid_date) {
        Ok(())
    } else {
        Err(ArchguardError::Config(
            "review Git window must be YYYY-MM-DD or a positive relative duration such as 180.days or 6.months".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use archguard_core::{DenyCycles, StoreConfig};

    use super::*;

    /// Write `content` to a fresh temp dir as `name` and return the dir.
    fn project_with_file(name: &str, content: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut file = fs::File::create(dir.path().join(name)).expect("create file");
        file.write_all(content.as_bytes()).expect("write file");
        dir
    }

    fn project_with_toml(content: &str) -> tempfile::TempDir {
        project_with_file(CONFIG_FILE_NAME, content)
    }

    /// `sources` contains exactly the six tracked sections. The `BTreeMap`
    /// hands its keys back sorted, hence the sorted comparison.
    fn assert_all_sections(sources: &BTreeMap<String, ConfigSource>) {
        let mut expected: Vec<&str> = SECTION_KEYS.to_vec();
        expected.sort_unstable();
        let keys: Vec<&str> = sources.keys().map(String::as_str).collect();
        assert_eq!(keys, expected);
    }

    #[test]
    fn missing_file_yields_default_config_and_all_sections_default() {
        let dir = tempfile::tempdir().expect("temp dir");
        let loaded = load_config(None, dir.path()).expect("load without file");
        assert_eq!(loaded.config, Config::default());
        assert_all_sections(&loaded.sources);
        assert!(
            loaded
                .sources
                .values()
                .all(|source| *source == ConfigSource::Default)
        );
    }

    #[test]
    fn partial_file_marks_only_rules_declared() {
        let dir = project_with_toml(
            "[rules]\n\
             deny_cycles = \"off\"\n",
        );
        let loaded = load_config(None, dir.path()).expect("load partial file");
        assert_all_sections(&loaded.sources);
        assert_eq!(loaded.sources["rules"], ConfigSource::Declared);
        for key in ["project", "modules", "layers", "store", "derive_layers"] {
            assert_eq!(loaded.sources[key], ConfigSource::Default, "section {key}");
        }
        assert_eq!(loaded.config.rules.deny_cycles, DenyCycles::Off);
        assert_eq!(loaded.config.project.source_roots, ["src"]);
        assert!(loaded.config.derive_layers);
    }

    #[test]
    fn declared_sections_are_tracked_including_layers_and_top_level_key() {
        let dir = project_with_toml(
            "derive_layers = false\n\
             [store]\n\
             db = \".archguard/other.db\"\n\
             [[layers]]\n\
             name = \"domain\"\n\
             contains = [\"domain/**\"]\n",
        );
        let loaded = load_config(None, dir.path()).expect("load full-ish file");
        assert_eq!(loaded.sources["derive_layers"], ConfigSource::Declared);
        assert_eq!(loaded.sources["layers"], ConfigSource::Declared);
        assert_eq!(loaded.sources["store"], ConfigSource::Declared);
        assert_eq!(loaded.sources["project"], ConfigSource::Default);
        assert!(!loaded.config.derive_layers);
        assert_eq!(loaded.config.store.db, ".archguard/other.db");
        assert_eq!(loaded.config.layers.len(), 1);
        assert_eq!(loaded.config.layers[0].name, "domain");
    }

    #[test]
    fn section_headers_without_values_declare_nothing() {
        // A header alone writes no value: every value of the section is
        // still its default, so the section must report `Default`.
        let dir = project_with_toml("[project]\n[modules]\n[rules]\n[semantic.java]\n");
        let loaded = load_config(None, dir.path()).expect("load header-only file");
        assert_eq!(loaded.config, Config::default());
        assert!(
            loaded
                .sources
                .values()
                .all(|source| *source == ConfigSource::Default),
            "sources: {:?}",
            loaded.sources
        );
    }

    #[test]
    fn empty_file_is_the_default_config() {
        let dir = project_with_toml("");
        let loaded = load_config(None, dir.path()).expect("load empty file");
        assert_eq!(loaded.config, Config::default());
        assert!(
            loaded
                .sources
                .values()
                .all(|source| *source == ConfigSource::Default)
        );
    }

    #[test]
    fn broken_toml_yields_config_error_naming_the_file() {
        let dir = project_with_toml("[project\nsource_roots = ");
        let err = load_config(None, dir.path()).expect_err("broken toml must fail");
        match &err {
            ArchguardError::Config(message) => {
                assert!(message.contains(CONFIG_FILE_NAME), "message: {message}");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
        assert!(err.to_string().starts_with("config error:"));
    }

    #[test]
    fn wrong_types_yield_config_error_naming_the_file() {
        let dir = project_with_toml("[modules]\ndepth = \"far\"\n");
        let err = load_config(None, dir.path()).expect_err("wrong type must fail");
        match &err {
            ArchguardError::Config(message) => {
                assert!(message.contains(CONFIG_FILE_NAME), "message: {message}");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn explicit_path_is_used_even_outside_the_root() {
        let root = tempfile::tempdir().expect("temp dir");
        let custom = project_with_file("custom.toml", "[store]\nsync_on_check = true\n");
        let loaded = load_config(Some(&custom.path().join("custom.toml")), root.path())
            .expect("load explicit file");
        assert!(loaded.config.store.sync_on_check);
        assert_eq!(loaded.sources["store"], ConfigSource::Declared);
        assert_eq!(loaded.sources["modules"], ConfigSource::Default);
        // A stray `archguard.toml` in the root is not consulted: with the
        // explicit file wins, `modules.depth` stays at its default.
        fs::write(root.path().join(CONFIG_FILE_NAME), "[modules]\ndepth = 9\n")
            .expect("write root file");
        let loaded = load_config(Some(&custom.path().join("custom.toml")), root.path())
            .expect("load explicit file with stray root config");
        assert_eq!(loaded.config.modules.depth, 2);
    }

    #[test]
    fn missing_explicit_path_yields_config_error_naming_the_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("absent.toml");
        let err = load_config(Some(&missing), dir.path()).expect_err("missing explicit file");
        match &err {
            ArchguardError::Config(message) => {
                assert!(message.contains("absent.toml"), "message: {message}");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn default_config_string_parses_to_the_default_config() {
        let dir = project_with_toml(&default_config_string());
        let loaded = load_config(None, dir.path()).expect("load stock defaults");
        assert_eq!(loaded.config, Config::default());
        assert!(loaded.config.derive_layers);
        assert_eq!(loaded.config.store, StoreConfig::default());
        // The stock file spells out every key but `[[layers]]` (commented
        // out — layers are derived), so that one stays Default.
        assert_eq!(loaded.sources["layers"], ConfigSource::Default);
        for key in ["project", "modules", "rules", "store", "derive_layers"] {
            assert_eq!(loaded.sources[key], ConfigSource::Declared, "section {key}");
        }
    }

    #[test]
    fn review_weights_and_gates_parse_and_are_declared() {
        let dir = project_with_toml(
            "[review]\n\
             health_weights = { tangledness = 2.0, coverage = 0.5 }\n\
             max_sdp_violations = 0\n\
             max_tangledness = 0.5\n\
             max_propagation_cost = 0.4\n\
             adr_required_on_accept_risk = true\n",
        );
        let loaded = load_config(None, dir.path()).expect("load review section");
        let review = &loaded.config.review;
        assert_eq!(
            review
                .health_weights
                .as_ref()
                .expect("weights")
                .get("tangledness"),
            Some(&2.0)
        );
        assert_eq!(
            review
                .health_weights
                .as_ref()
                .expect("weights")
                .get("coverage"),
            Some(&0.5)
        );
        assert_eq!(
            review.max_sdp_violations,
            Some(0),
            "0 is a real gate, not off"
        );
        assert_eq!(review.max_tangledness, Some(0.5));
        assert_eq!(review.max_propagation_cost, Some(0.4));
        assert!(review.adr_required_on_accept_risk);
        assert_eq!(loaded.sources["review"], ConfigSource::Declared);
    }

    #[test]
    fn review_gates_default_to_off_not_to_zero() {
        let dir = tempfile::tempdir().expect("temp dir");
        let loaded = load_config(None, dir.path()).expect("load without file");
        let review = &loaded.config.review;
        assert!(review.health_weights.is_none(), "unlisted parts weigh 1.0");
        assert_eq!(review.max_sdp_violations, None);
        assert_eq!(review.max_tangledness, None);
        assert_eq!(review.max_propagation_cost, None);
        assert!(!review.adr_required_on_accept_risk);
    }

    #[test]
    fn negative_health_weight_is_a_config_error() {
        let dir = project_with_toml("[review]\nhealth_weights = { hubs = -1.0 }\n");
        let err = load_config(None, dir.path()).expect_err("negative weight must fail");
        match &err {
            ArchguardError::Config(message) => {
                assert!(
                    message.contains("review.health_weights[hubs]"),
                    "message: {message}"
                );
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn normalized_gate_beyond_one_is_a_config_error() {
        for key in ["max_tangledness", "max_propagation_cost"] {
            let dir = project_with_toml(&format!("[review]\n{key} = 1.5\n"));
            let err = load_config(None, dir.path()).expect_err("{key} = 1.5 must be rejected");
            match &err {
                ArchguardError::Config(message) => {
                    assert!(
                        message.contains(&format!("review.{key}")),
                        "message: {message}"
                    );
                }
                other => panic!("expected Config error, got {other:?}"),
            }
        }
    }
}
