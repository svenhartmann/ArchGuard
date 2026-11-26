//! Project sets: `analyze_service_set` merges the Elasticsearch model of several projects (ADR 0024).
use archguard_engine::{
    analyze_service_set, analyze_services,
    core::{SearchArtifact, ServiceTopology},
    is_elasticsearch_gap,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

fn dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/es-set")
        .join(name)
}

fn put(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn set(names: &[&str]) -> ServiceTopology {
    let roots: Vec<PathBuf> = names.iter().map(|n| dir(n)).collect();
    analyze_service_set(&roots).unwrap()
}

fn subjects(r: &ServiceTopology, kind: &str) -> Vec<String> {
    let mut s: Vec<String> = r
        .elasticsearch
        .findings
        .iter()
        .filter(|f| f.kind == kind)
        .map(|f| f.subject.clone())
        .collect();
    s.sort();
    s
}

fn named<'a>(r: &'a ServiceTopology, kind: &str, name: &str) -> Vec<&'a SearchArtifact> {
    r.elasticsearch
        .artifacts
        .iter()
        .filter(|a| a.kind == kind && a.name.as_deref() == Some(name))
        .collect()
}

#[test]
fn adapter_alone_reports_the_undeclared_index_and_equals_analyze_services() {
    let alone = set(&["adapter"]);
    assert_eq!(alone, analyze_services(&dir("adapter")).unwrap());
    assert_eq!(
        serde_json::to_string(&alone).unwrap(),
        serde_json::to_string(&analyze_services(&dir("adapter")).unwrap()).unwrap()
    );
    assert_eq!(subjects(&alone, "undeclared_index"), ["orders"]);
    assert!(alone.elasticsearch.projects.is_empty());
    assert!(
        alone
            .elasticsearch
            .artifacts
            .iter()
            .all(|a| a.origins.is_empty())
    );
}

#[test]
fn infra_and_adapter_remove_the_finding_and_merge_origins() {
    let r = set(&["infra", "adapter"]);
    assert!(subjects(&r, "undeclared_index").is_empty());
    assert_eq!(r.elasticsearch.projects, ["infra", "adapter"]);
    let audit = named(&r, "index", "audit");
    assert_eq!(audit.len(), 1, "deduplicated");
    assert_eq!(audit[0].origins, ["infra", "adapter"]);
    let files: Vec<&str> = audit[0].evidence.iter().map(|e| e.file.as_str()).collect();
    assert!(files.contains(&"terraform/main.tf"), "{files:?}");
    assert!(files.contains(&"adapter:terraform/main.tf"), "{files:?}");
    for (kind, name) in [("index", "orders"), ("ingest_pipeline", "p1")] {
        let a = named(&r, kind, name);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].origins, ["infra"]);
    }
    let services: Vec<&str> = r
        .elasticsearch
        .usages
        .iter()
        .map(|u| u.service.as_str())
        .collect();
    assert!(
        !services.is_empty()
            && services
                .iter()
                .all(|s| *s == "adapter:compose.yaml::writer"),
        "{services:?}"
    );
    assert!(
        r.elasticsearch
            .usages
            .iter()
            .flat_map(|u| &u.evidence)
            .all(|e| e.file == "adapter:writer/Writer.java")
    );
}

#[test]
fn only_the_primary_topology_is_the_base_and_a_diagnostic_says_so() {
    let r = set(&["infra", "adapter"]);
    let infra = analyze_services(&dir("infra")).unwrap();
    assert_eq!(r.root, infra.root);
    assert_eq!(r.services, infra.services);
    assert_eq!(r.dependencies, infra.dependencies);
    assert!(r.services.iter().all(|s| !s.id.contains("writer")));
    assert_eq!(
        r.diagnostics
            .iter()
            .filter(|d| d.message.contains("only Elasticsearch is merged"))
            .count(),
        1
    );
}

#[test]
fn foreign_es_gaps_carry_the_project_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    put(
        tmp.path(),
        "gapped/terraform/main.tf",
        "resource \"elasticstack_elasticsearch_index\" \"x\" {\n  name = var.unknown\n}\n",
    );
    let roots = [dir("infra"), tmp.path().join("gapped")];
    let r = analyze_service_set(&roots).unwrap();
    let gaps: Vec<_> = r
        .diagnostics
        .iter()
        .filter(|d| is_elasticsearch_gap(d))
        .collect();
    assert!(!gaps.is_empty());
    assert!(
        gaps.iter().all(|d| d.file == "gapped:terraform/main.tf"),
        "{gaps:?}"
    );
    assert!(!r.complete);
}

#[test]
fn reversed_order_keeps_findings_and_swaps_the_prefix() {
    let a = set(&["infra", "adapter"]);
    let b = set(&["adapter", "infra"]);
    for kind in [
        "undeclared_index",
        "undeclared_pipeline",
        "shared_write_index",
    ] {
        assert_eq!(subjects(&a, kind), subjects(&b, kind), "{kind}");
    }
    assert_eq!(b.elasticsearch.projects, ["adapter", "infra"]);
    assert_eq!(named(&b, "index", "audit")[0].origins, ["adapter", "infra"]);
    assert!(
        b.elasticsearch
            .usages
            .iter()
            .all(|u| u.service == "compose.yaml::writer")
    );
    let files: Vec<&str> = named(&b, "index", "orders")[0]
        .evidence
        .iter()
        .map(|e| e.file.as_str())
        .collect();
    assert!(
        files.iter().all(|f| *f == "infra:terraform/main.tf"),
        "{files:?}"
    );
    assert_eq!(b.root, analyze_services(&dir("adapter")).unwrap().root);
}

#[test]
fn same_folder_names_get_a_counter_and_duplicates_in_one_project_stay_apart() {
    let tmp = tempfile::tempdir().unwrap();
    let decl = "resource \"elasticstack_elasticsearch_index\" \"i\" {\n  name = \"dup\"\n}\n";
    put(tmp.path(), "a/same/terraform/one/main.tf", decl);
    put(tmp.path(), "a/same/terraform/two/main.tf", decl);
    put(tmp.path(), "b/same/terraform/main.tf", decl);
    let r = analyze_service_set(&[tmp.path().join("a/same"), tmp.path().join("b/same")]).unwrap();
    assert_eq!(r.elasticsearch.projects, ["same", "same-2"]);
    let dups = named(&r, "index", "dup");
    assert_eq!(
        dups.len(),
        2,
        "two in the primary stay apart, the third merges into the first"
    );
    let mut origins: Vec<Vec<String>> = dups.iter().map(|a| a.origins.clone()).collect();
    origins.sort();
    assert_eq!(
        origins,
        [
            vec!["same".to_string()],
            vec!["same".to_string(), "same-2".to_string()]
        ]
    );
}

#[test]
fn a_member_that_cannot_be_analysed_becomes_a_diagnostic() {
    let missing = dir("does-not-exist");
    let r = analyze_service_set(&[dir("infra"), missing]).unwrap();
    assert!(!r.complete);
    assert!(
        r.diagnostics.iter().any(|d| d.file == "does-not-exist"),
        "{:?}",
        r.diagnostics
    );
    let err = analyze_service_set(&[dir("does-not-exist"), dir("infra")]).unwrap_err();
    assert!(err.to_string().contains("does-not-exist"), "{err}");
}

fn many(root: &Path, n: usize) {
    let mut tf = String::new();
    for i in 0..n {
        tf.push_str(&format!(
            "resource \"elasticstack_elasticsearch_index\" \"i{i}\" {{\n  name = \"ix{i}\"\n}}\n"
        ));
    }
    put(root, "terraform/main.tf", &tf);
}

fn best_of_three(n: usize) -> f64 {
    let tmp = tempfile::tempdir().unwrap();
    many(&tmp.path().join("a"), n);
    many(&tmp.path().join("b"), n);
    let roots = [tmp.path().join("a"), tmp.path().join("b")];
    (0..3)
        .map(|_| {
            let t = Instant::now();
            let r = analyze_service_set(&roots).unwrap();
            assert_eq!(r.elasticsearch.artifacts.len(), n);
            t.elapsed().as_secs_f64()
        })
        .fold(f64::MAX, f64::min)
}

#[test]
fn merge_time_grows_linearly_with_the_number_of_artifacts() {
    let ratio = best_of_three(2000) / best_of_three(500);
    assert!(
        ratio < 9.0,
        "4x artifacts took {ratio:.1}x as long; linear is about 4, quadratic 16"
    );
}

#[test]
fn a_member_with_an_es_gap_and_a_non_es_gap_reports_both() {
    let tmp = tempfile::tempdir().unwrap();
    let tf = "resource \"elasticstack_elasticsearch_index\" \"x\" {\n  name = var.unknown\n}\n";
    put(tmp.path(), "mixed/terraform/main.tf", tf);
    put(
        tmp.path(),
        "mixed/compose.yaml",
        "include:\n  - other.yaml\nservices:\n  s:\n    image: x\n",
    );
    let r = analyze_service_set(&[dir("infra"), tmp.path().join("mixed")]).unwrap();
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.file == "mixed:terraform/main.tf" && is_elasticsearch_gap(d))
    );
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.file == "mixed" && d.message.contains("incomplete outside Elasticsearch")),
        "{:?}",
        r.diagnostics
    );
}

fn kind_counts<T>(
    items: &[T],
    kind: impl Fn(&T) -> &str,
) -> std::collections::BTreeMap<String, usize> {
    let mut m = std::collections::BTreeMap::new();
    for i in items {
        *m.entry(kind(i).to_string()).or_insert(0) += 1;
    }
    m
}

fn print_counts(label: &str, r: &ServiceTopology) {
    let e = &r.elasticsearch;
    println!(
        "{label}: artifacts {:?}",
        kind_counts(&e.artifacts, |a| &a.kind)
    );
    println!(
        "{label}: merged {}",
        e.artifacts.iter().filter(|a| a.origins.len() > 1).count()
    );
    println!("{label}: usages {}", e.usages.len());
    println!(
        "{label}: findings {:?}",
        kind_counts(&e.findings, |f| &f.kind)
    );
    println!("{label}: projects {}", e.projects.len());
}

/// Acceptance probe R10 (spec.md): counts only, no names. Skipped unless the env var is set.
#[test]
#[ignore = "needs ARCHGUARD_ES_PROBE_SPLIT, see spec.md Abnahme-Probe R10"]
fn probe_split() {
    let Ok(p) = std::env::var("ARCHGUARD_ES_PROBE_SPLIT") else {
        return;
    };
    let p = PathBuf::from(p);
    let adapter = analyze_services(&p.join("adapter")).unwrap();
    print_counts("adapter alone", &adapter);
    let infra = analyze_services(&p.join("infra")).unwrap();
    print_counts("infra alone", &infra);
    let both = analyze_service_set(&[p.join("infra"), p.join("adapter")]).unwrap();
    print_counts("set", &both);
    println!(
        "set: diagnostics {}, complete {}",
        both.diagnostics.len(),
        both.complete
    );
    if p.join("whole").is_dir() {
        print_counts("unsplit", &analyze_services(&p.join("whole")).unwrap());
    }
}

/// Stands in for `analyze_services`: the fixture project holds a finished Elasticsearch model.
fn read_topology(root: &Path) -> Result<ServiceTopology, archguard_engine::core::ArchguardError> {
    let mut t = ServiceTopology::default();
    t.elasticsearch =
        serde_json::from_str(&fs::read_to_string(root.join("topology.json")).unwrap()).unwrap();
    Ok(t)
}

#[test]
fn same_name_other_flavor_is_not_merged() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/20");
    let t =
        archguard_search::merge_set(&[base.join("es"), base.join("os")], read_topology).unwrap();
    let json = serde_json::to_value(&t).unwrap();
    let arts = json["elasticsearch"]["artifacts"].as_array().unwrap();
    assert_eq!(arts.len(), 2);
    let mut flavors: Vec<&str> = arts
        .iter()
        .map(|a| a["flavor"].as_str().unwrap_or(""))
        .collect();
    flavors.sort();
    assert_eq!(flavors, ["", "opensearch"]);
}
