//! Acceptance probe R12 (ticket 44): counts index details read from a local corpus, numbers only.
//! Run: `ARCHGUARD_ES_PROBE=<copy> cargo test -p archguard-engine --test service_elasticsearch_detail_probe -- --ignored --nocapture`
#[path = "service_elasticsearch_detail_probe/forms.rs"]
mod forms;
use archguard_core::SearchArtifact;
use archguard_engine::{analyze_services, is_elasticsearch_gap};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// One entry per resource block (file and line), not per module instance.
fn blocks(arts: &[SearchArtifact], pick: impl Fn(&SearchArtifact) -> bool) -> Vec<&SearchArtifact> {
    let mut seen = BTreeSet::new();
    arts.iter()
        .filter(|a| pick(a))
        .filter(|a| {
            let e = a.evidence.first();
            seen.insert((e.map(|e| e.file.clone()), e.and_then(|e| e.line)))
        })
        .collect()
}

fn setting_count(idx: &[&SearchArtifact], key: &str) -> usize {
    idx.iter()
        .filter(|a| {
            a.detail
                .as_ref()
                .is_some_and(|d| d.settings.contains_key(key))
        })
        .count()
}

/// `kind.name` of the block a gap message starts with, without the module prefix.
fn block_address(m: &str) -> String {
    let a = m
        .split(' ')
        .next()
        .unwrap_or_default()
        .trim_end_matches(':');
    let parts: Vec<_> = a.split('.').collect();
    parts[parts.len().saturating_sub(2)..].join(".")
}

/// Fixed fragments of the gap texts; nothing from the corpus is printed.
fn gap_kind(m: &str) -> &'static str {
    [
        ("Elasticsearch name is not a literal", "NotLiteral"),
        ("Index name is not a literal", "IndexNotLiteral"),
        ("saved objects", "SavedObjects"),
        ("no top-level params index", "NoParamsIndex"),
        ("same source directory", "SameDir"),
        ("not read: larger", "Unread"),
        ("Source file not read", "Unread"),
        ("Index scan of the service stopped", "ScanStop"),
        ("JSON file", "Json"),
    ]
    .iter()
    .find(|(f, _)| m.contains(f))
    .map_or("other", |(_, k)| k)
}

#[test]
#[ignore = "reads the corpus named by ARCHGUARD_ES_PROBE"]
fn probe() {
    let Ok(root) = std::env::var("ARCHGUARD_ES_PROBE") else {
        return;
    };
    let r = analyze_services(Path::new(&root)).unwrap();
    let arts = &r.elasticsearch.artifacts;
    let idx = blocks(arts, |a| a.kind == "index");
    let with_fields = idx
        .iter()
        .filter(|a| a.detail.as_ref().is_some_and(|d| !d.fields.is_empty()))
        .count();
    let with_analysis = idx
        .iter()
        .filter(|a| a.detail.as_ref().is_some_and(|d| !d.analysis.is_empty()))
        .count();
    let templates = blocks(arts, |a| {
        a.kind.ends_with("template") && a.kind.contains("index_") || a.kind == "component_template"
    })
    .iter()
    .filter(|a| a.detail.is_some())
    .count();
    let ilm = blocks(arts, |a| {
        a.references.iter().any(|x| x.kind == "ilm_policy")
    })
    .len();
    let watch_refs = blocks(arts, |a| {
        a.kind == "watch" && a.references.iter().any(|x| x.kind == "index")
    })
    .len();
    let mut not_literal = (BTreeSet::new(), BTreeSet::new());
    let mut other: BTreeMap<&str, usize> = BTreeMap::new();
    for d in r.diagnostics.iter().filter(|d| is_elasticsearch_gap(d)) {
        let m = &d.message;
        if m.contains(": index mapping, settings or analysis are not literal") {
            let key = (d.file.clone(), block_address(m));
            if m.contains("elasticstack_elasticsearch_watch.") {
                not_literal.1.insert(key);
            } else {
                not_literal.0.insert(key);
            }
        } else {
            *other.entry(gap_kind(m)).or_default() += 1;
        }
    }
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for a in arts {
        *kinds.entry(a.kind.as_str()).or_default() += 1;
    }
    println!("index blocks: {}", idx.len());
    println!("with mapping (>= 1 field): {with_fields}");
    println!(
        "number_of_shards / refresh_interval / number_of_replicas: {} / {} / {}",
        setting_count(&idx, "number_of_shards"),
        setting_count(&idx, "refresh_interval"),
        setting_count(&idx, "number_of_replicas")
    );
    println!("with analysis: {with_analysis}");
    println!(
        "DetailNotLiteral at indices / at watches: {} / {}",
        not_literal.0.len(),
        not_literal.1.len()
    );
    println!(
        "ILM references: {ilm}; template details: {templates}; watch index references: {watch_refs}"
    );
    println!("artifacts: {} {kinds:?}", arts.len());
    println!("usages: {}", r.elasticsearch.usages.len());
    println!("other ES gaps by kind: {other:?}");
    for (k, n) in forms::breakdown(Path::new(&root)) {
        println!("form {k}: {n}");
    }
}
