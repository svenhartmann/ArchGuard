//! Immutable, explicitly recorded quality measurements and conservative deltas.
use crate::quality_findings::QualityReport;
use crate::test_coverage::{CoverageCounter, TestCoverageReport};
use archguard_core::{ArchguardError, CallAnalysis, Config, Graph, NodeKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualitySnapshot {
    #[serde(default)]
    pub operational: Option<crate::measurement_runs::MeasurementReport>,
    pub schema_version: u32,
    pub timestamp: u64,
    pub tag: Option<String>,
    pub git_commit: Option<String>,
    pub source_sha256: String,
    pub config_sha256: String,
    pub analyzer_signature: String,
    pub analyzers: Vec<QualityAnalyzerIdentity>,
    pub analyzer_versions_known: bool,
    pub coverage_format: Option<String>,
    pub coverage_sha256: String,
    pub quality_tools: Vec<String>,
    pub quality_sha256: String,
    pub producer_revision: String,
    pub modules: Vec<ModuleQualitySnapshot>,
    pub diagnostics: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct QualityAnalyzerIdentity {
    pub language: String,
    pub backend: String,
    pub version: Option<String>,
    pub project: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleQualitySnapshot {
    #[serde(default)]
    pub extended: BTreeMap<String, RecordedMetric>,
    pub node: String,
    pub files: usize,
    pub mapped_files: usize,
    pub source_sha256: String,
    pub coverage_status: String,
    pub line_coverage: Option<CoverageCounter>,
    pub coverage_files: usize,
    pub semantic_status: String,
    pub declared_methods: usize,
    pub measured_methods: usize,
    pub mean_complexity: Option<f64>,
    pub max_complexity: Option<u32>,
    pub max_nesting: Option<u32>,
    pub complexity_definition: Option<String>,
    pub active_findings: Option<u64>,
    pub findings_status: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordedMetric {
    pub value: Option<f64>,
    pub measured: usize,
    pub population: usize,
    pub definition: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityHistoryEntry {
    pub path: String,
    pub timestamp: u64,
    pub tag: Option<String>,
    pub git_commit: Option<String>,
    pub source_sha256: String,
    pub modules: usize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityHistoryReport {
    #[serde(default)]
    pub series: Vec<QualityHistoryPoint>,
    pub schema_version: u32,
    pub status: String,
    pub records: Vec<QualityHistoryEntry>,
    pub records_truncated: bool,
    pub latest: Option<QualitySnapshot>,
    pub comparison: Option<QualityComparison>,
    pub diagnostics: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityHistoryPoint {
    pub timestamp: u64,
    pub tag: Option<String>,
    pub modules: Vec<QualityModulePoint>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityModulePoint {
    pub node: String,
    pub metrics: BTreeMap<String, Option<f64>>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityComparison {
    pub before_timestamp: u64,
    pub after_timestamp: u64,
    pub modules: Vec<ModuleQualityDelta>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleQualityDelta {
    #[serde(default)]
    pub extended: BTreeMap<String, QualityMetricDelta>,
    pub node: String,
    pub status: String,
    pub line_coverage: QualityMetricDelta,
    pub mean_complexity: QualityMetricDelta,
    pub max_complexity: QualityMetricDelta,
    pub active_findings: QualityMetricDelta,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityMetricDelta {
    pub status: String,
    pub before: Option<f64>,
    pub after: Option<f64>,
    pub delta: Option<f64>,
    pub reason: Option<String>,
}

const MAX_RECORD_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MODULES: usize = 10_000;
const MAX_ENTRIES: usize = 20;
const MAX_DIRECTORY_ENTRIES: usize = 10_000;
const PROVENANCE: &str = "Source/configuration hashes identify the locally recorded inputs, not an authenticated CI artifact. Coverage and SARIF producer revision/tool configuration were not verified at import. Imported-report deltas are observations, not proof that a refactoring improved behavior.";
fn error(message: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("quality history: {message}"))
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest<T: Serialize>(value: &T) -> Result<String, ArchguardError> {
    Ok(hash(&serde_json::to_vec(value).map_err(error)?))
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|n| n.as_secs())
        .unwrap_or(0)
}
fn language(lang: &str) -> &str {
    match lang {
        "ts" => "typescript",
        "js" => "javascript",
        "cs" => "csharp",
        "py" => "python",
        "rs" => "rust",
        "kt" => "kotlin",
        other => other,
    }
}

/// Record current local source identities and measurements already obtained by
/// the caller. This function does not run compilers, tests, or producer tools.
pub fn build_quality_snapshot(
    root: &Path,
    config: &Config,
    graph: &Graph,
    calls: &CallAnalysis,
    coverage: &TestCoverageReport,
    quality: &QualityReport,
) -> Result<QualitySnapshot, ArchguardError> {
    let nodes: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id, n)).collect();
    let mut package_of = BTreeMap::new();
    let mut members = BTreeMap::<String, Vec<String>>::new();
    let mut languages = BTreeMap::<String, BTreeSet<String>>::new();
    for node in graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::File && !n.name.ends_with("/*"))
    {
        let package = node
            .parent
            .and_then(|p| nodes.get(&p))
            .filter(|n| n.kind == NodeKind::Module)
            .map(|n| n.name.as_str())
            .unwrap_or(".")
            .to_string();
        members
            .entry(package.clone())
            .or_default()
            .push(node.name.clone());
        if let Some(lang) = &node.lang {
            languages
                .entry(package.clone())
                .or_default()
                .insert(language(lang).into());
        }
        package_of.insert(node.name.clone(), package);
    }
    if members.len() > MAX_MODULES {
        return Err(error("more than 10000 package scopes"));
    }
    let mut identities = BTreeMap::<String, (String, String)>::new();
    let mut path_nodes = BTreeMap::new();
    let mut diagnostics=vec![PROVENANCE.into(),"Module scopes contain immediate real file members; nested packages are separate. Means describe measured method bodies, not all declarations. No source files, compilers, or reports are reconstructed for historical records.".into()];
    let mut source_bytes = 0u64;
    for source in crate::review_sources::collect_source_evidence(root, config, graph)? {
        let Some(node) = nodes
            .get(&source.node_id)
            .filter(|n| package_of.contains_key(&n.name))
        else {
            continue;
        };
        let Some(path) = source.path else {
            continue;
        };
        let bytes = crate::project_files::read_repo_file(root, Path::new(&path), MAX_SOURCE_BYTES)
            .map_err(error)?;
        source_bytes += bytes.len() as u64;
        if source_bytes > 256 * 1024 * 1024 {
            return Err(error(
                "configured source set exceeds 256 MiB history hashing budget",
            ));
        }
        identities.insert(node.name.clone(), (path.clone(), hash(&bytes)));
        path_nodes.insert(path, node.name.clone());
    }
    // Compiler sources use root-stripped paths; imported reports and hashes
    // use physical repository paths. Accept aliases only when unambiguous.
    let mut aliases = BTreeMap::<String, BTreeSet<Option<String>>>::new();
    for file in crate::pipeline::walk_files(root, config)? {
        let mapped = path_nodes.get(&file.rel_path).cloned();
        let stripped = archguard_extract::strip_root(&file.rel_path, &config.project.source_roots);
        aliases.entry(stripped).or_default().insert(mapped.clone());
        aliases.entry(file.rel_path).or_default().insert(mapped);
    }
    let semantic_nodes: BTreeMap<String, String> = aliases
        .into_iter()
        .filter_map(|(path, nodes)| {
            if nodes.len() != 1 {
                return None;
            }
            nodes.into_iter().next().flatten().map(|node| (path, node))
        })
        .collect();
    let mut methods = BTreeMap::<String, Vec<_>>::new();
    let represented_nodes: BTreeSet<_> = calls
        .classes
        .iter()
        .flat_map(|class| {
            std::iter::once(&class.file).chain(class.declaration_files.iter().flatten())
        })
        .chain(calls.methods.iter().map(|method| &method.file))
        .filter_map(|path| semantic_nodes.get(path))
        .collect();
    let mut unmapped_methods = 0usize;
    for method in &calls.methods {
        if let Some(package) = semantic_nodes
            .get(&method.file)
            .and_then(|n| package_of.get(n))
        {
            methods.entry(package.clone()).or_default().push(method);
        } else {
            unmapped_methods += 1;
        }
    }
    if unmapped_methods > 0 {
        diagnostics.push(format!("{unmapped_methods} method declarations have no exact source-to-package mapping; method coverage remains partial."));
    }
    let coverage_files: BTreeMap<_, _> = coverage
        .files
        .iter()
        .map(|f| (f.node.as_str(), f))
        .collect();
    let mut findings = BTreeMap::<String, Vec<_>>::new();
    for finding in &quality.findings {
        if let Some(package) = finding.node.as_ref().and_then(|n| package_of.get(n)) {
            findings.entry(package.clone()).or_default().push(finding);
        }
    }
    let mut modules = Vec::new();
    let mut extended = extended_metrics(root, config, graph, calls, &semantic_nodes, &package_of)?;
    for (package, mut names) in members {
        names.sort();
        let identity: Vec<_> = names
            .iter()
            .filter_map(|n| identities.get(n).map(|value| (n, value)))
            .collect();
        let mapped_files = identity.len();
        let mut line_coverage = CoverageCounter {
            covered: 0,
            total: 0,
        };
        let mut measured_files = 0usize;
        let mut stale = false;
        for name in &names {
            if let Some(file) = coverage_files.get(name.as_str()) {
                let fresh = file.status == "unchanged_since_import"
                    && identities.get(name).is_some_and(|(path, sha)| {
                        path == &file.path && sha == &file.source_sha256
                    });
                if !fresh {
                    stale = true;
                    continue;
                }
                if let Some(lines) = file
                    .lines
                    .as_ref()
                    .filter(|lines| lines.covered <= lines.total)
                {
                    line_coverage.covered = line_coverage
                        .covered
                        .checked_add(lines.covered)
                        .ok_or_else(|| error("coverage counter overflow"))?;
                    line_coverage.total = line_coverage
                        .total
                        .checked_add(lines.total)
                        .ok_or_else(|| error("coverage counter overflow"))?;
                    measured_files += 1;
                }
            }
        }
        let coverage_status = if stale {
            "stale"
        } else if coverage.status == "ready"
            && measured_files == names.len()
            && mapped_files == names.len()
        {
            "ready"
        } else if measured_files > 0 || coverage.status == "ready" {
            "partial"
        } else {
            coverage.status.as_str()
        };
        let package_methods = methods.get(&package).map(Vec::as_slice).unwrap_or_default();
        let measured: Vec<_> = package_methods
            .iter()
            .filter_map(|m| m.complexity.as_ref())
            .collect();
        let definitions: BTreeSet<_> = measured.iter().map(|m| m.definition.as_str()).collect();
        let supported = languages.get(&package).is_some_and(|langs| {
            !langs.is_empty()
                && langs.iter().all(|lang| {
                    let providers: Vec<_> = calls
                        .analyzers
                        .iter()
                        .filter(|a| &a.language == lang)
                        .collect();
                    !providers.is_empty() && providers.iter().all(|a| a.status == "ready")
                })
        });
        let semantic_status = if supported
            && calls.status == "ready"
            && mapped_files == names.len()
            && identity
                .iter()
                .all(|(name, _)| represented_nodes.contains(name))
            && unmapped_methods == 0
            && definitions.iter().all(|d| *d == "ast-decisions-v1")
        {
            "ready"
        } else if !measured.is_empty() {
            "partial"
        } else {
            "unavailable"
        };
        let package_findings = findings
            .get(&package)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let findings_stale = package_findings.iter().any(|f| {
            f.status != "unchanged_since_import"
                || f.node
                    .as_ref()
                    .and_then(|n| identities.get(n))
                    .is_none_or(|(path, sha)| {
                        f.path.as_ref() != Some(path) || f.source_sha256.as_ref() != Some(sha)
                    })
        });
        let findings_status = if findings_stale {
            "stale"
        } else {
            quality.status.as_str()
        };
        let active_findings = matches!(quality.status.as_str(), "ready" | "partial" | "stale")
            .then(|| {
                package_findings
                    .iter()
                    .filter(|f| {
                        !f.suppressed
                            && f.baseline_state.as_deref() != Some("absent")
                            && f.level != "none"
                            && f.status == "unchanged_since_import"
                            && f.node.as_ref().and_then(|n| identities.get(n)).is_some_and(
                                |(path, sha)| {
                                    f.path.as_ref() == Some(path)
                                        && f.source_sha256.as_ref() == Some(sha)
                                },
                            )
                    })
                    .count() as u64
            });
        modules.push(ModuleQualitySnapshot {
            extended: extended.remove(&package).unwrap_or_default(),
            node: package,
            files: names.len(),
            mapped_files,
            source_sha256: digest(&identity)?,
            coverage_status: coverage_status.into(),
            line_coverage: (measured_files > 0).then_some(line_coverage),
            coverage_files: measured_files,
            semantic_status: semantic_status.into(),
            declared_methods: package_methods.len(),
            measured_methods: measured.len(),
            mean_complexity: (!measured.is_empty()).then(|| {
                measured
                    .iter()
                    .map(|m| f64::from(m.cyclomatic))
                    .sum::<f64>()
                    / measured.len() as f64
            }),
            max_complexity: measured.iter().map(|m| m.cyclomatic).max(),
            max_nesting: measured.iter().map(|m| m.max_nesting).max(),
            complexity_definition: (definitions.len() == 1)
                .then(|| definitions.iter().next().unwrap().to_string()),
            active_findings,
            findings_status: findings_status.into(),
        });
    }
    let mut analyzers: Vec<_> = calls
        .analyzers
        .iter()
        .map(|a| QualityAnalyzerIdentity {
            language: a.language.clone(),
            backend: a.backend.clone(),
            version: a.version.clone(),
            project: a.project.clone(),
        })
        .collect();
    analyzers.sort();
    let mut quality_tools = quality.tools.clone();
    quality_tools.sort();
    quality_tools.dedup();
    let git_commit = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--verify", "HEAD"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit()));
    Ok(QualitySnapshot {
        operational: Some(crate::measurement_runs::read_measurement_runs(root)),
        schema_version: 1,
        timestamp: now(),
        tag: None,
        git_commit,
        source_sha256: digest(&identities)?,
        config_sha256: digest(config)?,
        analyzer_signature: digest(&analyzers)?,
        analyzers,
        analyzer_versions_known: !calls.analyzers.is_empty()
            && calls
                .analyzers
                .iter()
                .all(|a| a.version.as_ref().is_some_and(|v| !v.trim().is_empty())),
        coverage_format: coverage.format.clone(),
        coverage_sha256: digest(coverage)?,
        quality_tools,
        quality_sha256: digest(quality)?,
        producer_revision: "unverified".into(),
        modules,
        diagnostics,
    })
}

fn extended_metrics(
    root: &Path,
    config: &Config,
    graph: &Graph,
    calls: &CallAnalysis,
    path_nodes: &BTreeMap<String, String>,
    package_of: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, BTreeMap<String, RecordedMetric>>, ArchguardError> {
    let metrics = crate::code_metrics(calls);
    let mut samples = BTreeMap::<(String, String), Vec<Option<f64>>>::new();
    for class in &metrics.classes {
        let Some(package) = path_nodes.get(&class.file).and_then(|n| package_of.get(n)) else {
            continue;
        };
        for (key, value) in [
            ("cbo", class.cbo.value),
            ("rfc", class.rfc.value),
            ("dit", class.dit.value),
            ("noc", class.noc.value),
            ("wmc", class.wmc.value),
            ("lcom4", class.lcom4.map(|v| v as u64)),
        ] {
            samples
                .entry((package.clone(), key.into()))
                .or_default()
                .push(value.map(|v| v as f64));
        }
    }
    for method in &metrics.methods {
        let Some(package) = path_nodes.get(&method.file).and_then(|n| package_of.get(n)) else {
            continue;
        };
        for (key, value) in [
            ("cognitive", method.cognitive.map(f64::from)),
            ("maintainability", method.maintainability_index),
            (
                "halstead.volume",
                method.halstead.as_ref().map(|h| h.volume),
            ),
            (
                "halstead.difficulty",
                method.halstead.as_ref().and_then(|h| h.difficulty),
            ),
            (
                "halstead.effort",
                method.halstead.as_ref().and_then(|h| h.effort),
            ),
        ] {
            samples
                .entry((package.clone(), key.into()))
                .or_default()
                .push(value);
        }
    }
    let mut result = BTreeMap::<String, BTreeMap<String, RecordedMetric>>::new();
    for ((package, key), values) in samples {
        let known: Vec<_> = values.iter().copied().flatten().collect();
        let statistics = [
            (
                "mean",
                (!known.is_empty()).then(|| known.iter().sum::<f64>() / known.len() as f64),
            ),
            ("max", known.iter().copied().reduce(f64::max)),
            ("min", known.iter().copied().reduce(f64::min)),
        ];
        for (stat, value) in statistics {
            result.entry(package.clone()).or_default().insert(
                format!("{key}.{stat}"),
                RecordedMetric {
                    value,
                    measured: known.len(),
                    population: values.len(),
                    definition: "source-metrics/v1".into(),
                },
            );
        }
    }
    let node_names: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id, &n.name)).collect();
    let packages: BTreeSet<_> = package_of.values().cloned().collect();
    let ids: BTreeMap<_, _> = packages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.clone(), archguard_core::NodeId(i as u32)))
        .collect();
    let pairs: BTreeSet<_> = graph
        .edges
        .iter()
        .filter_map(|edge| {
            let a = package_of.get(*node_names.get(&edge.from)?)?;
            let b = package_of.get(*node_names.get(&edge.to)?)?;
            (a != b).then_some((ids[a], ids[b]))
        })
        .collect();
    let package_graph = Graph {
        schema_version: 1,
        root: graph.root.clone(),
        semantic: graph.semantic.clone(),
        review_kpis: None,
        nodes: ids
            .iter()
            .map(|(name, &id)| archguard_core::Node {
                id,
                name: name.clone(),
                kind: NodeKind::Module,
                parent: None,
                lang: None,
            })
            .collect(),
        edges: pairs
            .iter()
            .map(|&(from, to)| archguard_core::Edge {
                from,
                to,
                refs: vec![],
                count: None,
            })
            .collect(),
    };
    let sccs = archguard_analysis::find_sccs(&package_graph);
    let mut cyclic = BTreeMap::new();
    let mut sizes = BTreeMap::new();
    for scc in sccs {
        for id in &scc.node_ids {
            sizes.insert(*id, scc.node_ids.len());
        }
        for (a, _) in scc.edges {
            *cyclic.entry(a).or_insert(0usize) += 1;
        }
    }
    for package in packages {
        let id = ids[&package];
        let target = result.entry(package).or_default();
        for (key, value) in [
            (
                "package.cyclic_dependencies",
                *cyclic.get(&id).unwrap_or(&0),
            ),
            ("package.cycle_size", *sizes.get(&id).unwrap_or(&0)),
        ] {
            target.insert(
                key.into(),
                RecordedMetric {
                    value: Some(value as f64),
                    measured: 1,
                    population: 1,
                    definition: "direct-package-cycles/v1".into(),
                },
            );
        }
    }
    for p in crate::cohesion::analyze_cohesion(graph, None).packages {
        result.entry(p.node).or_default().insert(
            "package.internal_share".into(),
            RecordedMetric {
                value: p.internal_share,
                measured: usize::from(p.internal_share.is_some()),
                population: 1,
                definition: "package-internal-share/v1".into(),
            },
        );
    }
    for p in crate::knowledge_metrics(root, config, graph)?.packages {
        for (key, value) in [
            ("git.bus_factor_50", p.bus_factor_50.map(|n| n as f64)),
            (
                "git.top_share",
                (p.status == "ready").then_some(p.top_share).flatten(),
            ),
        ] {
            result.entry(p.node.clone()).or_default().insert(
                key.into(),
                RecordedMetric {
                    value,
                    measured: usize::from(value.is_some()),
                    population: 1,
                    definition: "git-touch-concentration/v1".into(),
                },
            );
        }
    }
    Ok(result)
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn valid(snapshot: &QualitySnapshot) -> bool {
    let mut names = BTreeSet::new();
    snapshot.schema_version == 1
        && snapshot.modules.len() <= MAX_MODULES
        && snapshot.producer_revision == "unverified"
        && [
            &snapshot.source_sha256,
            &snapshot.config_sha256,
            &snapshot.analyzer_signature,
            &snapshot.coverage_sha256,
            &snapshot.quality_sha256,
        ]
        .iter()
        .all(|v| valid_hash(v))
        && snapshot.tag.as_ref().is_none_or(|t| t.len() <= 1024)
        && snapshot.diagnostics.len() <= 100
        && snapshot.modules.iter().all(|m| {
            !m.node.is_empty()
                && names.insert(&m.node)
                && valid_hash(&m.source_sha256)
                && m.mapped_files <= m.files
                && m.coverage_files <= m.files
                && m.measured_methods <= m.declared_methods
                && m.line_coverage
                    .as_ref()
                    .is_none_or(|c| c.covered <= c.total)
                && m.mean_complexity.is_none_or(|v| v.is_finite() && v >= 1.0)
                && m.extended.len() <= 64
                && m.extended.values().all(|v| {
                    v.measured <= v.population
                        && v.value.is_none_or(|v| v.is_finite() && v >= 0.0)
                        && v.definition.len() <= 128
                })
        })
}

/// Explicit, create-only publication. A label never becomes a path component.
pub fn record_quality_snapshot(
    root: &Path,
    snapshot: &QualitySnapshot,
    tag: Option<&str>,
) -> Result<PathBuf, ArchguardError> {
    let mut snapshot = snapshot.clone();
    snapshot.tag = tag.map(str::to_string);
    snapshot.timestamp = now();
    if !valid(&snapshot) {
        return Err(error("invalid or oversized quality snapshot"));
    }
    let bytes = serde_json::to_vec_pretty(&snapshot).map_err(error)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(error("snapshot exceeds 8 MiB"));
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|n| n.as_nanos())
        .unwrap_or(0);
    for suffix in 0..10_000u32 {
        let relative = PathBuf::from(format!(
            "quality-history/{stamp:020}-{}-{suffix:06}.json",
            &snapshot.source_sha256[..16]
        ));
        match crate::project_files::write_project_file(root, &relative, &bytes, false) {
            Ok(_) => return Ok(root.join(".archguard").join(relative)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(error("quality snapshot filename collision budget exceeded"))
}

fn history_paths(root: &Path) -> std::io::Result<(Vec<PathBuf>, bool)> {
    for part in [".archguard", ".archguard/quality-history"] {
        let meta = fs::symlink_metadata(root.join(part))?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(std::io::Error::other(
                "history must be an ordinary project directory",
            ));
        }
    }
    let mut paths = Vec::new();
    let mut truncated = false;
    for (index, entry) in fs::read_dir(root.join(".archguard/quality-history"))?.enumerate() {
        if index == MAX_DIRECTORY_ENTRIES {
            truncated = true;
            break;
        }
        let entry = entry?;
        if entry.path().extension().is_some_and(|s| s == "json") {
            paths.push(PathBuf::from(".archguard/quality-history").join(entry.file_name()));
        }
    }
    paths.sort();
    paths.reverse();
    Ok((paths, truncated))
}
fn empty_report(status: &str) -> QualityHistoryReport {
    QualityHistoryReport {
        series: vec![],
        schema_version: 1,
        status: status.into(),
        records: vec![],
        records_truncated: false,
        latest: None,
        comparison: None,
        diagnostics: vec![PROVENANCE.into()],
    }
}

/// Bounded read-only overview: latest 20 record summaries and only the latest
/// full snapshot/comparison. Older immutable JSON files remain on disk.
pub fn read_quality_history(root: &Path) -> QualityHistoryReport {
    let (paths, truncated) = match history_paths(root) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return empty_report("not_recorded"),
        Err(e) => {
            let mut r = empty_report("error");
            r.diagnostics.push(e.to_string());
            return r;
        }
    };
    let mut report = empty_report("ready");
    report.records_truncated = truncated;
    let mut snapshots = Vec::new();
    let mut bytes_read = 0u64;
    for path in paths {
        if report.records.len() == MAX_ENTRIES {
            report.records_truncated = true;
            break;
        }
        let result = crate::project_files::read_repo_file(root, &path, MAX_RECORD_BYTES)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                bytes_read += bytes.len() as u64;
                if bytes_read > MAX_TOTAL_BYTES {
                    return Err("history reading budget exceeded".into());
                }
                serde_json::from_slice::<QualitySnapshot>(&bytes)
                    .map_err(|e| e.to_string())
                    .and_then(|s| {
                        if valid(&s) {
                            Ok(s)
                        } else {
                            Err("invalid snapshot metadata".into())
                        }
                    })
            });
        match result {
            Ok(snapshot) => {
                report.series.push(QualityHistoryPoint {
                    timestamp: snapshot.timestamp,
                    tag: snapshot.tag.clone(),
                    modules: snapshot
                        .modules
                        .iter()
                        .map(|m| {
                            let mut metrics = BTreeMap::from([
                                ("mean_complexity".into(), m.mean_complexity),
                                ("line_coverage".into(), percent(m)),
                            ]);
                            for key in [
                                "cbo.mean",
                                "lcom4.mean",
                                "cognitive.max",
                                "maintainability.mean",
                                "package.cyclic_dependencies",
                                "git.bus_factor_50",
                            ] {
                                metrics
                                    .insert(key.into(), m.extended.get(key).and_then(|v| v.value));
                            }
                            QualityModulePoint {
                                node: m.node.clone(),
                                metrics,
                            }
                        })
                        .collect(),
                });
                report.records.push(QualityHistoryEntry {
                    path: path.to_string_lossy().into_owned(),
                    timestamp: snapshot.timestamp,
                    tag: snapshot.tag.clone(),
                    git_commit: snapshot.git_commit.clone(),
                    source_sha256: snapshot.source_sha256.clone(),
                    modules: snapshot.modules.len(),
                });
                if snapshots.len() < 2 {
                    snapshots.push(snapshot);
                }
            }
            Err(_) => {
                report.status = "partial".into();
                if report.diagnostics.len() < 100 {
                    report.diagnostics.push("A quality history record was unreadable, oversized or invalid and was omitted.".into());
                }
            }
        }
        if bytes_read > MAX_TOTAL_BYTES {
            report.records_truncated = true;
            break;
        }
    }
    if truncated {
        report.status = "partial".into();
        report.diagnostics.push("Directory entry budget exceeded; the shown records may not include the newest records.".into());
    }
    if report.records_truncated {
        report.diagnostics.push("The overview is bounded to 20 records and 64 MiB of data; full immutable records remain in .archguard/quality-history/.".into());
    }
    if snapshots.len() == 2 {
        report.comparison = Some(compare_quality_snapshots(&snapshots[1], &snapshots[0]));
    }
    report.latest = snapshots.into_iter().next();
    report.series.reverse();
    if report.records.is_empty() && report.status == "ready" {
        report.status = "not_recorded".into();
    }
    report
}

/// Names/metadata are sufficient for explicitly create-only history updates;
/// read_quality_history separately validates all selected record contents.
pub fn quality_history_fingerprint(root: &Path) -> String {
    match history_paths(root) {
        Ok((paths, truncated)) => {
            let mut state = Vec::new();
            for path in paths.iter().take(MAX_DIRECTORY_ENTRIES) {
                let metadata = fs::symlink_metadata(root.join(path)).ok();
                state.push((
                    path,
                    metadata.as_ref().map(|m| m.len()),
                    metadata
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| (d.as_secs(), d.subsec_nanos())),
                ));
            }
            digest(&(state, truncated)).unwrap_or_else(|_| "unavailable".into())
        }
        Err(e) => format!("unavailable:{:?}", e.kind()),
    }
}

fn percent(module: &ModuleQualitySnapshot) -> Option<f64> {
    module
        .line_coverage
        .as_ref()
        .filter(|c| c.total > 0)
        .map(|c| 100.0 * c.covered as f64 / c.total as f64)
}
fn metric(
    before: Option<f64>,
    after: Option<f64>,
    incompatible: Option<&str>,
    unverified: bool,
) -> QualityMetricDelta {
    let reason = if before.is_none() || after.is_none() {
        Some("A measurement is missing or not applicable.")
    } else {
        incompatible
    };
    QualityMetricDelta{status:if reason.is_some(){"incomparable"}else if unverified{"unverified"}else{"comparable"}.into(),before,after,delta:if reason.is_none(){before.zip(after).map(|(a,b)|b-a)}else{None},reason:reason.map(str::to_string).or_else(||unverified.then(||"Imported report producer revision and configuration are unverified; this is a reported numeric difference, not a verified improvement.".into()))}
}

pub fn compare_quality_snapshots(
    before: &QualitySnapshot,
    after: &QualitySnapshot,
) -> QualityComparison {
    let old: BTreeMap<_, _> = before
        .modules
        .iter()
        .map(|m| (m.node.as_str(), m))
        .collect();
    let new: BTreeMap<_, _> = after.modules.iter().map(|m| (m.node.as_str(), m)).collect();
    let names: BTreeSet<_> = old.keys().chain(new.keys()).copied().collect();
    let mut modules = Vec::new();
    for name in names {
        let a = old.get(name).copied();
        let b = new.get(name).copied();
        let config_changed = (before.config_sha256 != after.config_sha256)
            .then_some("Analysis configuration changed.");
        let coverage_incompatible = config_changed
            .or_else(|| {
                (before.coverage_format != after.coverage_format)
                    .then_some("Coverage report format changed.")
            })
            .or_else(|| {
                a.zip(b).and_then(|(a, b)| {
                    (a.coverage_status != "ready" || b.coverage_status != "ready")
                        .then_some("Coverage is missing, partial or stale.")
                })
            });
        let method_incompatible = config_changed
            .or_else(|| {
                (before.analyzer_signature != after.analyzer_signature
                    || !before.analyzer_versions_known
                    || !after.analyzer_versions_known)
                    .then_some("Compiler identity/version changed or is unknown.")
            })
            .or_else(|| {
                a.zip(b).and_then(|(a, b)| {
                    if a.semantic_status != "ready" || b.semantic_status != "ready" {
                        Some("Semantic coverage is incomplete.")
                    } else if a.complexity_definition != b.complexity_definition {
                        Some("Complexity measurement definition changed.")
                    } else if a.measured_methods.saturating_mul(b.declared_methods)
                        != b.measured_methods.saturating_mul(a.declared_methods)
                    {
                        Some("The proportion of measured method declarations changed.")
                    } else {
                        None
                    }
                })
            });
        let findings_incompatible = config_changed
            .or_else(|| {
                (before.quality_tools != after.quality_tools)
                    .then_some("Imported analysis tools changed.")
            })
            .or_else(|| {
                a.zip(b).and_then(|(a, b)| {
                    (a.findings_status != "ready" || b.findings_status != "ready")
                        .then_some("Imported findings are missing, partial or stale.")
                })
            });
        modules.push(ModuleQualityDelta {
            extended: {
                let keys: BTreeSet<_> = a
                    .into_iter()
                    .flat_map(|m| m.extended.keys())
                    .chain(b.into_iter().flat_map(|m| m.extended.keys()))
                    .collect();
                keys.into_iter()
                    .map(|key| {
                        let x = a.and_then(|m| m.extended.get(key));
                        let y = b.and_then(|m| m.extended.get(key));
                        let incompatible = config_changed
                            .or_else(|| {
                                x.zip(y).and_then(|(x, y)| {
                                    if x.definition != y.definition {
                                        Some("Metric definition changed.")
                                    } else if x.measured.saturating_mul(y.population)
                                        != y.measured.saturating_mul(x.population)
                                    {
                                        Some("Measured population coverage changed.")
                                    } else {
                                        None
                                    }
                                })
                            })
                            .or(if key.starts_with("git.") {
                                None
                            } else {
                                method_incompatible
                            });
                        (
                            key.clone(),
                            metric(
                                x.and_then(|m| m.value),
                                y.and_then(|m| m.value),
                                incompatible,
                                false,
                            ),
                        )
                    })
                    .collect()
            },
            node: name.into(),
            status: match (a, b) {
                (None, _) => "added",
                (_, None) => "removed",
                _ => "present",
            }
            .into(),
            line_coverage: metric(
                a.and_then(percent),
                b.and_then(percent),
                coverage_incompatible,
                true,
            ),
            mean_complexity: metric(
                a.and_then(|m| m.mean_complexity),
                b.and_then(|m| m.mean_complexity),
                method_incompatible,
                false,
            ),
            max_complexity: metric(
                a.and_then(|m| m.max_complexity).map(f64::from),
                b.and_then(|m| m.max_complexity).map(f64::from),
                method_incompatible,
                false,
            ),
            active_findings: metric(
                a.and_then(|m| m.active_findings).map(|v| v as f64),
                b.and_then(|m| m.active_findings).map(|v| v as f64),
                findings_incompatible,
                true,
            ),
        });
    }
    QualityComparison {
        before_timestamp: before.timestamp,
        after_timestamp: after.timestamp,
        modules,
    }
}
