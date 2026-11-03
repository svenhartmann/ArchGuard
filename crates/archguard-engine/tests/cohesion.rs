use archguard_engine::{
    analyze_cohesion,
    core::{GraphBuilder, Ref},
};

fn graph() -> archguard_engine::core::Graph {
    let mut b = GraphBuilder::new();
    let a = b.ensure_path("pkg/a.ts", Some("ts"));
    let b1 = b.ensure_path("pkg/b.ts", Some("ts"));
    let c = b.ensure_path("pkg/c.ts", Some("ts"));
    let d = b.ensure_path("pkg/d.ts", Some("ts"));
    let x = b.ensure_path("other/x.ts", Some("ts"));
    b.ensure_path("alone/one.ts", Some("ts"));
    b.ensure_path("alone/two.ts", Some("ts"));
    b.ensure_pseudo("pkg");
    for (from, to, line) in [(a, b1, 1), (a, b1, 2), (c, d, 3), (a, x, 4), (x, d, 5)] {
        b.edge(
            from,
            to,
            Ref {
                file: "source.ts".into(),
                line,
                raw: "evidence".into(),
            },
        )
        .unwrap();
    }
    b.build("/project".into())
}

#[test]
fn distinct_dependency_pairs_and_disconnected_nontrivial_groups_support_split_review() {
    let report = analyze_cohesion(&graph(), None);
    let package = report.packages.iter().find(|p| p.node == "pkg").unwrap();
    assert_eq!(
        package.files, 4,
        "namespace placeholders are not source files"
    );
    assert_eq!(
        package.internal_dependencies, 2,
        "references are not distinct dependencies"
    );
    assert_eq!(package.incoming_dependencies, 1);
    assert_eq!(package.outgoing_dependencies, 1);
    assert_eq!(package.internal_share, Some(2.0 / 3.0));
    assert!(package.split_candidate);
    assert_eq!(package.groups.len(), 2);
    assert_eq!(package.groups[0].files, vec!["pkg/a.ts", "pkg/b.ts"]);
    assert_eq!(package.groups[0].boundaries[0].to, "other/x.ts");
    assert_eq!(package.groups[0].boundaries[0].evidence[0].line, 4);
    assert_eq!(package.co_change_status, "not_checked");
}

#[test]
fn isolated_files_and_single_file_packages_are_not_automatically_bad() {
    let report = analyze_cohesion(&graph(), None);
    let package = report.packages.iter().find(|p| p.node == "alone").unwrap();
    assert_eq!(package.internal_share, None);
    assert!(!package.split_candidate);
    assert_eq!(package.groups.len(), 2);
    assert!(package.reason.contains("insufficient"));
}

#[test]
fn nested_packages_keep_their_own_members_and_no_recursive_double_count() {
    let mut b = GraphBuilder::new();
    let a = b.ensure_path("a/root.ts", Some("ts"));
    let b1 = b.ensure_path("a/sub/leaf.ts", Some("ts"));
    b.edge(
        a,
        b1,
        Ref {
            file: "a/root.ts".into(),
            line: 1,
            raw: "leaf".into(),
        },
    )
    .unwrap();
    let report = analyze_cohesion(&b.build("/project".into()), None);
    let top = report.packages.iter().find(|p| p.node == "a").unwrap();
    assert_eq!(top.files, 1);
    assert_eq!(top.outgoing_dependencies, 1);
    assert_eq!(top.internal_dependencies, 0);
    assert_eq!(
        report
            .packages
            .iter()
            .find(|p| p.node == "a/sub")
            .unwrap()
            .incoming_dependencies,
        1
    );
}

#[test]
fn cross_group_co_change_warns_against_treating_groups_as_independent() {
    let graph = graph();
    let a = graph.nodes.iter().find(|n| n.name == "pkg/a.ts").unwrap();
    let c = graph.nodes.iter().find(|n| n.name == "pkg/c.ts").unwrap();
    let mut changes:archguard_engine::CoChangeReport=serde_json::from_value(serde_json::json!({
        "git":{"status":"available","since":"180.days","shallow":true,"truncated":false,"scanned_commits":3,"message":null},
        "options":{"since":"180.days","min_shared":3,"min_ratio":0.5,"max_results":50},
        "mapped_files":6,"total_files":6,"considered_commits":3,"skipped_bulk_commits":0,"pair_occurrences":3,"pair_search_truncated":false,"total_candidates":1,"results_truncated":false,"max_files_per_commit":100,"max_pair_occurrences":1000,"max_candidate_pairs":100,
        "pairs":[{"left":{"node_id":a.id,"name":a.name,"path":a.name},"right":{"node_id":c.id,"name":c.name,"path":c.name},"shared_commits":3,"left_commits":3,"right_commits":3,"left_to_right":1.0,"right_to_left":1.0,"jaccard":1.0,"evidence_commits":["abc"]}],"warnings":[]
    })).unwrap();
    changes.pairs.push(changes.pairs[0].clone());
    let report = analyze_cohesion(&graph, Some(&changes));
    let package = report.packages.iter().find(|p| p.node == "pkg").unwrap();
    assert_eq!(
        package.cross_group_co_changes, 1,
        "pair evidence deduplicated"
    );
    assert!(package.reason.contains("co-change"));
    assert_eq!(
        package.co_change_status, "partial",
        "shallow history is not full history"
    );
}
