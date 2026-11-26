//! Merging (ADR 0025): the sources of one project, and project sets (ADR 0024) where only the Elasticsearch model is merged.
use crate::context::SourceOutput;
use crate::findings::{self, is_elasticsearch_gap};
use archguard_core::{
    ArchguardError, SearchArtifact, ServiceDiagnostic, ServiceEvidence, ServiceTopology,
};
use archguard_flow_common::support::diagnostic;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Puts the sources together: artifacts sorted by (scope, address), usages, gaps, then the findings.
pub(crate) fn apply(report: &mut ServiceTopology, outputs: Vec<SourceOutput>) {
    for o in outputs {
        report.complete &= o.complete();
        report.elasticsearch.artifacts.extend(o.artifacts);
        report.elasticsearch.usages.extend(o.usages);
        report.diagnostics.extend(o.diagnostics);
    }
    let artifacts = &mut report.elasticsearch.artifacts;
    artifacts.sort_by(|a, b| (&a.scope, &a.address).cmp(&(&b.scope, &b.address)));
    merge_aliases(artifacts);
    findings::augment(report);
}

/// One alias artifact per (scope, flavor, name): alias blocks of several indices become one box
/// that references them all. The first artifact stays; references and evidence are appended.
fn merge_aliases(artifacts: &mut Vec<SearchArtifact>) {
    let mut first: HashMap<(String, String, String), usize> = HashMap::new();
    let mut out: Vec<SearchArtifact> = Vec::with_capacity(artifacts.len());
    for a in artifacts.drain(..) {
        // an alias on an unknown index (no reference) stays its own artifact
        let key = (a.kind == "alias" && !a.references.is_empty())
            .then(|| a.name.clone())
            .flatten()
            .map(|n| (a.scope.clone(), a.flavor.clone(), n));
        match key.as_ref().and_then(|k| first.get(k)) {
            Some(&i) => {
                let t = &mut out[i];
                t.references.extend(a.references);
                t.evidence.extend(a.evidence);
            }
            None => {
                first.extend(key.map(|k| (k, out.len())));
                out.push(a);
            }
        }
    }
    *artifacts = out;
}

/// (kind, flavor, literal name) -> index into the merged artifacts; the first artifact wins.
type Seen = HashMap<(String, String, String), usize>;

type Analyze = fn(&Path) -> Result<ServiceTopology, ArchguardError>;

/// Analyze each root on its own and merge the Elasticsearch maps; the first root is the primary
/// project and supplies services, dependencies and datastores. One root equals `analyze`.
/// `analyze` is passed in so that this file does not depend on `services.rs`.
pub fn merge_set(roots: &[PathBuf], analyze: Analyze) -> Result<ServiceTopology, ArchguardError> {
    let [first, rest @ ..] = roots else {
        return Err(ArchguardError::Config(
            "a project set needs at least one project".into(),
        ));
    };
    if rest.is_empty() {
        return analyze(first);
    }
    let names = project_names(roots);
    let mut base =
        analyze(first).map_err(|e| ArchguardError::Config(format!("{}: {e}", names[0])))?;
    let mut seen = Seen::new();
    for (i, a) in base.elasticsearch.artifacts.iter_mut().enumerate() {
        a.origins = vec![names[0].clone()];
        if let Some(n) = &a.name {
            seen.entry((a.kind.clone(), a.flavor.clone(), n.clone()))
                .or_insert(i);
        }
    }
    for (root, name) in rest.iter().zip(&names[1..]) {
        match analyze(root) {
            Ok(other) => merge_project(&mut base, &mut seen, name, other),
            Err(e) => diagnostic(
                &mut base,
                name,
                None,
                format!("Project could not be analysed: {e}"),
            ),
        }
    }
    base.elasticsearch.projects = names.clone();
    base.diagnostics.push(ServiceDiagnostic {
        file: ".".into(),
        service: None,
        message: format!(
            "Project set: only Elasticsearch is merged; services, dependencies and datastores are those of the primary project `{}`.",
            names[0]
        ),
    });
    base.diagnostics.sort();
    base.diagnostics.dedup();
    findings::augment(&mut base);
    Ok(base)
}

/// Folder names; a repeated name gets `-2`, `-3`, ...
fn project_names(roots: &[PathBuf]) -> Vec<String> {
    let mut count: HashMap<String, usize> = HashMap::new();
    roots
        .iter()
        .map(|r| {
            let base = r
                .file_name()
                .map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned());
            let n = count.entry(base.clone()).or_insert(0);
            *n += 1;
            if *n == 1 { base } else { format!("{base}-{n}") }
        })
        .collect()
}

fn prefixed(name: &str, text: &str) -> String {
    format!("{name}:{text}")
}

fn prefix_evidence(name: &str, evidence: &mut [ServiceEvidence]) {
    for e in evidence {
        e.file = prefixed(name, &e.file);
    }
}

fn merge_project(base: &mut ServiceTopology, seen: &mut Seen, name: &str, other: ServiceTopology) {
    let mut fresh = Seen::new();
    for mut a in other.elasticsearch.artifacts {
        prefix_evidence(name, &mut a.evidence);
        let key = a
            .name
            .clone()
            .map(|n| (a.kind.clone(), a.flavor.clone(), n));
        match key.as_ref().and_then(|k| seen.get(k)) {
            Some(&i) => merge_artifact(&mut base.elasticsearch.artifacts[i], a, name),
            None => {
                a.origins = vec![name.to_owned()];
                if let Some(k) = key {
                    fresh.entry(k).or_insert(base.elasticsearch.artifacts.len());
                }
                base.elasticsearch.artifacts.push(a);
            }
        }
    }
    for (k, i) in fresh {
        seen.entry(k).or_insert(i);
    }
    for mut u in other.elasticsearch.usages {
        u.service = prefixed(name, &u.service);
        prefix_evidence(name, &mut u.evidence);
        base.elasticsearch.usages.push(u);
    }
    merge_gaps(base, name, other.diagnostics, other.complete);
}

fn merge_artifact(into: &mut SearchArtifact, from: SearchArtifact, name: &str) {
    if !into.origins.iter().any(|o| o == name) {
        into.origins.push(name.to_owned());
    }
    if into.detail.is_none() {
        into.detail = from.detail;
    }
    for e in from.evidence {
        if !into.evidence.contains(&e) {
            into.evidence.push(e);
        }
    }
    for p in from.patterns {
        if !into.patterns.contains(&p) {
            into.patterns.push(p);
        }
    }
    for r in from.references {
        if !into.references.contains(&r) {
            into.references.push(r);
        }
    }
}

/// Elasticsearch gaps of a member, prefixed; other incompleteness is named, never dropped.
fn merge_gaps(
    base: &mut ServiceTopology,
    name: &str,
    diagnostics: Vec<ServiceDiagnostic>,
    complete: bool,
) {
    let other = diagnostics.iter().any(|d| !is_elasticsearch_gap(d));
    let mut gaps = 0;
    for mut d in diagnostics.into_iter().filter(is_elasticsearch_gap) {
        d.file = prefixed(name, &d.file);
        d.service = d.service.map(|s| prefixed(name, &s));
        base.diagnostics.push(d);
        gaps += 1;
    }
    if gaps > 0 {
        base.complete = false;
    }
    // shortcut: any non-ES diagnostic of an incomplete member counts, informational ones too.
    if !complete && (gaps == 0 || other) {
        diagnostic(
            base,
            name,
            None,
            "Project is incomplete outside Elasticsearch; see its own analysis.",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::SearchIndexDetail;

    fn art(detail: Option<SearchIndexDetail>) -> SearchArtifact {
        SearchArtifact {
            flavor: String::new(),
            kind: "index".into(),
            name: Some("audit".into()),
            address: "a".into(),
            scope: "s".into(),
            patterns: vec![],
            references: vec![],
            evidence: vec![],
            origins: vec![],
            detail,
        }
    }

    #[test]
    fn merge_keeps_the_first_detail_and_adopts_a_later_one() {
        let d = |n: &str| SearchIndexDetail {
            settings: [("number_of_shards".to_string(), n.to_string())].into(),
            ..Default::default()
        };
        let mut a = art(Some(d("1")));
        merge_artifact(&mut a, art(Some(d("2"))), "b");
        assert_eq!(a.detail, Some(d("1")));
        let mut c = art(None);
        merge_artifact(&mut c, art(Some(d("2"))), "b");
        assert_eq!(c.detail, Some(d("2")));
    }
}
