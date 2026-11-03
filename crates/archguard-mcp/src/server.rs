//! The MCP server handle every tool module implements methods on.

use std::path::PathBuf;

use rmcp::ErrorData;

/// Default page size for [`cycles_list`] (Kap. 10.1: „limit (Default 20)").
pub const DEFAULT_LIMIT: usize = 20;
/// The MCP tool handler bound to one project root.
#[derive(Debug, Clone)]
pub struct ArchguardTools {
    pub(crate) root: PathBuf,
}
impl ArchguardTools {
    /// Bind the tools to `root` (the project analysed when a tool does not
    /// override the path itself).
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

pub(crate) fn insight_error(error: impl std::fmt::Display) -> ErrorData {
    ErrorData::invalid_params(error.to_string(), None)
}
