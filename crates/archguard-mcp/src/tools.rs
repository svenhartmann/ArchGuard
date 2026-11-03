//! Response builders, the shared size guard and the local BFS behind the
//! T27 tools of Kap. 10.2 (`cycle_get`, `check`, `metrics`,
//! `module_neighborhood`, `edge_evidence`, `suggest_cuts`, `baseline_diff`,
//! `config_view`) and the T40 drill-down tools (`graph_expand`, `dsm`,
//! `simulate`).
//!
//! Everything here is pure data-in / JSON-out on top of the engine
//! facades; the `#[tool]` methods in `lib.rs` only wire parameters to
//! these functions. Following Kap. 10.1 every answer passes through
//! [`guarded`]: anything above [`MAX_RESPONSE_BYTES`] has its main list
//! shrunk and carries `"truncated": true`.
//!
//! The neighbourhood BFS is implemented locally on the `Graph` (Kap. 10.2
//! `module_neighborhood`) instead of reusing `archguard-report`'s exporter
//! focus logic, because that would require a report→mcp dependency edge
//! and the crate graph stays untouched.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::paths;
use archguard_engine::LoadedConfig;
use archguard_engine::analysis::{Baseline, SimOp, cycle_hash, find_sccs, simulate};
use archguard_engine::core::{
    Cut, Finding, FindingKind, Graph, LayerConfig, LayerOrigin, NodeId, Report, aggregate,
};
use archguard_engine::load_config;
use archguard_engine::report::to_dsm_json;
use archguard_engine::{Level, analyze_run as engine_run};
use rmcp::{ErrorData, schemars};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Kap. 10.1: a tool answer is meant to be small; above this many
/// serialized bytes [`guarded`] shrinks the main list field.
pub const MAX_RESPONSE_BYTES: usize = 4096;

/// Largest accepted `depth` of `module_neighborhood` (a wider horizon is
/// a full-graph question, not a context question).
const MAX_DEPTH: usize = 5;

/// Sample bound for the `check` finding previews (mirrors the
/// `sample_nodes[≤5]` bound of `cycles_list`).
const FINDING_SAMPLE: usize = 5;

// ------------------------------------------------------------ shared plumbing

/// The JSON-RPC `invalid params` error (-32602) every tool-level user
/// error maps to (unknown ids, unknown modules, missing baseline …).
pub fn invalid_params(message: impl Into<std::borrow::Cow<'static, str>>) -> ErrorData {
    ErrorData::invalid_params(message, None)
}

/// Validate a required string parameter (Kap. 10.2 lists `cycle_id`,
/// `module`, `from`/`to` as mandatory); missing or empty is -32602.
pub fn required_str<'a>(value: &'a Option<String>, field: &str) -> Result<&'a str, ErrorData> {
    value
        .as_deref()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| invalid_params(format!("missing required parameter `{field}`")))
}

/// Map a path-policy rejection ([`paths`]) to the tools' -32602 style.
fn policy<T>(result: Result<T, String>) -> Result<T, ErrorData> {
    result.map_err(invalid_params)
}

/// Canonicalize the server root and resolve the effective root (parameter
/// override or server root) through the one F02 path policy: relative
/// overrides resolve against the server root, never the process working
/// directory, and escaping paths are rejected before any file access.
pub fn resolve_root(server_root: &Path, path: Option<&str>) -> Result<PathBuf, ErrorData> {
    let root = policy(paths::server_root(server_root))?;
    match path {
        None => Ok(root),
        Some(arg) => policy(paths::resolve(&root, Path::new(arg), true)),
    }
}

/// Resolve one file argument (baseline file, database path) against the
/// canonical server root. `may_create == true` is for targets the store
/// opens create-if-missing: the target may be absent, but its parent must
/// be a verified directory inside the root (F02).
pub fn resolve_file_in_root(
    server_root: &Path,
    arg: &Path,
    may_create: bool,
) -> Result<PathBuf, ErrorData> {
    let root = policy(paths::server_root(server_root))?;
    policy(paths::resolve(&root, arg, !may_create))
}

/// Load the project configuration for `root`.
pub fn load_config_at(root: &Path) -> Result<LoadedConfig, ErrorData> {
    load_config(None, root)
        .map_err(|err| ErrorData::internal_error(format!("config error: {err}"), None))
}

/// The module-level view graph of a file graph at `depth` — the same cut
/// `archguard-engine`'s report builder uses, so node names match findings
/// and metrics.
#[cfg(test)]
pub fn module_view(graph: &Graph, depth: u32) -> Graph {
    aggregate(graph, &Cut::at_depth(graph, depth as usize))
}

/// A complete module-level run: the aggregated module view the report
/// was derived from, the report and the loaded configuration —
/// everything the Kap. 10.2 builders need from one implicit scan
/// (Kap. 10.1: every tool scans itself, with the T23 graph cache).
pub struct Run {
    pub file_graph: Graph,
    /// Canonical root the analysis ran at.
    pub root: PathBuf,
    /// Module view at `loaded.config.modules.depth` — the same cut the
    /// pipeline used, so view node names match findings and metrics.
    pub view: Graph,
    /// The analysis report.
    pub report: Report,
    /// The loaded configuration (declared values and defaults).
    pub loaded: LoadedConfig,
}

/// Resolve the root, load the config and run the cached module-level
/// analysis (the `#[tool]` methods' shared first step).
pub fn analyze_run(server_root: &Path, path: Option<&str>) -> Result<Run, ErrorData> {
    let root = resolve_root(server_root, path)?;
    let loaded = load_config_at(&root)?;
    let run = engine_run(&root, &loaded, Level::Module)
        .map_err(|err| ErrorData::internal_error(format!("analysis failed: {err}"), None))?;
    let loaded = run.effective_config();
    let view = run.view;
    let report = run.report;
    Ok(Run {
        file_graph: run.graph,
        root,
        view,
        report,
        loaded,
    })
}

/// Read and parse a baseline JSON (Kap. 7); a missing or unparsable file
/// is an invalid-params error for the calling tool.
pub fn read_baseline(path: &Path) -> Result<Baseline, ErrorData> {
    let text = std::fs::read_to_string(path).map_err(|err| {
        invalid_params(format!("baseline {} not readable: {err}", path.display()))
    })?;
    serde_json::from_str(&text).map_err(|err| {
        invalid_params(format!(
            "baseline {}: invalid baseline JSON: {err}",
            path.display()
        ))
    })
}

// -------------------------------------------------------------- size guard

/// Serialized size of a [`Value`].
fn size_of(value: &Value) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

/// The shared response guard (Kap. 10.1 „kleiner Kontext"): serialize
/// `value`; when the document exceeds [`MAX_RESPONSE_BYTES`], keep the
/// longest fitting prefix of its first non-empty list field (the list
/// the builders order first is the highest-signal one) and set
/// `"truncated": true`. Small results are returned untouched.
///
/// If no list can shrink the document small enough (e.g. one huge
/// scalar), the document is still marked `truncated` — the answer
/// reports its oversized state honestly instead of failing.
pub fn guarded<T: Serialize>(value: &T) -> Value {
    let mut doc =
        serde_json::to_value(value).expect("tool responses are plain data and always serializable");
    if size_of(&doc) <= MAX_RESPONSE_BYTES {
        return doc;
    }
    // The list to shrink: the document itself, or a top-level object
    // field. `rows` is preferred explicitly (F05 review): serde_json
    // iterates object keys alphabetically, and for graph responses that
    // would pick `columns` ("columns" < "rows") — shrinking the column
    // definitions while keeping every oversized row, breaking the
    // columns↔rows contract clients zip on. `rows` is the data list
    // there; for every other shape the first non-empty array remains
    // the intended (highest-signal) list.
    let key: Option<String> = match &doc {
        Value::Object(map) => {
            if matches!(map.get("rows"), Some(Value::Array(a)) if !a.is_empty()) {
                Some("rows".to_string())
            } else {
                map.iter().find_map(|(k, v)| {
                    matches!(v, Value::Array(a) if !a.is_empty()).then(|| k.clone())
                })
            }
        }
        _ => None,
    };
    let full: Vec<Value> = match (&mut doc, &key) {
        (Value::Object(map), Some(k)) => map
            .get_mut(k)
            .and_then(Value::as_array_mut)
            .map(std::mem::take)
            .unwrap_or_default(),
        (Value::Array(items), None) => std::mem::take(items),
        _ => Vec::new(),
    };
    // Writes a candidate prefix back into the list position.
    let write_list = |doc: &mut Value, list: &[Value]| match doc {
        Value::Object(map) => {
            if let Some(k) = &key
                && let Some(slot) = map.get_mut(k)
            {
                *slot = Value::Array(list.to_vec());
            }
        }
        Value::Array(slot) => *slot = list.to_vec(),
        _ => {}
    };
    // Binary-search the longest keeping prefix that fits.
    let (mut lo, mut hi) = (0usize, full.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        write_list(&mut doc, &full[..mid]);
        if size_of(&doc) <= MAX_RESPONSE_BYTES {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let items = full[..lo].to_vec();
    write_list(&mut doc, &items);
    if let Value::Object(map) = &mut doc {
        map.insert("truncated".to_string(), json!(true));
    }
    doc
}

// ---------------------------------------------------------------- cycle_get

/// Kap. 10.2 `cycle_get`: SCC detail for one cycle finding — member
/// nodes, internal edges with their merged import-site refs and the
/// suggested cuts carried by the finding.
pub fn cycle_get_value(report: &Report, view: &Graph, cycle_id: &str) -> Result<Value, ErrorData> {
    let finding = report
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::Cycle && f.id == cycle_id)
        .ok_or_else(|| invalid_params(format!("unknown cycle '{cycle_id}'")))?;
    let scc = find_sccs(view)
        .into_iter()
        .find(|scc| {
            scc.node_ids
                .iter()
                .filter_map(|id| view.node_name(*id))
                .eq(finding.nodes.iter().map(String::as_str))
        })
        .ok_or_else(|| invalid_params(format!("unknown cycle '{cycle_id}'")))?;
    let refs_of: BTreeMap<(u32, u32), &Vec<archguard_engine::core::Ref>> = view
        .edges
        .iter()
        .map(|edge| ((edge.from.0, edge.to.0), &edge.refs))
        .collect();
    let edges: Vec<Value> = scc
        .edges
        .iter()
        .map(|(from, to)| {
            let refs: Vec<Value> = refs_of
                .get(&(from.0, to.0))
                .map(|refs| {
                    refs.iter()
                        .map(|r| json!({"file": r.file, "line": r.line, "raw": r.raw}))
                        .collect()
                })
                .unwrap_or_default();
            json!({
                "from": view.node_name(*from).unwrap_or(""),
                "to": view.node_name(*to).unwrap_or(""),
                "refs": refs,
            })
        })
        .collect();
    let cuts: Vec<Value> = finding
        .suggested_cuts
        .iter()
        .map(
            |cut| json!({"from": cut.from, "to": cut.to, "score": cut.score, "reason": cut.reason}),
        )
        .collect();
    Ok(json!({
        "id": finding.id,
        "severity": finding.severity,
        "nodes": finding.nodes,
        "edge_count": finding.edge_count,
        "cycles_sampled": finding.cycles_sampled,
        "sampling": finding.sampling,
        "edges": edges,
        "suggested_cuts": cuts,
    }))
}

// -------------------------------------------------------------------- check

/// Small coverage summary for graph consumers. Full provider diagnostics live
/// in semantic_model; a large project must not overflow a graph tool response.
pub(crate) fn semantic_coverage_value(report: &Report) -> Value {
    let Some(coverage) = &report.semantic else {
        return Value::Null;
    };
    json!({
        "status": coverage.status.chars().take(32).collect::<String>(),
        "scope": "calls_and_inheritance",
        "projected_calls": coverage.projected_calls,
        "projected_supertypes": coverage.projected_supertypes,
        "skipped_project_calls": coverage.skipped_project_calls,
        "analyzer_count": coverage.analyzers.len(),
        "incomplete_analyzers": coverage.analyzers.iter().filter(|a| a.status != "ready" && a.status != "not_applicable").count(),
        "diagnostic_count": coverage.diagnostics.len(),
        "absence_of_cycles_proven": false,
        "limitation": "Includes imports plus resolved calls and inheritance. Type-only references are not covered; zero detected cycles is not proof of an acyclic codebase."
    })
}

/// Kap. 10.2 `check`: compact gate answer — status, summary counts,
/// findings count per kind and the first few findings. With a baseline,
/// cycle findings whose [`cycle_hash`] is accepted do not fail the gate
/// (same rule as `archguard check --baseline`).
pub fn check_value(report: &Report, baseline: Option<&Baseline>) -> Value {
    let accepted = |finding: &Finding| match (baseline, finding.kind) {
        (Some(baseline), FindingKind::Cycle) => {
            baseline.hashes.contains(&cycle_hash(&finding.nodes))
        }
        _ => false,
    };
    // Not `any(…)` — that short-circuits and would leave `counts`
    // incomplete once one Error fails the gate.
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut fails = false;
    for finding in &report.findings {
        *counts.entry(kind_name(finding.kind)).or_default() += 1;
        fails |= finding.severity == archguard_engine::core::Severity::Error && !accepted(finding);
    }
    let findings: Vec<Value> = report
        .findings
        .iter()
        .map(|finding| {
            let mut value = json!({
                "id": finding.id,
                "kind": finding.kind,
                "severity": finding.severity,
                "edge_count": finding.edge_count,
                "nodes": finding.nodes.iter().take(FINDING_SAMPLE).collect::<Vec<_>>(),
                "accepted_by_baseline": accepted(finding),
            });
            if let Some(message) = &finding.message {
                value["message"] = json!(message);
            }
            if !finding.evidence.is_empty() {
                value["evidence"] = json!(finding.evidence);
            }
            value
        })
        .collect();
    let mut summary = report.summary.clone();
    summary.status = if fails {
        archguard_engine::core::Status::Fail
    } else {
        archguard_engine::core::Status::Pass
    };
    json!({
        "status": if fails { "fail" } else { "pass" },
        "summary": summary,
        "semantic": semantic_coverage_value(report),
        "counts": counts,
        "findings": findings,
    })
}

/// The `snake_case` wire name of a [`FindingKind`] (its serde rename),
/// used as the key of the `check` counts map.
fn kind_name(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::Cycle => "cycle",
        FindingKind::LayerViolation => "layer_violation",
        FindingKind::MetricThreshold => "metric_threshold",
        FindingKind::LayerUnranked => "layer_unranked",
        FindingKind::DependencyRule => "dependency_rule",
        FindingKind::OwnershipRule => "ownership_rule",
        FindingKind::Orphan => "orphan",
        FindingKind::TargetArchitecture => "target_architecture",
    }
}

// ------------------------------------------------------------------ metrics

/// Kap. 10.2 `metrics`: top-`top` module rows by instability, then
/// fan-in+fan-out, then name — the CLI table as JSON. An unknown
/// `module` filter is an invalid-params error.
pub fn metrics_value(
    report: &Report,
    module: Option<&str>,
    top: usize,
) -> Result<Value, ErrorData> {
    if let Some(name) = module
        && !report.metrics.contains_key(name)
    {
        return Err(invalid_params(format!("unknown module '{name}'")));
    }
    let mut rows: Vec<(&str, &archguard_engine::core::ModuleMetrics)> = report
        .metrics
        .iter()
        .filter(|(name, _)| module.is_none_or(|wanted| name.as_str() == wanted))
        .map(|(name, metrics)| (name.as_str(), metrics))
        .collect();
    rows.sort_by(|a, b| {
        b.1.instability
            .partial_cmp(&a.1.instability)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then((b.1.fan_in + b.1.fan_out).cmp(&(a.1.fan_in + a.1.fan_out)))
            .then(a.0.cmp(b.0))
    });
    let total = rows.len();
    rows.truncate(top);
    let rows: Vec<Value> = rows
        .iter()
        .map(|(name, m)| {
            json!({
                "module": name,
                "fan_in": m.fan_in,
                "fan_out": m.fan_out,
                "instability": m.instability,
                "in_cycle": m.in_cycle,
            })
        })
        .collect();
    Ok(json!({ "total": total, "rows": rows }))
}

// -------------------------------------------------------- module_neighborhood

/// Traversal direction of [`neighborhood_value`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Both,
    In,
    Out,
}

/// Parse the optional `direction` parameter (default `"both"`).
pub fn parse_direction(direction: Option<&str>) -> Result<Direction, ErrorData> {
    match direction.unwrap_or("both") {
        "both" => Ok(Direction::Both),
        "in" => Ok(Direction::In),
        "out" => Ok(Direction::Out),
        other => Err(invalid_params(format!(
            "unknown direction {other:?}: expected \"both\", \"in\" or \"out\""
        ))),
    }
}

/// Kap. 10.2 `module_neighborhood`: the local-BFS context tool. Starting
/// at `module`, follow edges along `direction` for at most `depth`
/// hops; the answer lists the reached node names and every direction-
/// matching edge with both ends reached. BFS on [`Graph`] is local to
/// this crate on purpose (see the module docs).
pub fn neighborhood_value(
    view: &Graph,
    module: &str,
    depth: usize,
    direction: Direction,
) -> Result<Value, ErrorData> {
    if depth > MAX_DEPTH {
        return Err(invalid_params(format!(
            "depth {depth} too large: expected 0..={MAX_DEPTH}"
        )));
    }
    let id_of: BTreeMap<&str, NodeId> =
        view.nodes.iter().map(|n| (n.name.as_str(), n.id)).collect();
    let start = *id_of
        .get(module)
        .ok_or_else(|| invalid_params(format!("unknown module '{module}'")))?;
    // Frontier-based BFS: in round `d` only nodes reached in round `d-1`
    // expand, so `depth` means exactly "at most this many hops".
    let mut visited: BTreeSet<u32> = BTreeSet::new();
    visited.insert(start.0);
    let mut frontier: Vec<NodeId> = vec![start];
    for _ in 0..depth {
        let mut next: Vec<NodeId> = Vec::new();
        for edge in &view.edges {
            for (src, dst) in match direction {
                Direction::Out => vec![(edge.from, edge.to)],
                Direction::In => vec![(edge.to, edge.from)],
                Direction::Both => vec![(edge.from, edge.to), (edge.to, edge.from)],
            } {
                if frontier.contains(&src) && visited.insert(dst.0) {
                    next.push(dst);
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    let reached = |id: NodeId| visited.contains(&id.0);
    let name_of = |id: NodeId| view.node_name(id).unwrap_or("");
    let mut nodes: Vec<&str> = visited
        .iter()
        .filter_map(|raw| view.node_name(NodeId(*raw)))
        .collect();
    nodes.sort_unstable();
    let edges: Vec<Value> = view
        .edges
        .iter()
        .filter(|edge| reached(edge.from) && reached(edge.to))
        .map(|edge| {
            json!({
                "from": name_of(edge.from),
                "to": name_of(edge.to),
                "ref_count": edge.weight(),
            })
        })
        .collect();
    Ok(json!({
        "module": module,
        "depth": depth,
        "direction": format!("{direction:?}").to_lowercase(),
        "nodes": nodes,
        "edges": edges,
    }))
}

// -------------------------------------------------------------- edge_evidence

/// Kap. 10.2 `edge_evidence`: every import site (file, line, raw
/// statement) backing the module-view edge `from` → `to`. An edge
/// without evidence does not exist in the model → invalid params.
pub fn edge_evidence_value(view: &Graph, from: &str, to: &str) -> Result<Value, ErrorData> {
    let id_of: BTreeMap<&str, NodeId> =
        view.nodes.iter().map(|n| (n.name.as_str(), n.id)).collect();
    let (Some(&from_id), Some(&to_id)) = (id_of.get(from), id_of.get(to)) else {
        return Err(invalid_params(format!("unknown module '{from}' or '{to}'")));
    };
    let refs: Vec<Value> = view
        .edges
        .iter()
        .filter(|edge| edge.from == from_id && edge.to == to_id)
        .flat_map(|edge| edge.refs.iter())
        .map(|r| json!({"file": r.file, "line": r.line, "raw": r.raw}))
        .collect();
    if refs.is_empty() {
        return Err(invalid_params(format!("no edge {from} -> {to}")));
    }
    Ok(json!({ "from": from, "to": to, "refs": refs }))
}

// --------------------------------------------------------------- suggest_cuts

/// Kap. 10.2 `suggest_cuts`: the ranked [`archguard_engine::core::SuggestedCut`]s
/// the pipeline already attached to the cycle finding, capped at `max`.
pub fn suggest_cuts_value(
    report: &Report,
    cycle_id: &str,
    max: Option<usize>,
) -> Result<Value, ErrorData> {
    let finding = report
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::Cycle && f.id == cycle_id)
        .ok_or_else(|| invalid_params(format!("unknown cycle '{cycle_id}'")))?;
    let cuts: Vec<Value> = finding
        .suggested_cuts
        .iter()
        .take(max.unwrap_or(usize::MAX))
        .map(
            |cut| json!({"from": cut.from, "to": cut.to, "score": cut.score, "reason": cut.reason}),
        )
        .collect();
    Ok(json!({ "cycle_id": cycle_id, "cuts": cuts }))
}

// --------------------------------------------------------------- baseline_diff

/// Kap. 10.2 `baseline_diff`: current cycle findings vs a [`Baseline`]
/// file. Cycle identity is Kap. 7's [`cycle_hash`] over the finding's
/// sorted member names — the very hashes the baseline stores, so
/// `check --baseline` and this tool agree by construction (as in the
/// CLI's `baseline diff`).
pub fn baseline_diff_value(report: &Report, baseline_path: &Path) -> Result<Value, ErrorData> {
    let baseline = read_baseline(baseline_path)?;
    let current: Vec<(&String, String)> = report
        .findings
        .iter()
        .filter(|finding| finding.kind == FindingKind::Cycle)
        .map(|finding| (&finding.id, cycle_hash(&finding.nodes)))
        .collect();
    let current_hashes: BTreeSet<String> = current.iter().map(|(_, hash)| hash.clone()).collect();
    let new_cycles: Vec<Value> = current
        .iter()
        .filter(|(_, hash)| !baseline.hashes.contains(hash))
        .map(|(id, hash)| json!({"id": id, "hash": hash}))
        .collect();
    let removed_cycles: Vec<&String> = baseline
        .hashes
        .iter()
        .filter(|hash| !current_hashes.contains(*hash))
        .collect();
    Ok(json!({
        "baseline_file": baseline_path.display().to_string(),
        "current_cycles": current.len(),
        "new_cycles": new_cycles,
        "removed_cycles": removed_cycles,
        "status": if new_cycles.is_empty() { "ok" } else { "found" },
    }))
}

// ----------------------------------------------------------------- config_view

/// Kap. 10.2 `config_view`: the effective config's per-section
/// provenance from [`Report::config`] (produced by `analyze_effective`)
/// plus the layer list with its origin — declared rules (intent) vs
/// derived hypotheses.
///
/// The merged layer list is rebuilt from public APIs with the engine's
/// Kap. 6 step 7b semantics (`merge_layers`): declared entries first,
/// derived entries appended for names no declared entry claims — gated
/// by `derive_layers`. The derived depth mirrors the engine's private
/// `VIEW_MIN`/`VIEW_MAX` band (10..=100 view nodes).
pub fn config_view_value(
    _file_graph: &Graph,
    report: &Report,
    loaded: &LoadedConfig,
    section: Option<&str>,
) -> Result<Value, ErrorData> {
    let provenance: Value = match section {
        None => serde_json::to_value(&report.config)
            .expect("provenance is a string map and always serializable"),
        Some(wanted) => {
            let source = report.config.get(wanted).ok_or_else(|| {
                invalid_params(format!(
                    "unknown section '{wanted}': expected one of {}",
                    report.config.keys().cloned().collect::<Vec<_>>().join(", ")
                ))
            })?;
            json!({ wanted: source })
        }
    };
    let layers: Vec<Value> = loaded
        .config
        .layers
        .iter()
        .map(|layer| {
            layer_json(
                layer,
                match layer.origin {
                    LayerOrigin::Declared => "declared",
                    LayerOrigin::Derived => "derived",
                },
            )
        })
        .collect();
    Ok(json!({
        "provenance": provenance,
        "layers": layers,
        "derive_layers": loaded.config.derive_layers,
    }))
}

/// One `[[layers]]` entry as JSON; `origin` drives the agent's
/// intent-vs-hypothesis reading (Kap. 10.2).
fn layer_json(layer: &LayerConfig, origin: &str) -> Value {
    json!({
        "name": layer.name,
        "contains": layer.contains,
        "may_depend_on": layer.may_depend_on,
        "origin": origin,
    })
}

// View operations consume the engine's effective configuration.
fn effective_depth(_graph: &Graph, loaded: &LoadedConfig) -> u32 {
    loaded.config.modules.depth
}

fn effective_layers(_view: &Graph, loaded: &LoadedConfig) -> Vec<LayerConfig> {
    loaded.config.layers.clone()
}

/// Look a node up by its exact name (view node names are unique).
fn find_node(graph: &Graph, name: &str) -> Option<NodeId> {
    graph.nodes.iter().find(|n| n.name == name).map(|n| n.id)
}

/// The cut of the effective module view: [`Cut::at_depth`] at
/// [`effective_depth`].
fn effective_cut(graph: &Graph, loaded: &LoadedConfig) -> Cut {
    Cut::at_depth(graph, effective_depth(graph, loaded) as usize)
}

/// Zoom `base` in by every named module in order; `Err(name)` for the
/// first unknown name (the caller turns that into -32602). Expanding an
/// invisible or childless node is a no-op of [`Cut::expand`] itself and
/// stays one here.
fn expand_cut(graph: &Graph, base: &Cut, names: &[String]) -> Result<Cut, String> {
    let mut cut = Cut::new(base.nodes().iter().copied());
    for name in names {
        let id = find_node(graph, name).ok_or_else(|| name.clone())?;
        cut = cut.expand(graph, id);
    }
    Ok(cut)
}

/// The cut showing exactly the direct children of `parent`.
fn within_cut(graph: &Graph, parent: NodeId) -> Cut {
    Cut::new(graph.children_of(parent))
}

/// Precomputed arguments of the T38 DSM renderer: the reverse
/// topological order of the condensation (sinks first, cycle-group
/// members adjacent) and the set of edges inside cyclic components —
/// same recipe as the CLI's `view_util::dsm_layout` (the report crate
/// must not know about SCCs). Groups are the cyclic SCCs plus one
/// singleton per remaining node; Kahn runs on the condensation's
/// OUT-degree, ties break on the smallest member name, then on group
/// index. The condensation is a DAG, so the loop always empties; a
/// defensive tail keeps stray ids from vanishing.
fn dsm_layout(view: &Graph) -> (Vec<NodeId>, HashSet<(NodeId, NodeId)>) {
    let sccs = find_sccs(view);
    let mut groups: Vec<Vec<NodeId>> = Vec::new();
    let mut group_of: HashMap<NodeId, usize> = HashMap::new();
    for scc in &sccs {
        let gid = groups.len();
        for &id in &scc.node_ids {
            group_of.insert(id, gid);
        }
        groups.push(scc.node_ids.clone());
    }
    for node in &view.nodes {
        if let Entry::Vacant(entry) = group_of.entry(node.id) {
            let gid = groups.len();
            entry.insert(gid);
            groups.push(vec![node.id]);
        }
    }

    // Condensation: group → groups it depends on.
    let mut depends: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); groups.len()];
    for edge in &view.edges {
        if let (Some(&a), Some(&b)) = (group_of.get(&edge.from), group_of.get(&edge.to))
            && a != b
        {
            depends[a].insert(b);
        }
    }
    let mut pending: Vec<usize> = depends.iter().map(|set| set.len()).collect();

    let sort_key = |gid: usize| -> (String, usize) {
        let smallest = groups[gid]
            .iter()
            .filter_map(|id| view.node_name(*id))
            .min()
            .unwrap_or("?")
            .to_string();
        (smallest, gid)
    };

    let mut emitted = vec![false; groups.len()];
    let mut order: Vec<NodeId> = Vec::new();
    loop {
        let next = (0..groups.len())
            .filter(|&g| !emitted[g] && pending[g] == 0)
            .map(|g| (sort_key(g), g))
            .min()
            .map(|(_, g)| g);
        let Some(gid) = next else { break };
        emitted[gid] = true;
        order.extend(groups[gid].iter().copied());
        for p in 0..groups.len() {
            if !emitted[p] && depends[p].contains(&gid) {
                pending[p] -= 1;
            }
        }
    }
    // Defensive tail (unreachable for a real DAG condensation).
    for gid in 0..groups.len() {
        if !emitted[gid] {
            order.extend(groups[gid].iter().copied());
        }
    }

    let cyclic: HashSet<(NodeId, NodeId)> = sccs.iter().flat_map(|s| s.edges.clone()).collect();
    (order, cyclic)
}

// ------------------------------------------------------- T40 tool builders

/// Wire form of the frozen T37 [`SimOp`] enum for the `simulate` tool:
/// externally tagged, snake_case — the exact shape the T39 viewer's
/// "Export SimOps" button writes and `archguard simulate --ops` parses,
/// e.g. `{"cut_edge":{"from":"a","to":"b"}}`. A separate type (instead
/// of a direct `SimOp` parameter) because the analysis crate does not
/// derive `JsonSchema`.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SimOpWire {
    /// Drop the dependency edge `from → to` (subtree semantics).
    CutEdge { from: String, to: String },
    /// Re-parent `node` under the module `new_parent`.
    MoveNode { node: String, new_parent: String },
    /// Fold `absorbed` into `survivor`.
    Merge { survivor: String, absorbed: String },
}

impl SimOpWire {
    /// Convert to the analysis crate's op (identity on the fields).
    pub fn to_sim_op(&self) -> SimOp {
        match self {
            SimOpWire::CutEdge { from, to } => SimOp::CutEdge {
                from: from.clone(),
                to: to.clone(),
            },
            SimOpWire::MoveNode { node, new_parent } => SimOp::MoveNode {
                node: node.clone(),
                new_parent: new_parent.clone(),
            },
            SimOpWire::Merge { survivor, absorbed } => SimOp::Merge {
                survivor: survivor.clone(),
                absorbed: absorbed.clone(),
            },
        }
    }
}

/// `graph_expand` (T40): the direct children of `node` plus the
/// aggregated edges of the resulting zoomed view. Stateless — the
/// client holds the list of already-expanded module names and passes it
/// as `expanded`; the cut starts at the effective zoom, applies
/// `expanded` in order and then opens `node` itself, so the answer
/// renders exactly the drill-down the client draws. Unknown names are
/// -32602. The key names are chosen so [`guarded`] shrinks `"edges"`
/// first (alphabetically before `"node"`/`"nodes"`).
pub fn graph_expand_value(
    file_graph: &Graph,
    loaded: &LoadedConfig,
    node: &str,
    expanded: &[String],
) -> Result<Value, ErrorData> {
    let base = effective_cut(file_graph, loaded);
    let cut = expand_cut(file_graph, &base, expanded)
        .map_err(|name| invalid_params(format!("unknown module '{name}' for 'expanded'")))?;
    let id = find_node(file_graph, node)
        .ok_or_else(|| invalid_params(format!("unknown module '{node}'")))?;
    let cut = cut.expand(file_graph, id);
    let view = aggregate(file_graph, &cut);
    let visible: HashSet<&str> = view.nodes.iter().map(|n| n.name.as_str()).collect();
    let mut nodes: Vec<String> = file_graph
        .children_of(id)
        .iter()
        .filter_map(|&child| file_graph.node_name(child))
        .filter(|name| visible.contains(*name))
        .map(str::to_string)
        .collect();
    nodes.sort();
    let mut edges: Vec<(String, String, u64)> = view
        .edges
        .iter()
        .filter_map(|edge| {
            let from = view.node_name(edge.from)?;
            let to = view.node_name(edge.to)?;
            Some((from.to_string(), to.to_string(), edge.weight()))
        })
        .collect();
    edges.sort();
    Ok(json!({
        "node": node,
        "nodes": nodes,
        "edges": edges
            .into_iter()
            .map(|(from, to, ref_count)| json!({"from": from, "to": to, "ref_count": ref_count}))
            .collect::<Vec<Value>>(),
    }))
}

/// `dsm` (T40): the T38 matrix of the effective module view — the same
/// cut [`graph_expand_value`] and the CLI's `dsm` command use — or,
/// with `within`, exactly the direct children of that module, again
/// optionally zoomed by `expanded`. The response reuses the frozen T38
/// JSON keys (`order`, `cells`), so [`guarded`] shrinks the (larger)
/// `cells` array first. Unknown module names are -32602.
pub fn dsm_value(
    file_graph: &Graph,
    loaded: &LoadedConfig,
    expanded: &[String],
    within: Option<&str>,
) -> Result<Value, ErrorData> {
    let base = match within {
        Some(name) => match find_node(file_graph, name) {
            Some(id) => within_cut(file_graph, id),
            None => {
                return Err(invalid_params(format!(
                    "unknown module '{name}' for 'within'"
                )));
            }
        },
        None => effective_cut(file_graph, loaded),
    };
    let cut = expand_cut(file_graph, &base, expanded)
        .map_err(|name| invalid_params(format!("unknown module '{name}' for 'expanded'")))?;
    let view = aggregate(file_graph, &cut);
    let (order, cyclic) = dsm_layout(&view);
    let text = to_dsm_json(&view, &order, &cyclic, false);
    serde_json::from_str(&text)
        .map_err(|err| ErrorData::internal_error(format!("dsm json: {err}"), None))
}

/// `simulate` (T40): apply the virtual refactoring ops to the file
/// graph and hand back the frozen [`archguard_engine::analysis::SimDelta`]
/// — the diff of the before/after analyses of the effective view (the
/// graph on disk is never touched). Ops resolve against the FULL graph
/// (module or file names), so the view only decides what the analysis
/// compares. An empty op list is -32602, mirroring the CLI's usage
/// error; unparsable ops (unknown endpoints, illegal merge) are
/// -32602 as well.
pub fn simulate_value(
    file_graph: &Graph,
    loaded: &LoadedConfig,
    ops: &[SimOp],
) -> Result<Value, ErrorData> {
    if ops.is_empty() {
        return Err(invalid_params("no ops: pass at least one SimOp"));
    }
    let cut = effective_cut(file_graph, loaded);
    let view = aggregate(file_graph, &cut);
    let layers = effective_layers(&view, loaded);
    let delta = simulate(file_graph, &cut, ops, &layers)
        .map_err(|err| invalid_params(format!("simulate: {err}")))?;
    serde_json::to_value(&delta)
        .map_err(|err| ErrorData::internal_error(format!("simdelta: {err}"), None))
}

/// `trend` (T41): the snapshot history of `root` as a JSON page —
/// `{"count": N, "snapshots": [...]}`, the same shape the CLI's
/// `trend --format json` prints. Reading `.archguard/history/` needs
/// no analysis and no config; a missing history directory is an empty
/// list, not an error. `last == 0` means "all rows" (see
/// [`archguard_engine::read_trend`]).
pub fn trend_value(root: &Path, last: usize) -> Value {
    let rows = archguard_engine::read_trend(root, last);
    json!({
        "count": rows.len(),
        "snapshots": serde_json::to_value(&rows)
            .expect("snapshot rows are plain data and always serializable"),
    })
}

// --------------------------------------------------------------------- tests

impl crate::server::ArchguardTools {
    /// Resolve the effective root (override or server root) through the
    /// one F02 path policy, load the configuration and run the cached
    /// analysis (Kap. 10.1: every tool scans itself).
    pub(crate) fn analyze(
        &self,
        path: Option<&str>,
        level: Level,
    ) -> Result<
        (
            archguard_engine::core::Graph,
            archguard_engine::core::Report,
        ),
        rmcp::ErrorData,
    > {
        let root = resolve_root(&self.root, path)?;
        let loaded = archguard_engine::load_config(None, &root)
            .map_err(|err| rmcp::ErrorData::internal_error(format!("config error: {err}"), None))?;
        archguard_engine::analyze_run(&root, &loaded, level)
            .map(|run| (run.graph, run.report))
            .map_err(|err| rmcp::ErrorData::internal_error(format!("analysis failed: {err}"), None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::ConfigSource;
    use archguard_engine::core::{GraphBuilder, NodeKind, Ref, Severity, Summary};

    fn guardable_doc(items: usize) -> Value {
        let cycles: Vec<Value> = (0..items)
            .map(|i| {
                json!({
                    "id": format!("scc-{i}"),
                    "size": 3,
                    "sample_nodes": ["app/one", "app/two", "domain/three"],
                    "edge_count": 3,
                })
            })
            .collect();
        json!({ "total": items, "truncated": false, "cycles": cycles })
    }

    #[test]
    fn guard_keeps_small_results_untouched() {
        let doc = guardable_doc(3);
        let guarded = guarded(&doc);
        assert_eq!(guarded, doc, "a small answer must pass through unchanged");
        assert_eq!(guarded["truncated"], json!(false));
    }

    #[test]
    fn guard_truncates_oversized_list_and_flags_it() {
        let doc = guardable_doc(5000);
        let bytes = serde_json::to_vec(&doc).unwrap().len();
        assert!(bytes > MAX_RESPONSE_BYTES, "fixture must be oversized");
        let guarded = guarded(&doc);
        let out = serde_json::to_vec(&guarded).unwrap();
        assert!(
            out.len() <= MAX_RESPONSE_BYTES,
            "guarded answer is {} bytes",
            out.len()
        );
        assert_eq!(guarded["truncated"], json!(true));
        let kept = guarded["cycles"].as_array().unwrap().len();
        assert!(kept > 0 && kept < 5000, "kept {kept} of 5000 items");
        assert_eq!(guarded["total"], json!(5000), "totals stay truthful");
        // The kept prefix is the highest-signal one (builders sort first).
        assert_eq!(guarded["cycles"][0]["id"], json!("scc-0"));
    }

    #[test]
    fn guard_marks_unshrinkable_documents() {
        let doc = json!({ "msg": "x".repeat(10 * 1024) });
        let guarded = guarded(&doc);
        assert_eq!(guarded["truncated"], json!(true));
    }

    fn graph_from(modules: &[&str], edges: &[(&str, &str)]) -> Graph {
        let mut builder = GraphBuilder::new();
        let mut ids = BTreeMap::new();
        for module in modules {
            ids.insert(
                *module,
                builder.ensure_path(&format!("{module}/x.ts"), None),
            );
        }
        for (from, to) in edges {
            let r = Ref {
                file: format!("{from}/x.ts"),
                line: 1,
                raw: format!("import … from \"{to}\""),
            };
            builder
                .edge(ids[from], ids[to], r)
                .expect("edge between known nodes");
        }
        builder.build("root".to_string())
    }

    fn view_of(edges: &[(&str, &str)]) -> Graph {
        // Files hang one level below their root module, so the depth-0
        // cut makes the module itself the visible node and aggregates
        // each module's files and edges into it.
        let full = graph_from(&["app", "domain", "infra"], edges);
        module_view(&full, 0)
    }

    #[test]
    fn neighborhood_bfs_both_depth_one_and_direction_filters() {
        let view = view_of(&[("app", "domain"), ("infra", "app"), ("domain", "infra")]);
        let both = neighborhood_value(&view, "app", 1, Direction::Both).unwrap();
        assert_eq!(
            both["nodes"],
            json!(["app", "domain", "infra"]),
            "both direction reaches in- and out-neighbours"
        );
        let out = neighborhood_value(&view, "app", 1, Direction::Out).unwrap();
        assert_eq!(out["nodes"], json!(["app", "domain"]));
        assert_eq!(
            out["edges"],
            json!([{"from": "app", "to": "domain", "ref_count": 1}])
        );
        let in_ = neighborhood_value(&view, "app", 1, Direction::In).unwrap();
        assert_eq!(in_["nodes"], json!(["app", "infra"]));
        let deep = neighborhood_value(&view, "app", 2, Direction::Out).unwrap();
        assert_eq!(deep["nodes"], json!(["app", "domain", "infra"]));
    }

    #[test]
    fn neighborhood_rejects_unknown_module_and_bad_depth() {
        let view = view_of(&[("app", "domain")]);
        assert!(
            neighborhood_value(&view, "nope", 1, Direction::Both)
                .unwrap_err()
                .message
                .contains("unknown module 'nope'")
        );
        assert!(
            neighborhood_value(&view, "app", MAX_DEPTH + 1, Direction::Both)
                .unwrap_err()
                .message
                .contains("too large")
        );
    }

    #[test]
    fn edge_evidence_returns_refs_or_errors() {
        let view = view_of(&[("app", "domain")]);
        let value = edge_evidence_value(&view, "app", "domain").unwrap();
        assert_eq!(value["refs"][0]["file"], json!("app/x.ts"));
        assert_eq!(value["refs"][0]["line"], json!(1));
        assert!(value["refs"][0]["raw"].as_str().unwrap().contains("import"));
        assert!(edge_evidence_value(&view, "domain", "app").is_err());
        assert!(edge_evidence_value(&view, "app", "ghost").is_err());
    }

    fn report_with(findings: Vec<Finding>) -> Report {
        Report {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            summary: Summary {
                nodes: 3,
                edges: 3,
                sccs: 1,
                largest_scc: 3,
                layer_violations: 0,
                status: archguard_engine::core::Status::Fail,
                ccd: 0,
                acd: 0.0,
                nccd: 0.0,
            },
            config: BTreeMap::new(),
            findings,
            metrics: BTreeMap::new(),
        }
    }

    #[test]
    fn check_keeps_semantic_limitations_bounded_even_without_cycle_findings() {
        let mut report = report_with(Vec::new());
        report.summary.sccs = 0;
        report.semantic = Some(archguard_engine::core::SemanticDependencyCoverage {
            status: "partial".into(),
            projected_calls: 7,
            skipped_project_calls: 2,
            diagnostics: vec!["x".repeat(10_000); 100],
            ..Default::default()
        });
        let answer = check_value(&report, None);
        assert_eq!(answer["semantic"]["status"], "partial");
        assert_eq!(answer["semantic"]["scope"], "calls_and_inheritance");
        assert_eq!(answer["semantic"]["projected_calls"], 7);
        assert_eq!(answer["semantic"]["skipped_project_calls"], 2);
        assert_eq!(answer["semantic"]["absence_of_cycles_proven"], false);
        assert_eq!(answer["semantic"]["diagnostic_count"], 100);
        assert!(size_of(&answer) < MAX_RESPONSE_BYTES);
    }

    fn cycle_finding(id: &str, nodes: &[&str]) -> Finding {
        Finding {
            message: None,
            evidence: Vec::new(),
            kind: FindingKind::Cycle,
            id: id.to_string(),
            severity: Severity::Error,
            nodes: nodes.iter().map(|n| n.to_string()).collect(),
            edge_count: nodes.len() as u32,
            cycles_sampled: 1,
            sampling: None,
            suggested_cuts: vec![archguard_engine::core::SuggestedCut {
                from: "app".to_string(),
                to: "domain".to_string(),
                score: 0.75,
                reason: "breaks the sampled cycle".to_string(),
            }],
        }
    }

    #[test]
    fn cycle_get_and_suggest_cuts_shape_and_errors() {
        let view = view_of(&[("app", "domain"), ("domain", "app")]);
        let report = report_with(vec![cycle_finding("scc-1", &["app", "domain"])]);
        let detail = cycle_get_value(&report, &view, "scc-1").unwrap();
        assert_eq!(detail["nodes"], json!(["app", "domain"]));
        assert_eq!(detail["edges"].as_array().unwrap().len(), 2);
        assert_eq!(detail["edges"][0]["refs"][0]["file"], json!("app/x.ts"));
        assert_eq!(detail["suggested_cuts"][0]["score"], json!(0.75));
        assert!(cycle_get_value(&report, &view, "scc-9").is_err());

        let cuts = suggest_cuts_value(&report, "scc-1", Some(1)).unwrap();
        assert_eq!(cuts["cuts"].as_array().unwrap().len(), 1);
        assert!(suggest_cuts_value(&report, "scc-9", None).is_err());
    }

    #[test]
    fn check_counts_kinds_and_baseline_accepts_cycles() {
        let report = report_with(vec![
            cycle_finding("scc-1", &["app", "domain"]),
            Finding {
                message: None,
                evidence: Vec::new(),
                kind: FindingKind::LayerViolation,
                id: "layer-1".to_string(),
                severity: Severity::Error,
                nodes: vec!["app".to_string()],
                edge_count: 1,
                cycles_sampled: 0,
                sampling: None,
                suggested_cuts: Vec::new(),
            },
        ]);
        let plain = check_value(&report, None);
        assert_eq!(plain["status"], json!("fail"));
        assert_eq!(plain["counts"]["cycle"], json!(1));
        assert_eq!(plain["counts"]["layer_violation"], json!(1));
        assert_eq!(plain["findings"].as_array().unwrap().len(), 2);

        // Accepting only the cycle still fails (layer violation left)…
        let accepted = Baseline {
            schema_version: 1,
            hashes: BTreeSet::from([cycle_hash(&["app".to_string(), "domain".to_string()])]),
        };
        let with_baseline = check_value(&report, Some(&accepted));
        assert_eq!(with_baseline["status"], json!("fail"));
        assert_eq!(
            with_baseline["findings"][0]["accepted_by_baseline"],
            json!(true)
        );

        // …accepting it and dropping the other finding passes.
        let only_cycle = report_with(vec![cycle_finding("scc-1", &["app", "domain"])]);
        assert_eq!(
            check_value(&only_cycle, Some(&accepted))["status"],
            json!("pass")
        );
    }

    #[test]
    fn metrics_sorts_by_instability_and_filters_module() {
        let mut report = report_with(Vec::new());
        report.metrics = BTreeMap::from([
            (
                "core".to_string(),
                archguard_engine::core::ModuleMetrics {
                    fan_in: 3,
                    fan_out: 0,
                    instability: 0.0,
                    in_cycle: false,
                },
            ),
            (
                "ui".to_string(),
                archguard_engine::core::ModuleMetrics {
                    fan_in: 0,
                    fan_out: 5,
                    instability: 1.0,
                    in_cycle: true,
                },
            ),
        ]);
        let all = metrics_value(&report, None, 10).unwrap();
        assert_eq!(all["rows"][0]["module"], json!("ui"));
        assert_eq!(all["rows"][1]["module"], json!("core"));
        let one = metrics_value(&report, Some("core"), 10).unwrap();
        assert_eq!(one["rows"].as_array().unwrap().len(), 1);
        assert!(metrics_value(&report, Some("ghost"), 10).is_err());
    }

    #[test]
    fn baseline_diff_lists_added_and_removed() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("archguard-mcp-t27-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("baseline.json");
        let kept = cycle_hash(&["app".to_string(), "domain".to_string()]);
        let stale = cycle_hash(&["old".to_string()]);
        let baseline = Baseline {
            schema_version: 1,
            hashes: BTreeSet::from([kept, stale.clone()]),
        };
        std::fs::write(&path, serde_json::to_string(&baseline).unwrap()).unwrap();

        let report = report_with(vec![
            cycle_finding("scc-1", &["app", "domain"]),
            cycle_finding("scc-2", &["a", "b", "c"]),
        ]);
        let diff = baseline_diff_value(&report, &path).unwrap();
        assert_eq!(diff["status"], json!("found"));
        assert_eq!(diff["new_cycles"][0]["id"], json!("scc-2"));
        assert_eq!(diff["removed_cycles"][0], json!(stale));
        assert_eq!(diff["current_cycles"], json!(2));

        // Missing file is an error, not a panic.
        assert!(baseline_diff_value(&report, &dir.join("ghost.json")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_view_preserves_effective_layer_origins() {
        let file_graph = graph_from(
            &["app", "domain", "infra"],
            &[("app", "domain"), ("domain", "infra")],
        );
        let mut report = report_with(Vec::new());
        report.config = BTreeMap::from([
            ("project".to_string(), ConfigSource::Derived),
            ("modules".to_string(), ConfigSource::Derived),
            ("layers".to_string(), ConfigSource::Derived),
        ]);
        let loaded = LoadedConfig {
            config: archguard_engine::core::Config {
                derive_layers: true,
                layers: vec![
                    LayerConfig {
                        name: "ui".to_string(),
                        contains: vec!["app/*".to_string()],
                        may_depend_on: vec!["core".to_string()],
                        origin: Default::default(),
                    },
                    LayerConfig {
                        name: "core".into(),
                        contains: vec!["infra/**".into()],
                        may_depend_on: vec![],
                        origin: LayerOrigin::Derived,
                    },
                ],
                ..Default::default()
            },
            sources: BTreeMap::new(),
        };
        let value = config_view_value(&file_graph, &report, &loaded, None).unwrap();
        assert_eq!(value["provenance"]["layers"], json!("derived"));
        let names: Vec<&str> = value["layers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["origin"].as_str().unwrap())
            .collect();
        assert_eq!(names[0], "declared", "declared layers come first");
        assert!(
            names[1..].iter().all(|o| *o == "derived"),
            "appended entries are derived hypotheses: {names:?}"
        );
        assert!(
            value["layers"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|l| l["origin"] == json!("derived"))
                .all(|l| l["name"] != "ui"),
            "derived entries never override a declared name"
        );
        // Section filter + unknown section error.
        let one = config_view_value(&file_graph, &report, &loaded, Some("layers")).unwrap();
        assert_eq!(one["provenance"], json!({"layers": "derived"}));
        assert!(config_view_value(&file_graph, &report, &loaded, Some("nope")).is_err());
    }

    #[test]
    fn parse_direction_accepts_documented_values() {
        assert_eq!(parse_direction(None).unwrap(), Direction::Both);
        assert_eq!(parse_direction(Some("in")).unwrap(), Direction::In);
        assert_eq!(parse_direction(Some("out")).unwrap(), Direction::Out);
        assert!(parse_direction(Some("sideways")).is_err());
    }

    #[test]
    fn node_kinds_survive_the_view_aggregation() {
        // Guards the assumption behind `module_view`: at depth 1 the
        // module directories become the graph's nodes.
        let view = view_of(&[("app", "domain")]);
        assert!(view.nodes.iter().all(|n| n.kind == NodeKind::Module));
        assert!(view.nodes.iter().any(|n| n.name == "app"));
    }

    /// Declared `[modules] depth = 0` so the T40 helpers use the
    /// module-level cut deterministically (roots are depth 0, so the
    /// module itself is the visible node).
    fn loaded_declared() -> LoadedConfig {
        let mut config = archguard_engine::core::Config::default();
        config.modules.depth = 0;
        LoadedConfig {
            config,
            sources: BTreeMap::from([("modules".to_string(), ConfigSource::Declared)]),
        }
    }

    #[test]
    fn graph_expand_reveals_children_and_aggregates_edges() {
        let full = graph_from(&["app", "domain"], &[("app", "domain")]);
        let loaded = loaded_declared();
        let value = graph_expand_value(&full, &loaded, "app", &[]).unwrap();
        assert_eq!(value["node"], json!("app"));
        assert_eq!(
            value["nodes"],
            json!(["app/x.ts"]),
            "the child replaces the module in the zoomed view"
        );
        assert_eq!(
            value["edges"],
            json!([{"from": "app/x.ts", "to": "domain", "ref_count": 1}]),
            "the module edge is re-projected onto the zoomed view"
        );
        // Stateless drill-down: the client-held `expanded` list stays
        // open while `node` is opened on top.
        let value = graph_expand_value(&full, &loaded, "app", &["domain".to_string()]).unwrap();
        assert_eq!(value["nodes"], json!(["app/x.ts"]));
        assert_eq!(
            value["edges"],
            json!([{"from": "app/x.ts", "to": "domain/x.ts", "ref_count": 1}]),
            "both sides are zoomed"
        );
        // Unknown names are invalid params, not empty answers.
        assert!(graph_expand_value(&full, &loaded, "ghost", &[]).is_err());
        assert!(graph_expand_value(&full, &loaded, "app", &["ghost".to_string()]).is_err());
    }

    #[test]
    fn dsm_value_marks_cycle_cells_and_validates_names() {
        let full = graph_from(
            &["app", "domain", "infra"],
            &[("app", "domain"), ("domain", "app")],
        );
        let loaded = loaded_declared();
        let value = dsm_value(&full, &loaded, &[], None).unwrap();
        let mut order: Vec<&str> = value["order"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        order.sort_unstable();
        assert_eq!(order, ["app", "domain", "infra"]);
        // Both cycle directions appear; row/column numbering follows
        // the reverse topological `order`, which is symmetric here.
        let mut cells: Vec<Vec<serde_json::Value>> = value["cells"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_array().unwrap().clone())
            .collect();
        cells.sort_by(|a, b| {
            (a[0].as_u64().unwrap(), a[1].as_u64().unwrap())
                .cmp(&(b[0].as_u64().unwrap(), b[1].as_u64().unwrap()))
        });
        assert_eq!(
            cells,
            vec![
                vec![json!(0), json!(1), json!(1), json!(true)],
                vec![json!(1), json!(0), json!(1), json!(true)]
            ],
            "the cycle's cells carry the cyclic flag"
        );
        assert!(dsm_value(&full, &loaded, &[], Some("ghost")).is_err());
        assert!(dsm_value(&full, &loaded, &["ghost".to_string()], None).is_err());
        // `within` shows exactly the direct children of the module.
        let value = dsm_value(&full, &loaded, &[], Some("app")).unwrap();
        assert_eq!(value["order"], json!(["app/x.ts"]));
        assert_eq!(value["cells"], json!([]));
    }

    #[test]
    fn simulate_value_cuts_the_cycle_and_reports_the_delta() {
        let full = graph_from(
            &["app", "domain", "infra"],
            &[("app", "domain"), ("domain", "app")],
        );
        let mut loaded = loaded_declared();
        // No derived layers: the delta is purely about the one SCC, so
        // the positional ids are stable (single cycle, no renumbering).
        loaded.config.derive_layers = false;
        let ops = vec![
            SimOpWire::CutEdge {
                from: "app".to_string(),
                to: "domain".to_string(),
            }
            .to_sim_op(),
        ];
        let delta = simulate_value(&full, &loaded, &ops).unwrap();
        assert_eq!(delta["resolved_findings"], json!(["scc-1"]));
        assert_eq!(delta["introduced_findings"], json!([]));
        assert_eq!(delta["scc_count_before"], json!(1));
        assert_eq!(delta["scc_count_after"], json!(0));
        assert!(
            delta["acd_after"].as_f64().unwrap() <= delta["acd_before"].as_f64().unwrap(),
            "cutting a cycle edge must not worsen ACD"
        );
        // Ops resolve against the FULL graph: the file-level cut works.
        let file_ops = vec![SimOp::CutEdge {
            from: "app/x.ts".to_string(),
            to: "domain/x.ts".to_string(),
        }];
        let delta = simulate_value(&full, &loaded, &file_ops).unwrap();
        assert_eq!(delta["scc_count_after"], json!(0));
        assert!(
            simulate_value(&full, &loaded, &[]).is_err(),
            "empty op list is rejected"
        );
        assert!(
            simulate_value(
                &full,
                &loaded,
                &[SimOp::CutEdge {
                    from: "ghost".to_string(),
                    to: "app".to_string(),
                }]
            )
            .is_err(),
            "unknown endpoints are invalid params"
        );
    }

    #[test]
    fn sim_op_wire_matches_the_frozen_t37_shape() {
        const WIRE: &str = r#"[{"cut_edge":{"from":"a","to":"b"}},
            {"move_node":{"node":"x","new_parent":"p"}},
            {"merge":{"survivor":"a","absorbed":"b"}}]"#;
        let ops: Vec<SimOpWire> = serde_json::from_str(WIRE).unwrap();
        assert_eq!(
            ops.iter().map(SimOpWire::to_sim_op).collect::<Vec<_>>(),
            vec![
                SimOp::CutEdge {
                    from: "a".to_string(),
                    to: "b".to_string(),
                },
                SimOp::MoveNode {
                    node: "x".to_string(),
                    new_parent: "p".to_string(),
                },
                SimOp::Merge {
                    survivor: "a".to_string(),
                    absorbed: "b".to_string(),
                },
            ]
        );
        // The same text parses as the frozen SimOp type itself.
        assert_eq!(
            serde_json::from_str::<Vec<SimOp>>(WIRE).unwrap(),
            ops.iter().map(SimOpWire::to_sim_op).collect::<Vec<_>>()
        );
        assert!(serde_json::from_str::<SimOpWire>("{\"cut_edge\":{\"from\":\"a\"}}").is_err());
    }

    /// T41: `trend_value` pages the history exactly like the CLI's
    /// `trend --format json` — oldest first, `last` cuts to the newest
    /// rows, and a missing history is an empty page, not an error.
    #[test]
    fn trend_value_pages_the_snapshot_history() {
        let dir = tempfile::tempdir().expect("tempdir");
        let history = dir.path().join(".archguard/history");
        std::fs::create_dir_all(&history).expect("history dir");
        for (timestamp, edges) in [(100_u64, 7_u32), (200, 9)] {
            let snapshot = archguard_engine::Snapshot {
                analysis_scope: None,
                schema_version: 1,
                timestamp,
                git_sha: "abc1234".to_string(),
                tag: (timestamp == 200).then(|| "v1".to_string()),
                nodes: 5,
                edges,
                sccs: 2,
                largest_scc: 3,
                acd: 0.5,
                nccd: 0.25,
                cycle_hashes: vec![],
                review_kpis: None,
            };
            std::fs::write(
                history.join(format!("{timestamp}-abc1234.json")),
                serde_json::to_string(&snapshot).expect("serialize snapshot"),
            )
            .expect("write snapshot file");
        }
        let all = trend_value(dir.path(), 0);
        assert_eq!(all["count"], 2);
        let rows = all["snapshots"].as_array().expect("snapshots array");
        assert_eq!(rows[0]["timestamp"], 100, "oldest first");
        assert_eq!(rows[0]["tag"], Value::Null, "first row is untagged");
        assert_eq!(rows[1]["tag"], "v1", "tag survives the round trip");
        assert_eq!(rows[1]["edges"], 9);
        let newest = trend_value(dir.path(), 1);
        assert_eq!(newest["count"], 1);
        assert_eq!(
            newest["snapshots"][0]["timestamp"], 200,
            "--last keeps newest"
        );
        let missing = trend_value(Path::new("/definitely-not-an-archguard-project"), 0);
        assert_eq!(missing["count"], 0, "missing history is an empty page");
    }
}
