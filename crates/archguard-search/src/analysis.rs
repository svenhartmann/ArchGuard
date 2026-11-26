//! Stub (red step).
use archguard_core::SearchIndexDetail;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainRef {
    pub name: String,
    pub class: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchAnalysisPart {
    pub kind: String,
    pub name: String,
    pub kind_type: Option<String>,
    pub char_filters: Vec<ChainRef>,
    pub tokenizer: Option<ChainRef>,
    pub filters: Vec<ChainRef>,
    pub params: Vec<(String, String)>,
    pub definition: String,
}

pub fn search_builtin(_stage: &str, _name: &str) -> bool {
    false
}

pub fn search_analysis_parts(_detail: &SearchIndexDetail) -> Vec<SearchAnalysisPart> {
    Vec::new()
}
