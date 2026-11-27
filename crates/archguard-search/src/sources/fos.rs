//! FOSElasticaBundle YAML (ticket 28). Rules: a YAML file is read when it mentions `fos_elastica`;
//! only the root key `fos_elastica` counts (a nested one, `fos_elastica_x:` or `indexes:` elsewhere
//! is none). Each `indexes.<key>` is an `index` named by a literal `index_name`, else by the key;
//! `%env(…)%`, `%param%` and `${…}` in the name, in `settings`/`properties` values, or a
//! `settings`/`properties` that is no mapping are a gap and the index is left out. `settings` (with
//! `index.analysis`) and `properties.<field>.{type, analyzer, search_analyzer, normalizer, fields,
//! properties}` fill the detail like a JSON mapping; `persistence.driver/model` only appear in the
//! evidence text. `indexes` that is no mapping and `when@<env>` blocks are a gap. `scope` = `fos:<folder>`, evidence = the key's line. A file that cannot be
//! read or parsed is a gap when it is named `fos_elastica.y*ml` or has a root `fos_elastica:` line. Trust
//! boundary: files below the root through the safe reader, at most 5 000 YAML files, no test
//! paths, no network, no aliases; contents are data.
// shortcut: YAML aliases are refused (a small file can expand exponentially); lift on demand.
use super::json::detail_from_json;
use crate::context::{SearchContext, SourceOutput};
use crate::findings::{EsGap, es_gap};
use archguard_core::{SearchArtifact, ServiceEvidence, ServiceTopology};
use archguard_flow_common::support::{relative, test_path};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

const MAX_FILES: usize = 5_000;
const MARKER: &str = "fos_elastica";
const SKIPPED_DIRS: &[&str] = &[".terraform", ".git", "node_modules", "vendor", "var"];

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
        .filter(|rel| rel.ends_with(".yml") || rel.ends_with(".yaml"))
        .filter(|rel| !test_path(rel) && !rel.split('/').any(|p| SKIPPED_DIRS.contains(&p)))
        .collect();
    files.sort();
    files
}

fn named_like_config(rel: &str) -> bool {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    file.starts_with("fos_elastica.y")
}

fn read_file(ctx: &SearchContext, rel: &str, out: &mut ServiceTopology) {
    let Some(text) = (ctx.read)(ctx.root, &ctx.root.join(rel)) else {
        if named_like_config(rel) {
            es_gap(out, EsGap::DefinitionUnread, rel, None, &[]);
        }
        return;
    };
    if !text.contains(MARKER) {
        return;
    }
    let value: Option<Value> = (!has_alias(&text))
        .then(|| serde_yaml_ng::from_str(&text).ok())
        .flatten();
    let Some(value) = value else {
        // Only a file that is a config or has the root line is a gap; a service reference
        // (`@fos_elastica.finder…`) in ordinary Symfony YAML is a lookalike.
        if named_like_config(rel)
            || text
                .lines()
                .any(|l| l.trim_end().starts_with("fos_elastica:"))
        {
            es_gap(out, EsGap::DefinitionUnread, rel, None, &[]);
        }
        return;
    };
    let (lines, indexes_line) = key_lines(&text);
    let Some(indexes) = indexes_of(&value, indexes_line, rel, out) else {
        return;
    };
    for (key, entry) in indexes {
        let line = lines.get(key.as_str()).copied().or(indexes_line);
        match name_of(key, entry).filter(|_| !has_variable(entry)) {
            Some(name) => out
                .elasticsearch
                .artifacts
                .push(artifact(rel, key, name, entry, line)),
            None => {
                let at = line.map(|l| l.to_string()).unwrap_or_default();
                es_gap(out, EsGap::IndexNotLiteral, rel, None, &[&at]);
            }
        }
    }
}

/// The `fos_elastica.indexes` mapping; a gap is reported when it exists but cannot be read.
fn indexes_of<'a>(
    value: &'a Value,
    indexes_line: Option<u32>,
    rel: &str,
    out: &mut ServiceTopology,
) -> Option<&'a Map<String, Value>> {
    // Symfony `when@<env>:` blocks are not read; an unseen block must not vanish silently,
    // also next to a root block.
    let in_env = value.as_object().is_some_and(|m| {
        m.iter()
            .any(|(k, v)| k.starts_with("when@") && v.get(MARKER).is_some())
    });
    if in_env {
        es_gap(out, EsGap::DefinitionUnread, rel, None, &[]);
    }
    let indexes = value.get(MARKER)?.get("indexes")?;
    let Some(indexes) = indexes.as_object() else {
        // `indexes: '%param%'` or `%env(json:…)%`: the names are unknown.
        let at = indexes_line.map(|l| l.to_string()).unwrap_or_default();
        es_gap(out, EsGap::IndexNotLiteral, rel, None, &[&at]);
        return None;
    };
    Some(indexes)
}

fn has_alias(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim_start().trim_start_matches("- ");
        t.starts_with('*')
            || t.split_once(": ")
                .is_some_and(|(_, v)| v.trim_start().starts_with('*'))
    })
}

/// First line of every direct child key of `fos_elastica: indexes:` plus the line of `indexes:`
/// itself, built once per file. Trailing ` #` comments are ignored (shortcut: a ` #` inside a
/// quoted key would be cut).
fn key_lines(text: &str) -> (HashMap<&str, u32>, Option<u32>) {
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut map = HashMap::new();
    let mut indexes_line = None;
    let (mut in_root, mut in_indexes, mut child) = (false, false, None::<usize>);
    for (i, l) in text.lines().enumerate() {
        let t = l.trim();
        let t = t.split(" #").next().unwrap_or(t).trim_end();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let (ind, no) = (indent(l), u32::try_from(i + 1).unwrap_or(u32::MAX));
        if ind == 0 {
            in_root = t == "fos_elastica:";
            in_indexes = false;
        } else if in_indexes {
            let c = *child.get_or_insert(ind);
            if ind < c {
                in_indexes = false;
            } else if let (true, Some((k, _))) = (ind == c, t.split_once(':')) {
                map.entry(k.trim_matches(['\'', '"'])).or_insert(no);
            }
        }
        if in_root && !in_indexes && child.is_none() && t.starts_with("indexes:") {
            indexes_line.get_or_insert(no);
            in_indexes = t == "indexes:";
        }
    }
    (map, indexes_line)
}

/// `settings` or `properties` that is no mapping, or any `%…%`/`${…}` string below them: the
/// detail would be dropped or name an undeclared analyzer, so the index is a gap instead.
fn has_variable(entry: &Value) -> bool {
    fn var(v: &Value) -> bool {
        match v {
            Value::String(s) => s.contains('%') || s.contains("${"),
            Value::Array(a) => a.iter().any(var),
            Value::Object(m) => m.values().any(var),
            _ => false,
        }
    }
    ["settings", "properties"]
        .iter()
        .filter_map(|k| entry.get(k))
        .any(|v| !v.is_object() || var(v))
}

/// The literal index name: `index_name` when given, else the key; `None` when it is not literal.
fn name_of<'a>(key: &'a str, entry: &'a Value) -> Option<&'a str> {
    let name = match entry.get("index_name") {
        Some(v) => v.as_str()?,
        None => key,
    };
    (!name.contains('%') && !name.contains("${")).then_some(name)
}

fn artifact(rel: &str, key: &str, name: &str, entry: &Value, line: Option<u32>) -> SearchArtifact {
    let mappings = entry
        .get("properties")
        .filter(|p| p.is_object())
        .map(|p| json!({ "properties": p }));
    let detail = detail_from_json(mappings.as_ref(), entry.get("settings"));
    let folder = rel.rsplit_once('/').map_or(".", |(dir, _)| dir);
    SearchArtifact {
        flavor: String::new(),
        kind: "index".into(),
        name: Some(name.into()),
        address: format!("{rel}:{key}"),
        scope: format!("fos:{folder}"),
        patterns: vec![],
        references: vec![],
        evidence: vec![ServiceEvidence {
            file: rel.into(),
            pointer: format!("/{MARKER}/indexes/{key}"),
            line,
            description: format!(
                "index declared in FOSElasticaBundle configuration {rel}{}. Read as data; no cluster access.",
                persistence(entry)
            ),
        }],
        origins: vec![],
        detail: (detail != Default::default()).then_some(detail),
    }
}

/// `; persistence <driver> model <class>` as a description only, never a reference.
fn persistence(entry: &Value) -> String {
    let p: Option<&Map<String, Value>> = entry.get("persistence").and_then(Value::as_object);
    let get = |k: &str| p.and_then(|p| p.get(k)).and_then(Value::as_str);
    match (get("driver"), get("model")) {
        (Some(d), Some(m)) => format!("; persistence {d} model {m}"),
        (Some(x), None) | (None, Some(x)) => format!("; persistence {x}"),
        _ => String::new(),
    }
}
