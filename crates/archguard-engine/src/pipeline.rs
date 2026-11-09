//! The full analysis pipeline (T22, ARCHITECTURE.md Kap. 6):
//! walk → extract → resolve → build → view → analyse → report.
//!
//! The derivation of the effective config (Kap. 6 steps 2 and 7)
//! lives in [`effective`](crate::effective); [`analyze`](crate::effective::analyze) feeds it a
//! plain config whose sections all count as undesclared, so they get
//! derived. What this module does, in Kap. 6 order:
//!
//! * **Walk** (step 3): [`archguard_extract::walk`] with the configured
//!   exclude globs.
//! * **Extract** (step 4): every file is read and scanned by the
//!   extractor registered for its language, against its *stripped*
//!   path (the source-root prefix removed by [`strip_root`]) so all
//!   downstream names are source-root-relative.
//! * **Resolve** (step 5): `TargetSpec::Path` specs via
//!   [`resolve_relative`] against the file tree; `TargetSpec::Symbol`
//!   specs via the [`DeclMap`] built from the real declarations, the
//!   path-derived entries and the PSR-4 prefixes of a `composer.json`
//!   in the project root (when present). Unique file → file edge,
//!   module-only → `<module>/*` pseudo node, unknown → **warning**
//!   (emitted on stderr by `analyze`, no edge).
//! * **Build** (step 6): one file node per source file — named by its
//!   [`Declaration`] module path plus file name when it declares one,
//!   else by its stripped path — plus the edges, frozen by
//!   [`GraphBuilder::build`].
//! * **View** (step 8): `Cut::at_depth(modules.depth)` (or every file
//!   node itself at [`Level::File`]) projected through
//!   [`archguard_core::aggregate`] — the view is itself a [`Graph`].
//! * **Analyse + report** (steps 9–10): SCCs → sampled cycles (caps
//!   100 cycles / 20 length) → cut suggestions (max 5 per SCC) →
//!   layer check → metrics → summary. Cycle findings are [`Severity`]
//!   `Error` exactly when `rules.deny_cycles` names the analysed
//!   [`Level`], `Warning` otherwise.

#[path = "scanned_file.rs"]
mod scanned_file;
use scanned_file::ScannedFile;
#[path = "rust_imports.rs"]
mod rust_imports;
pub(crate) use rust_imports::{CrateItems, ItemTarget, hash_manifests};

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use archguard_analysis::{
    check_layers, compute_metrics, find_sccs, sample_cycles_bounded, suggest_cuts, summarize,
};
use archguard_core::{
    ArchguardError, Config, ConfigSource, Cut, DenyCycles, Finding, FindingKind, Graph,
    GraphBuilder, NodeId, Ref, Report, Severity, aggregate,
};
use archguard_extract::{
    DeclMap, Declaration, Lang, RawImport, SourceFile, SymbolTarget, TargetSpec, extractor_for,
    parse_psr4, resolve_relative, strip_root, walk,
};

/// `schema_version` of the emitted [`Report`] (Kap. 5).
const REPORT_SCHEMA_VERSION: u32 = 1;
/// Cycle sampling bound per SCC (Kap. 7: `max_cycles_per_scc`).
pub(crate) const MAX_CYCLES_PER_SCC: usize = 100;
/// Maximum cycle length in nodes (Kap. 7: `max_cycle_len`).
pub(crate) const MAX_CYCLE_LEN: usize = 20;
/// Cut suggestions per SCC finding.
const MAX_CUTS_PER_SCC: usize = 5;
/// Upper bound of external-import warnings echoed to stderr per run.
const MAX_PRINTED_WARNINGS: usize = 20;
/// The [`Report`] config sections whose provenance `analyze` reports.
pub(crate) const PROVENANCE_KEYS: [&str; 11] = [
    "project",
    "modules",
    "layers",
    "rules",
    "review",
    "performance",
    "architecture",
    "semantic",
    "dependency_rules",
    "ownership_rules",
    "store",
];

/// The granularity [`analyze`](crate::effective::analyze) reports at (CLI/MCP `--level`, Kap. 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// The default view: modules named by `modules.depth` (Kap. 5.2).
    Module,
    /// Every source file is a node of its own (raw tree, `--level file`).
    File,
}

/// Echo the collected external-import warnings to stderr the way
/// [`analyze`] does (bounded, with an overflow note).
pub(crate) fn print_warnings(warnings: &[String]) {
    for warning in warnings.iter().take(MAX_PRINTED_WARNINGS) {
        eprintln!("warning: {warning}");
    }
    if let Some(overflow) = warnings.len().checked_sub(MAX_PRINTED_WARNINGS) {
        eprintln!("warning: ...and {overflow} more unresolved import(s).");
    }
}

/// Kap. 6 step 3 alone: the walk with the configured exclude globs.
/// Crate-internal so the graph cache (T23) can fingerprint exactly the
/// file set [`build_graph`] would extract.
pub(crate) fn walk_files(root: &Path, config: &Config) -> Result<Vec<SourceFile>, ArchguardError> {
    let mut files = walk(root, &config.project.exclude)?;
    let languages = &config.project.languages;
    if !languages.is_empty() && !languages.iter().any(|language| language == "auto") {
        files.retain(|file| languages.iter().any(|l| l == lang_name(file.lang)));
    }
    Ok(files)
}

/// Kap. 6 steps 4–6: extract, resolve and build over an already
/// walked file set; hands back the graph plus the collected
/// external-import warnings.
pub(crate) fn build_graph(
    root: &Path,
    files: &[SourceFile],
    config: &Config,
) -> Result<(Graph, Vec<String>), ArchguardError> {
    // ---- Kap. 6 step 4: extract every file against its stripped path.
    let mut scanned: Vec<ScannedFile> = Vec::new();
    for file in files {
        let Some(extractor) = extractor_for(file.lang) else {
            continue; // a language walk accepted but no scanner registered
        };
        let content = String::from_utf8_lossy(&fs::read(&file.path)?).into_owned();
        let stripped = strip_root(&file.rel_path, &config.project.source_roots);
        let extraction = extractor.extract(&stripped, &content);
        let node_name = file_node_name(&stripped, &extraction.declarations);
        scanned.push(ScannedFile {
            rel_path: file.rel_path.clone(),
            stripped,
            node_name,
            lang: lang_name(file.lang),
            extraction,
        });
    }

    // ---- Kap. 6 step 5 preparation: the declaration map of the whole
    // run — real declarations, path-derived entries and PSR-4 prefixes.
    let mut declarations: Vec<Declaration> = scanned
        .iter()
        .flat_map(|file| file.extraction.declarations.clone())
        .collect();
    declarations.extend(psr4_declarations(root, &scanned)?);
    // A Java import cannot bind a C# file merely because its namespace matches.
    // JavaScript and TypeScript intentionally share a module namespace.
    let mut indexes = BTreeMap::new();
    for family in scanned
        .iter()
        .map(|file| import_family(file.lang))
        .collect::<BTreeSet<_>>()
    {
        let paths: Vec<_> = scanned
            .iter()
            .filter(|file| import_family(file.lang) == family)
            .map(|file| file.stripped.clone())
            .collect();
        indexes.insert(family, ImportIndex::new(&paths, &declarations));
    }

    // ---- Kap. 6 step 6: one file node (plus ancestor modules) per file.
    let mut builder = GraphBuilder::new();
    let mut node_of: HashMap<String, NodeId> = HashMap::new();
    for file in &scanned {
        let id = builder.ensure_path(&file.node_name, Some(file.lang));
        node_of.insert(file.stripped.clone(), id);
    }

    // Configured roots can strip two paths to one name; such files share a node.
    let mut first_path: BTreeMap<&str, &str> = BTreeMap::new();
    let mut merged = Vec::new();
    for file in &scanned {
        if let Some(previous) = first_path.insert(&file.stripped, &file.rel_path) {
            merged.push(format!(
                "project.source_roots map {previous} and {} to the same node {}; their dependencies are merged",
                file.rel_path, file.stripped
            ));
        }
    }

    // ---- Kap. 6 step 5: resolve every import, record its edge or warn.
    let mut go_packages: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in scanned.iter().filter(|file| file.lang == "go") {
        go_packages
            .entry(crate::go_sources::package_path(root, &file.rel_path))
            .or_default()
            .insert(parent_dir(&file.stripped).to_string());
    }
    let (rust_edges, mut warnings) = rust_imports::edges(root, &scanned)?;
    warnings.extend(merged);
    for (from, to, evidence) in rust_edges {
        builder.edge(node_of[&from], node_of[&to], evidence)?;
    }
    for file in &scanned {
        if file.lang == "rs" {
            continue;
        }
        let from = node_of[&file.stripped];
        let index = &indexes[import_family(file.lang)];
        for raw_import in &file.extraction.imports {
            let resolved = if file.lang == "go" {
                match &raw_import.target {
                    TargetSpec::Symbol { path, .. } => go_packages
                        .get(path)
                        .filter(|dirs| dirs.len() == 1)
                        .map(|dirs| builder.ensure_pseudo(dirs.first().unwrap())),
                    _ => None,
                }
            } else if file.lang == "py" {
                resolve_python(raw_import, &index.dirs, &node_of, &mut builder)
            } else {
                resolve_target(
                    raw_import,
                    &index.declarations,
                    &index.stems,
                    &index.dirs,
                    &node_of,
                    &mut builder,
                    file.lang,
                )
            };
            let Some(to) = resolved else {
                // A dotted name in code may be a member access, not a dependency.
                if !matches!(raw_import.target, TargetSpec::Qualified(_)) {
                    warnings.push(external_warning(raw_import));
                }
                continue;
            };
            builder.edge(
                from,
                to,
                Ref {
                    file: file.stripped.clone(),
                    line: raw_import.line,
                    raw: raw_import.raw.clone(),
                },
            )?;
        }
    }
    let full = builder.build(root.to_string_lossy().into_owned());
    Ok((full, warnings))
}

/// Kap. 6 steps 8–10 over a complete graph: default view, analyses
/// (SCCs, cycles, cuts, layers, metrics) and the [`Report`]. Cheap
/// enough to rerun on every cached run (T23).
pub(crate) fn build_report(
    full: &Graph,
    config: &Config,
    level: Level,
) -> Result<Report, ArchguardError> {
    // ---- Kap. 6 step 8: the default view, itself a Graph.
    let cut = match level {
        Level::Module => Cut::at_depth(full, config.modules.depth as usize),
        // No module is that deep: every file node is its own visible
        // ancestor-or-self, exactly the `--level file` view.
        Level::File => Cut::at_depth(full, usize::MAX),
    };
    let view = aggregate(full, &cut);

    // ---- Kap. 6 step 9: analyse the view.
    let sccs = find_sccs(&view);
    let severity = if deny_matches(config.rules.deny_cycles, level) {
        Severity::Error
    } else {
        Severity::Warning
    };
    let mut findings = cycle_findings(&view, &sccs, severity, "");
    // Display zoom cannot turn the configured cycle gate off. Keep the
    // display findings as warnings and add a separately identified gate view.
    let gate_level = match config.rules.deny_cycles {
        DenyCycles::Module => Some(Level::Module),
        DenyCycles::File => Some(Level::File),
        DenyCycles::Off => None,
    };
    if let Some(gate_level) = gate_level.filter(|gate| *gate != level) {
        let depth = match gate_level {
            Level::Module => config.modules.depth as usize,
            Level::File => usize::MAX,
        };
        let gate_view = aggregate(full, &Cut::at_depth(full, depth));
        let prefix = if gate_level == Level::File {
            "file-"
        } else {
            "module-"
        };
        findings.extend(cycle_findings(
            &gate_view,
            &find_sccs(&gate_view),
            Severity::Error,
            prefix,
        ));
    }
    findings.extend(check_layers(&view, &config.layers)?);
    findings.extend(archguard_analysis::check_dependency_rules(
        full,
        &config.dependency_rules,
    )?);
    findings.extend(archguard_analysis::check_orphans(full, &config.review)?);
    findings.extend(crate::target_architecture::check_target_architecture(
        full, config,
    )?);
    findings.extend(metric_findings(full, config)?);
    findings.sort_by(|a, b| a.severity.cmp(&b.severity).then_with(|| a.id.cmp(&b.id)));

    let metrics = compute_metrics(&view, &sccs);
    let summary = summarize(&view, &sccs, &findings);
    Ok(Report {
        semantic: full.semantic.clone(),
        review_kpis: full.review_kpis.clone(),
        schema_version: REPORT_SCHEMA_VERSION,
        summary,
        config: PROVENANCE_KEYS
            .iter()
            .map(|key| ((*key).to_string(), ConfigSource::Default))
            .collect(),
        findings,
        metrics,
    })
}

/// Metric gates always use the configured module view; SCC size follows
/// the file policy when enabled. Baselines only accept Cycle findings.
fn metric_findings(full: &Graph, config: &Config) -> Result<Vec<Finding>, ArchguardError> {
    let view = aggregate(full, &Cut::at_depth(full, config.modules.depth as usize));
    let file_view;
    let scc_view = if config.rules.deny_cycles == DenyCycles::File {
        file_view = aggregate(full, &Cut::at_depth(full, usize::MAX));
        &file_view
    } else {
        &view
    };
    let mut findings = Vec::new();
    let mut emit = |id: String, nodes: Vec<String>, edges: u32| {
        findings.push(Finding {
            message: None,
            evidence: Vec::new(),
            kind: FindingKind::MetricThreshold,
            id,
            severity: Severity::Error,
            nodes,
            edge_count: edges,
            cycles_sampled: 0,
            sampling: None,
            suggested_cuts: Vec::new(),
        })
    };
    for scc in find_sccs(scc_view) {
        if scc.node_ids.len() > config.rules.max_scc_size as usize {
            emit(
                format!("metric-max-scc-size-{}", scc.id),
                scc.node_ids
                    .iter()
                    .filter_map(|id| scc_view.node_name(*id).map(str::to_owned))
                    .collect(),
                scc.edges.len() as u32,
            );
        }
    }
    let (_, acd, _) = archguard_analysis::lakos_metrics(&view);
    if config.rules.max_acd > 0.0 && acd > config.rules.max_acd as f64 {
        emit(
            "metric-max-acd".into(),
            view.nodes.iter().map(|node| node.name.clone()).collect(),
            view.edges.len() as u32,
        );
    }
    if config.rules.max_instability_core > 0.0 {
        let mut sets = Vec::new();
        for layer in &config.layers {
            let mut builder = globset::GlobSetBuilder::new();
            for pattern in &layer.contains {
                builder.add(
                    globset::Glob::new(pattern)
                        .map_err(|err| ArchguardError::Config(err.to_string()))?,
                );
            }
            sets.push(
                builder
                    .build()
                    .map_err(|err| ArchguardError::Config(err.to_string()))?,
            );
        }
        let metrics = compute_metrics(&view, &find_sccs(&view));
        for (name, metric) in metrics {
            let Some(index) = sets.iter().position(|set| set.is_match(&name)) else {
                continue;
            };
            let layer = &config.layers[index];
            if layer.origin == archguard_core::LayerOrigin::Declared
                && layer.may_depend_on.is_empty()
                && metric.instability > config.rules.max_instability_core
            {
                emit(
                    format!("metric-max-instability-core-{name}"),
                    vec![name],
                    metric.fan_out,
                );
            }
        }
    }
    Ok(findings)
}

fn cycle_findings(
    view: &Graph,
    sccs: &[archguard_analysis::Scc],
    severity: Severity,
    prefix: &str,
) -> Vec<Finding> {
    sccs.iter()
        .map(|scc| {
            let sample = sample_cycles_bounded(
                view,
                scc,
                MAX_CYCLES_PER_SCC,
                MAX_CYCLE_LEN,
                archguard_analysis::DEFAULT_EDGE_BUDGET,
            );
            let cycles = sample.cycles;
            let mut suggested_cuts = suggest_cuts(view, scc, &cycles, MAX_CUTS_PER_SCC);
            for cut in &mut suggested_cuts {
                cut.reason
                    .push_str("; root-limited sample, not an exhaustive cycle search");
            }
            Finding {
                message: None,
                evidence: Vec::new(),
                kind: FindingKind::Cycle,
                id: format!("{prefix}{}", scc.id),
                severity,
                nodes: scc
                    .node_ids
                    .iter()
                    .filter_map(|&id| view.node_name(id).map(str::to_string))
                    .collect(),
                edge_count: scc.edges.len() as u32,
                cycles_sampled: cycles.len() as u32,
                sampling: Some(sample.sampling),
                suggested_cuts,
            }
        })
        .collect()
}

/// The graph node name of a scanned file: its declared module path plus
/// the file name when the file declares a module, else its stripped
/// path (Kap. 6.1: logical hierarchies hang under their declaration,
/// physical ones under their directory).
pub(crate) fn file_node_name(stripped: &str, declarations: &[Declaration]) -> String {
    let file_name = stripped.rsplit('/').next().unwrap_or(stripped);
    match declarations
        .first()
        .map(|declaration| declaration.module_path.as_str())
        .filter(|module| !module.is_empty())
    {
        Some(module) => format!("{module}/{file_name}"),
        None => stripped.to_string(),
    }
}

/// Whether `deny_cycles` forbids cycles *at* `level` (Kap. 11):
/// `"module"` forbids module cycles, `"file"` file cycles — each one
/// makes findings of its own level gate failures; `"off"` never does.
fn deny_matches(deny: DenyCycles, level: Level) -> bool {
    matches!(
        (deny, level),
        (DenyCycles::Module, Level::Module) | (DenyCycles::File, Level::File)
    )
}

/// The [`Lang`] id written to the `lang` field of file nodes; also the
/// derived `[project].languages` entry for that language (T46).
pub(crate) fn lang_name(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust => "rs",
        Lang::Ts => "ts",
        Lang::Js => "js",
        Lang::Py => "py",
        Lang::Java => "java",
        Lang::CSharp => "cs",
        Lang::Php => "php",
        Lang::Go => "go",
        Lang::Kotlin => "kt",
    }
}

struct ImportIndex {
    declarations: DeclMap,
    stems: BTreeMap<String, Vec<String>>,
    dirs: BTreeSet<String>,
}
impl ImportIndex {
    /// The index of one import family over its stripped `paths`, binding
    /// only the `declarations` made in those files.
    fn new(paths: &[String], declarations: &[Declaration]) -> Self {
        let path_set: HashSet<&str> = paths.iter().map(String::as_str).collect();
        let local: Vec<_> = declarations
            .iter()
            .filter(|d| path_set.contains(d.file.as_str()))
            .cloned()
            .collect();
        let mut stems: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut dirs = BTreeSet::new();
        for path in paths {
            stems.entry(stem_path(path)).or_default().push(path.clone());
            let mut dir = parent_dir(path);
            loop {
                dirs.insert(dir.to_owned());
                if dir.is_empty() {
                    break;
                }
                dir = parent_dir(dir);
            }
        }
        ImportIndex {
            declarations: DeclMap::new(&local, paths),
            stems,
            dirs,
        }
    }
}
fn import_family(lang: &str) -> &str {
    match lang {
        "ts" => "js",
        // Kotlin and Java share the JVM package namespace and import each other.
        "kt" => "java",
        _ => lang,
    }
}
fn resolve_python(
    raw: &RawImport,
    dirs: &BTreeSet<String>,
    nodes: &HashMap<String, NodeId>,
    builder: &mut GraphBuilder,
) -> Option<NodeId> {
    let module_file = |path: &str| {
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        nodes
            .get(&format!("{path}.py"))
            .or_else(|| nodes.get(&format!("{prefix}__init__.py")))
            .copied()
    };
    let (target, fallback) = match &raw.target {
        TargetSpec::Qualified(_) => return None,
        TargetSpec::Path(path) => {
            let mut parts: Vec<_> = parent_dir(&raw.source)
                .split('/')
                .filter(|p| !p.is_empty())
                .collect();
            for part in path.split('/') {
                match part {
                    "." | "" => {}
                    ".." => {
                        parts.pop()?;
                    }
                    other => parts.push(other),
                }
            }
            let target = parts.join("/");
            let fallback = parent_dir(&target).to_owned();
            (target, Some(fallback))
        }
        TargetSpec::Symbol { path, stem } => {
            let target = stem
                .as_ref()
                .map_or_else(|| path.clone(), |stem| format!("{path}/{stem}"));
            let fallback =
                (raw.raw.split_whitespace().next() == Some("from")).then(|| path.clone());
            (target, fallback)
        }
    };
    module_file(&target)
        .or_else(|| fallback.as_deref().and_then(module_file))
        .or_else(|| {
            dirs.contains(&target)
                .then(|| builder.ensure_pseudo(&target))
        })
}

/// Resolve one raw import to the graph node its edge ends at, `None`
/// when the target is external (the caller warns and drops the edge).
fn resolve_target(
    raw_import: &RawImport,
    decl_map: &DeclMap,
    stem_files: &BTreeMap<String, Vec<String>>,
    known_dirs: &BTreeSet<String>,
    node_of: &HashMap<String, NodeId>,
    builder: &mut GraphBuilder,
    lang: &str,
) -> Option<NodeId> {
    match &raw_import.target {
        TargetSpec::Path(spec) => {
            let resolved = resolve_relative(&raw_import.source, spec)?;
            let indexed = format!("{resolved}/index");
            let found = stem_files.get(&resolved).or_else(|| {
                matches!(lang, "js" | "ts")
                    .then(|| stem_files.get(&indexed))
                    .flatten()
            });
            match found {
                Some(files) if files.len() == 1 => node_of.get(&files[0]).copied(),
                // The stem names several files: the containing module's
                // pseudo node stands in, mirroring DeclMap's ambiguity rule.
                Some(_) => Some(builder.ensure_pseudo(parent_dir(&resolved))),
                // A spec naming a directory becomes that module's
                // pseudo node; anything else is external.
                None if known_dirs.contains(&resolved) => Some(builder.ensure_pseudo(&resolved)),
                None => None,
            }
        }
        TargetSpec::Symbol { path, stem } => match decl_map.resolve_symbol(path, stem.as_deref()) {
            SymbolTarget::File(file) => node_of.get(&file).copied(),
            SymbolTarget::Module(module) => Some(builder.ensure_pseudo(&module)),
            SymbolTarget::External
                if import_family(lang) == "java" || (lang == "cs" && stem.is_some()) =>
            {
                let mut scope = path.as_str();
                let mut found = None;
                while let Some((parent, outer)) = scope.rsplit_once('/') {
                    if let SymbolTarget::File(file) = decl_map.resolve_symbol(parent, Some(outer)) {
                        found = node_of.get(&file).copied();
                        break;
                    }
                    scope = parent;
                }
                found
            }
            SymbolTarget::External => None,
        },
        // Longest declared prefix first, so `a.b.Outer.Inner` reaches the
        // `Outer` file and `a.b.C.method()` the `C` file.
        TargetSpec::Qualified(segments) => (1..segments.len())
            .rev()
            .find_map(|split| {
                decl_map.declared_file(&segments[..split].join("/"), &segments[split])
            })
            .and_then(|file| node_of.get(&file).copied()),
    }
}

/// The warning text for one unresolved import (Kap. 6 step 5: extern,
/// Warnung, keine Kante).
fn external_warning(raw_import: &RawImport) -> String {
    let spec = match &raw_import.target {
        TargetSpec::Path(spec) => spec.clone(),
        TargetSpec::Symbol { path, stem } => match stem {
            Some(stem) => format!("{path}/{stem}"),
            None => path.clone(),
        },
        TargetSpec::Qualified(segments) => segments.join("."),
    };
    format!(
        "unresolved external import `{spec}` in {} (line {}): {}",
        raw_import.source,
        raw_import.line,
        raw_import.raw.trim()
    )
}

/// The synthetic [`Declaration`]s of the `composer.json` PSR-4 prefixes:
/// every scanned file under a prefix's source directory is additionally
/// declared under `<prefix>/<subdirectory>` (Kap. 6.1 PHP row). A
/// missing or unreadable `composer.json` simply declares nothing — the
/// parse itself is infallible (T43).
fn psr4_declarations(
    root: &Path,
    scanned: &[ScannedFile],
) -> Result<Vec<Declaration>, ArchguardError> {
    let Ok(content) = fs::read_to_string(root.join("composer.json")) else {
        return Ok(Vec::new());
    };
    let mut declarations = Vec::new();
    for (prefix, dir) in parse_psr4(&content) {
        let dir_prefix = format!("{dir}/");
        for file in scanned.iter().filter(|file| file.lang == "php") {
            // PSR-4 directories are project-root-relative, like walked paths.
            let Some(rest) = file.rel_path.strip_prefix(&dir_prefix) else {
                continue;
            };
            declarations.push(Declaration {
                module_path: join_module(&prefix, parent_dir(rest)),
                file: file.stripped.clone(),
            });
        }
    }
    Ok(declarations)
}

/// `prefix/rest` with empty halves collapsed (the root namespace has an
/// empty prefix).
fn join_module(prefix: &str, rest: &str) -> String {
    match (prefix.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_string(),
        (_, true) => prefix.to_string(),
        (false, false) => format!("{prefix}/{rest}"),
    }
}

/// The directory part of a `/`-joined path; `""` for a path with no
/// directory component.
fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

/// The path as `resolve_relative` returns it: the last component
/// without its file extension (a leading-dot file keeps its name).
fn stem_path(path: &str) -> String {
    let (dir, last) = match path.rsplit_once('/') {
        Some((dir, last)) => (Some(dir), last),
        None => (None, path),
    };
    let stem = match last.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => last,
    };
    match dir {
        Some(dir) => format!("{dir}/{stem}"),
        None => stem.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_matches_only_names_its_own_level() {
        assert!(deny_matches(DenyCycles::Module, Level::Module));
        assert!(deny_matches(DenyCycles::File, Level::File));
        assert!(!deny_matches(DenyCycles::Module, Level::File));
        assert!(!deny_matches(DenyCycles::File, Level::Module));
        assert!(!deny_matches(DenyCycles::Off, Level::Module));
        assert!(!deny_matches(DenyCycles::Off, Level::File));
    }

    #[test]
    fn file_node_name_uses_the_declaration_then_the_stripped_path() {
        let declaration = Declaration {
            module_path: "com/example".to_string(),
            file: "weird/place/Order.java".to_string(),
        };
        assert_eq!(
            file_node_name("weird/place/Order.java", std::slice::from_ref(&declaration)),
            "com/example/Order.java"
        );
        // No declaration, or a root-namespace one: the stripped path wins.
        assert_eq!(
            file_node_name("app/billing/invoice.ts", &[]),
            "app/billing/invoice.ts"
        );
        let root_module = Declaration {
            module_path: String::new(),
            file: "script.php".to_string(),
        };
        assert_eq!(file_node_name("script.php", &[root_module]), "script.php");
    }

    #[test]
    fn stem_path_strips_only_the_last_extension() {
        assert_eq!(stem_path("a/b/util.ts"), "a/b/util");
        assert_eq!(stem_path("a/main.d.ts"), "a/main.d");
        assert_eq!(stem_path("script"), "script");
        assert_eq!(stem_path(".hidden"), ".hidden");
    }

    #[test]
    fn join_module_collapses_empty_halves() {
        assert_eq!(join_module("App", "Model"), "App/Model");
        assert_eq!(join_module("", "Model"), "Model");
        assert_eq!(join_module("App", ""), "App");
        assert_eq!(join_module("", ""), "");
    }
}

#[cfg(test)]
mod go_graph_tests {
    use super::*;
    #[test]
    fn imports_target_unique_local_packages_and_cache_tracks_modules() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("src/store")).unwrap();
        std::fs::write(d.path().join("go.mod"), "module example.org/shop\n").unwrap();
        std::fs::write(
            d.path().join("src/api.go"),
            "package api\nimport \"example.org/shop/src/store\"\n",
        )
        .unwrap();
        std::fs::write(d.path().join("src/store/db.go"), "package store\n").unwrap();
        let mut config = archguard_core::Config::default();
        config.project.source_roots = vec!["src".into()];
        let files = walk_files(d.path(), &config).unwrap();
        let (graph, warnings) = build_graph(d.path(), &files, &config).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(graph.edges.len(), 1);
        assert!(graph.nodes.iter().any(|n| n.name == "api.go"));
        assert!(graph.nodes.iter().any(|n| n.name == "store/*"));
        let before = crate::go_sources::fingerprint(d.path(), &files);
        std::fs::write(d.path().join("go.mod"), "module example.org/other\n").unwrap();
        assert_ne!(before, crate::go_sources::fingerprint(d.path(), &files));
        let (graph, warnings) = build_graph(d.path(), &files, &config).unwrap();
        assert!(graph.edges.is_empty());
        assert_eq!(warnings.len(), 1);
    }
}

#[cfg(test)]
mod qualified_name_tests {
    use super::*;

    fn graph_of(files: &[(&str, &str)]) -> (Graph, Vec<String>) {
        let d = tempfile::tempdir().unwrap();
        for (rel, content) in files {
            let path = d.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        let mut config = Config::default();
        config.project.source_roots = vec!["src".into()];
        let files = walk_files(d.path(), &config).unwrap();
        build_graph(d.path(), &files, &config).unwrap()
    }

    fn targets(graph: &Graph, from: &str) -> Vec<String> {
        let from = graph.nodes.iter().find(|n| n.name == from).unwrap().id;
        let mut names: Vec<_> = graph
            .out_edges(from)
            .map(|edge| graph.node_name(edge.to).unwrap().to_string())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn java_qualified_names_link_only_declared_package_types() {
        let (graph, warnings) = graph_of(&[
            (
                "src/a/User.java",
                "package a;\npublic class User {\n    b.Repo r = new b.Repo();\n    b.Outer.Inner i;\n    void f() { order.items(); repo.Items.size(); java.util.List.of(); }\n}\n",
            ),
            ("src/b/Repo.java", "package b;\npublic class Repo {}\n"),
            (
                "src/b/Outer.java",
                "package b;\npublic class Outer { public static class Inner {} }\n",
            ),
            // `repo` is only a directory, not a declared package.
            (
                "src/repo/Items.java",
                "package storage;\npublic class Items {}\n",
            ),
        ]);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            targets(&graph, "a/User.java"),
            ["b/Outer.java", "b/Repo.java"]
        );
    }

    #[test]
    fn csharp_qualified_names_link_only_declared_namespace_types() {
        let (graph, warnings) = graph_of(&[
            (
                "src/A/User.cs",
                "namespace A;\npublic class User {\n    public B.Repo R = new B.Repo();\n    public void F() { Console.WriteLine(1); Repo.Items.Count(); }\n}\n",
            ),
            ("src/B/Repo.cs", "namespace B;\npublic class Repo {}\n"),
            // `Repo` is only a directory, not a declared namespace.
            (
                "src/Repo/Items.cs",
                "namespace Storage;\npublic class Items {}\n",
            ),
        ]);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(targets(&graph, "A/User.cs"), ["B/Repo.cs"]);
    }
}
