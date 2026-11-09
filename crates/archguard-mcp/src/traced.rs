//! Each tool call is one request of ArchGuard's self-instrumentation
//! (ADR 0004): a root span named after the tool, with the engine's phase
//! spans below it. Without `ARCHGUARD_PROBE=1` the span is inert.

use archguard_engine::self_trace::{entry_span, tracing::Instrument};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ListToolsResult, PaginatedRequestParams, ServerConfig,
    Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};

use crate::server::ArchguardTools;

/// The served handler: [`ArchguardTools`] with a request span per tool call.
pub(crate) struct Traced(pub(crate) ArchguardTools);

impl ServerHandler for Traced {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let span = entry_span(&request.name);
        self.0.call_tool(request, context).instrument(span).await
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.0.list_tools(request, context).await
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.0.get_tool(name)
    }

    fn get_info(&self) -> ServerConfig {
        self.0.get_info()
    }
}
