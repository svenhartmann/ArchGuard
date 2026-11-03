use archguard_engine::{analyze_services, core::ServiceDependencyKind};
use std::{collections::BTreeSet, path::PathBuf};

#[test]
fn shop_example_has_two_business_cycles_and_separate_infrastructure() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/service-shop");
    let report = analyze_services(&root).unwrap();
    assert_eq!(
        report.services.len(),
        14,
        "the demo must include real service boundaries and infrastructure"
    );
    assert_eq!(report.compose_files.len(), 3);
    assert!(report.monorepo);
    assert!(report.complete, "{:?}", report.diagnostics);
    assert!(report.diagnostics.is_empty());
    let name = |id: &str| {
        report
            .services
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .name
            .clone()
    };
    let contexts: BTreeSet<_> = report
        .services
        .iter()
        .filter_map(|s| s.build_context.as_deref())
        .collect();
    assert_eq!(contexts.len(), 10);
    for context in contexts {
        assert!(
            root.join(context).join("server.js").is_file(),
            "{context} must contain a service implementation"
        );
    }
    let pairs = |kind| {
        report
            .dependencies
            .iter()
            .filter(|d| d.kind == kind)
            .map(|d| (name(&d.from), name(&d.to)))
            .collect::<BTreeSet<_>>()
    };
    let expected = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        pairs(ServiceDependencyKind::ConfiguredHttp),
        expected(&[
            ("gateway", "catalog"),
            ("gateway", "orders"),
            ("gateway", "inventory"),
            ("catalog", "pricing"),
            ("catalog", "inventory"),
            ("pricing", "promotions"),
            ("promotions", "catalog"),
            ("orders", "payments"),
            ("orders", "inventory"),
            ("orders", "shipping"),
            ("orders", "notifications"),
            ("payments", "risk"),
            ("risk", "orders"),
            ("shipping", "inventory"),
            ("shipping", "notifications"),
        ])
    );
    assert_eq!(
        pairs(ServiceDependencyKind::Startup),
        expected(&[
            ("catalog", "catalog-db"),
            ("pricing", "redis"),
            ("promotions", "redis"),
            ("orders", "orders-db"),
            ("inventory", "rabbitmq"),
            ("shipping", "rabbitmq"),
            ("notifications", "rabbitmq"),
        ])
    );
    let cycles: BTreeSet<BTreeSet<_>> = report
        .cycles
        .iter()
        .map(|cycle| {
            assert_eq!(
                cycle.kind,
                ServiceDependencyKind::ConfiguredHttp,
                "startup ordering must remain acyclic"
            );
            cycle.nodes.iter().map(|id| name(id)).collect()
        })
        .collect();
    assert_eq!(
        cycles,
        [
            vec!["orders", "payments", "risk"],
            vec!["catalog", "pricing", "promotions"]
        ]
        .into_iter()
        .map(|group| group.into_iter().map(str::to_owned).collect())
        .collect()
    );
    assert!(
        report
            .dependencies
            .iter()
            .any(|d| d.from.split("::").next() != d.to.split("::").next()),
        "the example must exercise cross-Compose dependencies"
    );
    for dependency in &report.dependencies {
        assert!(!dependency.evidence.is_empty());
        for evidence in &dependency.evidence {
            assert!(root.join(&evidence.file).is_file());
            assert!(evidence.pointer.starts_with("/services/"));
        }
    }
}
