use archguard_engine::{
    analysis::analyze_review,
    core::{Cut, GitEvidence, GitStatus, GraphBuilder, ReviewSources, SourceEvidence, aggregate},
};
use std::{fs, process::Command};
#[test]
fn browser_and_rust_agree_on_every_review_field_at_several_scopes() {
    let mut builder = GraphBuilder::new();
    let a = builder.ensure_path("A/a.ts", None);
    let b = builder.ensure_path("B/b.ts", None);
    let c = builder.ensure_path("B/c.ts", None);
    let q = builder.ensure_path("Quiet/q.ts", None);
    for (from, to) in [(a, b), (b, a), (a, c)] {
        builder
            .edge(
                from,
                to,
                archguard_engine::core::Ref {
                    file: "A/a.ts".into(),
                    line: 1,
                    raw: "import target".into(),
                },
            )
            .unwrap();
    }
    let graph = builder.build("/tmp/parity".into());
    let sources = ReviewSources {
        files: [(a, Some(4)), (b, Some(0)), (c, None), (q, Some(0))]
            .into_iter()
            .map(|(id, changes)| SourceEvidence {
                node_id: id,
                path: Some(format!("source-{}.ts", id.0)),
                lines: Some(12),
                changes,
            })
            .collect(),
        git: GitEvidence {
            status: GitStatus::Available,
            since: "180.days".into(),
            shallow: true,
            truncated: false,
            scanned_commits: 4,
            message: None,
        },
        mapped_files: 4,
        total_files: 4,
    };
    let dir = tempfile::tempdir().unwrap();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    for depth in [0, 1, 99] {
        let view = aggregate(&graph, &Cut::at_depth(&graph, depth));
        let expected = analyze_review(&graph, &view, &sources, &["Quiet/**".into()]).unwrap();
        let input = serde_json::json!({"full":graph,"view":view,"context":{"sources":sources,"entry_point_ids":expected.entry_point_ids,"entry_points":expected.entry_points},"expected":expected});
        let path = dir.path().join("parity.json");
        fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
        let out = Command::new("node")
            .current_dir(repo)
            .arg("scripts/viewer/test_review_parity.js")
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "depth{depth}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
