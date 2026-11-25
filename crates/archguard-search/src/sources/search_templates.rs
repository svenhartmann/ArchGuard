//! Search templates in `*.json` files below Terraform directories (ADR 0023).
use crate::context::SearchContext;
use crate::context::SourceOutput;
use crate::findings::{EsGap, es_gap};
use archguard_core::{SearchArtifact, ServiceEvidence, ServiceTopology};
use archguard_flow_common::support::{relative, test_path};
use std::collections::BTreeSet;
use std::path::Path;

pub(crate) fn extract_with(ctx: &SearchContext) -> SourceOutput {
    let mut out = ServiceTopology::default();
    search_templates(ctx, &mut out);
    SourceOutput::from_scratch(out)
}

/// Search templates in `*.json` files below directories that contain `.tf` files (ADR 0023).
/// Detection is a text match of `"lang": "mustache"`, not a JSON parse: templates with mustache
/// sections are not strict JSON. The name is assumed from the file name.
fn search_templates(ctx: &SearchContext, report: &mut ServiceTopology) {
    let (root, discovery) = (ctx.root, ctx.discovery);
    let (mut tf_dirs, mut jsons) = (BTreeSet::new(), Vec::new());
    for entry in discovery.walk(root).flatten() {
        let path = entry.path();
        if !entry.file_type().is_some_and(|t| t.is_file())
            || path.components().any(|c| c.as_os_str() == ".terraform")
        {
            continue;
        }
        let rel = relative(root, path);
        match path.extension().and_then(|x| x.to_str()) {
            Some("tf") => {
                tf_dirs.insert(parent(&rel).to_owned());
            }
            Some("json") => jsons.push(rel),
            _ => {}
        }
    }
    jsons.sort();
    for rel in jsons {
        if test_path(&rel) || !under(&tf_dirs, &rel) {
            continue;
        }
        let Some((text, whole)) = read_json(ctx, &rel) else {
            es_gap(report, EsGap::JsonUnread, &rel, None, &[]);
            continue;
        };
        let Some(at) = mustache_lang(&text) else {
            continue;
        };
        if !whole {
            es_gap(report, EsGap::JsonTooLarge, &rel, None, &[]);
            continue;
        }
        let line = u32::try_from(text[..at].matches('\n').count() + 1).ok();
        let name = Path::new(&rel)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_owned);
        report.elasticsearch.artifacts.push(SearchArtifact {
            kind: "search_template".into(),
            name,
            address: rel.clone(),
            scope: "json".into(),
            patterns: vec![],
            references: vec![],
            evidence: vec![ServiceEvidence {
                file: rel.clone(),
                pointer: "/script/lang".into(),
                line,
                description: format!(
                    "Search template declared in JSON file {rel}; name assumed from file name. Read as text; no cluster access."
                ),
            }],
            origins: vec![],
            detail: None,
        });
    }
}

/// Whether a directory of `path` (or the root) holds a `.tf` file.
fn under(tf_dirs: &BTreeSet<String>, path: &str) -> bool {
    let mut dir = parent(path);
    while !tf_dirs.contains(dir) {
        if dir.is_empty() {
            return false;
        }
        dir = parent(dir);
    }
    true
}

/// Text of a JSON file and whether it is complete. A file over 1 MiB yields its first 1 MiB only.
// shortcut: a pattern beyond the first 1 MiB of a larger file is not seen; stream when a repo needs it.
fn read_json(ctx: &SearchContext, rel: &str) -> Option<(String, bool)> {
    if let Some(text) = (ctx.read)(ctx.root, &ctx.root.join(rel)) {
        return Some((text, true));
    }
    use std::io::Read;
    let path = ctx.root.join(rel);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() <= 1024 * 1024 {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(1024 * 1024)
        .read_to_end(&mut bytes)
        .ok()?;
    Some((String::from_utf8_lossy(&bytes).into_owned(), false))
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Byte offset of the first `"lang"\s*:\s*"mustache"`.
fn mustache_lang(text: &str) -> Option<usize> {
    text.match_indices("\"lang\"")
        .map(|(at, key)| (at, &text[at + key.len()..]))
        .find_map(|(at, rest)| {
            let rest = rest.trim_start().strip_prefix(':')?.trim_start();
            rest.starts_with("\"mustache\"").then_some(at)
        })
}
