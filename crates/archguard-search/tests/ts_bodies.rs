//! TypeScript/JavaScript client calls with object literals become artifacts (ticket 26).
use archguard_core::{SearchArtifact, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/26")
}

fn analyze() -> ServiceTopology {
    let root = root();
    let mut report = ServiceTopology::default();
    let discovery = Discovery::new(&root, &mut report).expect("discovery");
    let read = |_: &Path, p: &Path| std::fs::read_to_string(p).ok();
    archguard_search::analyze(&root, &discovery, &read, &mut report);
    report
}

fn ts(report: &ServiceTopology) -> Vec<&SearchArtifact> {
    let mut all: Vec<_> = report
        .elasticsearch
        .artifacts
        .iter()
        .filter(|a| a.scope.starts_with("ts:"))
        .collect();
    all.sort_by_key(|a| a.name.clone());
    all
}

fn named<'a>(report: &'a ServiceTopology, name: &str) -> &'a SearchArtifact {
    ts(report)
        .into_iter()
        .find(|a| a.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no artifact {name}"))
}

fn gaps(report: &ServiceTopology) -> Vec<String> {
    report
        .diagnostics
        .iter()
        .filter(|d| archguard_search::is_elasticsearch_gap(d))
        .map(|d| format!("{}: {}", d.file, d.message))
        .collect()
}

#[test]
fn client_calls_become_artifacts_and_lookalikes_do_not() {
    let report = analyze();
    let got: Vec<String> = ts(&report)
        .iter()
        .map(|a| {
            format!(
                "{} {} {} [{}]",
                a.kind,
                a.name.as_deref().unwrap_or("?"),
                a.scope,
                a.flavor
            )
        })
        .collect();
    let want = [
        "index calls-v1 ts:shop []",
        "index consts-v1 ts:shop []",
        "index deep-v1 ts:shop []",
        "index esc-v1 ts:shop []",
        "index late-v1 ts:shop []",
        "index legacy-v1 ts:shop []",
        "component_template logs-base ts:shop []",
        "ilm_policy logs-ilm ts:shop []",
        "ingest_pipeline logs-pipe ts:shop []",
        "index_template logs-tpl ts:shop []",
        "index orders-v1 ts:shop []",
        "index os-v1 ts:search [opensearch]",
        "index partial-v1 ts:shop []",
        "index self-v1 ts:shop []",
        "index umlaut-v1 ts:shop []",
    ];
    assert_eq!(
        got, want,
        "lookalikes, foreign clients and test files must not match"
    );
}

fn fields(a: &SearchArtifact) -> Vec<String> {
    let d = a.detail.as_ref().expect("detail");
    d.fields
        .iter()
        .map(|f| {
            format!(
                "{}:{}:{}",
                f.path,
                f.kind,
                f.analyzer.as_deref().unwrap_or("")
            )
        })
        .collect()
}

#[test]
fn client_8_top_level_and_client_7_body_give_detail_with_evidence() {
    let report = analyze();
    let a = named(&report, "orders-v1");
    assert_eq!(fields(a), ["sku:keyword:", "title:text:standard"]);
    let d = a.detail.as_ref().unwrap();
    assert_eq!(d.settings["number_of_shards"], "2");
    assert_eq!(d.settings["refresh_interval"], "5s");
    assert_eq!(a.address, "shop/src/setup.ts:6");
    assert_eq!(a.evidence[0].file, "shop/src/setup.ts");
    assert_eq!(a.evidence[0].line, Some(6));
    let legacy = named(&report, "legacy-v1");
    assert_eq!(fields(legacy), ["name:text:"]);
    assert_eq!(
        legacy.detail.as_ref().unwrap().settings["number_of_shards"],
        "1"
    );
    let os = named(&report, "os-v1");
    assert_eq!(
        os.detail.as_ref().unwrap().settings["number_of_shards"],
        "3"
    );
}

#[test]
fn templates_policies_pipelines_and_mappings() {
    let report = analyze();
    let t = named(&report, "logs-tpl");
    assert_eq!(t.patterns, ["logs-*"]);
    assert_eq!(t.references.len(), 1);
    assert_eq!(t.references[0].name, "logs-base");
    assert_eq!(
        t.detail.as_ref().unwrap().settings["number_of_replicas"],
        "1"
    );
    assert_eq!(fields(named(&report, "logs-base")), ["ts:date:"]);
    assert_eq!(fields(named(&report, "late-v1")), ["extra:keyword:"]);
}

#[test]
fn module_constants_resolve_and_non_literals_are_gaps_per_attribute() {
    let report = analyze();
    let c = named(&report, "consts-v1");
    assert_eq!(fields(c), ["code:keyword:"]);
    assert_eq!(
        c.detail.as_ref().unwrap().settings["number_of_replicas"],
        "0"
    );
    let p = named(&report, "partial-v1");
    assert_eq!(fields(p), ["id:keyword:"]);
    assert!(
        p.detail.as_ref().unwrap().settings.is_empty(),
        "spread: all or nothing"
    );
    assert!(named(&report, "calls-v1").detail.is_none());
    let got = gaps(&report);
    let has = |needle: &str| got.iter().any(|g| g.contains(needle));
    assert!(
        has("shop/src/consts.ts:11: ") && has("not read: settings."),
        "{got:?}"
    );
    assert!(
        has("shop/src/consts.ts:16: ") && has("not read: mappings."),
        "{got:?}"
    );
    assert!(
        has("shop/src/consts.ts: Index name is not a literal"),
        "{got:?}"
    );
    assert!(
        !got.iter()
            .any(|g| g.contains("lookalike") || g.contains("noimport")),
        "{got:?}"
    );
}

#[test]
fn hostile_literals_are_gaps_not_crashes_or_wrong_values() {
    let report = analyze();
    let got = gaps(&report);
    let gap = |name: &str| {
        let a = named(&report, name);
        assert_eq!(
            a.detail.as_ref().unwrap().settings["number_of_shards"],
            "1",
            "the literal settings of {name} stay"
        );
        assert!(a.detail.as_ref().unwrap().fields.is_empty(), "{name}");
        let at = format!("shop/src/hostile.ts:{}: ", a.evidence[0].line.unwrap());
        assert!(
            got.iter()
                .any(|g| g.contains(&at) && g.contains("not read: mappings.")),
            "{name}: {got:?}"
        );
    };
    gap("self-v1");
    gap("deep-v1");
    gap("esc-v1");
}

#[test]
fn non_ascii_text_in_a_skipped_member_is_a_gap_not_a_panic() {
    let report = analyze();
    let a = named(&report, "umlaut-v1");
    assert_eq!(a.detail.as_ref().unwrap().fields.len(), 1);
    let at = format!("shop/src/nonascii.ts:{}: ", a.evidence[0].line.unwrap());
    assert!(
        gaps(&report)
            .iter()
            .any(|g| g.contains(&at) && g.contains("not read: settings")),
        "{:?}",
        gaps(&report)
    );
}
