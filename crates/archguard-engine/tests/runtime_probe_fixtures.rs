//! Real output of the ArchGuard probes (ADR 0004): `fixtures/runtime/shop-java.jsonl`
//! from hse-perf-java and `fixtures/runtime/shop-dotnet.jsonl` from hse-perf-net,
//! each recorded from its sample shop with a known N+1 endpoint. They keep the
//! importer and the producers on the same contract (probes/CONVENTIONS.md).
use archguard_engine::{core::RuntimeProfile, runtime_profile};
use std::fs;

fn profile(fixture: &str) -> RuntimeProfile {
    let path = format!(
        "{}/../../fixtures/runtime/{fixture}",
        env!("CARGO_MANIFEST_DIR")
    );
    let dir = tempfile::tempdir().unwrap();
    let run = dir.path().join(".archguard/runtime/run");
    fs::create_dir_all(&run).unwrap();
    fs::copy(&path, run.join("traces-0001.jsonl")).unwrap();
    fs::write(
        dir.path().join("archguard.toml"),
        "[runtime]\nn_plus_one_min_calls = 3\n",
    )
    .unwrap();
    runtime_profile(dir.path(), None).expect("probe output is runtime input")
}

/// The single N+1 finding: (function, file, line). The .NET probe cuts absolute
/// paths below the project root (CONVENTIONS.md).
fn n_plus_one(p: &RuntimeProfile) -> (String, String, u32) {
    let found: Vec<_> = p
        .findings
        .iter()
        .filter(|f| f.kind == "n_plus_one")
        .collect();
    assert_eq!(found.len(), 1, "{:#?}", p.findings);
    let site = found[0].call_site.clone().expect("call site");
    assert_eq!(site.source.as_deref(), Some("stack"));
    (
        site.function.unwrap_or_default(),
        site.file.unwrap_or_default(),
        site.line.unwrap_or_default(),
    )
}

fn assert_clean(fixture: &str) {
    let path = format!(
        "{}/../../fixtures/runtime/{fixture}",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = fs::read_to_string(path).unwrap();
    for forbidden in [
        "/Users/",
        "/home/",
        "host.name",
        "process.command",
        "process.executable",
        "db.user",
        "user_agent.original",
        "client.address",
        "exception.message",
        "url.query",
    ] {
        assert!(!text.contains(forbidden), "{fixture} contains {forbidden}");
    }
}

#[test]
fn java_probe_output_finds_the_n_plus_one_at_its_line() {
    let p = profile("shop-java.jsonl");
    assert!(p.complete, "{:#?}", p.diagnostics);
    assert_eq!(p.sampling, vec!["always_on"]);
    let (function, file, line) = n_plus_one(&p);
    assert_eq!(function, "com.example.shop.OrderRepository.findProduct");
    assert!(
        file.ends_with("com/example/shop/OrderRepository.java"),
        "{file}"
    );
    assert_eq!(line, 38);
    // The endpoint ran more than once; the request with the most calls holds the N+1.
    let request = p
        .requests
        .iter()
        .filter(|r| r.name.contains("n-plus-one"))
        .max_by_key(|r| r.db_calls)
        .unwrap();
    assert!(request.db_calls >= 5, "{}", request.db_calls);
    assert!(
        request
            .spans
            .iter()
            .any(|s| s.db.as_deref().is_some_and(|d| d.starts_with("SELECT")))
    );
    assert_clean("shop-java.jsonl");
}

#[test]
fn dotnet_probe_output_finds_the_n_plus_one_at_its_line() {
    let p = profile("shop-dotnet.jsonl");
    assert!(p.complete, "{:#?}", p.diagnostics);
    assert_eq!(p.sampling, vec!["always_on"]);
    let (function, file, line) = n_plus_one(&p);
    assert_eq!(function, "Shop.Orders.OrderEndpoints.GetOrderSummary");
    assert!(file == "Orders/OrderEndpoints.cs", "{file}");
    assert_eq!(line, 36);
    // The endpoint ran more than once; the request with the most calls holds the N+1.
    let request = p
        .requests
        .iter()
        .filter(|r| r.name.contains("summary"))
        .max_by_key(|r| r.db_calls)
        .unwrap();
    assert!(request.db_calls >= 6, "{}", request.db_calls);
    assert_clean("shop-dotnet.jsonl");
}

#[test]
fn outgoing_calls_name_their_target_without_query() {
    for fixture in ["shop-java.jsonl", "shop-dotnet.jsonl"] {
        let p = profile(fixture);
        let target = p
            .requests
            .iter()
            .flat_map(|r| &r.spans)
            .find(|s| s.kind == "client" && s.db.is_none())
            .unwrap_or_else(|| panic!("{fixture}: an outgoing call"));
        assert!(
            target.name.contains("/carrier/quote"),
            "{fixture}: {}",
            target.name
        );
        assert!(!target.name.contains('?'), "{fixture}: {}", target.name);
    }
}
