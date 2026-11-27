//! Definition files `*.json`, `*.yml`, `*.yaml` classified by key shape (ADR 0023, history
//! 2026-11-25; rules of the shapes in `json.rs`). Name = file stem, `scope` = `json:<folder>`.
//! A file without ES shape yields nothing and no gap; one that does not parse is a gap only under
//! `elasticsearch/`, `opensearch/`, `mappings/` or with `template` in its name. Search templates
//! (`"lang": "mustache"`) stay with `search_templates`. Trust boundary: files below the root read
//! through the safe reader (no symlinks, 1 MiB), at most 5 000 files, no test paths; no network.
use super::json::{Definition, search_definition};
use super::search_templates::mustache_lang;
use crate::context::{SearchContext, SourceOutput};
use crate::findings::{EsGap, es_gap};
use archguard_core::{SearchArtifact, ServiceEvidence, ServiceTopology};
use archguard_flow_common::support::{relative, test_path};
use serde_json::Value;
use std::path::Path;

const MAX_FILES: usize = 5_000;
const SKIPPED_DIRS: &[&str] = &[".terraform", ".git", "node_modules"];

pub(crate) fn extract_with(ctx: &SearchContext) -> SourceOutput {
    let mut out = ServiceTopology::default();
    let mut files = candidates(ctx);
    if files.len() > MAX_FILES {
        files.truncate(MAX_FILES);
        es_gap(&mut out, EsGap::DefinitionScanStop, ".", None, &[]);
    }
    for rel in files {
        read_file(ctx, &rel, &mut out);
    }
    SourceOutput::from_scratch(out)
}

fn candidates(ctx: &SearchContext) -> Vec<String> {
    let mut files: Vec<String> = ctx
        .discovery
        .walk(ctx.root)
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| relative(ctx.root, e.path()))
        .filter(|rel| matches!(ext(rel), "json" | "yml" | "yaml"))
        .filter(|rel| !test_path(rel) && !rel.split('/').any(|p| SKIPPED_DIRS.contains(&p)))
        .collect();
    files.sort();
    files
}

fn ext(rel: &str) -> &str {
    Path::new(rel)
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
}

/// Directories and file names where a broken definition is worth a gap.
fn gap_path(rel: &str) -> bool {
    let (dirs, file) = rel.rsplit_once('/').unwrap_or(("", rel));
    file.contains("template")
        || dirs
            .split('/')
            .any(|d| matches!(d, "elasticsearch" | "opensearch" | "mappings"))
}

fn read_file(ctx: &SearchContext, rel: &str, out: &mut ServiceTopology) {
    let text = (ctx.read)(ctx.root, &ctx.root.join(rel));
    if text.as_deref().is_some_and(|t| mustache_lang(t).is_some()) {
        return;
    }
    let Some(value) = text.as_deref().and_then(|t| parse(rel, t)) else {
        if gap_path(rel) {
            es_gap(out, EsGap::DefinitionUnread, rel, None, &[]);
        }
        return;
    };
    if let (Some(def), Some(text)) = (search_definition(&value, None), text) {
        out.elasticsearch.artifacts.push(artifact(rel, &text, def));
    }
}

fn parse(rel: &str, text: &str) -> Option<Value> {
    if ext(rel) == "json" {
        return serde_json::from_str(text).ok();
    }
    // shortcut: YAML aliases are refused (a small file can expand exponentially); lift on demand.
    (!has_alias(text))
        .then(|| serde_yaml_ng::from_str(text).ok())
        .flatten()
}

fn has_alias(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim_start().trim_start_matches("- ");
        t.starts_with('*')
            || t.split_once(": ")
                .is_some_and(|(_, v)| v.trim_start().starts_with('*'))
    })
}

fn artifact(rel: &str, text: &str, def: Definition) -> SearchArtifact {
    let line = text
        .lines()
        .position(|l| l.contains(def.key))
        .and_then(|i| u32::try_from(i + 1).ok());
    let stem = Path::new(rel).file_stem().and_then(|s| s.to_str());
    SearchArtifact {
        flavor: def.flavor.into(),
        kind: def.kind.into(),
        name: stem.map(str::to_owned),
        address: rel.into(),
        scope: format!("json:{}", rel.rsplit_once('/').map_or(".", |(dir, _)| dir)),
        patterns: def.patterns,
        references: def.references,
        evidence: vec![ServiceEvidence {
            file: rel.into(),
            pointer: format!("/{}", def.key),
            line,
            description: format!(
                "{} declared in file {rel}; kind by key shape, name assumed from file name. Read as data; no cluster access.",
                def.kind
            ),
        }],
        origins: vec![],
        detail: def.detail,
    }
}

/// The artifact a file declares, by the evidence path; later sources (curl `-d @file`, Spring
/// `mappingPath`) add to it instead of declaring a second one.
#[allow(dead_code)] // first reader: tickets 23 and 25
pub(crate) fn find_file_artifact<'a>(
    artifacts: &'a [SearchArtifact],
    file: &str,
) -> Option<&'a SearchArtifact> {
    artifacts
        .iter()
        .find(|a| a.evidence.iter().any(|e| e.file == file))
}
