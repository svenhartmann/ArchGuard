//! `access_layers`: the access chain per endpoint (LOGIC.md section 7, item 8),
//! read-only and paginated within 4 KiB.
use crate::engineering_tools::EngineeringParams;
use crate::server::ArchguardTools;
use crate::ui_flow_tools::{check_revision, revision};
use archguard_engine::core::{ChainFrom, UiFlow, access_chain};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AccessLayersParams {
    #[serde(flatten)]
    pub page: EngineeringParams,
    /// `endpoint` (default): one row per endpoint with its screens; `screen`: one row per screen/endpoint pair.
    pub from: Option<String>,
    /// `service-guarded`: only endpoints protected below the endpoint, not by it.
    pub only: Option<String>,
    /// Revision from a previous page. Rejects a changed flow; restart at offset 0.
    pub expected_revision: Option<String>,
}

pub fn validate(p: &AccessLayersParams) -> Result<(), ErrorData> {
    crate::engineering_tools::validate(&p.page)?;
    if p.from
        .as_deref()
        .is_some_and(|f| !["endpoint", "screen"].contains(&f))
        || p.only.as_deref().is_some_and(|o| o != "service-guarded")
        || p.expected_revision
            .as_deref()
            .is_some_and(|v| v.is_empty() || v.len() > 256 || v.contains('\0'))
    {
        return Err(crate::tools::invalid_params(
            "from must be endpoint or screen; only must be service-guarded; expected_revision 1..=256 bytes",
        ));
    }
    Ok(())
}

pub fn access_layers_value(
    flow: &UiFlow,
    show_unguarded: bool,
    p: &AccessLayersParams,
) -> Result<Value, ErrorData> {
    validate(p)?;
    let from = match p.from.as_deref() {
        Some("screen") => ChainFrom::Screen,
        _ => ChainFrom::Endpoint,
    };
    let rev = revision(&(flow, &p.from, &p.only, show_unguarded));
    check_revision(&rev, p.expected_revision.as_deref())?;
    let rows = access_chain(flow, from, p.only.is_some(), show_unguarded);
    let q = p.page.query.as_deref().unwrap_or("").trim().to_lowercase();
    let items: Vec<Value> = (rows.iter())
        .map(|r| json!({"kind":"access_chain","chain":r}))
        .filter(|r| q.is_empty() || r.to_string().to_lowercase().contains(&q))
        .collect();
    crate::engineering_tools::page(
        if flow.complete {
            "complete"
        } else {
            "incomplete"
        },
        json!({"definition":flow.definition,"revision":rev,
            "limitations":"Static evidence. effective is the endpoint's access ANDed with every enforced service layer; path_only, hint and data_filter layers are listed and never change it. enforced_at none means no check was read, not that the endpoint is open. Screens come from the frontend-to-backend join; an endpoint without a screen may still be called."}),
        items.iter(),
        &p.page,
        Clone::clone,
    )
}

#[tool_router(router = access_layers_router, vis = "pub(crate)")]
impl ArchguardTools {
    #[tool(
        description = "Access chain per backend endpoint: the screens that call it, its own check, the checks below it (service, data) with how each holds (enforced, path_only, hint, data_filter) and the effective access. enforced_at says where the protection is (endpoint, service, both, none); service_guarded marks endpoints protected only below the endpoint. from: endpoint|screen; only: service-guarded. Static evidence; none/undeclared never means open. Read-only; flattened 4 KiB pages, follow next_offset with metadata.revision as expected_revision."
    )]
    pub(crate) async fn access_layers(
        &self,
        Parameters(params): Parameters<AccessLayersParams>,
    ) -> Result<String, ErrorData> {
        validate(&params)?;
        let root = crate::tools::resolve_root(&self.root, params.page.path.as_deref())?;
        let loaded = crate::tools::load_config_at(&root)?;
        let server_root = crate::tools::resolve_root(&self.root, None)?;
        let ctx = archguard_engine::extract_ctx::ExtractContext::new(&root, &loaded.config)
            .confined_to(&server_root);
        let flow =
            archguard_engine::ui_flow::analyze_with(&ctx).map_err(crate::server::insight_error)?;
        access_layers_value(
            &flow,
            loaded.config.views.access_layers.show_unguarded,
            &params,
        )
        .map(|v| v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{
        AccessState, Condition, EndpointAccess, ServiceHow, ServiceKind, ServiceLayer,
    };

    fn flow() -> UiFlow {
        let ep = |path: &str| EndpointAccess {
            method: "GET".into(),
            path: path.into(),
            access: AccessState::Open,
            application: None,
            file: "C.java".into(),
            line: 1,
            source: "test".into(),
            handler: None,
            sends: Default::default(),
        };
        let a = ep("/a");
        let layer = ServiceLayer {
            endpoint_file: a.file.clone(),
            endpoint_line: 1,
            endpoint_method: "GET".into(),
            endpoint_path: "/a".into(),
            kind: ServiceKind::Service,
            target: "S.m".into(),
            file: "S.java".into(),
            line: 2,
            state: AccessState::Protected(Condition::Role("B".into())),
            how: ServiceHow::Enforced,
            note: String::new(),
        };
        let mut b = ep("/b");
        b.line = 2;
        UiFlow {
            endpoints: vec![a, b],
            service_layers: vec![layer],
            ..UiFlow::default()
        }
    }

    #[test]
    fn pages_rows_and_filters_service_guarded() {
        let params = |only: Option<&str>, limit| AccessLayersParams {
            page: EngineeringParams {
                limit: Some(limit),
                ..EngineeringParams::default()
            },
            only: only.map(String::from),
            ..AccessLayersParams::default()
        };
        let first = access_layers_value(&flow(), true, &params(None, 1)).unwrap();
        assert_eq!(first["total"], 2);
        assert_eq!(first["next_offset"], 1);
        assert_eq!(first["items"][0]["chain"]["enforced_at"], "service");
        let only =
            access_layers_value(&flow(), true, &params(Some("service-guarded"), 10)).unwrap();
        assert_eq!(only["total"], 1);
        assert_eq!(only["items"][0]["chain"]["path"], "/a");
        let bad = AccessLayersParams {
            from: Some("x".into()),
            ..AccessLayersParams::default()
        };
        assert!(validate(&bad).is_err());
    }
}
