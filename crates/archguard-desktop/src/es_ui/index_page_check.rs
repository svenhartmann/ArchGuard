//! The index page model: what the page shows for an index, a template and a ghost.

use archguard_engine::core::{
    SearchAnalysis, SearchArtifact, SearchField, SearchIndexDetail, SearchIndexUsage,
    SearchReference, ServiceDiagnostic, ServiceEvidence, ServiceTopology,
};
use archguard_engine::search_graph;

use super::index_page::*;

fn ev(file: &str, line: u32) -> ServiceEvidence {
    ServiceEvidence {
        file: file.into(),
        pointer: String::new(),
        line: Some(line),
        description: String::new(),
    }
}

fn art(kind: &str, name: &str, patterns: &[&str], refs: &[(&str, &str)]) -> SearchArtifact {
    SearchArtifact {
        flavor: String::new(),
        kind: kind.into(),
        name: Some(name.into()),
        address: format!("res.{name}"),
        scope: "main".into(),
        patterns: patterns.iter().map(|p| (*p).into()).collect(),
        references: refs
            .iter()
            .map(|(k, n)| SearchReference {
                kind: (*k).into(),
                name: (*n).into(),
            })
            .collect(),
        evidence: vec![ev("main.tf", 3)],
        origins: vec![],
        detail: None,
    }
}

fn field(path: &str, kind: &str) -> SearchField {
    SearchField {
        path: path.into(),
        kind: kind.into(),
        ..Default::default()
    }
}

fn usage(service: &str, index: &str, access: &str, file: &str, line: u32) -> SearchIndexUsage {
    SearchIndexUsage {
        service: service.into(),
        index: index.into(),
        access: access.into(),
        evidence: vec![ev(file, line)],
    }
}

fn topology() -> ServiceTopology {
    let mut orders = art("index", "orders", &[], &[]);
    orders.origins = vec!["infra".into()];
    let mut title = field("title", "text");
    title.analyzer = Some("german".into());
    title.search_analyzer = Some("std".into());
    let mut raw = field("title.raw", "keyword");
    raw.normalizer = Some("lower".into());
    orders.detail = Some(SearchIndexDetail {
        fields: vec![
            field("customer", "object"),
            field("customer.name", "text"),
            title,
            raw,
        ],
        settings: [
            ("refresh_interval", "30s"),
            ("zeta", "z"),
            ("number_of_replicas", "2"),
            ("alpha", "a"),
            ("number_of_shards", "1"),
            ("default_pipeline", "p1"),
            ("final_pipeline", "p2"),
            ("lifecycle.name", "hot"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect(),
        analysis: vec![SearchAnalysis {
            kind: "analyzer".into(),
            name: "german".into(),
            definition: "{\"type\":\"custom\"}".into(),
        }],
    });
    let mut t = ServiceTopology::default();
    let es = &mut t.elasticsearch;
    es.projects = vec!["main".into(), "adapter".into()];
    es.artifacts = vec![
        orders,
        art("index", "plain", &[], &[]),
        art("index", "computed", &[], &[]),
        art("alias", "orders-alias", &[], &[("index", "orders")]),
        art("index_template", "tpl", &["ord*"], &[]),
        art("index_template", "other", &["zzz*"], &[]),
        art("transform", "tf", &[], &[("index", "orders")]),
        art("transform", "tf-other", &[], &[("index", "plain")]),
        art("data_view", "dv", &["orders*"], &[]),
        art("watch", "w", &["orders"], &[]),
    ];
    es.usages = vec![
        usage("writer", "orders", "write", "w.py", 7),
        usage("reader", "orders-alias", "read", "r.py", 9),
        usage("adapter:feed", "orders", "write", "adapter:f.py", 2),
        usage("writer", "plain", "write", "w.py", 1),
    ];
    t.diagnostics.push(ServiceDiagnostic {
        file: "main.tf".into(),
        service: None,
        message: "res.computed: index mapping, settings or analysis are not literal (file(), local., var. and references are not evaluated); not read: mappings.".into(),
    });
    t
}

fn page(t: &ServiceTopology, title: &str) -> IndexPage {
    let g = search_graph(t);
    let n = g
        .nodes
        .iter()
        .find(|n| n.title == title || n.title.starts_with(&format!("{title} ·")))
        .expect(title);
    index_page(t, n)
}

fn link(kind: &str, name: &str) -> Link {
    Link {
        kind: kind.into(),
        name: name.into(),
    }
}

#[test]
fn overview_has_declaration_scope_address_and_origins_a_ghost_has_none() {
    let t = topology();
    let o = page(&t, "orders").overview;
    assert_eq!(o.declaration, Some(("main.tf".into(), Some(3))));
    assert_eq!(
        (o.scope.as_str(), o.address.as_str()),
        ("main", "res.orders")
    );
    assert_eq!(o.origins, ["infra"]);
    assert!(page(&t, "plain").overview.origins.is_empty());
    let mut t = topology();
    t.elasticsearch
        .usages
        .push(usage("writer", "ghost", "write", "w.py", 4));
    let g = page(&t, "ghost");
    assert_eq!(g.overview.declaration, None);
    assert_eq!(g.note, Some(DetailNote::NotDeclared));
    assert_eq!(g.usages.len(), 1);
}

#[test]
fn mapping_rows_have_depth_short_name_type_and_analyzers() {
    let m = page(&topology(), "orders").mapping;
    let rows: Vec<_> = m
        .iter()
        .map(|r| (r.depth, r.name.as_str(), r.kind.as_str(), r.parent))
        .collect();
    assert_eq!(
        rows,
        [
            (0, "customer", "object", true),
            (1, "name", "text", false),
            (0, "title", "text", false),
            (1, "raw", "keyword", false),
        ]
    );
    let t = &m[2];
    assert_eq!(t.analyzer.as_deref(), Some("german"));
    assert_eq!(t.search_analyzer.as_deref(), Some("std"));
    assert_eq!(m[3].normalizer.as_deref(), Some("lower"));
}

#[test]
fn settings_come_in_fixed_order_then_alphabetical_without_link_settings() {
    let p = page(&topology(), "orders");
    let keys: Vec<_> = p.settings.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        [
            "number_of_shards",
            "number_of_replicas",
            "refresh_interval",
            "alpha",
            "zeta"
        ]
    );
    assert_eq!(p.analysis.len(), 1);
    assert_eq!(
        (p.analysis[0].kind.as_str(), p.analysis[0].name.as_str()),
        ("analyzer", "german")
    );
    assert_eq!(p.note, None);
}

#[test]
fn links_are_aliases_pipelines_ilm_templates_and_things_pointing_at_the_index() {
    let l = page(&topology(), "orders").links;
    for want in [
        link("alias", "orders-alias"),
        link("ingest_pipeline", "p1"),
        link("ingest_pipeline", "p2"),
        link("ilm_policy", "hot"),
        link("index_template", "tpl"),
        link("transform", "tf"),
        link("data_view", "dv"),
        link("watch", "w"),
    ] {
        assert!(l.contains(&want), "{want:?} missing in {l:?}");
    }
    assert!(!l.contains(&link("index_template", "other")));
    assert!(!l.contains(&link("transform", "tf-other")));
}

#[test]
fn usages_cover_the_index_and_its_aliases_and_foreign_ones_are_not_clickable() {
    let u = page(&topology(), "orders").usages;
    let rows: Vec<_> = u
        .iter()
        .map(|u| {
            (
                u.service.as_str(),
                u.access.as_str(),
                u.file.as_str(),
                u.line,
                u.clickable,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("writer", "write", "w.py", Some(7), true),
            ("reader", "read", "r.py", Some(9), true),
            ("adapter:feed", "write", "adapter:f.py", Some(2), false),
        ]
    );
}

#[test]
fn without_detail_the_note_says_not_read_when_a_gap_names_the_address_else_not_declared() {
    let t = topology();
    let c = page(&t, "computed");
    assert_eq!(c.note, Some(DetailNote::NotRead));
    assert!(c.mapping.is_empty() && c.settings.is_empty());
    assert_eq!(page(&t, "plain").note, Some(DetailNote::NotDeclared));
    // Another gap naming the address is no "not read".
    let mut t = topology();
    t.diagnostics[0].message =
        "res.computed: Elasticsearch name is not a literal; mappings is unknown.".into();
    assert_eq!(page(&t, "computed").note, Some(DetailNote::NotDeclared));
    // A detail gap of another address is none either.
    let mut t = topology();
    t.diagnostics[0].message = t.diagnostics[0]
        .message
        .replace("res.computed", "res.plain");
    assert_eq!(page(&t, "computed").note, Some(DetailNote::NotDeclared));
}

/// The Slint rows of the page: own usages open the editor at their line, foreign ones do not.
#[test]
fn the_page_view_keeps_own_usages_clickable_and_foreign_ones_not() {
    use slint::Model;
    let mut s = super::state::build_from(std::sync::Arc::new(Ok(topology())));
    let at = s
        .graph
        .nodes
        .iter()
        .position(|n| n.title.starts_with("orders ·"));
    s.focus_on(at.expect("orders"));
    let p = super::page_view::page(&s).expect("an index has a page");
    let rows: Vec<_> = p
        .usages
        .iter()
        .map(|u| (u.text.to_string(), u.file.to_string(), u.line, u.clickable))
        .collect();
    assert_eq!(
        rows,
        [
            ("writer · write · w.py:7".into(), "w.py".into(), 7, true),
            ("reader · read · r.py:9".into(), "r.py".into(), 9, true),
            (
                "adapter:feed · write · adapter:f.py:2".into(),
                "adapter:f.py".into(),
                2,
                false
            ),
        ]
    );
}

/// The scope reads like the graph labels' root path: no `terraform:` prefix.
#[test]
fn the_overview_scope_is_the_short_root_path() {
    let mut t = topology();
    t.elasticsearch.artifacts[0].scope = "terraform:infra/main".into();
    assert_eq!(page(&t, "orders").overview.scope, "infra/main");
}
