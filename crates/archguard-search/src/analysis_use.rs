//! Who uses an analysis part, a field's role and index/search differences (ADR 0023,
//! es-classdiagram ticket 02). Built on `search_analysis_parts`; no second chain evaluation.
use archguard_core::{SearchField, SearchIndexDetail};

/// What a field is within its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFieldRole {
    Field,
    MultiField,
    Property,
}

/// One user of an analysis part; `role` is `index`, `search`, `normalizer`, `default`,
/// `default_search` (subject: field path) or `chain` (subject: `<kind>:<name>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchAnalysisUser {
    pub subject: String,
    pub role: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchAnalysisUse {
    pub kind: String,
    pub name: String,
    pub users: Vec<SearchAnalysisUser>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchAnalyzerDifference {
    pub index: String,
    pub search: String,
    pub tokenizer_differs: bool,
    pub only_index: Vec<String>,
    pub only_search: Vec<String>,
}

pub fn search_field_role(_fields: &[SearchField], _i: usize) -> SearchFieldRole {
    SearchFieldRole::Field
}

pub fn search_analysis_users(_detail: &SearchIndexDetail) -> Vec<SearchAnalysisUse> {
    Vec::new()
}

pub fn search_analyzer_difference(
    _detail: &SearchIndexDetail,
    _path: &str,
) -> Option<SearchAnalyzerDifference> {
    None
}
