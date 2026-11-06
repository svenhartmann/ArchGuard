//! The shared cases of `scripts/viewer/services_world_cases.json`: the
//! web's islands and visible connections on real topologies
//! (`scripts/viewer/test_viewer_services_world_cases.js` checks the web
//! against the same file).

use std::path::Path;

use serde_json::Value;

use archguard_engine::core::ServiceDependencyKind as Kind;

use super::links::{Direction, Source};
use super::world::{KINDS, ServicesWorld};

fn repo() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn kind_name(k: Kind) -> String {
    serde_json::to_value(k)
        .expect("kind")
        .as_str()
        .expect("name")
        .to_string()
}

fn apply(w: &mut ServicesWorld, c: &Value) {
    let text = |k: &str| c[k].as_str().unwrap_or_default().to_string();
    let deployment = Some(text("deployment")).filter(|d| d != "all");
    w.filter.source = match text("source").as_str() {
        "code" => Source::Code,
        "configuration" => Source::Configuration,
        "observed" => Source::Observed,
        _ => Source::All,
    };
    w.filter.kind = KINDS.into_iter().find(|&k| kind_name(k) == text("kind"));
    w.filter.direction = match text("direction").as_str() {
        "incoming" => Direction::Incoming,
        "outgoing" => Direction::Outgoing,
        "downstream" => Direction::Downstream,
        "upstream" => Direction::Upstream,
        _ => Direction::Both,
    };
    w.filter.cycles_only = c["cycles_only"].as_bool().unwrap_or(false);
    w.set_islands(deployment, c["all_projects"].as_bool().unwrap_or(false));
    let selected = c["selected"].as_str();
    w.selected = selected.and_then(|p| w.inventory.iter().position(|i| i.path == p));
}

fn actual(w: &ServicesWorld) -> (Vec<String>, Vec<Value>) {
    let islands = w
        .shown
        .iter()
        .map(|&i| w.inventory[i].path.clone())
        .collect();
    let path = |i: usize| w.inventory[i].path.clone();
    let links = w
        .visible()
        .into_iter()
        .map(|i| {
            let l = &w.links[i];
            serde_json::json!([
                path(l.from),
                path(l.to),
                kind_name(l.kind),
                l.cyclic,
                l.evidence.len()
            ])
        })
        .collect();
    (islands, links)
}

#[test]
fn shared_cases_match_the_web() {
    let file = repo().join("scripts/viewer/services_world_cases.json");
    let shared: Value =
        serde_json::from_str(&std::fs::read_to_string(file).expect("cases")).expect("json");
    let mut count = 0;
    for entry in shared.as_array().expect("topologies") {
        let name = entry["topology"].as_str().expect("topology");
        let topology = archguard_engine::analyze_services(&repo().join(name)).expect("topology");
        let mut w = ServicesWorld::new(std::sync::Arc::new(Ok(topology))).expect(name);
        let default = entry["default_deployment"].as_str().filter(|d| *d != "all");
        assert_eq!(
            w.filter.deployment.as_deref(),
            default,
            "{name}: default variant"
        );
        for c in entry["cases"].as_array().expect("cases") {
            apply(&mut w, c);
            let label = format!("{name}: {}", c["case"]);
            let (islands, links) = actual(&w);
            let expected: Vec<String> =
                serde_json::from_value(c["islands"].clone()).expect("islands");
            assert_eq!(islands, expected, "{label} (islands)");
            assert_eq!(
                &links,
                c["connections"].as_array().expect("connections"),
                "{label}"
            );
            count += 1;
        }
    }
    assert_eq!(count, 19);
}
