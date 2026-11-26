//! The source table: one row per input format. A new format is one module and one row here.
use crate::context::SearchContext;
pub(crate) use crate::context::SourceOutput;

pub(crate) mod code;
pub(crate) mod constants;
pub(crate) mod json;
pub(crate) mod json_files;
pub(crate) mod search_templates;
pub(crate) mod spring;
pub(crate) mod spring_syntax;
pub(crate) mod terraform;
pub(crate) mod terraform_detail;
pub(crate) mod terraform_eval;
pub(crate) mod terraform_links;
pub(crate) mod terraform_locals;
pub(crate) mod terraform_other;
pub(crate) mod terraform_scope;

/// Order matters only for equal sort keys of the merged artifacts: terraform before templates.
pub(crate) const SOURCES: &[fn(&SearchContext) -> SourceOutput] = &[
    terraform::extract_with,
    terraform_other::extract_with,
    search_templates::extract_with,
    json_files::extract_with,
    code::extract_with,
    spring::extract_with,
];
