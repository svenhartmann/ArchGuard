//! Project compiler-resolved dependencies onto the existing file graph.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use archguard_core::{
    ArchguardError, CallAnalysis, CallClass, Config, Edge, Graph, NodeId, NodeKind, Ref,
};
use archguard_extract::{SourceFile, extractor_for, strip_root};

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ProjectionSummary {
    pub projected_calls: usize,
    pub projected_supertypes: usize,
    pub projected_type_uses: usize,
    pub skipped_type_uses: usize,
    pub skipped_project_calls: usize,
    pub skipped_supertypes: usize,
    pub added_edges: usize,
}

pub(crate) fn apply_semantic_dependencies(
    files: &[SourceFile],
    config: &Config,
    graph: &mut Graph,
    calls: &CallAnalysis,
) -> Result<ProjectionSummary, ArchguardError> {
    let mut summary = ProjectionSummary::default();
    if calls.classes.is_empty() && calls.calls.is_empty() {
        return Ok(summary);
    }
    let sources = source_nodes(files, config, graph)?;
    let names: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (node.id, node.name.as_str()))
        .collect();
    // A duplicated compiler identity is ambiguous even when its first record
    // happens to match a graph node. Never choose an arbitrary declaration.
    let mut classes = BTreeMap::new();
    for class in &calls.classes {
        classes
            .entry(class.id.as_str())
            .and_modify(|entry| *entry = None)
            .or_insert(Some(class));
    }
    let mut methods = BTreeMap::new();
    for method in &calls.methods {
        methods
            .entry(method.id.as_str())
            .and_modify(|entry| *entry = None)
            .or_insert(Some(method));
    }
    let class_node = |class: &CallClass| {
        sources
            .get(class.file.as_str())
            .copied()
            .filter(|id| names.get(id).copied() == Some(class.graph_node.as_str()))
    };
    let unique_class_target = |class: &CallClass| match class.declaration_files.as_deref() {
        Some([file]) if file == &class.file => class_node(class),
        Some(_) => None,
        None if class.language.as_deref() == Some("csharp") => None,
        None => class_node(class),
    };
    // Keep source locations canonical and deduplicated independently of compiler
    // traversal order. The column disambiguates repeated expressions on one line.
    let mut evidence = BTreeMap::<_, BTreeSet<_>>::new();
    for call in calls
        .calls
        .iter()
        .filter(|call| call.resolution == "project")
    {
        let endpoints = (|| {
            let owner = classes.get(call.from_class.as_str()).copied().flatten()?;
            class_node(owner)?;
            let target = classes.get(call.to_class.as_deref()?).copied().flatten()?;
            class_node(target)?;
            let from = sources.get(call.file.as_str()).copied()?;
            // C# partial types can declare the called method in a different file
            // from the first class declaration. Implicit constructors have no
            // method record and fall back to the verified class declaration.
            let to = match call.to_method.as_deref().and_then(|id| methods.get(id)) {
                Some(Some(method)) if method.class_id == target.id => {
                    sources.get(method.file.as_str()).copied()?
                }
                Some(_) => return None,
                None => unique_class_target(target)?,
            };
            Some((from, to))
        })();
        let Some((from, to)) = endpoints else {
            summary.skipped_project_calls += 1;
            continue;
        };
        if from != to
            && evidence.entry((from, to)).or_default().insert((
                call.file.clone(),
                call.line,
                format!("[semantic call] column {}: {}", call.column, call.code),
            ))
        {
            summary.projected_calls += 1;
        }
    }
    for class in &calls.classes {
        for parent in &class.supertypes {
            // Unknown identities are external supertypes, not local edges.
            let Some(parent_entry) = classes.get(parent.as_str()) else {
                continue;
            };
            let to = parent_entry.and_then(unique_class_target);
            let unique_source = classes
                .get(class.id.as_str())
                .copied()
                .flatten()
                .and_then(class_node)
                .is_some();
            let sites: Vec<_> = if let Some(sites) = &class.supertype_sites {
                sites
                    .iter()
                    .filter(|site| &site.target == parent)
                    .map(|site| (site.file.as_str(), site.line))
                    .collect()
            } else if class.language.as_deref() != Some("csharp") {
                // Java has one source declaration per type. A legacy C# type
                // can be partial and provides no evidence for its base clause.
                vec![(class.file.as_str(), class.line)]
            } else {
                vec![]
            };
            let Some(to) = to.filter(|_| unique_source && !sites.is_empty()) else {
                summary.skipped_supertypes += 1;
                continue;
            };
            for (file, line) in sites {
                let Some(&from) = sources.get(file) else {
                    summary.skipped_supertypes += 1;
                    continue;
                };
                if from != to
                    && evidence.entry((from, to)).or_default().insert((
                        file.to_string(),
                        line,
                        format!("[semantic supertype] {} -> {}", class.name, parent),
                    ))
                {
                    summary.projected_supertypes += 1;
                }
            }
        }
    }
    if let Some(details) = &calls.details {
        for used in details
            .type_uses
            .iter()
            .filter(|u| u.resolution == "project")
        {
            let endpoints = (|| {
                let owner = classes.get(used.from_class.as_str()).copied().flatten()?;
                class_node(owner)?;
                let target = classes.get(used.to_class.as_deref()?).copied().flatten()?;
                Some((
                    sources.get(used.file.as_str()).copied()?,
                    unique_class_target(target)?,
                ))
            })();
            let Some((from, to)) = endpoints else {
                summary.skipped_type_uses += 1;
                continue;
            };
            if from != to
                && evidence.entry((from, to)).or_default().insert((
                    used.file.clone(),
                    used.line,
                    format!("[semantic type:{}] {}", used.role, used.code),
                ))
            {
                summary.projected_type_uses += 1;
            }
        }
    }
    let mut indices: BTreeMap<_, _> = graph
        .edges
        .iter()
        .enumerate()
        .map(|(i, edge)| ((edge.from, edge.to), i))
        .collect();
    for ((from, to), references) in evidence {
        let index = *indices.entry((from, to)).or_insert_with(|| {
            summary.added_edges += 1;
            graph.edges.push(Edge {
                from,
                to,
                refs: vec![],
                count: Some(0),
            });
            graph.edges.len() - 1
        });
        let edge = &mut graph.edges[index];
        let mut existing: BTreeSet<_> = edge
            .refs
            .iter()
            .map(|r| (r.file.clone(), r.line, r.raw.clone()))
            .collect();
        let before = existing.len();
        existing.extend(references);
        // Preserve counts on complete graphs and the unknown count of a legacy
        // sampled graph, rather than inventing a total from its evidence sample.
        if let Some(count) = &mut edge.count {
            *count += (existing.len() - before) as u64;
        }
        edge.refs = existing
            .into_iter()
            .map(|(file, line, raw)| Ref { file, line, raw })
            .collect();
    }
    graph.edges.sort_by_key(|edge| (edge.from, edge.to));
    Ok(summary)
}

/// Use the extraction pipeline's naming rule, retaining only one-to-one source
/// mappings. This covers partial classes and files with several namespaces;
/// guessing the namespace from a call's class would identify the wrong file.
fn source_nodes(
    files: &[SourceFile],
    config: &Config,
    graph: &Graph,
) -> Result<BTreeMap<String, NodeId>, ArchguardError> {
    let nodes: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::File && !node.name.ends_with("/*"))
        .map(|node| (node.name.as_str(), node.id))
        .collect();
    let mut by_source = BTreeMap::<String, Vec<NodeId>>::new();
    let mut by_node = BTreeMap::<NodeId, usize>::new();
    for file in files {
        let Some(extractor) = extractor_for(file.lang) else {
            continue;
        };
        let content = fs::read_to_string(&file.path)?;
        let source = strip_root(&file.rel_path, &config.project.source_roots);
        let extraction = extractor.extract(&source, &content);
        let name = crate::pipeline::file_node_name(&source, &extraction.declarations);
        let Some(&node) = nodes.get(name.as_str()) else {
            continue;
        };
        by_source.entry(source).or_default().push(node);
        *by_node.entry(node).or_default() += 1;
    }
    Ok(by_source
        .into_iter()
        .filter_map(|(source, nodes)| {
            (nodes.len() == 1 && by_node[&nodes[0]] == 1).then(|| (source, nodes[0]))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_analysis::find_sccs;
    use archguard_core::{CallClass, CallMethod, Cut, MethodCall, Ref, aggregate};
    use std::fs;

    fn fixture(sources: &[(&str, &str)]) -> (tempfile::TempDir, Vec<SourceFile>, Config, Graph) {
        let root = tempfile::tempdir().unwrap();
        for (path, source) in sources {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
        let config = Config::default();
        let files = crate::pipeline::walk_files(root.path(), &config).unwrap();
        let graph = crate::pipeline::build_graph(root.path(), &files, &config)
            .unwrap()
            .0;
        (root, files, config, graph)
    }
    fn class(id: &str, file: &str, graph_node: &str) -> CallClass {
        CallClass {
            language: Some("csharp".into()),
            id: id.into(),
            name: id.into(),
            package: graph_node.rsplit_once('/').map_or("", |(p, _)| p).into(),
            file: file.into(),
            graph_node: graph_node.into(),
            line: 2,
            kind: "class".into(),
            is_abstract: Some(false),
            visibility: Some("public".into()),
            supertypes: vec![],
            supertype_sites: None,
            declaration_files: Some(vec![file.into()]),
        }
    }
    fn call(from: &str, to: &str, file: &str, code: &str) -> MethodCall {
        MethodCall {
            from_class: from.into(),
            from_method: None,
            to_class: Some(to.into()),
            to_method: None,
            target_label: format!("{to}.Run()"),
            dispatch: "static".into(),
            resolution: "project".into(),
            file: file.into(),
            line: 4,
            column: 9,
            code: code.into(),
        }
    }
    fn analysis(classes: Vec<CallClass>, calls: Vec<MethodCall>) -> CallAnalysis {
        CallAnalysis {
            status: "partial".into(),
            files: classes.len(),
            classes,
            calls,
            ..Default::default()
        }
    }
    fn pair(graph: &Graph, from: &str, to: &str) -> bool {
        graph.edges.iter().any(|edge| {
            graph.node_name(edge.from) == Some(from) && graph.node_name(edge.to) == Some(to)
        })
    }

    #[test]
    fn resolved_field_and_signature_types_close_cycles_without_calls() {
        let (_root, files, config, mut graph) = fixture(&[
            ("a/A.cs", "namespace a; class A { b.B field; }"),
            (
                "b/B.cs",
                "namespace b; class B { a.A Return(a.A arg)=>arg; }",
            ),
        ]);
        let mut calls = analysis(
            vec![
                class("a.A", "a/A.cs", "a/A.cs"),
                class("b.B", "b/B.cs", "b/B.cs"),
            ],
            vec![],
        );
        calls.details=Some(serde_json::from_value(serde_json::json!({"definition":"source-types-state/v1","class_states":[],"type_uses":[
            {"from_class":"a.A","from_method":null,"to_class":"b.B","target_label":"b.B","role":"field","resolution":"project","file":"a/A.cs","line":1,"code":"b.B"},
            {"from_class":"b.B","from_method":null,"to_class":"a.A","target_label":"a.A","role":"return","resolution":"project","file":"b/B.cs","line":1,"code":"a.A"}
        ]})).unwrap());
        apply_semantic_dependencies(&files, &config, &mut graph, &calls).unwrap();
        assert!(pair(&graph, "a/A.cs", "b/B.cs"));
        assert!(pair(&graph, "b/B.cs", "a/A.cs"));
        assert!(
            graph
                .edges
                .iter()
                .all(|e| e.refs.iter().any(|r| r.raw.starts_with("[semantic type:")))
        );
        calls.details.as_mut().unwrap().type_uses[0].resolution = "unresolved".into();
        graph.edges.clear();
        apply_semantic_dependencies(&files, &config, &mut graph, &calls).unwrap();
        assert!(!pair(&graph, "a/A.cs", "b/B.cs"));
    }

    #[test]
    fn resolved_calls_close_file_and_package_cycles_without_imports() {
        let (_root, files, config, mut graph) = fixture(&[
            (
                "A.cs",
                "namespace Left;\nclass A { void Run() { Right.B.Run(); } }",
            ),
            (
                "B.cs",
                "namespace Right;\nclass B { void Run() { Left.A.Run(); } }",
            ),
        ]);
        assert!(graph.edges.is_empty());
        assert!(find_sccs(&graph).is_empty());
        let calls = analysis(
            vec![
                class("a", "A.cs", "Left/A.cs"),
                class("b", "B.cs", "Right/B.cs"),
            ],
            vec![
                call("a", "b", "A.cs", "Right.B.Run()"),
                call("b", "a", "B.cs", "Left.A.Run()"),
            ],
        );
        let result = apply_semantic_dependencies(&files, &config, &mut graph, &calls).unwrap();
        assert_eq!(
            find_sccs(&graph).len(),
            1,
            "compiler-resolved calls must participate in file SCCs"
        );
        assert_eq!(
            find_sccs(&aggregate(&graph, &Cut::at_depth(&graph, 1))).len(),
            1
        );
        assert_eq!(result.projected_calls, 2);
        assert_eq!(result.added_edges, 2);
        assert_eq!(graph.edges[0].refs[0].file, "A.cs");
        assert_eq!(graph.edges[0].refs[0].line, 4);
        assert!(graph.edges[0].refs[0].raw.contains("[semantic call]"));
        assert!(graph.edges[0].refs[0].raw.contains("Right.B.Run()"));
    }

    #[test]
    fn projection_preserves_import_evidence_and_is_idempotent() {
        let (_root, files, config, mut graph) = fixture(&[
            ("A.cs", "namespace P;\nclass A {}"),
            ("B.cs", "namespace P;\nclass B {}"),
        ]);
        let from = graph.nodes.iter().find(|n| n.name == "P/A.cs").unwrap().id;
        let to = graph.nodes.iter().find(|n| n.name == "P/B.cs").unwrap().id;
        let import = Ref {
            file: "A.cs".into(),
            line: 1,
            raw: "using P.B;".into(),
        };
        graph.edges.push(archguard_core::Edge {
            from,
            to,
            refs: vec![import.clone()],
            count: Some(1),
        });
        let calls = analysis(
            vec![class("a", "A.cs", "P/A.cs"), class("b", "B.cs", "P/B.cs")],
            vec![
                call("a", "b", "A.cs", "B.Run()"),
                call("a", "b", "A.cs", "B.Run()"),
            ],
        );
        apply_semantic_dependencies(&files, &config, &mut graph, &calls).unwrap();
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].weight(), 2);
        assert!(graph.edges[0].refs.contains(&import));
        let before = graph.clone();
        apply_semantic_dependencies(&files, &config, &mut graph, &calls).unwrap();
        assert_eq!(graph, before);
    }

    #[test]
    fn partial_class_calls_use_actual_caller_and_target_method_files() {
        let (_root, files, config, mut graph) = fixture(&[
            ("A.cs", "namespace P;\npartial class A {}"),
            (
                "A.Other.cs",
                "namespace P;\npartial class A { void Other() {} }",
            ),
            ("B.cs", "namespace P;\nclass B {}"),
        ]);
        let mut calls = analysis(
            vec![class("a", "A.cs", "P/A.cs"), class("b", "B.cs", "P/B.cs")],
            vec![
                call("a", "b", "A.Other.cs", "B.Run()"),
                call("b", "a", "B.cs", "a.Other()"),
            ],
        );
        calls.calls[1].to_method = Some("a#Other()".into());
        calls.classes[0].declaration_files = Some(vec!["A.cs".into(), "A.Other.cs".into()]);
        calls.methods.push(CallMethod {
            id: "a#Other()".into(),
            class_id: "a".into(),
            name: "Other".into(),
            signature: "Other()".into(),
            file: "A.Other.cs".into(),
            line: 2,
            end_line: None,
            is_static: false,
            complexity: None,
        });
        apply_semantic_dependencies(&files, &config, &mut graph, &calls).unwrap();
        assert!(pair(&graph, "P/A.Other.cs", "P/B.cs"));
        assert!(pair(&graph, "P/B.cs", "P/A.Other.cs"));
        assert!(!pair(&graph, "P/B.cs", "P/A.cs"));
        assert_eq!(find_sccs(&graph).len(), 1);
    }

    #[test]
    fn external_unresolved_and_ambiguous_targets_never_create_edges() {
        let (_root, files, config, mut graph) = fixture(&[
            ("A.cs", "namespace P;\nclass A {}"),
            ("B.cs", "namespace P;\nclass B {}"),
        ]);
        let mut calls = analysis(
            vec![class("a", "A.cs", "P/A.cs"), class("b", "B.cs", "P/B.cs")],
            vec![
                call("a", "b", "A.cs", "external()"),
                call("a", "b", "A.cs", "unresolved()"),
                call("a", "missing", "A.cs", "missing()"),
                call("a", "a", "A.cs", "self()"),
            ],
        );
        calls.calls[0].resolution = "external".into();
        calls.calls[1].resolution = "unresolved".into();
        let result = apply_semantic_dependencies(&files, &config, &mut graph, &calls).unwrap();
        assert!(graph.edges.is_empty());
        assert_eq!(result.skipped_project_calls, 1);
        calls.classes.push(class("b", "A.cs", "P/A.cs"));
        calls.calls = vec![call("a", "b", "A.cs", "ambiguous()")];
        let result = apply_semantic_dependencies(&files, &config, &mut graph, &calls).unwrap();
        assert!(graph.edges.is_empty());
        assert_eq!(result.skipped_project_calls, 1);
    }

    #[test]
    fn duplicate_logical_files_are_not_guessed() {
        let (_root, files, config, mut graph) = fixture(&[
            ("first/A.cs", "namespace P;\nclass A {}"),
            ("second/A.cs", "namespace P;\nclass A {}"),
            ("B.cs", "namespace P;\nclass B {}"),
        ]);
        let calls = analysis(
            vec![
                class("a", "first/A.cs", "P/A.cs"),
                class("b", "B.cs", "P/B.cs"),
            ],
            vec![call("a", "b", "first/A.cs", "B.Run()")],
        );
        let result = apply_semantic_dependencies(&files, &config, &mut graph, &calls).unwrap();
        assert!(graph.edges.is_empty());
        assert_eq!(result.skipped_project_calls, 1);
    }

    #[test]
    fn compiler_supertypes_participate_in_cycles_with_declaration_evidence() {
        let (_root, files, config, mut graph) = fixture(&[
            ("Base.cs", "namespace P;\nclass Base {}"),
            ("Child.cs", "namespace P;\nclass Child : Base {}"),
        ]);
        let mut child = class("child", "Child.cs", "P/Child.cs");
        child.supertypes = vec!["base".into(), "external:System.Object".into()];
        child.supertype_sites = Some(vec![archguard_core::CallSupertypeSite {
            target: "base".into(),
            file: "Child.cs".into(),
            line: 2,
        }]);
        let calls = analysis(
            vec![class("base", "Base.cs", "P/Base.cs"), child],
            vec![call("base", "child", "Base.cs", "new Child()")],
        );
        let result = apply_semantic_dependencies(&files, &config, &mut graph, &calls).unwrap();
        assert_eq!(find_sccs(&graph).len(), 1);
        assert_eq!(result.projected_supertypes, 1);
        let edge = graph
            .edges
            .iter()
            .find(|e| graph.node_name(e.from) == Some("P/Child.cs"))
            .unwrap();
        assert_eq!(edge.refs[0].file, "Child.cs");
        assert_eq!(edge.refs[0].line, 2);
        assert!(edge.refs[0].raw.starts_with("[semantic supertype]"));
    }

    #[test]
    fn partial_supertypes_use_the_base_clause_declaration_file() {
        let (_root, files, config, mut graph) = fixture(&[
            ("Base.cs", "namespace P;\nclass Base {}"),
            ("Child.cs", "namespace P;\npartial class Child {}"),
            (
                "Child.Inheritance.cs",
                "namespace P;\npartial class Child : Base {}",
            ),
        ]);
        let mut child = class("child", "Child.cs", "P/Child.cs");
        child.supertypes = vec!["base".into()];
        child.supertype_sites = Some(vec![archguard_core::CallSupertypeSite {
            target: "base".into(),
            file: "Child.Inheritance.cs".into(),
            line: 2,
        }]);
        let calls = analysis(vec![class("base", "Base.cs", "P/Base.cs"), child], vec![]);
        apply_semantic_dependencies(&files, &config, &mut graph, &calls).unwrap();
        assert!(pair(&graph, "P/Child.Inheritance.cs", "P/Base.cs"));
        assert!(
            !pair(&graph, "P/Child.cs", "P/Base.cs"),
            "first partial declaration does not contain the base clause"
        );
        assert_eq!(graph.edges[0].refs[0].file, "Child.Inheritance.cs");
    }

    #[test]
    fn implicit_constructor_of_a_partial_target_does_not_choose_its_first_file() {
        let (_root, files, config, mut graph) = fixture(&[
            ("A.cs", "namespace P;\npartial class A {}"),
            ("A.Other.cs", "namespace P;\npartial class A {}"),
            ("B.cs", "namespace P;\nclass B {}"),
        ]);
        let mut partial = class("a", "A.cs", "P/A.cs");
        partial.declaration_files = Some(vec!["A.cs".into(), "A.Other.cs".into()]);
        let mut constructor = call("b", "a", "B.cs", "new A()");
        constructor.dispatch = "constructor".into();
        constructor.to_method = Some("a#.ctor()".into());
        let calls = analysis(
            vec![partial, class("b", "B.cs", "P/B.cs")],
            vec![constructor],
        );
        let result = apply_semantic_dependencies(&files, &config, &mut graph, &calls).unwrap();
        assert!(
            graph.edges.is_empty(),
            "implicit partial constructors have no unique declaring file"
        );
        assert_eq!(result.skipped_project_calls, 1);
    }

    #[test]
    fn legacy_csharp_supertypes_need_sites_but_java_declarations_are_unique() {
        let (_root, files, config, graph) = fixture(&[
            ("Base.cs", "namespace P;\nclass Base {}"),
            ("Child.cs", "namespace P;\nclass Child : Base {}"),
        ]);
        let mut child = class("child", "Child.cs", "P/Child.cs");
        child.supertypes = vec!["base".into()];
        let mut calls = analysis(vec![class("base", "Base.cs", "P/Base.cs"), child], vec![]);
        let mut csharp = graph.clone();
        let result = apply_semantic_dependencies(&files, &config, &mut csharp, &calls).unwrap();
        assert!(csharp.edges.is_empty());
        assert_eq!(result.skipped_supertypes, 1);
        calls.classes[1].language = Some("java".into());
        let mut java = graph;
        let result = apply_semantic_dependencies(&files, &config, &mut java, &calls).unwrap();
        assert!(pair(&java, "P/Child.cs", "P/Base.cs"));
        assert_eq!(result.projected_supertypes, 1);
    }

    #[test]
    fn a_partial_supertype_has_no_unique_file_target() {
        let (_root, files, config, mut graph) = fixture(&[
            ("Base.cs", "namespace P;\npartial class Base {}"),
            ("Base.Other.cs", "namespace P;\npartial class Base {}"),
            ("Child.cs", "namespace P;\nclass Child : Base {}"),
        ]);
        let mut base = class("base", "Base.cs", "P/Base.cs");
        base.declaration_files = Some(vec!["Base.cs".into(), "Base.Other.cs".into()]);
        let mut child = class("child", "Child.cs", "P/Child.cs");
        child.supertypes = vec!["base".into()];
        child.supertype_sites = Some(vec![archguard_core::CallSupertypeSite {
            target: "base".into(),
            file: "Child.cs".into(),
            line: 2,
        }]);
        let result = apply_semantic_dependencies(
            &files,
            &config,
            &mut graph,
            &analysis(vec![base, child], vec![]),
        )
        .unwrap();
        assert!(graph.edges.is_empty());
        assert_eq!(result.skipped_supertypes, 1);
    }
}
