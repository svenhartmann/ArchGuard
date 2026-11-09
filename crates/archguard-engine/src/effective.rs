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
    LayerConfig, LayerOrigin, ProjectConfig, Report, Severity, Status, aggregate,
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
    /// Node name per extracted file, for joining sources to `graph` nodes.
    pub source_names: pipeline::SourceNames,
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
/// External-import warnings go to stderr as in [`analyze`].
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

#[tracing::instrument(skip_all)]
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
    let (mut graph, names, mut warnings, cache_hit) =
        crate::cache::extract(root, &files, &config, cached)?;
    // The disk cache contains imports only. Compiler evidence is projected on
    // every run using its own source/project/toolchain fingerprints, so a warm
    // import cache cannot hide changed method bodies or recovered toolchains.
    crate::semantic_calls::integrate_graph_named(root, &files, &config, &mut graph, Some(&names))?;

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
    let (mut report, policy_warnings) =
        build_effective_report(root, (&graph, &names), &config, level)?;
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
        source_names: names,
    })
}

/// Analyse with already effective policy values, preserving declared/derived
/// layer origins. Shared with Git comparison, where derivation must stay frozen.
#[tracing::instrument(skip_all)]
pub(crate) fn build_effective_report<'a>(
    root: &Path,
    graph: impl Into<pipeline::NamedGraph<'a>>,
    config: &Config,
    level: Level,
) -> Result<(Report, Vec<String>), ArchguardError> {
    let named = graph.into();
    let graph = named.graph;
    let mut report = pipeline::build_report(graph, config, level)?;
    let mut warnings = Vec::new();
    if !config.ownership_rules.is_empty() {
        let ownership_sources = crate::review_sources::collect_file_sources(root, config, named)?;
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
#[tracing::instrument(skip_all)]
pub fn derive_project_hints(root: &Path) -> Result<(Vec<String>, Vec<String>), ArchguardError> {
    let files = walk(root, &ProjectConfig::default().exclude)?;

    let mut per_dir: BTreeMap<&str, usize> = BTreeMap::new();
    for file in &files {
        if let Some((dir, _)) = file.rel_path.split_once('/') {
            *per_dir.entry(dir).or_default() += 1;
        }
    }
    let mut roots = ranked_roots(&per_dir, files.len());

    // A directory with `__init__.py` is a Python package, not a source root:
    // the root is the parent of its outermost package (flat layout `mypkg/`
    // → `.`), so `import mypkg.b` keeps its package name and resolves.
    roots = roots
        .into_iter()
        .map(|dir| python_package_parent(root, dir))
        .collect();
    roots.sort();
    roots.dedup();

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
        && crate::project_files::read_manifest(root, Path::new("go.mod"))
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

/// `dir` itself, or — when it holds `__init__.py` — the parent of the
/// outermost enclosing Python package (`.` for a top-level package).
fn python_package_parent(root: &Path, mut dir: String) -> String {
    while dir != "." && root.join(&dir).join("__init__.py").is_file() {
        dir = match dir.rsplit_once('/') {
            Some((parent, _)) => parent.to_string(),
            None => ".".to_string(),
        };
    }
    dir
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

/// [`analyze`] with the graph cache: walk, fingerprint,
/// and on a fingerprint hit load `.archguard/cache.json` instead of
/// extracting and building — the analyses always rerun. On a miss the
/// full pipeline runs and the fresh graph is saved for the next run;
/// an unwritable or symlinked (unsafe) cache only warns, it never
/// fails the analysis.
pub fn analyze_cached(
    root: &Path,
    config: &Config,
    level: Level,
) -> Result<(Graph, Report), ArchguardError> {
    let (graph, report, _cache_hit) = analyze_cached_inner(root, config, level)?;
    Ok((graph, report))
}

/// [`analyze_cached`] additionally reporting whether the graph came
/// from the cache — the observable seam its tests assert on.
pub(crate) fn analyze_cached_inner(
    root: &Path,
    config: &Config,
    level: Level,
) -> Result<(Graph, Report, bool), ArchguardError> {
    let loaded = crate::config::LoadedConfig {
        config: config.clone(),
        sources: crate::config::all_default_sources(),
    };
    let run = crate::effective::analyze_run(root, &loaded, level)?;
    Ok((run.graph, run.report, run.cache_hit))
}

/// Run the complete pipeline over `root` with `config` **as the
/// declared config** at `level`: as nothing of it is marked as written
/// by the user, the effective-config derivation (T46, Kap. 6 steps 2
/// and 7) fills the `project`, `modules` and `layers` sections from
/// the project itself; `rules`, `store` and `derive_layers` are taken
/// from `config` verbatim. Callers holding a [`LoadedConfig`] with
/// real provenance call
/// [`analyze_effective`](crate::effective::analyze_effective) instead.
///
/// Unresolved (external) import targets are reported as warnings on
/// stderr — the [`FindingKind`] vocabulary of Kap. 5 has no external
/// kind, so they never appear as findings and never fail the run.
pub fn analyze(
    root: &Path,
    config: &Config,
    level: Level,
) -> Result<(Graph, Report), ArchguardError> {
    let (graph, report, warnings) = analyze_with_warnings(root, config, level)?;
    pipeline::print_warnings(&warnings);
    Ok((graph, report))
}

/// The effective pipeline body with an all-`Default` provenance
/// standing in for `config` (see [`analyze`]), additionally handing
/// back the collected external-import warnings (what [`analyze`]
/// prints). Crate-internal so tests can assert the warnings themselves.
pub(crate) fn analyze_with_warnings(
    root: &Path,
    config: &Config,
    level: Level,
) -> Result<(Graph, Report, Vec<String>), ArchguardError> {
    let loaded = crate::config::LoadedConfig {
        config: config.clone(),
        sources: crate::config::all_default_sources(),
    };
    let run = effective_run(root, &loaded, level)?;
    Ok((run.graph, run.report, run.warnings))
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
    fn python_package_dir_is_not_a_source_root() {
        // Flat layout: `mypkg` is the package, so its parent is the root.
        let dir = fixture(&[
            ("pyproject.toml", "[project]\nname = \"mypkg\"\n"),
            ("mypkg/__init__.py", ""),
            ("mypkg/a.py", "from mypkg import b\n"),
            ("mypkg/b.py", "from mypkg import a\n"),
        ]);
        let (roots, _) = derive_project_hints(dir.path()).expect("hints");
        assert_eq!(roots, ["."]);

        // src layout: `src` holds the package and stays the root.
        let dir = fixture(&[
            ("src/mypkg/__init__.py", ""),
            ("src/mypkg/a.py", "from mypkg import b\n"),
            ("src/mypkg/b.py", "from mypkg import a\n"),
        ]);
        let (roots, _) = derive_project_hints(dir.path()).expect("hints");
        assert_eq!(roots, ["src"]);
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

#[cfg(test)]
mod pipeline_tests {
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

    /// Names of the nodes of `graph` that `name` is an edge to from
    /// `from_name`, for asserting resolved edges by name.
    fn targets(graph: &Graph, from_name: &str) -> Vec<String> {
        let from = graph
            .nodes
            .iter()
            .find(|node| node.name == from_name)
            .unwrap_or_else(|| panic!("test: no node {from_name}"))
            .id;
        graph
            .out_edges(from)
            .map(|edge| {
                graph
                    .node_name(edge.to)
                    .expect("test: known id")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn external_imports_warn_and_never_produce_edges() {
        let dir = fixture(&[
            ("src/app/uses_lib.ts", "import { x } from \"react\";\n"),
            ("src/app/util.ts", "export const x = 1;\n"),
        ]);
        let (graph, report, warnings) =
            analyze_with_warnings(dir.path(), &Config::default(), Level::Module).expect("analyze");
        assert!(graph.edges.is_empty(), "unresolved imports create no edges");
        assert_eq!(report.summary.status, archguard_core::Status::Pass);
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("react") && warnings[0].contains("app/uses_lib.ts"),
            "warning: {}",
            warnings[0]
        );
    }

    #[test]
    fn path_spec_imports_edge_file_to_file() {
        let dir = fixture(&[
            ("src/app/main.ts", "import { helper } from \"./util\";\n"),
            ("src/app/util.ts", "export const helper = 1;\n"),
        ]);
        let (graph, report, warnings) =
            analyze_with_warnings(dir.path(), &Config::default(), Level::Module).expect("analyze");
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(targets(&graph, "app/main.ts"), ["app/util.ts"]);
        assert_eq!(report.summary.status, archguard_core::Status::Pass);
    }

    #[test]
    fn psr4_prefixes_make_namespace_imports_resolvable_without_declarations() {
        // `Home.php` has no `namespace` declaration of its own and
        // `User.php` none either: only the PSR-4 mapping of
        // `composer.json` connects `use App\Model\User;` to the file.
        let dir = fixture(&[
            (
                "composer.json",
                r#"{"autoload": {"psr-4": {"App\\": "src/"}}}"#,
            ),
            ("src/Controller/Home.php", "<?php\nuse App\\Model\\User;\n"),
            ("src/Model/User.php", "<?php\nclass User {}\n"),
        ]);
        let (graph, _report, warnings) =
            analyze_with_warnings(dir.path(), &Config::default(), Level::Module).expect("analyze");
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(
            targets(&graph, "Controller/Home.php"),
            ["Model/User.php"],
            "PSR-4 must resolve the use-statement to the class file"
        );
    }

    #[test]
    fn symbolic_import_without_target_warns_and_pseudo_nodes_edge() {
        // Java: a package nothing declares → warning; the declared
        // package's unknown class → pseudo node `<module>/*`.
        let dir = fixture(&[
            (
                "src/com/example/App.java",
                "package com.example;\nimport com.example.Order;\nimport com.unknown.Thing;\n",
            ),
            (
                "src/com/example/Order.java",
                "package com.example;\nclass Order {}\n",
            ),
        ]);
        let (graph, _report, warnings) =
            analyze_with_warnings(dir.path(), &Config::default(), Level::Module).expect("analyze");
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert!(
            warnings[0].contains("com/unknown/Thing"),
            "warning: {warnings:?}"
        );
        // App.java edges to the uniquely matched Order.java; the
        // unresolved import left no edge.
        assert_eq!(
            targets(&graph, "com/example/App.java"),
            ["com/example/Order.java"]
        );
    }
    #[test]
    fn python_symbols_resolve_to_their_module_and_packages_keep_init_files() {
        let dir = fixture(&[
            (
                "src/main.py",
                "import helper; from\thelper import Thing\nfrom pkg import Exported; import helper.missing\n",
            ),
            ("src/helper.py", "class Thing: pass\n"),
            ("src/pkg/__init__.py", "Exported = 1\n"),
            (
                "src/pkg/child.py",
                "from . import Exported\nfrom .model import Model\n",
            ),
            ("src/pkg/model.py", "class Model: pass\n"),
        ]);
        let (g, _, warnings) =
            analyze_with_warnings(dir.path(), &Config::default(), Level::File).unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("helper.missing"));
        assert_eq!(targets(&g, "main.py"), ["helper.py", "pkg/__init__.py"]);
        assert_eq!(
            targets(&g, "pkg/child.py"),
            ["pkg/__init__.py", "pkg/model.py"]
        );
    }
    #[test]
    fn module_resolution_does_not_cross_language_boundaries_and_js_uses_index_files() {
        let dir = fixture(&[
            ("src/main.ts", "import './only-rust'; import './folder';"),
            ("src/only-rust.rs", "pub struct Rust;"),
            ("src/folder/index.js", "export const a=1;"),
            ("src/Consumer.java", "package p; import foreign.Type;"),
            ("src/Type.cs", "namespace foreign; public class Type {}"),
        ]);
        let (g, _, warnings) =
            analyze_with_warnings(dir.path(), &Config::default(), Level::File).unwrap();
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(targets(&g, "main.ts"), ["folder/index.js"]);
        assert!(targets(&g, "p/Consumer.java").is_empty());
    }
    #[test]
    fn java_nested_type_imports_resolve_to_the_enclosing_type_file() {
        let dir = fixture(&[
            (
                "src/Outer.java",
                "package domain; public class Outer {public static class Inner {}}",
            ),
            (
                "src/Client.java",
                "package client; import domain.Outer.Inner;",
            ),
        ]);
        let (g, _, warnings) =
            analyze_with_warnings(dir.path(), &Config::default(), Level::File).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(targets(&g, "client/Client.java"), ["domain/Outer.java"]);
    }
}

#[cfg(test)]
mod cache_entry_tests {
    use super::*;
    use crate::cache::{
        CACHE_FILE, CacheEntry, extraction_fingerprint, load_graph_if_fresh, save_graph,
    };
    use crate::pipeline::Level;
    use archguard_core::Status;
    use std::fs::File;
    use std::time::SystemTime;

    /// Write `files` (path, content) into a fresh temp dir, mirroring
    /// the T22 integration fixture: a → b → c → a cycle under `src`.
    fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        for (rel, content) in files {
            let path = dir.path().join(rel);
            fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
            fs::write(path, content).expect("write file");
        }
        dir
    }

    fn cycle_fixture() -> tempfile::TempDir {
        fixture(&[
            (
                "src/a.ts",
                "import { b } from \"./b\";\nexport const a = b + 1;\n",
            ),
            (
                "src/b.ts",
                "import { c } from \"./c\";\nexport const b = c + 1;\n",
            ),
            (
                "src/c.ts",
                "import { a } from \"./a\";\nexport const c = a + 1;\n",
            ),
        ])
    }

    #[test]
    fn second_run_is_served_from_the_cache() {
        let dir = cycle_fixture();
        let (graph1, report1, hit1) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 1");
        assert!(!hit1, "first run must build, not hit");

        // The single cache entry exists and carries a fingerprint that
        // matches a freshly computed one.
        let cache = dir.path().join(".archguard");
        assert!(cache.join(CACHE_FILE).is_file(), "cache entry missing");
        let raw = fs::read_to_string(cache.join(CACHE_FILE)).expect("cache entry file");
        let entry: CacheEntry = serde_json::from_str(&raw).expect("cache entry parses");
        assert_eq!(
            entry.fingerprint,
            current_fingerprint(dir.path()),
            "stored fingerprint mismatch"
        );

        let (graph2, report2, hit2) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 2");
        assert!(hit2, "second run must be served from the cache");
        assert_eq!(graph1, graph2);
        assert_eq!(report1, report2);
        assert_eq!(
            report2.summary.status,
            Status::Fail,
            "the cycle is still found"
        );
    }

    #[test]
    fn changed_mtime_and_length_miss_the_cache() {
        let dir = cycle_fixture();
        let (_, _, hit1) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 1");
        assert!(!hit1);

        // Modify one fixture file: different content (→ different len)
        // and an explicitly pinned mtime (→ different mtime), so the
        // fingerprint changes no matter the filesystem's time granularity.
        let path = dir.path().join("src/b.ts");
        fs::write(
            &path,
            "import { c } from \"./c\";\nexport const b = c + 1000;\n",
        )
        .expect("rewrite");
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        File::options()
            .write(true)
            .open(&path)
            .expect("open")
            .set_modified(time)
            .expect("set mtime");

        let (graph, report, hit) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 2");
        assert!(!hit, "changed mtime+length must invalidate the cache");
        assert_eq!(graph.edges.len(), 3, "graph rebuilt correctly");
        assert_eq!(report.summary.sccs, 1);
        assert_eq!(report.summary.status, Status::Fail);

        // A third, unmodified run hits again.
        let (_, _, hit3) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 3");
        assert!(hit3, "fresh cache after rebuild");
    }

    /// The freshly computed fingerprint of everything currently under
    /// `dir` with the default config.
    fn current_fingerprint(dir: &Path) -> String {
        let files = pipeline::walk_files(dir, &Config::default()).expect("walk");
        let mut config = Config::default();
        (config.project.source_roots, config.project.languages) =
            crate::effective::derive_project_hints(dir).unwrap();
        extraction_fingerprint(dir, &files, &config).unwrap()
    }

    /// A graph claiming "no cycle" — anything a poisoned cache could
    /// serve to flip the analysis verdict.
    fn foreign_graph() -> Graph {
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "foreign".to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    // F03 regression: the cache lives under the repository-controlled
    // root, so every component of its path is attacker-controlled and
    // may be swapped for a symlink at any moment.

    #[cfg(unix)]
    #[test]
    fn a_symlinked_cache_directory_cannot_redirect_writes_outside_the_root() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().expect("outside dir");
        let sentinel = outside.path().join("sentinel");
        fs::write(&sentinel, b"do not touch").expect("sentinel");

        let dir = cycle_fixture();
        symlink(outside.path(), dir.path().join(".archguard")).expect("symlink cache dir");

        // The cache is an optional optimisation: analysis must stay
        // correct even though the cache cannot be used.
        let (graph, report, hit) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module)
                .expect("analysis survives an unsafe cache directory");
        assert!(!hit, "an unsafe cache directory is never served");
        assert_eq!(graph.edges.len(), 3, "graph built from the sources");
        assert_eq!(report.summary.status, Status::Fail);

        // Nothing was created or altered through the symlink.
        assert_eq!(
            fs::read(&sentinel).expect("sentinel readable"),
            b"do not touch",
            "external sentinel untouched"
        );
        for name in ["graph.json", "fingerprint", "cache.json"] {
            assert!(
                !outside.path().join(name).exists(),
                "{name} must not appear outside the root"
            );
        }

        let (_, _, hit2) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 2");
        assert!(!hit2, "an unsafe cache directory stays untrusted");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_cache_files_cannot_overwrite_external_sentinels() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().expect("outside dir");
        let victims: Vec<std::path::PathBuf> = [
            "victim-graph.json",
            "victim-fingerprint",
            "victim-cache.json",
        ]
        .iter()
        .map(|name| {
            let path = outside.path().join(name);
            fs::write(&path, b"ORIGINAL").expect("write victim");
            path
        })
        .collect();

        let dir = cycle_fixture();
        let cache = dir.path().join(".archguard");
        fs::create_dir_all(&cache).expect("cache dir");
        symlink(&victims[0], cache.join("graph.json")).expect("symlink graph");
        symlink(&victims[1], cache.join("fingerprint")).expect("symlink fingerprint");
        symlink(&victims[2], cache.join("cache.json")).expect("symlink cache entry");

        let (graph, _, hit) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 1");
        assert!(!hit, "symlinked cache files are never served");
        assert_eq!(graph.edges.len(), 3);

        for victim in &victims {
            assert_eq!(
                fs::read(victim).expect("victim readable"),
                b"ORIGINAL",
                "{} untouched",
                victim.display()
            );
        }

        // The cache works again once the entry is a real regular file
        // (replaced as a symlink, never written through one).
        let entry = fs::symlink_metadata(cache.join("cache.json")).expect("cache entry");
        assert!(
            entry.is_file(),
            "cache entry is a regular file, not a symlink"
        );
        let (_, _, hit2) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 2");
        assert!(hit2, "the regular cache entry is served");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_cache_entry_serves_no_foreign_graph() {
        use std::os::unix::fs::symlink;

        let dir = cycle_fixture();
        analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("warm");
        let fp = current_fingerprint(dir.path());

        // A valid-looking cache document (right fingerprint, cycle-free
        // graph) placed outside the root, reached through a symlink.
        let outside = tempfile::tempdir().expect("outside dir");
        let poison = outside.path().join("poison.json");
        let poison_bytes = format!(
            "{{\"fingerprint\":\"{fp}\",\"graph\":{}}}",
            serde_json::to_string(&foreign_graph()).expect("json")
        );
        fs::write(&poison, &poison_bytes).expect("write poison");
        let _ = fs::remove_file(dir.path().join(".archguard").join("cache.json"));
        symlink(&poison, dir.path().join(".archguard").join("cache.json")).expect("symlink");

        let (graph, report, hit) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run");
        assert!(!hit, "a symlinked cache entry is not trusted");
        assert_eq!(graph.edges.len(), 3, "graph rebuilt, not poisoned");
        assert_eq!(report.summary.status, Status::Fail, "no verdict flip");
        assert_eq!(
            fs::read(&poison).expect("poison readable"),
            poison_bytes.as_bytes(),
            "poison file untouched"
        );
    }

    #[test]
    fn a_stale_or_hand_written_pair_is_never_trusted_as_a_mixed_generation() {
        let dir = cycle_fixture();
        let (graph_a, _, hit1) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("run 1");
        assert!(!hit1);
        let fp_a = current_fingerprint(dir.path());
        let files = pipeline::walk_files(dir.path(), &Config::default()).unwrap();
        let (imports_a, _) = pipeline::build_graph(dir.path(), &files, &Config::default()).unwrap();
        assert!(
            imports_a.semantic.is_none(),
            "disk cache contains import extraction only"
        );
        assert!(
            graph_a.semantic.is_some(),
            "analysis attaches fresh compiler coverage"
        );
        assert_eq!(graph_a.nodes, imports_a.nodes);
        assert_eq!(
            graph_a.edges, imports_a.edges,
            "TS fixture has import evidence only"
        );
        assert_eq!(
            load_graph_if_fresh(dir.path(), &fp_a).as_ref(),
            Some(&imports_a),
            "the saved generation is served for its own fingerprint"
        );

        // Simulate a crash between the (formerly two) cache writes: the
        // graph file replaced while the fingerprint is still the old
        // one. The store must never serve the foreign graph under the
        // old fingerprint.
        let foreign = foreign_graph();
        fs::write(
            dir.path().join(".archguard").join("graph.json"),
            serde_json::to_vec(&foreign).expect("json"),
        )
        .expect("plant foreign graph file");
        let served = load_graph_if_fresh(dir.path(), &fp_a);
        assert_ne!(
            served.as_ref(),
            Some(&foreign),
            "an old fingerprint must never pair with a foreign graph"
        );

        // A save for another fingerprint is visible all-or-nothing.
        save_graph(dir.path(), &foreign, "fp-B").expect("save B");
        assert_eq!(
            load_graph_if_fresh(dir.path(), "fp-B").as_ref(),
            Some(&foreign)
        );
        assert!(
            load_graph_if_fresh(dir.path(), &fp_a).is_none(),
            "generation A is fully replaced"
        );

        // A torn entry (interrupted write, corruption) is a miss, never
        // a panic, and recovers by rebuilding.
        fs::write(
            dir.path().join(".archguard").join("cache.json"),
            b"{\"fingerprint\":\"fp-B\",\"graph\":{\"nod",
        )
        .expect("torn entry");
        assert!(
            load_graph_if_fresh(dir.path(), "fp-B").is_none(),
            "torn cache is a miss"
        );
        let (graph_c, _, hit_c) =
            analyze_cached_inner(dir.path(), &Config::default(), Level::Module).expect("rebuild");
        assert!(!hit_c);
        assert_eq!(graph_c, graph_a, "rebuilt from sources, verdict intact");
    }

    #[test]
    fn runs_carry_source_names_and_consumers_reuse_them() {
        use crate::pipeline::{extractions, source_node_name};
        // A declared package names the Java node apart from its path.
        let dir = fixture(&[
            (
                "src/weird/place/Order.java",
                "package shop;\npublic class Order {}\n",
            ),
            (
                "src/a.ts",
                "import { b } from \"./b\";\nexport const a = b;\n",
            ),
            ("src/b.ts", "export const b = 1;\n"),
            ("src/notes.txt", "not a source\n"),
        ]);
        let loaded = LoadedConfig {
            config: Config::default(),
            sources: crate::config::all_default_sources(),
        };
        let cold = analyze_run_quiet(dir.path(), &loaded, Level::Module).expect("cold run");
        assert!(!cold.cache_hit);
        let files = pipeline::walk_files(dir.path(), &cold.config).expect("walk");
        let roots = &cold.config.project.source_roots;
        let extracted: pipeline::SourceNames = files
            .iter()
            .filter_map(|file| {
                let stripped = strip_root(&file.rel_path, roots);
                let name = source_node_name(file, &stripped, None, None).expect("name")?;
                Some((file.rel_path.clone(), name))
            })
            .collect();
        assert_eq!(cold.source_names, extracted);
        assert_eq!(
            cold.source_names["src/weird/place/Order.java"],
            "shop/Order.java"
        );

        let before = extractions::count();
        let warm = analyze_run_quiet(dir.path(), &loaded, Level::Module).expect("warm run");
        assert!(warm.cache_hit);
        assert_eq!(extractions::count(), before, "a cache hit extracts nothing");
        assert_eq!(warm.source_names, cold.source_names);

        let since = &warm.config.review.git_since;
        let before = extractions::count();
        let plain = crate::review_sources::collect_review_sources(
            dir.path(),
            &warm.config,
            &warm.graph,
            since,
        )
        .expect("plain");
        assert_eq!(
            extractions::count() - before,
            3,
            "without names every source is extracted"
        );
        let named_graph = (&warm.graph, &warm.source_names);
        let before = extractions::count();
        let named = crate::review_sources::collect_review_sources(
            dir.path(),
            &warm.config,
            named_graph,
            since,
        )
        .expect("named");
        assert_eq!(
            extractions::count(),
            before,
            "with names no source is extracted"
        );
        assert_eq!(named, plain);
        assert_eq!(named.mapped_files, 3);
    }
}
