//! Task-oriented discovery and exact, paginated architecture maps.
use crate::engineering_tools::EngineeringParams;
use archguard_engine::{ArchitectureAtlas, core::DEVELOPMENT_PATHS};
use rmcp::{ErrorData, schemars};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DevelopmentGuideParams {
    #[serde(flatten)]
    pub page: EngineeringParams,
    /// graph, dsm, calls, services, architecture, assessment, quality or security.
    pub view: Option<String>,
    /// understand, improve or verify. Omit for all routes.
    pub mission: Option<String>,
}
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ArchitectureAtlasParams {
    #[serde(flatten)]
    pub page: EngineeringParams,
    /// Exact graph package/file name. Omit to skip single-package prefixes automatically.
    pub within: Option<String>,
    /// Revision from a previous atlas page. Rejects changed graph/scope; restart at offset 0.
    pub expected_revision: Option<String>,
}
fn routes() -> Vec<Value> {
    serde_json::from_str(DEVELOPMENT_PATHS).expect("development routes")
}
pub fn validate_guide(p: &DevelopmentGuideParams) -> Result<(), ErrorData> {
    crate::engineering_tools::validate(&p.page)?;
    if p.view
        .as_ref()
        .is_some_and(|view| !routes().iter().any(|r| r["view"] == *view))
    {
        return Err(crate::tools::invalid_params("Unknown development view"));
    }
    if p.mission
        .as_deref()
        .is_some_and(|m| !matches!(m, "understand" | "improve" | "verify"))
    {
        return Err(crate::tools::invalid_params(
            "mission must be understand, improve or verify",
        ));
    }
    Ok(())
}
pub fn validate_atlas(p: &ArchitectureAtlasParams) -> Result<(), ErrorData> {
    crate::engineering_tools::validate(&p.page)?;
    if p.within
        .as_ref()
        .is_some_and(|v| v.is_empty() || v.len() > 1024 || v.contains('\0'))
    {
        return Err(crate::tools::invalid_params(
            "within must be an exact name of 1..=1024 bytes without NUL",
        ));
    }
    if p.expected_revision
        .as_ref()
        .is_some_and(|v| v.len() != 64 || !v.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return Err(crate::tools::invalid_params(
            "expected_revision must be 64 hexadecimal characters",
        ));
    }
    Ok(())
}
pub fn guide_value(
    run: &crate::tools::Run,
    p: &DevelopmentGuideParams,
) -> Result<Value, ErrorData> {
    let q = p.page.query.as_deref().unwrap_or("").to_lowercase();
    let server_only = [
        "cycles_list",
        "edge_evidence",
        "dsm",
        "suggest_cuts",
        "simulate",
    ];
    let rows:Vec<_>=routes().into_iter().filter(|r|p.view.as_ref().is_none_or(|v|r["view"]==*v)&&p.mission.as_ref().is_none_or(|m|r["missions"].as_array().unwrap().iter().any(|v|v==m))).map(|r|{
        let mut tools=r["tools"].clone();
        for tool in tools.as_array_mut().unwrap(){
            let scoped=!server_only.contains(&tool["name"].as_str().unwrap());
            tool["project_scope"]=json!(if scoped{"selected_project"}else{"server_root_only"});
            if scoped { if tool["arguments"].is_null(){tool["arguments"]=json!({});}tool["arguments"]["path"]=json!(run.root); }
        }
        json!({"view":r["view"],"title":r["title"]["en"],"purpose":r["help"]["en"],"missions":r["missions"],"tools":tools})
    }).filter(|r|q.is_empty()||r.to_string().to_lowercase().contains(&q)).collect();
    let status = run
        .file_graph
        .semantic
        .as_ref()
        .map(|s| s.status.as_str())
        .unwrap_or("imports_only");
    crate::engineering_tools::page(
        status,
        json!({"project":run.root,"source_files":run.file_graph.nodes.iter().filter(|n|n.kind==archguard_engine::core::NodeKind::File&&!n.name.ends_with("/*")).count(),"workflow":[{"step":"understand constraints","tool":"architecture_context"},{"step":"inspect a proposed change","tool":"change_context","requires_one_of":["targets","symbols","base"]},{"step":"select relevant tests","tool":"test_impact","requires":["targets"]},{"step":"check after editing and running project tests","tool":"check"},{"step":"compare the result","tool":"compare","requires":["base"]}],"limitations":"Guidance and static observations, not a successful build or test run. Read source evidence, use project tests, then re-analyze. Server-root-only tools do not inherit path; rebind before using them on a nested project. Unknown or stale evidence is not a pass."}),
        rows.iter(),
        &p.page,
        Clone::clone,
    )
}
pub fn atlas_value(
    atlas: &ArchitectureAtlas,
    p: &ArchitectureAtlasParams,
) -> Result<Value, ErrorData> {
    if p.expected_revision
        .as_ref()
        .is_some_and(|v| v != &atlas.revision)
    {
        return Err(crate::tools::invalid_params(
            "Atlas graph/scope changed. Restart at offset 0 without expected_revision.",
        ));
    }
    let mut rows = Vec::new();
    for area in &atlas.areas {
        let mut row = json!(area);
        row["kind"] = json!("area");
        row["node_kind"] = json!(area.kind);
        rows.push(row);
    }
    for edge in &atlas.connections {
        let mut row = json!(edge);
        row["kind"] = json!("connection");
        rows.push(row);
    }
    let q = p.page.query.as_deref().unwrap_or("").to_lowercase();
    crate::engineering_tools::page(
        &atlas.status,
        json!({"definition":atlas.definition,"revision":atlas.revision,"scope":atlas.scope,"areas":atlas.areas.len(),"connections":atlas.connections.len(),"cycle_groups":atlas.cycle_groups,"external_connections":atlas.external_connections,"limitations":"Cycles belong to this package/file aggregation, not necessarily to individual files. incoming/outgoing count visible area neighbors; external counts count original source connections crossing the selected scope. Unknown evidence counts remain null. No runtime or change-risk prediction. Follow all pages using the same query, scope and revision."}),
        rows.iter()
            .filter(|r| q.is_empty() || r.to_string().to_lowercase().contains(&q)),
        &p.page,
        Clone::clone,
    )
}

pub fn world_value(
    world: &archguard_engine::ArchitectureWorld,
    p: &ArchitectureAtlasParams,
) -> Result<Value, ErrorData> {
    if p.expected_revision
        .as_ref()
        .is_some_and(|r| r != &world.revision)
    {
        return Err(crate::tools::invalid_params(
            "World evidence changed. Restart at offset 0 without expected_revision.",
        ));
    }
    let query = p.page.query.as_deref().unwrap_or("").to_lowercase();
    crate::engineering_tools::page(
        &world.status,
        json!({"definition":world.definition,"revision":world.revision,"scope":p.within,"limitations":"Observed maximum source-method complexity, not a complete quality score. File coverage is not class coverage; report/source revision at import is unverified. Missing, stale and ambiguous measurements remain null. Use architecture_atlas for aggregation cycles and architecture_findings for repair evidence."}),
        world.entities.iter().filter(|row| {
            p.within.as_ref().is_none_or(|scope| {
                row.node == *scope
                    || row
                        .node
                        .strip_prefix(scope)
                        .is_some_and(|tail| tail.starts_with('/'))
            }) && (query.is_empty()
                || row.id.to_lowercase().contains(&query)
                || row.node.to_lowercase().contains(&query))
        }),
        &p.page,
        |row| json!(row),
    )
}
