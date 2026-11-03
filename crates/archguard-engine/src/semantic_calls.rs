//! Shared call model and coverage across independent language backends.
use archguard_core::{CallAnalysis, CallAnalyzer, Config};
use archguard_extract::Lang;
use std::collections::BTreeMap;
use std::path::Path;

/// Enrich the import graph before any SCC, gate or metric is calculated.
/// Only compiler-resolved source relationships are projected; unsupported
/// languages and missing references remain explicit coverage limitations.
pub(crate) fn integrate_graph(
    root: &Path,
    files: &[archguard_extract::SourceFile],
    config: &Config,
    graph: &mut archguard_core::Graph,
) -> Result<(), archguard_core::ArchguardError> {
    let calls = analyze_calls(root, config);
    let projection =
        crate::semantic_graph::apply_semantic_dependencies(files, config, graph, &calls)?;
    let mut diagnostics = calls.diagnostics;
    if projection.skipped_project_calls > 0
        || projection.skipped_supertypes > 0
        || projection.skipped_type_uses > 0
    {
        diagnostics.push(format!(
            "{} project calls and {} supertype relationships could not be mapped to unique source files; no dependency was invented.",
            projection.skipped_project_calls, projection.skipped_supertypes
        ));
    }
    if projection.skipped_type_uses > 0 {
        diagnostics.push(format!(
            "{} resolved type uses have ambiguous source-file mappings and were not projected.",
            projection.skipped_type_uses
        ));
    }
    let status = if calls.status == "ready"
        && (projection.skipped_project_calls > 0
            || projection.skipped_supertypes > 0
            || projection.skipped_type_uses > 0)
    {
        "partial".to_string()
    } else {
        calls.status
    };
    graph.semantic = Some(archguard_core::SemanticDependencyCoverage {
        status,
        analyzers: calls.analyzers,
        projected_calls: projection.projected_calls,
        projected_supertypes: projection.projected_supertypes,
        projected_type_uses: projection.projected_type_uses,
        skipped_type_uses: projection.skipped_type_uses,
        skipped_project_calls: projection.skipped_project_calls,
        diagnostics,
    });
    Ok(())
}

/// Detect source languages once and combine independently reported providers.
/// Unimplemented providers remain explicit coverage gaps, not clean call graphs.
pub fn analyze_calls(root: &Path, config: &Config) -> CallAnalysis {
    let files = match crate::pipeline::walk_files(root, config) {
        Ok(files) => files,
        Err(error) => {
            return CallAnalysis {
                status: "unavailable".into(),
                java_release: 0,
                diagnostics: vec![error.to_string()],
                ..Default::default()
            };
        }
    };
    let mut counts = BTreeMap::new();
    for file in files {
        *counts.entry(file.lang).or_insert(0usize) += 1;
    }
    let mut reports = Vec::new();
    let mut typescript_analyzed = false;
    let script_counts = [
        ("typescript", *counts.get(&Lang::Ts).unwrap_or(&0)),
        ("javascript", *counts.get(&Lang::Js).unwrap_or(&0)),
    ];
    for (lang, count) in counts {
        let language = match lang {
            Lang::Java => "java",
            Lang::CSharp => "csharp",
            Lang::Ts => "typescript",
            Lang::Js => "javascript",
            Lang::Py => "python",
            Lang::Rust => "rust",
            Lang::Php => "php",
            Lang::Go => "go",
            Lang::Kotlin => "kotlin",
        };
        let mut report = match lang {
            Lang::Java => crate::java_calls::analyze_java_calls(root, config),
            Lang::CSharp => crate::csharp_calls::analyze_csharp_calls(root, config),
            Lang::Go => crate::go_calls::analyze_go_calls(root, config),
            Lang::Rust => crate::rust_calls::analyze_rust_calls(root, config),
            Lang::Py => crate::python_calls::analyze_python_calls(root, config),
            Lang::Ts | Lang::Js => {
                if typescript_analyzed {
                    continue;
                }
                typescript_analyzed = true;
                crate::typescript_calls::analyze_typescript_calls(root, config)
            }
            _ => CallAnalysis {
                status: "unavailable".into(),
                files: count,
                java_release: 0,
                diagnostics: vec![format!(
                    "Semantic analysis for {language} is not implemented; import analysis remains available."
                )],
                ..Default::default()
            },
        };
        if report.files == 0 && report.status == "unavailable" {
            report.files = if matches!(lang, Lang::Ts | Lang::Js) {
                script_counts.iter().map(|(_, count)| count).sum()
            } else {
                count
            };
            if report.analyzers.len() == 1 {
                report.analyzers[0].files = count;
            }
        }
        if matches!(lang, Lang::Ts | Lang::Js) && report.status == "unavailable" {
            report.analyzers = script_counts
                .iter()
                .filter(|(_, count)| *count > 0)
                .map(|(language, count)| archguard_core::CallAnalyzer {
                    language: (*language).into(),
                    backend: "typescript".into(),
                    status: "unavailable".into(),
                    files: *count,
                    version: None,
                    project: config.semantic.typescript.project.clone(),
                    diagnostics: report.diagnostics.clone(),
                })
                .collect();
        }
        reports.push((
            if matches!(lang, Lang::Ts | Lang::Js) {
                "typescript"
            } else {
                language
            },
            report,
        ));
    }
    combine(reports)
}

/// Live reports also invalidate on project settings and reference changes,
/// even when every source file and import edge remains identical.
pub fn semantic_fingerprints(root: &Path, config: &Config) -> Vec<Result<[u8; 32], String>> {
    vec![
        crate::java_calls::java_calls_fingerprint(root, config),
        crate::csharp_calls::csharp_calls_fingerprint(root, config),
        crate::typescript_calls::typescript_calls_fingerprint(root, config),
        crate::go_calls::go_calls_fingerprint(root, config),
    ]
}

fn combine(reports: Vec<(&str, CallAnalysis)>) -> CallAnalysis {
    let mut result = CallAnalysis {
        java_release: 0,
        ..Default::default()
    };
    let mut complete = true;
    let mut usable = false;
    for (language, mut report) in reports {
        if report.status == "not_applicable" {
            continue;
        }
        complete &= report.status == "ready";
        usable |= matches!(report.status.as_str(), "ready" | "partial");
        result.files += report.files;
        if language == "java" {
            result.java_release = report.java_release;
        }
        if report.analyzers.is_empty() {
            report.analyzers.push(CallAnalyzer {
                language: language.into(),
                backend: if language == "java" {
                    "javac"
                } else if language == "csharp" {
                    "roslyn"
                } else {
                    "none"
                }
                .into(),
                status: report.status.clone(),
                files: report.files,
                version: None,
                project: None,
                diagnostics: report.diagnostics.clone(),
            });
        }
        // Keep legacy Java identities. All additional providers have a stable
        // language prefix, independent of which other providers are available.
        let identity = |value: &str| {
            if language == "java" {
                value.to_owned()
            } else {
                format!("{language}:{value}")
            }
        };
        for class in &mut report.classes {
            class.language.get_or_insert_with(|| language.into());
            class.id = identity(&class.id);
            class.supertypes = class.supertypes.iter().map(|id| identity(id)).collect();
            if let Some(sites) = &mut class.supertype_sites {
                for site in sites {
                    site.target = identity(&site.target);
                }
            }
        }
        for method in &mut report.methods {
            method.id = identity(&method.id);
            method.class_id = identity(&method.class_id);
        }
        for call in &mut report.calls {
            call.from_class = identity(&call.from_class);
            call.from_method = call.from_method.as_deref().map(identity);
            call.to_class = call.to_class.as_deref().map(identity);
            call.to_method = call.to_method.as_deref().map(identity);
        }
        if let Some(mut details) = report.details {
            for used in &mut details.type_uses {
                used.from_class = identity(&used.from_class);
                used.from_method = used.from_method.as_deref().map(identity);
                used.to_class = used.to_class.as_deref().map(identity);
            }
            for state in &mut details.class_states {
                state.class_id = identity(&state.class_id);
                for field in &mut state.fields {
                    field.id = identity(&field.id);
                }
                for access in &mut state.accesses {
                    access.method = identity(&access.method);
                    access.field = identity(&access.field);
                }
            }
            let combined = result.details.get_or_insert_with(Default::default);
            combined.enumeration_complete = Some(
                combined.enumeration_complete.unwrap_or(true)
                    && details.enumeration_complete == Some(true),
            );
            for metric in &mut details.method_metrics {
                metric.method_id = identity(&metric.method_id);
            }
            for parent in &mut details.inheritance {
                parent.class_id = identity(&parent.class_id);
                parent.base_class = parent.base_class.as_deref().map(identity);
            }
            combined.method_metrics.extend(details.method_metrics);
            combined.inheritance.extend(details.inheritance);
            combined.type_uses.extend(details.type_uses);
            combined.class_states.extend(details.class_states);
        }
        result.analyzers.extend(report.analyzers);
        result.classes.extend(report.classes);
        result.methods.extend(report.methods);
        result.calls.extend(report.calls);
        result.diagnostics.extend(
            report
                .diagnostics
                .into_iter()
                .map(|message| format!("{language}: {message}")),
        );
    }
    result.status = if result.analyzers.is_empty() {
        "not_applicable"
    } else if complete {
        "ready"
    } else if usable {
        "partial"
    } else {
        "unavailable"
    }
    .into();
    let cohesion = crate::class_cohesion(&result);
    if let Some(details) = &mut result.details {
        details.cohesion = cohesion;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report(language: &str, status: &str) -> CallAnalysis {
        serde_json::from_value(serde_json::json!({
            "status":status,"java_release":if language=="java" {21} else {0},"files":1,
            "analyzers":[{"language":language,"backend":"compiler","status":status,"files":1,"version":null,"project":null,"diagnostics":[]}],
            "classes":[{"id":"p.A","name":"p.A","package":"p","file":"A","graph_node":"p/A","line":1,"kind":"class","supertypes":["p.Base"],"supertype_sites":[{"target":"p.Base","file":"A","line":1}]}],
            "methods":[{"id":"p.A#run()","class_id":"p.A","name":"run","signature":"run()","file":"A","line":2,"is_static":false}],
            "calls":[{"from_class":"p.A","from_method":"p.A#run()","to_class":"p.A","to_method":"p.A#run()","target_label":"run()","dispatch":"instance","resolution":"project","file":"A","line":2,"column":10,"code":"a.run()"}],"diagnostics":[]
        })).unwrap()
    }
    #[test]
    fn mixed_languages_keep_identity_edges_evidence_and_independent_coverage() {
        let result = combine(vec![
            ("java", report("java", "ready")),
            ("csharp", report("csharp", "partial")),
        ]);
        assert_eq!(result.status, "partial");
        assert_eq!(result.files, 2);
        assert_eq!(result.analyzers.len(), 2);
        assert_eq!(result.classes[0].id, "p.A");
        assert_eq!(result.classes[1].id, "csharp:p.A");
        assert_eq!(result.classes[1].name, "p.A");
        assert_eq!(result.classes[1].supertypes, ["csharp:p.Base"]);
        assert_eq!(
            result.classes[1].supertype_sites.as_ref().unwrap()[0].target,
            "csharp:p.Base"
        );
        assert_eq!(result.methods[1].class_id, result.classes[1].id);
        assert_eq!(
            result.calls[1].to_method.as_ref().unwrap(),
            &result.methods[1].id
        );
        assert_eq!(result.calls[1].code, "a.run()");
        assert_eq!(result.classes[1].language.as_deref(), Some("csharp"));
        assert_eq!(
            combine(vec![("csharp", report("csharp", "partial"))]).classes[0].id,
            result.classes[1].id,
            "IDs stay stable when another language is added"
        );
    }
    #[test]
    fn missing_backend_never_reports_complete_coverage() {
        let missing = CallAnalysis {
            status: "unavailable".into(),
            files: 3,
            java_release: 0,
            diagnostics: vec!["No semantic backend".into()],
            ..Default::default()
        };
        assert_eq!(
            combine(vec![("python", missing.clone())]).status,
            "unavailable"
        );
        assert_eq!(combine(vec![]).status, "not_applicable");
        let result = combine(vec![("java", report("java", "ready")), ("python", missing)]);
        assert_eq!(result.status, "partial");
        assert_eq!(result.files, 4);
        assert_eq!(result.analyzers[1].language, "python");
        assert_eq!(result.analyzers[1].status, "unavailable");
        assert_eq!(result.analyzers[1].files, 3);
    }
}
