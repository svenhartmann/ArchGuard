//! Which records a section lists: `engineeringRows` and
//! `engineeringScope` of `viewer_engineering_quality.js` in Rust.

use std::collections::{BTreeMap, BTreeSet};

use archguard_engine::{
    ClassMetrics, CohesionPackage, DuplicateGroup, KnowledgeMetric, MeasurementGroup,
    MethodMetrics, ModuleQualitySnapshot, MutationFile, MutationRecord, QualityHistoryReport,
    TestCaseEvidence, TestSourceMapping,
};

/// One node of the original source graph (graph order = tree order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeInfo {
    pub name: String,
    pub parent: Option<usize>,
    pub module: bool,
}

/// The source graph the sections filter with.
#[derive(Clone, Debug, Default)]
pub struct Source {
    pub nodes: Vec<NodeInfo>,
    /// File dependencies `(from, to)` as node indices.
    pub edges: Vec<(usize, usize)>,
    /// Scanned files `(node, path)` (review sources).
    pub files: Vec<(usize, String)>,
}

/// The selected package or file with everything below it.
#[derive(Clone, Debug, Default)]
pub struct Scope {
    /// `None`: the entire source scope.
    pub ids: Option<BTreeSet<usize>>,
    pub names: BTreeSet<String>,
    pub paths: BTreeSet<String>,
}

impl Scope {
    pub fn all(&self) -> bool {
        self.ids.is_none()
    }

    fn has(&self, name: &str) -> bool {
        self.all() || self.names.contains(name)
    }
}

/// `engineeringScope`: the target node and its descendants.
pub fn scope(source: &Source, target: &str) -> Scope {
    if target.is_empty() {
        return Scope::default();
    }
    let mut ids = BTreeSet::new();
    if let Some(start) = source.nodes.iter().position(|n| n.name == target) {
        let mut children: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (i, n) in source.nodes.iter().enumerate() {
            if let Some(p) = n.parent {
                children.entry(p).or_default().push(i);
            }
        }
        let mut stack = vec![start];
        ids.insert(start);
        while let Some(n) = stack.pop() {
            for &c in children.get(&n).into_iter().flatten() {
                if ids.insert(c) {
                    stack.push(c);
                }
            }
        }
    }
    Scope {
        names: ids.iter().map(|&i| source.nodes[i].name.clone()).collect(),
        paths: source
            .files
            .iter()
            .filter(|(n, _)| ids.contains(n))
            .map(|(_, p)| p.clone())
            .collect(),
        ids: Some(ids),
    }
}

/// `engineeringMatches`: case-insensitive substring.
pub fn matches(query: &str, text: &str) -> bool {
    query.is_empty() || text.to_lowercase().contains(&query.to_lowercase())
}

/// Highest first; unknown (`None`) counts as -1, as in the web.
fn desc(a: Option<f64>, b: Option<f64>) -> std::cmp::Ordering {
    b.unwrap_or(-1.0).total_cmp(&a.unwrap_or(-1.0))
}

/// Heatmap and class metrics, by CBO (unknown last), then class id.
pub fn classes<'a>(rows: &'a [ClassMetrics], scope: &Scope, query: &str) -> Vec<&'a ClassMetrics> {
    let mut out: Vec<_> = rows
        .iter()
        .filter(|r| {
            scope.has(&r.node) || scope.names.contains(&r.file) || scope.paths.contains(&r.file)
        })
        .filter(|r| matches(query, &format!("{}  {}", r.class_id, r.file)))
        .collect();
    let cbo = |r: &ClassMetrics| r.cbo.value.map(|v| v as f64);
    out.sort_by(|a, b| desc(cbo(a), cbo(b)).then_with(|| a.class_id.cmp(&b.class_id)));
    out
}

/// Method maintainability, by Cognitive (unknown last), then method id.
pub fn methods<'a>(
    rows: &'a [MethodMetrics],
    scope: &Scope,
    query: &str,
) -> Vec<&'a MethodMetrics> {
    let node = |r: &MethodMetrics| r.node.as_deref().is_some_and(|n| scope.names.contains(n));
    let mut out: Vec<_> = rows
        .iter()
        .filter(|r| {
            scope.all() || node(r) || scope.names.contains(&r.file) || scope.paths.contains(&r.file)
        })
        .filter(|r| matches(query, &format!("{} {} {}", r.class_id, r.method_id, r.file)))
        .collect();
    let cognitive = |r: &MethodMetrics| r.cognitive.map(f64::from);
    out.sort_by(|a, b| {
        desc(cognitive(a), cognitive(b)).then_with(|| a.method_id.cmp(&b.method_id))
    });
    out
}

/// Knowledge concentration, by largest share, touches, node.
pub fn knowledge<'a>(
    rows: &'a [KnowledgeMetric],
    scope: &Scope,
    query: &str,
) -> Vec<&'a KnowledgeMetric> {
    let mut out: Vec<_> = rows
        .iter()
        .filter(|r| scope.has(&r.node) && matches(query, &r.node))
        .collect();
    out.sort_by(|a, b| {
        desc(a.top_share, b.top_share)
            .then(b.touches.cmp(&a.touches))
            .then_with(|| a.node.cmp(&b.node))
    });
    out
}

/// Build, runtime and test runs; project-wide, so only the search applies.
pub fn measurements<'a>(rows: &'a [MeasurementGroup], query: &str) -> Vec<&'a MeasurementGroup> {
    rows.iter()
        .filter(|r| {
            let text = [&r.kind, &r.name, &r.revision, &r.environment, &r.producer];
            matches(query, &text.map(String::as_str).join(" "))
        })
        .collect()
}

/// Package cohesion: split candidates first, then by name.
pub fn cohesion<'a>(
    rows: &'a [CohesionPackage],
    scope: &Scope,
    query: &str,
) -> Vec<&'a CohesionPackage> {
    let inside = |p: &CohesionPackage| {
        scope.has(&p.node)
            || p.groups
                .iter()
                .any(|g| g.files.iter().any(|f| scope.names.contains(f)))
    };
    let mut out: Vec<_> = rows
        .iter()
        .filter(|p| inside(p) && matches(query, &p.node))
        .collect();
    out.sort_by(|a, b| {
        b.split_candidate
            .cmp(&a.split_candidate)
            .then_with(|| a.node.cmp(&b.node))
    });
    out
}

/// A module of the latest quality snapshot, or one the comparison removed.
#[derive(Clone, Debug)]
pub struct HistoryRow<'a> {
    pub node: String,
    pub snapshot: Option<&'a ModuleQualitySnapshot>,
}

/// Quality history: the target's lineage (itself and its ancestors) plus
/// its subtree; removed modules below a selected package stay listed.
pub fn history<'a>(
    report: &'a QualityHistoryReport,
    source: &Source,
    target: &str,
    scope: &Scope,
    query: &str,
) -> Vec<HistoryRow<'a>> {
    let mut lineage = scope.names.clone();
    let selected = source.nodes.iter().position(|n| n.name == target);
    let removed_prefix = selected
        .filter(|&i| source.nodes[i].module)
        .map(|i| format!("{}/", source.nodes[i].name.trim_end_matches('/')));
    if selected.is_some_and(|i| !source.nodes[i].module && source.nodes[i].parent.is_none()) {
        lineage.insert(".".into());
    }
    let mut seen = BTreeSet::new();
    let mut at = selected;
    while let Some(i) = at.filter(|&i| seen.insert(i)) {
        lineage.insert(source.nodes[i].name.clone());
        at = source.nodes[i].parent;
    }
    let latest = report
        .latest
        .as_ref()
        .map_or(&[][..], |l| l.modules.as_slice());
    let present: BTreeSet<&str> = latest.iter().map(|m| m.node.as_str()).collect();
    let removed = report
        .comparison
        .iter()
        .flat_map(|c| &c.modules)
        .filter(|m| m.status == "removed" && !present.contains(m.node.as_str()))
        .map(|m| HistoryRow {
            node: m.node.clone(),
            snapshot: None,
        });
    let keep = |r: &HistoryRow| {
        let below = || {
            r.snapshot.is_none()
                && removed_prefix
                    .as_ref()
                    .is_some_and(|p| r.node.starts_with(p.as_str()))
        };
        (scope.all() || lineage.contains(&r.node) || below()) && matches(query, &r.node)
    };
    latest
        .iter()
        .map(|m| HistoryRow {
            node: m.node.clone(),
            snapshot: Some(m),
        })
        .chain(removed)
        .filter(keep)
        .collect()
}

/// For each node, the nodes with an edge into it.
fn incoming(source: &Source) -> BTreeMap<usize, BTreeSet<usize>> {
    let mut map: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for &(from, to) in &source.edges {
        map.entry(to).or_default().insert(from);
    }
    map
}

#[derive(Clone, Debug)]
pub struct TestRow<'a> {
    pub test: &'a TestCaseEvidence,
    pub matched: Vec<&'a TestSourceMapping>,
    pub unknown: bool,
}

/// Tests and change impact: tests mapped to the scope or to its transitive
/// dependents first, then by duration; unknown mappings stay listed.
pub fn tests<'a>(
    rows: &'a [TestCaseEvidence],
    source: &Source,
    scope: &Scope,
    query: &str,
) -> Vec<TestRow<'a>> {
    let mut affected = scope.ids.clone().unwrap_or_default();
    let into = incoming(source);
    let mut stack: Vec<usize> = affected.iter().copied().collect();
    while let Some(n) = stack.pop() {
        for &from in into.get(&n).into_iter().flatten() {
            if affected.insert(from) {
                stack.push(from);
            }
        }
    }
    let names: BTreeSet<&str> = affected
        .iter()
        .map(|&i| source.nodes[i].name.as_str())
        .collect();
    let mut out: Vec<TestRow> = rows
        .iter()
        .map(|test| {
            let fresh: Vec<&TestSourceMapping> = test
                .mappings
                .iter()
                .filter(|m| m.node.is_some() && m.status == "unchanged_since_import")
                .collect();
            TestRow {
                test,
                unknown: fresh.is_empty() || fresh.len() < test.mappings.len(),
                matched: fresh
                    .into_iter()
                    .filter(|m| m.node.as_deref().is_some_and(|n| names.contains(n)))
                    .collect(),
            }
        })
        .filter(|r| scope.all() || !r.matched.is_empty() || r.unknown)
        .filter(|r| {
            matches(
                query,
                &format!("{} {} {}", r.test.name, r.test.suite, r.test.id),
            )
        })
        .collect();
    out.sort_by(|a, b| {
        b.matched
            .len()
            .cmp(&a.matched.len())
            .then(desc(a.test.duration_seconds, b.test.duration_seconds))
            .then_with(|| a.test.id.cmp(&b.test.id))
    });
    out
}

#[derive(Clone, Debug)]
pub struct MutationRow<'a> {
    pub file: &'a MutationFile,
    pub mutant: &'a MutationRecord,
    pub incoming: usize,
}

/// Test effectiveness: survived and uncovered first, then incoming files.
pub fn mutations<'a>(
    files: &'a [MutationFile],
    source: &Source,
    scope: &Scope,
    query: &str,
) -> Vec<MutationRow<'a>> {
    let into = incoming(source);
    let ids: BTreeMap<&str, usize> = source
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.name.as_str(), i))
        .collect();
    let mut out = Vec::new();
    for file in files {
        let node = file.node.as_deref().unwrap_or("");
        if !scope.has(node) {
            continue;
        }
        let incoming = ids
            .get(node)
            .and_then(|i| into.get(i))
            .map_or(0, BTreeSet::len);
        for mutant in &file.mutants {
            let text = format!("{} {} {}", file.path, mutant.mutator_name, mutant.status);
            if matches(query, &text) {
                out.push(MutationRow {
                    file,
                    mutant,
                    incoming,
                });
            }
        }
    }
    let rank = |s: &str| match s {
        "Survived" => 0,
        "NoCoverage" => 1,
        "Pending" => 2,
        _ => 3,
    };
    out.sort_by(|a, b| {
        rank(&a.mutant.status)
            .cmp(&rank(&b.mutant.status))
            .then(b.incoming.cmp(&a.incoming))
            .then_with(|| a.file.path.cmp(&b.file.path))
    });
    out
}

/// Code duplication, largest token count first.
pub fn duplicates<'a>(
    rows: &'a [DuplicateGroup],
    scope: &Scope,
    query: &str,
) -> Vec<&'a DuplicateGroup> {
    let mut out: Vec<_> = rows
        .iter()
        .filter(|g| {
            scope.all()
                || g.occurrences
                    .iter()
                    .any(|o| o.node.as_deref().is_some_and(|n| scope.names.contains(n)))
        })
        .filter(|g| {
            let paths: Vec<&str> = g.occurrences.iter().map(|o| o.path.as_str()).collect();
            matches(
                query,
                &format!("{} {} {} {}", g.id, g.status, paths.join(" "), g.snippet),
            )
        })
        .collect();
    out.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.id.cmp(&b.id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality_ui::heatmap::tests::{class, count};

    /// `a/` with files `a/x.rs`, `a/y.rs`; `b/` with `b/z.rs`.
    /// Edges: a/y.rs → a/x.rs, b/z.rs → a/y.rs.
    fn source() -> Source {
        let node = |name: &str, parent: Option<usize>, module: bool| NodeInfo {
            name: name.into(),
            parent,
            module,
        };
        Source {
            nodes: vec![
                node("a/", None, true),
                node("a/x.rs", Some(0), false),
                node("a/y.rs", Some(0), false),
                node("b/", None, true),
                node("b/z.rs", Some(3), false),
            ],
            edges: vec![(2, 1), (4, 2)],
            files: vec![(1, "crate/a/x.rs".into()), (4, "crate/b/z.rs".into())],
        }
    }

    #[test]
    fn scope_holds_the_target_subtree_and_its_scanned_paths() {
        let s = scope(&source(), "a/");
        assert_eq!(s.ids, Some(BTreeSet::from([0, 1, 2])));
        assert!(s.names.contains("a/y.rs") && !s.names.contains("b/z.rs"));
        assert_eq!(s.paths, BTreeSet::from(["crate/a/x.rs".to_string()]));
        assert!(scope(&source(), "").all());
        // An unknown target matches nothing, as in the web.
        let missing = scope(&source(), "nope/");
        assert_eq!(missing.ids, Some(BTreeSet::new()));
        assert!(matches("", "x") && matches("BASE", "rs::Baseline") && !matches("q", "x"));
    }

    #[test]
    fn classes_filter_by_scope_and_search_and_sort_by_cbo() {
        let mut rows = vec![
            class("rust:a/x.rs::B", "a/x.rs", None, None),
            class("rust:a/x.rs::A", "a/x.rs", None, None),
            class("rust:b/z.rs::C", "b/z.rs", None, None),
            class("rust:crate/a/x.rs::P", "crate/a/x.rs", Some(1), None),
        ];
        rows[3].node = "elsewhere".into();
        rows[0].cbo = count(Some(5), "ready");
        let all = classes(&rows, &Scope::default(), "");
        let ids: Vec<_> = all.iter().map(|r| r.class_id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "rust:a/x.rs::B",
                "rust:crate/a/x.rs::P",
                "rust:a/x.rs::A",
                "rust:b/z.rs::C"
            ]
        );
        let scoped = classes(&rows, &scope(&source(), "a/"), "");
        assert_eq!(
            scoped.len(),
            3,
            "node, file or scanned path inside the scope"
        );
        assert_eq!(classes(&rows, &Scope::default(), "z.RS").len(), 1);
    }

    fn method(id: &str, cognitive: Option<u32>) -> MethodMetrics {
        MethodMetrics {
            method_id: id.into(),
            node: Some("a/x.rs".into()),
            class_id: "A".into(),
            file: "a/x.rs".into(),
            line: 1,
            cognitive,
            halstead: None,
            source_lines: None,
            maintainability_index: None,
        }
    }

    #[test]
    fn methods_sort_by_cognitive_with_unknown_last() {
        let rows = vec![
            method("m1", None),
            method("m2", Some(3)),
            method("m0", Some(3)),
            method("m3", Some(9)),
        ];
        let ids: Vec<_> = methods(&rows, &Scope::default(), "")
            .iter()
            .map(|m| m.method_id.as_str())
            .collect();
        assert_eq!(ids, ["m3", "m0", "m2", "m1"]);
    }

    fn knowledge_row(node: &str, share: Option<f64>, touches: u64) -> KnowledgeMetric {
        KnowledgeMetric {
            node: node.into(),
            touches,
            contributors: 1,
            top_share: share,
            bus_factor_50: Some(1),
            authors: Vec::new(),
            status: "ready".into(),
        }
    }

    #[test]
    fn knowledge_sorts_by_share_then_touches_then_node() {
        let rows = vec![
            knowledge_row("b/z.rs", Some(1.0), 1),
            knowledge_row("a/x.rs", Some(1.0), 4),
            knowledge_row("a/y.rs", None, 9),
            knowledge_row("a/w.rs", Some(0.5), 2),
        ];
        let ids: Vec<_> = knowledge(&rows, &Scope::default(), "")
            .iter()
            .map(|r| r.node.as_str())
            .collect();
        assert_eq!(ids, ["a/x.rs", "b/z.rs", "a/w.rs", "a/y.rs"]);
        assert_eq!(knowledge(&rows, &scope(&source(), "b/"), "").len(), 1);
    }

    fn package(node: &str, split: bool, files: &[&str]) -> CohesionPackage {
        CohesionPackage {
            node: node.into(),
            files: files.len(),
            internal_dependencies: 0,
            incoming_dependencies: 0,
            outgoing_dependencies: 0,
            internal_share: None,
            groups: vec![archguard_engine::CohesionGroup {
                files: files.iter().map(|f| f.to_string()).collect(),
                internal_dependencies: 0,
                incoming_dependencies: 0,
                outgoing_dependencies: 0,
                boundaries: Vec::new(),
                boundaries_truncated: false,
            }],
            split_candidate: split,
            co_change_status: "not_checked".into(),
            cross_group_co_changes: 0,
            reason: String::new(),
        }
    }

    #[test]
    fn cohesion_lists_split_candidates_first_and_packages_with_scoped_files() {
        let rows = vec![
            package("b/", false, &["b/z.rs"]),
            package("c/", false, &[]),
            package("a/", true, &["a/x.rs"]),
        ];
        let ids: Vec<_> = cohesion(&rows, &Scope::default(), "")
            .iter()
            .map(|p| p.node.as_str())
            .collect();
        assert_eq!(ids, ["a/", "b/", "c/"]);
        let file = scope(&source(), "a/x.rs");
        let ids: Vec<_> = cohesion(&rows, &file, "")
            .iter()
            .map(|p| p.node.as_str())
            .collect();
        assert_eq!(ids, ["a/"], "a package whose group holds the scoped file");
    }

    fn test_case(id: &str, duration: Option<f64>, mapped: &[(&str, &str)]) -> TestCaseEvidence {
        TestCaseEvidence {
            id: id.into(),
            suite: "s".into(),
            name: id.into(),
            class_name: None,
            duration_seconds: duration,
            outcome: "passed".into(),
            message: None,
            mappings: mapped
                .iter()
                .map(|(node, status)| TestSourceMapping {
                    path: node.to_string(),
                    node: Some(node.to_string()),
                    lines: Vec::new(),
                    source_sha256: None,
                    status: status.to_string(),
                })
                .collect(),
        }
    }

    fn mutant(status: &str) -> MutationRecord {
        let at = archguard_engine::MutationPosition { line: 1, column: 1 };
        MutationRecord {
            id: status.into(),
            mutator_name: "Negate".into(),
            status: status.into(),
            location: archguard_engine::MutationLocation {
                start: at.clone(),
                end: at,
            },
            description: None,
            replacement: None,
            original: String::new(),
            original_truncated: false,
            duration_ms: None,
            covered_by: Vec::new(),
            killed_by: Vec::new(),
        }
    }

    #[test]
    fn mutations_list_survivors_first_then_by_incoming_files() {
        let file = |node: &str, statuses: &[&str]| MutationFile {
            path: node.into(),
            node: Some(node.into()),
            language: "java".into(),
            source_sha256: String::new(),
            status: "unchanged_since_import".into(),
            mutants: statuses.iter().map(|s| mutant(s)).collect(),
        };
        // a/x.rs has one incoming file (a/y.rs), b/z.rs none.
        let files = vec![
            file("b/z.rs", &["Killed", "Survived"]),
            file("a/x.rs", &["Survived", "NoCoverage"]),
        ];
        let out = mutations(&files, &source(), &Scope::default(), "");
        let rows: Vec<_> = out
            .iter()
            .map(|r| (r.file.path.as_str(), r.mutant.status.as_str(), r.incoming))
            .collect();
        assert_eq!(
            rows,
            [
                ("a/x.rs", "Survived", 1),
                ("b/z.rs", "Survived", 0),
                ("a/x.rs", "NoCoverage", 1),
                ("b/z.rs", "Killed", 0)
            ]
        );
        assert_eq!(
            mutations(&files, &source(), &scope(&source(), "b/"), "").len(),
            2
        );
    }

    #[test]
    fn history_keeps_the_lineage_and_removed_modules_below_a_package() {
        let mut report =
            archguard_engine::read_quality_history(std::path::Path::new("/nonexistent"));
        let delta = archguard_engine::QualityMetricDelta {
            status: "removed".into(),
            before: None,
            after: None,
            delta: None,
            reason: None,
        };
        let removed = |node: &str| archguard_engine::ModuleQualityDelta {
            extended: Default::default(),
            node: node.into(),
            status: "removed".into(),
            line_coverage: delta.clone(),
            mean_complexity: delta.clone(),
            max_complexity: delta.clone(),
            active_findings: delta.clone(),
        };
        report.comparison = Some(archguard_engine::QualityComparison {
            before_timestamp: 0,
            after_timestamp: 1,
            modules: vec![removed("a/old"), removed("b/gone")],
        });
        let nodes = |rows: Vec<HistoryRow>| rows.into_iter().map(|r| r.node).collect::<Vec<_>>();
        assert_eq!(
            nodes(history(&report, &source(), "", &Scope::default(), "")),
            ["a/old", "b/gone"]
        );
        assert_eq!(
            nodes(history(
                &report,
                &source(),
                "a/",
                &scope(&source(), "a/"),
                ""
            )),
            ["a/old"]
        );
        assert!(
            history(
                &report,
                &source(),
                "a/x.rs",
                &scope(&source(), "a/x.rs"),
                ""
            )
            .is_empty()
        );
    }

    #[test]
    fn tests_follow_dependents_of_the_scope() {
        let fresh = "unchanged_since_import";
        let rows = vec![
            test_case("t_x", Some(1.0), &[("a/x.rs", fresh)]),
            test_case("t_z", Some(5.0), &[("b/z.rs", fresh)]),
            test_case("t_none", None, &[]),
            test_case(
                "t_y_stale",
                Some(9.0),
                &[("a/y.rs", "changed_since_import")],
            ),
        ];
        // Selecting a/x.rs affects a/y.rs and b/z.rs (they depend on it).
        let out = tests(&rows, &source(), &scope(&source(), "a/x.rs"), "");
        let ids: Vec<_> = out.iter().map(|r| r.test.id.as_str()).collect();
        assert_eq!(ids, ["t_z", "t_x", "t_y_stale", "t_none"]);
        assert!(out[3].unknown && out[2].unknown && !out[0].unknown);
        // Selecting b/z.rs: t_x is mapped and fresh but unaffected.
        let out = tests(&rows, &source(), &scope(&source(), "b/z.rs"), "");
        assert!(out.iter().all(|r| r.test.id != "t_x"));
    }
}
