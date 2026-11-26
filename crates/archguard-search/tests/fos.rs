//! FOSElasticaBundle YAML configuration becomes index artifacts (ticket 28).
use archguard_core::{SearchArtifact, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/28")
}

fn analyze() -> ServiceTopology {
    let root = root();
    let mut report = ServiceTopology::default();
    let discovery = Discovery::new(&root, &mut report).expect("discovery");
    // The broken file stands for an unreadable one (too large, symlink, no permission).
    let read = |_: &Path, p: &Path| {
        (!p.ends_with("broken/fos_elastica.yaml"))
            .then(|| std::fs::read_to_string(p).ok())
            .flatten()
    };
    archguard_search::analyze(&root, &discovery, &read, &mut report);
    report
}

fn fos(report: &ServiceTopology) -> Vec<&SearchArtifact> {
    let mut all: Vec<_> = report
        .elasticsearch
        .artifacts
        .iter()
        .filter(|a| a.scope.starts_with("fos:"))
        .collect();
    all.sort_by_key(|a| a.name.clone());
    all
}

#[test]
fn only_indexes_below_the_fos_elastica_root_are_artifacts() {
    let report = analyze();
    let got: Vec<_> = fos(&report)
        .iter()
        .map(|a| {
            format!(
                "{} {} {}",
                a.kind,
                a.name.as_deref().unwrap_or("?"),
                a.scope
            )
        })
        .collect();
    let want = [
        "index catalog fos:config/packages",
        "index custom_idx fos:extra",
        "index dev_only fos:config/packages/dev",
        "index shop_v2 fos:config/packages",
    ];
    assert_eq!(
        got, want,
        "lookalikes, test paths, env and parameter names are none"
    );
}

#[test]
fn fields_settings_analysis_and_evidence_of_an_index() {
    let report = analyze();
    let all = fos(&report);
    let a = all
        .iter()
        .find(|a| a.name.as_deref() == Some("catalog"))
        .expect("catalog");
    assert_eq!(a.flavor, "");
    assert!(a.patterns.is_empty() && a.references.is_empty());
    let d = a.detail.as_ref().expect("detail");
    let fields: Vec<_> = d
        .fields
        .iter()
        .map(|f| {
            format!(
                "{} {} {} {} {}",
                f.path,
                f.kind,
                f.analyzer.as_deref().unwrap_or("-"),
                f.search_analyzer.as_deref().unwrap_or("-"),
                f.normalizer.as_deref().unwrap_or("-")
            )
        })
        .collect();
    assert_eq!(
        fields,
        [
            "address object - - -",
            "address.city keyword - - -",
            "name text - - -",
            "name.raw keyword - - -",
            "sku keyword - - lower_norm",
            "title text title_an standard -",
        ]
    );
    assert_eq!(
        d.settings.get("number_of_shards").map(String::as_str),
        Some("3")
    );
    assert_eq!(d.analysis.len(), 1);
    assert_eq!(d.analysis[0].name, "title_an");
    let e = &a.evidence[0];
    assert_eq!(e.file, "config/packages/fos_elastica.yaml");
    assert_eq!(e.line, Some(5));
    assert!(
        e.description.contains("Acme\\Shop\\Entity\\Product"),
        "{}",
        e.description
    );
}

#[test]
fn index_name_wins_over_the_key() {
    let report = analyze();
    let all = fos(&report);
    let a = all
        .iter()
        .find(|a| a.name.as_deref() == Some("shop_v2"))
        .expect("shop_v2");
    assert_eq!(a.address, "config/packages/fos_elastica.yaml:shop_data");
}

#[test]
fn env_and_parameter_names_and_unreadable_files_are_gaps() {
    let report = analyze();
    let mut gaps: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| archguard_search::is_elasticsearch_gap(d))
        .map(|d| format!("{}: {}", d.file, d.message))
        .collect();
    gaps.sort();
    let [broken, env, param] = gaps.as_slice() else {
        panic!("{gaps:?}")
    };
    assert!(broken.starts_with("broken/fos_elastica.yaml: Definition file could not be read"));
    assert!(env.starts_with("config/packages/fos_elastica.yaml: Index name is not a literal"));
    assert!(env.ends_with("Source line 37."), "{env}");
    assert!(param.ends_with("Source line 41."), "{param}");
    assert!(!report.complete);
}
