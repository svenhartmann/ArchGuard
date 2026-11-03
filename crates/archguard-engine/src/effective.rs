//! Effective config derivation (T46, ARCHITECTURE.md Kap. 6 steps 2
//! and 7, Kap. 11): the declared delta of `archguard.toml` is filled up
//! per section — `deklariert ⇐ abgeleitet ⇐ Default` — and the
//! provenance of every section flows into [`Report::config`].
//!
//! * **Project** (step 2, before the walk): when `[project]` was not
//!   written, [`derive_project_hints`] derives the source roots from
//!   the top-level directories holding the bulk of the source files
//!   (overridden by a `composer.json`'s PSR-4 directories) and the
//!   languages from the file extensions.
//! * **Depth** (step 7, after the build): when `[modules]` was not
//!   written, [`archguard_analysis::suggest_depth`] picks the zoom that
//!   lands the view in the 10–100 node band.
//! * **Layers** (step 7, after view and SCCs): when `derive_layers`
//!   is on, [`archguard_analysis::infer_layers`] ranks the view; the
//!   derived entries merge with the declared ones **per name**
//!   (Kap. 11: a declared entry wins its name, unmatched derived
//!   entries append). A cycle that blocked ranking surfaces once as
//!   an Info [`FindingKind::LayerUnranked`] hint.
//! * **`archguard init`**: [`effective_config_string`] renders what an
//!   empty declared config derives over a project, as the Kap. 11
//!   example file — provenance comments and the measured gate
//!   proposals commented out.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use archguard_analysis::{find_sccs, infer_layers, suggest_depth};
use archguard_core::{
    ArchguardError, Config, ConfigSource, Cut, DenyCycles, Finding, FindingKind, Graph,
    LayerConfig, LayerOrigin, ProjectConfig, Report, ReviewSources, Severity, Status, aggregate,
};
use archguard_extract::{extractor_for, parse_psr4, strip_root, walk};

use crate::config::LoadedConfig;
use crate::pipeline::{self, Level};

/// Share of the source files the derived roots must cover together.
const ROOT_COVERAGE_PERCENT: usize = 80;
/// Cap on simultaneously derived source roots.
const MAX_DERIVED_ROOTS: usize = 3;
/// Root of a project whose walk names no dominant directory.
const FALLBACK_ROOT: &str = "src";
/// Language list of a project without scannable files.
const AUTO_LANGUAGE: &str = "auto";
/// Target band for the number of visible view nodes (Kap. 11).
const VIEW_MIN: usize = 10;
/// Upper end of the [`VIEW_MIN`] band.
const VIEW_MAX: usize = 100;
/// Nodes named in the unranked hint.
const MAX_UNRANKED_NODES: usize = 10;
/// Id of the single [`FindingKind::LayerUnranked`] finding.
const UNRANKED_FINDING_ID: &str = "layers-unranked";

/// One complete effective run: everything the derivation produced,
/// kept so callers (and `archguard init`) can look behind the report.
#[derive(Debug, Clone)]
pub struct AnalysisRun {
    /// The full hierarchical graph (Kap. 6 step 6).
    pub graph: Graph,
    /// The report of the default view, provenance filled.
    pub report: Report,
    /// The effective config the run actually analysed with.
    pub config: Config,
    /// The analysed view — input to the layer derivation.
    pub view: Graph,
    /// External-import warnings, as [`pipeline::print_warnings`] prints them.
    pub warnings: Vec<String>,
    pub cache_hit: bool,
}

impl AnalysisRun {
    /// Effective values and provenance for view/simulation consumers.
    /// Never feed this back as declared configuration on a later run.
    pub fn effective_config(&self) -> LoadedConfig {
        LoadedConfig {
            config: self.config.clone(),
            sources: self.report.config.clone(),
        }
    }
}

/// Shared CLI/MCP analysis contract. Extraction may be cached; derivation,
/// view selection and gates always run against the current declared config.
pub fn analyze_run(
    root: &Path,
    loaded: &LoadedConfig,
    level: Level,
) -> Result<AnalysisRun, ArchguardError> {
    let run = analyze_run_quiet(root, loaded, level)?;
    pipeline::print_warnings(&run.warnings);
    Ok(run)
}

/// The shared cached analysis without writing diagnostics to stderr.
/// Warnings remain in [`AnalysisRun::warnings`] on both cache hits and misses,
/// so repeated consumers can choose when to present changed diagnostics.
pub fn analyze_run_quiet(
    root: &Path,
    loaded: &LoadedConfig,
    level: Level,
) -> Result<AnalysisRun, ArchguardError> {
    run_inner(root, loaded, level, true)
}

/// The complete pipeline over `root` with `loaded` as the **declared**
/// config: sections the user wrote are used verbatim, the rest is
/// derived (Kap. 6 steps 2 and 7) before walk, extract, resolve and
/// build run against the effective values. The provenance of every
/// section lands in [`Report::config`].
///
/// External-import warnings go to stderr as in [`analyze`](crate::pipeline::analyze).
pub fn analyze_effective(
    root: &Path,
    loaded: &LoadedConfig,
    level: Level,
) -> Result<(Graph, Report), ArchguardError> {
    let run = effective_run(root, loaded, level)?;
    pipeline::print_warnings(&run.warnings);
    Ok((run.graph, run.report))
}

/// The effective pipeline body: derivation, walk, extract+build, view,
/// layer merge, report — handing the whole run back to the caller.
pub(crate) fn effective_run(
    root: &Path,
    loaded: &LoadedConfig,
    level: Level,
) -> Result<AnalysisRun, ArchguardError> {
    run_inner(root, loaded, level, false)
}

fn run_inner(
    root: &Path,
    loaded: &LoadedConfig,
    level: Level,
    cached: bool,
) -> Result<AnalysisRun, ArchguardError> {
    crate::config::validate_config(&loaded.config)?;
    let mut config = loaded.config.clone();
    let mut sources = loaded.sources.clone();

    // ---- Kap. 6 step 2: project derivation, before the walk.
    if !is_declared(&sources, "project") {
        let (roots, languages) = derive_project_hints(root)?;
        config.project.source_roots = roots;
        config.project.languages = languages;
        sources.insert("project".to_string(), ConfigSource::Derived);
    }

    // ---- Kap. 6 steps 3–6: walk, extract, resolve, build.
    let files = pipeline::walk_files(root, &config)?;
    let (mut graph, mut warnings, cache_hit) = if cached {
        crate::cache::extract_cached(root, &files, &config)?
    } else {
        let (graph, warnings) = pipeline::build_graph(root, &files, &config)?;
        (graph, warnings, false)
    };
    // The disk cache contains imports only. Compiler evidence is projected on
    // every run using its own source/project/toolchain fingerprints, so a warm
    // import cache cannot hide changed method bodies or recovered toolchains.
    crate::semantic_calls::integrate_graph(root, &files, &config, &mut graph)?;

    // ---- Kap. 6 step 7a: the zoom, measured on the finished graph.
    if !is_declared(&sources, "modules") {
        config.modules.depth = suggest_depth(&graph, VIEW_MIN, VIEW_MAX);
        sources.insert("modules".to_string(), ConfigSource::Derived);
    }

    // The view and its SCCs, computed once: both feed the derivation
    // and `build_report` rederives them cheaply from the same inputs.
    let cut = match level {
        Level::Module => Cut::at_depth(&graph, config.modules.depth as usize),
        Level::File => Cut::at_depth(&graph, usize::MAX),
    };
    let view = aggregate(&graph, &cut);
    let sccs = find_sccs(&view);

    // ---- Kap. 6 step 7b: layer derivation and the merge per name.
    let derived = config.derive_layers.then(|| infer_layers(&view, &sccs));
    config.layers = merge_layers(
        &config.layers,
        derived.as_ref().map_or_else(Vec::new, |d| d.layers.clone()),
    );
    let has_declared = config
        .layers
        .iter()
        .any(|layer| layer.origin == LayerOrigin::Declared);
    let has_derived_layer = config
        .layers
        .iter()
        .any(|layer| layer.origin == LayerOrigin::Derived);
    sources.insert(
        "layers".to_string(),
        match (has_declared, has_derived_layer) {
            (true, true) => ConfigSource::Mixed,
            (true, false) => ConfigSource::Declared,
            (false, true) => ConfigSource::Derived,
            (false, false) => ConfigSource::Default,
        },
    );

    // ---- R23: attach the review KPIs here, not in `build_graph`: the
    // graph cache is keyed by the project section only, so the KPIs
    // must see the fully effective config (depth, declared layers) and
    // must be recomputed on every run — including a cache hit, which
    // replays an imports-only graph without them. A source that
    // vanished since the walk is skipped, exactly like the extractor
    // skips files it cannot read. Attaching before the report is built
    // lets the report inherit them like `semantic` (R26).
    let mut scanned: Vec<(String, archguard_extract::Lang, String)> = Vec::new();
    for file in &files {
        // Same scanned set as `build_graph`: only files with a
        // registered extractor became graph nodes, and the scans key
        // on the source-root-relative node name.
        if extractor_for(file.lang).is_none() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&file.path) else {
            continue;
        };
        scanned.push((
            strip_root(&file.rel_path, &config.project.source_roots),
            file.lang,
            content,
        ));
    }
    let kpi_sources: Vec<crate::kpi_payload::ScannedSource> = scanned
        .iter()
        .map(|(path, lang, content)| crate::kpi_payload::ScannedSource {
            path,
            lang: *lang,
            content,
        })
        .collect();
    graph.review_kpis = Some(crate::kpi_payload::compute_review_kpis(
        root,
        &config,
        &graph,
        &kpi_sources,
    )?);

    // ---- Kap. 6 steps 8–10: analyse and report with the effective config.
    crate::config::validate_layer_targets(&config)?;
    let (mut report, policy_warnings) = build_effective_report(root, &graph, &config, level)?;
    warnings.extend(policy_warnings);

    // ---- Kap. 5: the provenance of every tracked section.
    report.config = pipeline::PROVENANCE_KEYS
        .iter()
        .map(|key| {
            (
                (*key).to_string(),
                sources.get(*key).copied().unwrap_or(ConfigSource::Default),
            )
        })
        .collect();

    Ok(AnalysisRun {
        graph,
        report,
        config,
        view,
        warnings,
        cache_hit,
    })
}

/// Analyse with already effective policy values, preserving declared/derived
/// layer origins. Shared with Git comparison, where derivation must stay frozen.
pub(crate) fn build_effective_report(
    root: &Path,
    graph: &Graph,
    config: &Config,
    level: Level,
) -> Result<(Report, Vec<String>), ArchguardError> {
    let mut report = pipeline::build_report(graph, config, level)?;
    let mut warnings = Vec::new();
    if !config.ownership_rules.is_empty() {
        let files = crate::review_sources::collect_source_evidence(root, config, graph)?;
        let ownership_sources = ReviewSources {
            mapped_files: files.iter().filter(|file| file.path.is_some()).count() as u32,
            total_files: files.len() as u32,
            files,
            ..ReviewSources::default()
        };
        let ownership = crate::ownership::analyze_ownership(
            root,
            graph,
            &ownership_sources,
            &config.ownership_rules,
        )?;
        if ownership.status != crate::ownership::OwnershipStatus::Available || !ownership.complete {
            return Err(ArchguardError::Config(format!(
                "ownership_rules require complete CODEOWNERS coverage: {}",
                ownership
                    .warnings
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            )));
        }
        warnings.extend(ownership.warnings);
        report.findings.extend(ownership.findings);
        report
            .findings
            .sort_by(|a, b| a.severity.cmp(&b.severity).then_with(|| a.id.cmp(&b.id)));
        if report
            .findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
        {
            report.summary.status = Status::Fail;
        }
    }

    // A cycle that blocked ranking is one Info hint, not a gate —
    // Info findings never fail the run (Kap. 11 layer promotion).
    let view = aggregate(
        graph,
        &Cut::at_depth(
            graph,
            if level == Level::File {
                usize::MAX
            } else {
                config.modules.depth as usize
            },
        ),
    );
    let derived = config
        .derive_layers
        .then(|| infer_layers(&view, &find_sccs(&view)));
    if config
        .layers
        .iter()
        .any(|layer| layer.origin == LayerOrigin::Derived)
        && let Some(derived) = &derived
        && !derived.unranked.is_empty()
    {
        report.findings.push(Finding {
            message: None,
            evidence: Vec::new(),
            kind: FindingKind::LayerUnranked,
            id: UNRANKED_FINDING_ID.to_string(),
            severity: Severity::Info,
            nodes: derived
                .unranked
                .iter()
                .take(MAX_UNRANKED_NODES)
                .cloned()
                .collect(),
            edge_count: 0,
            cycles_sampled: 0,
            sampling: None,
            suggested_cuts: Vec::new(),
        });
        report
            .findings
            .sort_by(|a, b| a.severity.cmp(&b.severity).then_with(|| a.id.cmp(&b.id)));
    }

    Ok((report, warnings))
}

/// Kap. 6 step 2: source roots and languages from the project itself.
///
/// One walk with the default exclude globs. Roots are the top-level
/// directories that together hold [`ROOT_COVERAGE_PERCENT`] % of the
/// source files (at most [`MAX_DERIVED_ROOTS`], returned name-sorted;
/// [`FALLBACK_ROOT`] when none qualifies), overridden by the PSR-4
/// directories of a root `composer.json` when it names any. Languages
/// are the distinct file extensions, sorted; a project without any
/// scannable file yields `(["src"], ["auto"])`. Roots that would give two
/// files the same stripped path are dropped in favour of `["."]`.
pub fn derive_project_hints(root: &Path) -> Result<(Vec<String>, Vec<String>), ArchguardError> {
    let files = walk(root, &ProjectConfig::default().exclude)?;

    let mut per_dir: BTreeMap<&str, usize> = BTreeMap::new();
    for file in &files {
        if let Some((dir, _)) = file.rel_path.split_once('/') {
            *per_dir.entry(dir).or_default() += 1;
        }
    }
    let mut roots = ranked_roots(&per_dir, files.len());

    // A composer.json's PSR-4 directories are the authoritative roots.
    if let Ok(content) = fs::read_to_string(root.join("composer.json")) {
        let mut psr4: Vec<String> = parse_psr4(&content)
            .into_iter()
            .map(|(_, dir)| dir.trim_matches('/').to_string())
            .filter(|dir| !dir.is_empty())
            .collect();
        psr4.sort();
        psr4.dedup();
        if !psr4.is_empty() {
            roots = psr4;
        }
    }

    // A Go module's directories are package identities, not disposable source
    // roots. Preserve them even when one package holds most source files.
    if files
        .iter()
        .any(|file| file.lang == archguard_extract::Lang::Go)
        && crate::service_projects::read_manifest(root, Path::new("go.mod"))
            .ok()
            .and_then(|source| archguard_extract::go::module_path(&source))
            .is_some()
    {
        roots = vec![".".into()];
    }

    // Stripping must never give two files of different directories one name,
    // as `cli/src/lib.rs` and `core/src/lib.rs` would under `["cli", "core"]`.
    let mut stripped = BTreeSet::new();
    if !files
        .iter()
        .all(|file| stripped.insert(strip_root(&file.rel_path, &roots)))
    {
        roots = vec![".".into()];
    }

    let mut languages: Vec<String> = files
        .iter()
        .map(|file| pipeline::lang_name(file.lang).to_string())
        .collect();
    languages.sort();
    languages.dedup();
    if languages.is_empty() {
        languages.push(AUTO_LANGUAGE.to_string());
    }
    Ok((roots, languages))
}

/// The [`effective_config_string`] of `root` as one complete run, kept
/// for the gate proposals (its view, SCCs and metrics).
pub fn effective_config_string(root: &Path) -> String {
    let loaded = LoadedConfig {
        config: Config::default(),
        sources: crate::config::all_default_sources(),
    };
    match effective_run(root, &loaded, Level::Module) {
        Ok(run) => render_effective_toml(
            &run.config,
            archguard_analysis::lakos_metrics(&run.view).1 as f32,
            measured_core_instability(&run.config, &run.report),
        ),
        // An unanalysable root still gets a valid starting file.
        Err(_) => render_effective_toml(&Config::default(), 0.0, 0.0),
    }
}

/// Whether `sources` marks `key` as written by the user.
fn is_declared(sources: &BTreeMap<String, ConfigSource>, key: &str) -> bool {
    sources.get(key) == Some(&ConfigSource::Declared)
}

/// Kap. 6 step 7b "Korrektur pro Name": every declared entry is kept
/// (its origin forced to [`LayerOrigin::Declared`], which drives the
/// severity of [`archguard_analysis::check_layers`]); a derived entry
/// whose name no declared entry claims appends untouched.
fn merge_layers(declared: &[LayerConfig], derived: Vec<LayerConfig>) -> Vec<LayerConfig> {
    let mut merged: Vec<LayerConfig> = declared
        .iter()
        .map(|entry| LayerConfig {
            origin: LayerOrigin::Declared,
            ..entry.clone()
        })
        .collect();
    let names: BTreeSet<String> = merged.iter().map(|entry| entry.name.clone()).collect();
    let appended: Vec<LayerConfig> = derived
        .into_iter()
        .filter(|entry| !names.contains(&entry.name))
        .collect();
    merged.extend(appended);
    merged
}

/// The top-level directories by descending file count until
/// [`ROOT_COVERAGE_PERCENT`] % of `total` files are covered, capped at
/// [`MAX_DERIVED_ROOTS`], name-sorted; [`FALLBACK_ROOT`] when none
/// qualifies (empty walk or files only at the root itself).
fn ranked_roots(per_dir: &BTreeMap<&str, usize>, total: usize) -> Vec<String> {
    let mut ranked: Vec<(usize, &str)> =
        per_dir.iter().map(|(dir, count)| (*count, *dir)).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));

    let mut chosen: Vec<&str> = Vec::new();
    let mut covered = 0usize;
    for (count, dir) in ranked {
        if chosen.len() >= MAX_DERIVED_ROOTS || covered * 100 >= total * ROOT_COVERAGE_PERCENT {
            break;
        }
        chosen.push(dir);
        covered += count;
    }
    if chosen.is_empty() {
        return vec![FALLBACK_ROOT.to_string()];
    }
    let mut roots: Vec<String> = chosen.into_iter().map(str::to_string).collect();
    roots.sort();
    roots
}

/// The measured core instability: the largest `instability` among the
/// metrics entries the bottom layer's `contains` globs name directly
/// (derived entries are `<node>/**`, which maps back one-to-one);
/// `0.0` without layers or matching metrics. Commented-out proposal
/// only, like the Lakos ACD proposal.
fn measured_core_instability(config: &Config, report: &Report) -> f32 {
    let Some(bottom) = config
        .layers
        .iter()
        .find(|layer| layer.may_depend_on.is_empty())
    else {
        return 0.0;
    };
    let mut best = 0.0f32;
    for entry in &bottom.contains {
        let name = entry.strip_suffix("/**").unwrap_or(entry);
        if let Some(metrics) = report.metrics.get(name) {
            best = best.max(metrics.instability);
        }
    }
    best
}

/// Render the effective config as the Kap. 11 `archguard.toml` example:
/// values from `config`, provenance comments per section, the two
/// gate proposals commented out with their measured values.
fn render_effective_toml(config: &Config, max_acd: f32, max_instability_core: f32) -> String {
    let deny_cycles = match config.rules.deny_cycles {
        DenyCycles::Module => "module",
        DenyCycles::File => "file",
        DenyCycles::Off => "off",
    };
    let acd = format!("{max_acd:.2}");
    let instability = format!("{max_instability_core:.2}");

    let mut out = String::new();
    let _ = writeln!(
        out,
        "# archguard.toml — abgeleitet von `archguard init` (ARCHITECTURE.md, Kap. 11)."
    );
    let _ = writeln!(
        out,
        "# Deklarierte Einträge überschreiben abgeleitete pro Name."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "derive_layers = {}", config.derive_layers);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "[project]                   # abgeleitet (Manifest-Sniffing + Endungs-Statistik)"
    );
    let _ = writeln!(
        out,
        "source_roots = {}",
        toml_list(&config.project.source_roots)
    );
    let _ = writeln!(
        out,
        "exclude = {}   # Default",
        toml_list(&config.project.exclude)
    );
    let _ = writeln!(
        out,
        "languages = {}          # abgeleitet",
        toml_list(&config.project.languages)
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "[modules]");
    let _ = writeln!(
        out,
        "depth = {}                   # abgeleitet (Ziel: 10–100 Sicht-Knoten)",
        config.modules.depth
    );
    let _ = writeln!(out);
    for (index, layer) in config.layers.iter().enumerate() {
        if index == 0 {
            let _ = writeln!(
                out,
                "[[layers]]                  # abgeleitet: Dependency-Ranking + Namensaffinität"
            );
        } else {
            let _ = writeln!(out, "[[layers]]");
        }
        let _ = writeln!(out, "name = {}", toml_str(&layer.name));
        let _ = writeln!(out, "contains = {}", toml_list(&layer.contains));
        if layer.may_depend_on.is_empty() {
            let _ = writeln!(
                out,
                "may_depend_on = []          # unterste Schicht (keine ausgehenden Kanten)"
            );
        } else {
            let _ = writeln!(out, "may_depend_on = {}", toml_list(&layer.may_depend_on));
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "[rules]");
    let _ = writeln!(
        out,
        "deny_cycles = \"{deny_cycles}\"      # Intent (Default)"
    );
    let _ = writeln!(
        out,
        "max_scc_size = {}            # Intent (Default)",
        config.rules.max_scc_size
    );
    let _ = writeln!(out, "baseline = {}", toml_str(&config.rules.baseline));
    let _ = writeln!(
        out,
        "# max_acd = {acd:<4}                  # gemessen: {acd} — Gate-Vorschlag von init (0 = aus)"
    );
    let _ = writeln!(
        out,
        "# max_instability_core = {instability:<4}     # gemessene Core-Instabilität — Gate-Vorschlag (0 = aus)"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "[store]                     # Default");
    let _ = writeln!(out, "db = {}", toml_str(&config.store.db));
    let _ = writeln!(out, "sync_on_check = {}", config.store.sync_on_check);
    let _ = writeln!(
        out,
        "\n[review]\n{}",
        toml::to_string(&config.review).expect("review settings are TOML values")
    );
    let _ = writeln!(
        out,
        "[performance]\n{}",
        toml::to_string(&config.performance).expect("performance settings are TOML values")
    );
    for rule in &config.dependency_rules {
        let _ = writeln!(
            out,
            "[[dependency_rules]]\n{}",
            toml::to_string(rule).expect("dependency rule is a TOML value")
        );
    }
    for rule in &config.ownership_rules {
        let _ = writeln!(
            out,
            "[[ownership_rules]]\n{}",
            toml::to_string(rule).expect("ownership rule is a TOML value")
        );
    }
    let mut architecture = toml::Table::new();
    architecture.insert(
        "architecture".into(),
        toml::Value::try_from(&config.architecture).expect("architecture context is a TOML value"),
    );
    let _ = writeln!(
        out,
        "{}",
        toml::to_string(&architecture).expect("architecture settings are TOML values")
    );
    out
}

/// A TOML basic string with `\` and `"` escaped.
fn toml_str(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A TOML inline array of strings.
fn toml_list(values: &[String]) -> String {
    let items: Vec<String> = values.iter().map(|value| toml_str(value)).collect();
    format!("[{}]", items.join(", "))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use archguard_core::{ConfigSource, LayerOrigin, Status};

    use super::*;

    /// Write `files` (path, content) into a fresh temp dir.
    fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        for (rel, content) in files {
            let path = dir.path().join(rel);
            fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
            fs::write(path, content).expect("write file");
        }
        dir
    }

    /// A clean three-layer project `shop/{api,app,domain}`: at the derived
    /// view depth the modules `shop/api`, `shop/app`, `shop/domain` become
    /// the visible nodes, so their trailing segments hit the naming
    /// affinity vocabulary exactly (Kap. 7) — the derived layers are named
    /// `domain`, `app`, `api`.
    fn layered_fixture() -> tempfile::TempDir {
        fixture(&[
            (
                "src/shop/api/one.ts",
                "import { a } from \"../app/one\";\nexport const one = a + 1;\n",
            ),
            (
                "src/shop/app/one.ts",
                "import { d } from \"../domain/one\";\nexport const a = d + 1;\n",
            ),
            ("src/shop/domain/one.ts", "export const d = 1;\n"),
        ])
    }

    /// `loaded` with `config` and the given provenance overrides.
    fn loaded_with(config: Config, declared: &[&str]) -> LoadedConfig {
        let mut sources = crate::config::all_default_sources();
        for key in declared {
            sources.insert((*key).to_string(), ConfigSource::Declared);
        }
        LoadedConfig { config, sources }
    }

    fn names(layers: &[LayerConfig]) -> Vec<&str> {
        layers.iter().map(|layer| layer.name.as_str()).collect()
    }

    #[test]
    fn derive_project_hints_takes_the_dominant_top_level_dirs() {
        // `src` alone covers 8 of 9 files → single root.
        let mut files: Vec<(String, String)> = (0..8)
            .map(|i| {
                (
                    format!("src/mod{i}.ts"),
                    "export const x = 1;\n".to_string(),
                )
            })
            .collect();
        files.push((
            "tools/gen.js".to_string(),
            "export const g = 1;\n".to_string(),
        ));
        files.push((
            "bin/main.js".to_string(),
            "export const m = 1;\n".to_string(),
        ));
        let dir = fixture(
            &files
                .iter()
                .map(|(p, c)| (p.as_str(), c.as_str()))
                .collect::<Vec<_>>(),
        );
        let (roots, languages) = derive_project_hints(dir.path()).expect("hints");
        assert_eq!(roots, ["src"]);
        assert_eq!(languages, ["js", "ts"]);
    }

    #[test]
    fn derive_project_hints_caps_roots_at_three_in_name_order() {
        // Four equally sized dirs: the coverage loop may reach the cap
        // before 80 % and must still return exactly three, sorted. File names
        // differ per dir, so stripping the roots keeps every file distinct.
        let mut files: Vec<(String, String)> = Vec::new();
        for dir in ["alpha", "beta", "gamma", "delta"] {
            for i in 0..3 {
                files.push((
                    format!("{dir}/{dir}{i}.ts"),
                    "export const x = 1;\n".to_string(),
                ));
            }
        }
        let dir = fixture(
            &files
                .iter()
                .map(|(p, c)| (p.as_str(), c.as_str()))
                .collect::<Vec<_>>(),
        );
        let (roots, languages) = derive_project_hints(dir.path()).expect("hints");
        assert_eq!(roots.len(), 3, "capped at MAX_DERIVED_ROOTS");
        assert!(roots.windows(2).all(|w| w[0] < w[1]), "sorted: {roots:?}");
        assert_eq!(languages, ["ts"]);
    }

    #[test]
    fn derive_project_hints_falls_back_to_src_and_auto() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (roots, languages) = derive_project_hints(dir.path()).expect("hints");
        assert_eq!(roots, ["src"]);
        assert_eq!(languages, ["auto"]);

        // Files only at the project root itself: still the fallback.
        let dir = fixture(&[("solo.ts", "export const s = 1;\n")]);
        let (roots, languages) = derive_project_hints(dir.path()).expect("hints");
        assert_eq!(roots, ["src"]);
        assert_eq!(languages, ["ts"]);
    }

    #[test]
    fn composer_psr4_dirs_override_the_derived_roots() {
        let dir = fixture(&[
            ("src/a.ts", "export const a = 1;\n"),
            ("src/b.ts", "export const b = 1;\n"),
            ("src/c.ts", "export const c = 1;\n"),
            ("src/d.ts", "export const d = 1;\n"),
            ("app/e.ts", "export const e = 1;\n"),
            (
                "composer.json",
                r#"{"autoload": {"psr-4": {"App\\\\": "app/", "Lib\\\\": "lib/"}}}"#,
            ),
        ]);
        let (roots, _languages) = derive_project_hints(dir.path()).expect("hints");
        assert_eq!(roots, ["app", "lib"], "PSR-4 dirs win, sorted and deduped");
    }

    #[test]
    fn merge_layers_lets_declared_win_per_name() {
        let declared = vec![LayerConfig {
            name: "app".to_string(),
            contains: vec!["app/**".to_string()],
            may_depend_on: vec!["domain".to_string()],
            origin: LayerOrigin::Declared,
        }];
        let derived = vec![
            LayerConfig {
                name: "domain".to_string(),
                contains: vec!["domain/**".to_string()],
                may_depend_on: vec![],
                origin: LayerOrigin::Derived,
            },
            LayerConfig {
                name: "app".to_string(),
                contains: vec!["app/**".to_string()],
                may_depend_on: vec!["domain".to_string()],
                origin: LayerOrigin::Derived,
            },
        ];
        let merged = merge_layers(&declared, derived);
        assert_eq!(names(&merged), ["app", "domain"]);
        assert_eq!(
            merged[0].origin,
            LayerOrigin::Declared,
            "declared wins its name"
        );
        assert_eq!(merged[1].origin, LayerOrigin::Derived);

        // An all-declared merge contains no derived entry at all.
        let merged = merge_layers(&merged, vec![]);
        assert!(merged.iter().all(|l| l.origin == LayerOrigin::Declared));
    }

    #[test]
    fn undeclared_project_modules_layers_report_as_derived() {
        let dir = layered_fixture();
        let run = effective_run(
            dir.path(),
            &loaded_with(Config::default(), &[]),
            Level::Module,
        )
        .expect("run");
        assert_eq!(run.report.config["project"], ConfigSource::Derived);
        assert_eq!(run.report.config["modules"], ConfigSource::Derived);
        assert_eq!(run.report.config["layers"], ConfigSource::Derived);
        assert_eq!(run.report.config["rules"], ConfigSource::Default);
        assert_eq!(run.report.config["store"], ConfigSource::Default);
        // The three layers carry the naming-affinity names.
        assert_eq!(names(&run.config.layers), ["domain", "app", "api"]);
        assert!(
            run.config
                .layers
                .iter()
                .all(|layer| layer.origin == LayerOrigin::Derived)
        );
        assert_eq!(run.report.summary.status, Status::Pass);
    }

    #[test]
    fn declared_sections_stay_verbatim_and_declared() {
        // Four files per layer module: the measured view depth would be
        // 2 (12 file nodes inside the 10–100 target), the declared one
        // is 1 — it must survive untouched, and at depth 1 the visible
        // module names collide with the three declared layer names.
        let mut files: Vec<(String, String)> = Vec::new();
        for i in 0..4 {
            files.push((
                format!("src/shop/api/one{i}.ts"),
                format!("import {{ a{i} }} from \"../app/one{i}\";\nexport const v = a{i};\n"),
            ));
            files.push((
                format!("src/shop/app/one{i}.ts"),
                format!("import {{ d{i} }} from \"../domain/one{i}\";\nexport const a = d{i};\n"),
            ));
            files.push((
                format!("src/shop/domain/one{i}.ts"),
                format!("export const d{i} = {i};\n"),
            ));
        }
        let dir = fixture(
            &files
                .iter()
                .map(|(p, c)| (p.as_str(), c.as_str()))
                .collect::<Vec<_>>(),
        );
        let mut config = Config::default();
        config.modules.depth = 1;
        config.layers = vec![
            LayerConfig {
                name: "domain".to_string(),
                contains: vec!["domain/**".to_string()],
                may_depend_on: vec![],
                origin: LayerOrigin::Declared,
            },
            LayerConfig {
                name: "app".to_string(),
                contains: vec!["app/**".to_string()],
                may_depend_on: vec!["domain".to_string()],
                origin: LayerOrigin::Declared,
            },
            LayerConfig {
                name: "api".to_string(),
                contains: vec!["api/**".to_string()],
                may_depend_on: vec!["domain".to_string(), "app".to_string()],
                origin: LayerOrigin::Declared,
            },
        ];
        let run = effective_run(
            dir.path(),
            &loaded_with(config, &["modules", "layers"]),
            Level::Module,
        )
        .expect("run");
        assert_eq!(run.report.config["modules"], ConfigSource::Declared);
        assert_eq!(run.report.config["layers"], ConfigSource::Declared);
        assert_eq!(run.report.config["project"], ConfigSource::Derived);
        // Verbatim: the declared depth survives untouched, although a
        // different one would have been measured for this tree.
        assert_eq!(run.config.modules.depth, 1);
        assert_ne!(run_depth(dir.path()), 1, "measured depth differs");
        // No derived duplicates: exactly the three declared entries.
        assert_eq!(names(&run.config.layers), ["domain", "app", "api"]);
        assert!(
            run.config
                .layers
                .iter()
                .all(|layer| layer.origin == LayerOrigin::Declared)
        );
    }

    #[test]
    fn a_declared_project_is_not_overridden() {
        let dir = layered_fixture();
        let config = Config {
            project: ProjectConfig {
                source_roots: vec!["docs".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };
        let run = effective_run(
            dir.path(),
            &loaded_with(config, &["project"]),
            Level::Module,
        )
        .expect("run");
        assert_eq!(run.report.config["project"], ConfigSource::Declared);
        assert_eq!(run.config.project.source_roots, ["docs"], "verbatim");
        assert_eq!(
            run.config.project.languages,
            Config::default().project.languages,
            "verbatim default"
        );
        // The declared roots steer node naming (stripping), the walk
        // itself covers the project root regardless (Kap. 6 step 3).
        assert!(!run.config.layers.is_empty(), "layers still derived");
        assert!(
            run.config
                .layers
                .iter()
                .all(|layer| layer.origin == LayerOrigin::Derived)
        );
    }

    #[test]
    fn a_declared_override_of_one_derived_name_yields_mixed() {
        let dir = layered_fixture();
        let config = Config {
            layers: vec![LayerConfig {
                name: "app".to_string(),
                contains: vec!["app/**".to_string()],
                may_depend_on: vec!["domain".to_string()],
                origin: LayerOrigin::Declared,
            }],
            ..Default::default()
        };
        let run = effective_run(dir.path(), &loaded_with(config, &["layers"]), Level::Module)
            .expect("run");
        assert_eq!(run.report.config["layers"], ConfigSource::Mixed);
        // Declared first, then the unmatched derived entries in their
        // bottom-up ranking order.
        assert_eq!(names(&run.config.layers), ["app", "domain", "api"]);
        let app = run
            .config
            .layers
            .iter()
            .find(|layer| layer.name == "app")
            .expect("app layer");
        assert_eq!(app.origin, LayerOrigin::Declared, "declared wins the name");
    }

    #[test]
    fn derive_layers_off_keeps_layers_default_and_emits_nothing() {
        let dir = layered_fixture();
        let config = Config {
            derive_layers: false,
            ..Default::default()
        };
        let run = effective_run(dir.path(), &loaded_with(config, &[]), Level::Module).expect("run");
        assert!(run.config.layers.is_empty());
        assert_eq!(run.report.config["layers"], ConfigSource::Default);
        assert!(
            run.report
                .findings
                .iter()
                .all(|finding| finding.kind != FindingKind::LayerUnranked)
        );
    }

    #[test]
    fn a_cycle_that_blocks_ranking_yields_one_info_hint() {
        // `app` ⇄ `api` form an SCC, `domain` ranks below them.
        let dir = fixture(&[
            (
                "src/shop/api/one.ts",
                "import { a } from \"../app/one\";\nexport const one = a + 1;\n",
            ),
            (
                "src/shop/app/one.ts",
                "import { d } from \"../domain/one\";\nimport { p } from \"../api/one\";\nexport const a = d;\nexport const b = p + 1;\n",
            ),
            ("src/shop/domain/one.ts", "export const d = 1;\n"),
        ]);
        let run = effective_run(
            dir.path(),
            &loaded_with(Config::default(), &[]),
            Level::Module,
        )
        .expect("run");
        let unranked: Vec<&Finding> = run
            .report
            .findings
            .iter()
            .filter(|finding| finding.kind == FindingKind::LayerUnranked)
            .collect();
        assert_eq!(unranked.len(), 1, "exactly one hint: {:?}", unranked);
        let hint = unranked[0];
        assert_eq!(hint.id, UNRANKED_FINDING_ID);
        assert_eq!(hint.severity, Severity::Info);
        assert_eq!(hint.nodes, ["shop/api", "shop/app"]);
        assert_eq!(hint.edge_count, 0);
        assert_eq!(hint.cycles_sampled, 0);
        assert!(hint.suggested_cuts.is_empty());
        // The hint alone never fails the run — only the cycle gate does;
        // with deny_cycles=module at module level the cycle is an Error,
        // so check that removing the gate leaves Pass with the hint.
        let mut config = Config::default();
        config.rules.deny_cycles = DenyCycles::Off;
        config.rules.max_scc_size = 100;
        let run = effective_run(dir.path(), &loaded_with(config, &[]), Level::Module).expect("run");
        assert_eq!(run.report.summary.status, Status::Pass);
        assert!(
            run.report
                .findings
                .iter()
                .any(|finding| finding.kind == FindingKind::LayerUnranked)
        );
    }

    #[test]
    fn unranked_hint_is_capped_and_sorted() {
        // Twelve `c…` files importing every `peer…` and every peer
        // importing its twin form ONE component; a separate `entry`
        // depends on that component, so exactly one derived layer stays
        // rankable and the hint must carry the first ten unranked names.
        let mut files: Vec<(String, String)> = Vec::new();
        for i in 0..12 {
            let content: String = (0..12)
                .map(|j| format!("import {{ m{j} }} from \"./peer{j}\";\n"))
                .collect();
            files.push((format!("src/c{i}.ts"), content));
        }
        for j in 0..12 {
            files.push((
                format!("src/peer{j}.ts"),
                format!("import {{ c{j} }} from \"./c{j}\";\nexport const m{j} = 1;\n"),
            ));
        }
        files.push((
            "src/entry.ts".to_string(),
            "import { c } from \"./c0\";\nexport const e = c;\n".to_string(),
        ));
        let dir = fixture(
            &files
                .iter()
                .map(|(p, c)| (p.as_str(), c.as_str()))
                .collect::<Vec<_>>(),
        );
        let run = effective_run(
            dir.path(),
            &loaded_with(Config::default(), &[]),
            Level::Module,
        )
        .expect("run");
        assert!(run.warnings.is_empty(), "warnings: {:?}", run.warnings);
        // Only `entry` ranks (the whole component is unrankable), so one
        // derived layer is active — the hint's precondition.
        assert_eq!(names(&run.config.layers), ["layer-1"]);
        let hint = run
            .report
            .findings
            .iter()
            .find(|finding| finding.kind == FindingKind::LayerUnranked)
            .expect("hint with a derived layer active");
        assert_eq!(hint.nodes.len(), MAX_UNRANKED_NODES);
        assert_eq!(
            hint.nodes,
            [
                "c0.ts", "c1.ts", "c10.ts", "c11.ts", "c2.ts", "c3.ts", "c4.ts", "c5.ts", "c6.ts",
                "c7.ts"
            ],
            "first ten of the sorted unranked names"
        );
    }

    #[test]
    fn effective_config_string_is_a_parsable_provenanced_file() {
        let dir = layered_fixture();
        let text = effective_config_string(dir.path());
        assert!(text.contains("# abgeleitet"), "text: {text}");
        assert!(text.contains("# Default"), "text: {text}");
        assert!(text.contains("# gemessen"), "text: {text}");
        assert!(text.contains("[[layers]]"), "text: {text}");
        for name in ["domain", "app", "api"] {
            assert!(text.contains(&format!("name = \"{name}\"")), "text: {text}");
        }
        assert!(text.contains("source_roots = [\"src\"]"), "text: {text}");
        assert!(text.contains("languages = [\"ts\"]"), "text: {text}");
        assert!(
            text.contains("# max_acd = 2.00"),
            "Lakos ACD proposal line: {text}"
        );
        // The rendered file really parses back into the effective config.
        let parsed: Config = toml::from_str(&text).expect("rendered toml parses");
        assert_eq!(parsed.project.source_roots, ["src"]);
        assert_eq!(parsed.project.languages, ["ts"]);
        assert_eq!(names(&parsed.layers), ["domain", "app", "api"]);
        assert_eq!(parsed.modules.depth, run_depth(dir.path()));
    }

    /// The depth `suggest_depth` measures for `dir` (mirrors the run).
    fn run_depth(dir: &std::path::Path) -> u32 {
        let files = pipeline::walk_files(dir, &Config::default()).expect("walk");
        let (graph, _) = pipeline::build_graph(dir, &files, &Config::default()).expect("build");
        suggest_depth(&graph, VIEW_MIN, VIEW_MAX)
    }

    #[test]
    fn effective_config_string_of_an_empty_dir_is_the_defaults() {
        let dir = tempfile::tempdir().expect("temp dir");
        let text = effective_config_string(dir.path());
        let parsed: Config = toml::from_str(&text).expect("parses");
        assert_eq!(parsed.project, Config::default().project);
        assert_eq!(parsed.rules, Config::default().rules);
        assert_eq!(parsed.store, Config::default().store);
        assert!(parsed.layers.is_empty());
        // Nothing to zoom into: the measured depth is the minimum.
        assert_eq!(parsed.modules.depth, 1);
        assert!(text.contains("languages = [\"auto\"]"));
    }

    #[test]
    fn toml_escaping_survives_backslashes_and_quotes() {
        assert_eq!(toml_str("a\\b\"c"), "\"a\\\\b\\\"c\"");
        assert_eq!(
            toml_list(&["x".to_string(), "y".to_string()]),
            "[\"x\", \"y\"]"
        );
    }
}
