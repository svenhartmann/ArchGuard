//! `.http` files and curl calls in scripts become artifacts (ticket 25).
use archguard_core::{SearchArtifact, ServiceDiagnostic, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/25")
}

fn unreadable(p: &Path) -> Option<String> {
    (!p.ends_with("unreadable.sh"))
        .then(|| std::fs::read_to_string(p).ok())
        .flatten()
}

fn analyze() -> ServiceTopology {
    let root = root();
    let mut report = ServiceTopology::default();
    let discovery = Discovery::new(&root, &mut report).expect("discovery");
    let read = |_: &Path, p: &Path| unreadable(p);
    archguard_search::analyze(&root, &discovery, &read, &mut report);
    report
}

fn http(report: &ServiceTopology) -> Vec<&SearchArtifact> {
    let mut all: Vec<_> = report
        .elasticsearch
        .artifacts
        .iter()
        .filter(|a| a.scope.starts_with("http:"))
        .collect();
    all.sort_by_key(|a| (a.scope.clone(), a.name.clone()));
    all
}

fn named<'a>(report: &'a ServiceTopology, name: &str) -> &'a SearchArtifact {
    http(report)
        .into_iter()
        .find(|a| a.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no artifact {name}"))
}

fn gaps<'a>(report: &'a ServiceTopology, file: &str) -> Vec<&'a ServiceDiagnostic> {
    report
        .diagnostics
        .iter()
        .filter(|d| d.file == file && archguard_search::is_elasticsearch_gap(d))
        .collect()
}

#[test]
fn every_rule_yields_its_kind_and_name_and_lookalikes_yield_nothing() {
    let report = analyze();
    let got: Vec<_> = http(&report)
        .iter()
        .map(|a| {
            format!(
                "{} {} {} {}",
                a.scope,
                a.kind,
                a.name.as_deref().unwrap_or("?"),
                a.flavor
            )
        })
        .collect();
    let want = [
        "http:api component_template shop-base ",
        "http:api alias shop-current ",
        "http:api ilm_policy shop-ilm ",
        "http:api ism_policy shop-ism opensearch",
        "http:api index_template shop-legacy ",
        "http:api alias shop-multi ",
        "http:api ism_policy shop-old-ism opensearch",
        "http:api index shop-orders ",
        "http:api ingest_pipeline shop-pipe ",
        "http:api index_template shop-tpl ",
        "http:docker index shop-docker ",
        "http:docker component_template shop-docker-comp ",
        "http:make index shop-make ",
        "http:notes index shop-one ",
        "http:notes index shop-three ",
        "http:notes index_template shop-two ",
        "http:scripts index shop-attached ",
        "http:scripts ingest_pipeline shop-clean ",
        "http:scripts index shop-eq ",
        "http:scripts index shop-from-file ",
        "http:scripts index shop-glued ",
        "http:scripts ilm_policy shop-ilm2 ",
        "http:scripts ism_policy shop-ism2 opensearch",
        "http:scripts index shop-json ",
        "http:scripts index shop-lines ",
        "http:scripts index shop-logs ",
    ];
    assert_eq!(
        got, want,
        "GET, DELETE, _search, multi-segment paths, echo, comments and mycurl create nothing"
    );
}

#[test]
fn the_body_goes_through_the_classifier_and_the_evidence_is_the_request_line() {
    let report = analyze();
    let a = named(&report, "shop-orders");
    let d = a.detail.as_ref().expect("detail");
    assert_eq!(d.settings["number_of_shards"], "2");
    assert_eq!(d.fields[0].path, "title");
    assert_eq!(d.fields[0].analyzer.as_deref(), Some("folded"));
    assert_eq!(a.evidence[0].file, "api/requests.http");
    assert_eq!(a.evidence[0].line, Some(4));
    let t = named(&report, "shop-tpl");
    assert_eq!(t.patterns, ["shop-*"]);
    assert_eq!(t.references[0].name, "shop-base");
    assert_eq!(t.evidence[0].line, Some(13));
    let l = named(&report, "shop-legacy");
    assert_eq!(l.patterns, ["old-*"], "a body without a header line");
}

#[test]
fn alias_actions_add_become_alias_references() {
    let report = analyze();
    let refs = |n: &str| -> Vec<String> {
        named(&report, n)
            .references
            .iter()
            .map(|r| format!("{} {}", r.kind, r.name))
            .collect()
    };
    assert_eq!(refs("shop-current"), ["index shop-orders"]);
    assert_eq!(refs("shop-multi"), ["index shop-a", "index shop-b"]);
    assert_eq!(named(&report, "shop-current").evidence[0].line, Some(54));
}

#[test]
fn curl_flags_continuations_and_commands_after_operators_are_read() {
    let report = analyze();
    let l = named(&report, "shop-logs");
    assert_eq!(l.evidence[0].file, "scripts/setup.sh");
    assert_eq!(l.evidence[0].line, Some(3));
    assert_eq!(l.detail.as_ref().expect("detail").fields[0].path, "msg");
    let p = named(&report, "shop-clean");
    assert_eq!(p.evidence[0].line, Some(4), "-XPUT, line continuation");
    assert_eq!(named(&report, "shop-ilm2").evidence[0].line, Some(7));
    assert_eq!(named(&report, "shop-docker").evidence[0].line, Some(2));
    assert_eq!(named(&report, "shop-docker-comp").evidence[0].line, Some(3));
    assert_eq!(named(&report, "shop-make").evidence[0].line, Some(2));
}

#[test]
fn data_files_below_the_root_add_their_detail() {
    let report = analyze();
    let a = named(&report, "shop-from-file");
    let d = a.detail.as_ref().expect("detail");
    assert_eq!(d.settings["number_of_shards"], "3");
    assert_eq!(d.fields[0].path, "sku");
    assert!(
        a.evidence
            .iter()
            .any(|e| e.file == "scripts/bodies/orders.json"),
        "the body file is named in the evidence"
    );
}

#[test]
fn variables_outside_files_and_unreadable_input_are_gaps_never_silent() {
    let report = analyze();
    assert_eq!(
        gaps(&report, "api/requests.http").len(),
        3,
        "name variable, bad body, alias variable"
    );
    assert_eq!(
        gaps(&report, "scripts/setup.sh").len(),
        3,
        "${{INDEX}}, outside the root, absolute"
    );
    assert_eq!(gaps(&report, "broken/unreadable.sh").len(), 1);
    assert!(gaps(&report, "docker/Dockerfile").is_empty());
    assert!(gaps(&report, "make/Makefile").is_empty());
    assert!(!report.complete);
    for n in [
        "shop-outside",
        "shop-absolute",
        "shop-broken",
        "shop-unreadable",
    ] {
        assert!(
            http(&report).iter().all(|a| a.name.as_deref() != Some(n)),
            "{n} stays unknown"
        );
    }
}

#[test]
fn a_data_file_is_one_artifact_not_two() {
    let report = analyze();
    for file in ["scripts/bodies/orders.json", "scripts/bodies/attached.json"] {
        let owners: Vec<_> = report
            .elasticsearch
            .artifacts
            .iter()
            .filter(|a| a.evidence.iter().any(|e| e.file == file))
            .map(|a| format!("{} {}", a.scope, a.name.as_deref().unwrap_or("?")))
            .collect();
        assert_eq!(owners.len(), 1, "{file}: {owners:?}");
        assert!(owners[0].starts_with("http:scripts"), "{owners:?}");
    }
}

#[test]
fn attached_and_unusual_data_options_and_multiline_quotes_are_read() {
    let report = analyze();
    let paths = |n: &str| -> Vec<String> {
        let d = named(&report, n).detail.as_ref().expect("detail");
        d.fields.iter().map(|f| f.path.clone()).collect()
    };
    assert_eq!(paths("shop-attached"), ["att"], "-d@file");
    assert_eq!(
        named(&report, "shop-glued")
            .detail
            .as_ref()
            .expect("detail")
            .settings["number_of_shards"],
        "4",
        "-d'{{...}}'"
    );
    assert_eq!(
        named(&report, "shop-eq")
            .detail
            .as_ref()
            .expect("detail")
            .settings["number_of_shards"],
        "5",
        "--data=..."
    );
    assert_eq!(paths("shop-json"), ["j"], "--json");
    assert_eq!(paths("shop-lines"), ["m"], "a quote that spans lines");
    assert_eq!(named(&report, "shop-lines").evidence[0].line, Some(7));
    assert!(
        http(&report)
            .iter()
            .all(|a| a.name.as_deref() != Some("shop-usage")),
        "curl inside a quoted echo is text"
    );
    assert!(gaps(&report, "scripts/forms.sh").is_empty());
}

#[test]
fn a_body_ends_at_its_blank_line_so_comments_between_requests_do_not_break_it() {
    let report = analyze();
    for n in ["shop-one", "shop-two", "shop-three"] {
        assert_eq!(
            named(&report, n).scope,
            "http:notes",
            "{n} is read despite a comment after its body"
        );
    }
    let bad = gaps(&report, "notes/comments.http");
    assert_eq!(bad.len(), 1, "only shop-bad is a gap");
    assert!(
        bad[0].message.ends_with("Source line 19."),
        "the gap names the request line: {}",
        bad[0].message
    );
}
