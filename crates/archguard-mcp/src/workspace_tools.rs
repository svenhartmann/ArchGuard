//! Repository-wide evidence with explicit opt-in for compiler/source analysis.
use crate::engineering_tools::EngineeringParams;
use archguard_engine::{WorkspaceSourceReport, WorkspaceTestImpact, core::WorkspaceReport};
use rmcp::{ErrorData, schemars};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WorkspaceContextParams {
    #[serde(flatten)]
    pub page: EngineeringParams,
    /// Opt into bounded per-project source analysis and supported compiler calls.
    pub sources: bool,
    /// Exact project/file identities or repository-relative source paths. Requires sources=true.
    pub targets: Option<Vec<String>>,
}
pub fn validate(p: &WorkspaceContextParams) -> Result<(), ErrorData> {
    crate::engineering_tools::validate(&p.page)?;
    if p.targets.as_ref().is_some_and(|v| {
        !v.is_empty()
            && (!p.sources
                || v.len() > 256
                || v.iter()
                    .any(|t| t.is_empty() || t.len() > 4096 || t.contains('\0')))
    }) {
        return Err(crate::tools::invalid_params(
            "targets require sources=true and at most 256 nonempty identities of at most 4096 bytes without NUL",
        ));
    }
    Ok(())
}
pub fn workspace_value(
    workspace: &WorkspaceReport,
    sources: Option<&WorkspaceSourceReport>,
    impact: Option<&WorkspaceTestImpact>,
    p: &WorkspaceContextParams,
) -> Result<Value, ErrorData> {
    validate(p)?;
    let mut rows = Vec::new();
    for project in &workspace.projects {
        rows.push(json!({"kind":"project","data":project}));
    }
    for edge in &workspace.dependencies {
        rows.push(json!({"kind":"project_dependency","data":edge}));
    }
    for cycle in &workspace.cycles {
        rows.push(json!({"kind":"project_cycle","members":cycle}));
    }
    for item in &workspace.unresolved {
        rows.push(json!({"kind":"unresolved_dependency","data":item}));
    }
    for message in &workspace.diagnostics {
        rows.push(json!({"kind":"diagnostic","scope":"manifests","message":message}));
    }
    for message in &workspace.limitations {
        rows.push(json!({"kind":"limitation","scope":"manifests","message":message}));
    }
    if let Some(source) = sources {
        for project in &source.projects {
            rows.push(json!({"kind":"source_project","data":project}));
        }
        for file in &source.files {
            rows.push(json!({"kind":"source_file","data":file}));
        }
        let names: std::collections::BTreeMap<_, _> = source
            .graph
            .nodes
            .iter()
            .map(|n| (n.id, n.name.as_str()))
            .collect();
        for edge in &source.graph.edges {
            let from = names.get(&edge.from);
            let to = names.get(&edge.to);
            rows.push(json!({"kind":"source_dependency","from":from,"to":to,"references":edge.count,"evidence_samples":edge.refs.len()}));
            for evidence in &edge.refs {
                rows.push(json!({"kind":"source_evidence","from":from,"to":to,"data":evidence}));
            }
        }
        for cycle in &source.cycles {
            rows.push(json!({"kind":"source_cycle","members":cycle}));
        }
        for class in &source.calls.classes {
            rows.push(json!({"kind":"class","data":class}));
        }
        for method in &source.calls.methods {
            rows.push(json!({"kind":"method","data":method}));
        }
        for call in &source.calls.calls {
            rows.push(json!({"kind":"call","data":call}));
        }
        for analyzer in &source.calls.analyzers {
            rows.push(json!({"kind":"analyzer","data":analyzer}));
        }
        for message in &source.calls.diagnostics {
            rows.push(json!({"kind":"diagnostic","scope":"calls","message":message}));
        }
        for test in &source.tests {
            rows.push(json!({"kind":"test","id":test.id,"project":test.project,"suite":test.suite,"name":test.name,"outcome":test.outcome,"duration_seconds":test.duration_seconds,"mapping_count":test.mappings.len()}));
            for mapping in &test.mappings {
                rows.push(json!({"kind":"test_mapping","test_id":test.id,"data":mapping}));
            }
        }
        for message in &source.diagnostics {
            rows.push(json!({"kind":"diagnostic","scope":"sources","message":message}));
        }
        for message in &source.limitations {
            rows.push(json!({"kind":"limitation","scope":"sources","message":message}));
        }
    }
    if let Some(impact) = impact {
        for target in &impact.targets {
            rows.push(json!({"kind":"impact_target","target":target}));
        }
        for target in &impact.unresolved_targets {
            rows.push(json!({"kind":"unresolved_target","target":target}));
        }
        for file in &impact.affected_files {
            rows.push(json!({"kind":"affected_file","id":file}));
        }
        for project in &impact.affected_projects {
            rows.push(json!({"kind":"affected_project","id":project}));
        }
        for test in &impact.selected {
            rows.push(json!({"kind":"recommended_test","data":test}));
        }
        for message in &impact.diagnostics {
            rows.push(json!({"kind":"diagnostic","scope":"impact","message":message}));
        }
    }
    let query = p.page.query.as_deref().unwrap_or("").trim().to_lowercase();
    if !query.is_empty() {
        rows.retain(|r| r.to_string().to_lowercase().contains(&query));
    }
    let status = if sources.is_some_and(|s| s.status != "ready")
        || impact.is_some_and(|i| i.status != "ready")
    {
        "partial"
    } else {
        &workspace.status
    };
    let metadata = json!({"root":workspace.root,"manifest_status":workspace.status,"projects":workspace.projects.len(),"dependencies":workspace.dependencies.len(),"project_cycles":workspace.cycles.len(),"unresolved_dependencies":workspace.unresolved.len(),"sources_requested":p.sources,"source_status":sources.map(|s|&s.status),"files":sources.map(|s|s.files.len()),"source_dependencies":sources.map(|s|s.graph.edges.len()),"source_cycles":sources.map(|s|s.cycles.len()),"call_status":sources.map(|s|&s.calls.status),"calls":sources.map(|s|s.calls.calls.len()),"tests":sources.map(|s|s.tests.len()),"impact_status":impact.map(|i|&i.status),"affected_projects":impact.map(|i|i.affected_projects.len()),"selected_tests":impact.map(|i|i.selected.len()),"unknown_tests":impact.map(|i|i.unknown_tests),"safe_to_skip_other_tests":false});
    crate::engineering_tools::page(status, metadata, rows.iter(), &p.page, Clone::clone)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_records_are_bounded_and_paged_without_losing_unknowns() {
        let report:WorkspaceReport=serde_json::from_value(json!({"schema_version":1,"root":".","status":"partial","projects":(0..80).map(|i|json!({"id":format!("project:{i}/pom.xml"),"name":format!("p{i}"),"path":i.to_string(),"manifest":format!("{i}/pom.xml"),"ecosystem":"maven","kind":"library","runtime":null,"evidence":[]})).collect::<Vec<_>>(),"dependencies":[],"cycles":[],"unresolved":[{"project":"project:0/pom.xml","target":"unresolved","reason":"not a guessed edge","evidence":[]}],"diagnostics":["x".repeat(6000)],"limitations":["No runtime evidence"]})).unwrap();
        let mut offset = 0;
        let mut count = 0;
        let mut omitted = false;
        loop {
            let p = WorkspaceContextParams {
                page: EngineeringParams {
                    offset: Some(offset),
                    limit: Some(200),
                    ..Default::default()
                },
                ..Default::default()
            };
            let page = workspace_value(&report, None, None, &p).unwrap();
            assert!(page.to_string().len() <= 4096);
            assert_eq!(page["status"], "partial");
            assert_eq!(page["metadata"]["projects"], 80);
            let items = page["items"].as_array().unwrap();
            count += items.len();
            omitted |= items.iter().any(|r| r["omitted"] == true);
            if let Some(next) = page["next_offset"].as_u64() {
                assert!(next as usize > offset);
                offset = next as usize;
            } else {
                break;
            }
        }
        assert_eq!(count, 83);
        assert!(
            omitted,
            "oversized diagnostics must have advancing omission markers"
        );
        let p = WorkspaceContextParams {
            page: EngineeringParams {
                query: Some("unresolved".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let filtered = workspace_value(&report, None, None, &p).unwrap();
        assert_eq!(filtered["total"], 1);
        assert_eq!(filtered["metadata"]["projects"], 80);
    }
    #[test]
    fn targets_cannot_silently_trigger_compilers() {
        assert!(
            validate(&WorkspaceContextParams {
                targets: Some(vec!["app/file.java".into()]),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            validate(&WorkspaceContextParams {
                sources: true,
                targets: Some(vec!["../not-an-access-path".into()]),
                ..Default::default()
            })
            .is_ok()
        );
        assert!(
            validate(&WorkspaceContextParams {
                sources: true,
                targets: Some(vec![String::new()]),
                ..Default::default()
            })
            .is_err()
        );
    }
}
