//! Spring Data Elasticsearch / OpenSearch annotations (ticket 23).
use crate::context::{SearchContext, SourceOutput};

pub(crate) fn extract_with(_ctx: &SearchContext) -> SourceOutput {
    SourceOutput::default()
}
