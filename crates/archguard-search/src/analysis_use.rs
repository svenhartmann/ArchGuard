//! Who uses an analysis part, a field's role and index/search differences (ADR 0023,
//! es-classdiagram ticket 02). Built on `search_analysis_parts`; no second chain evaluation.
use crate::analysis::{ChainRef, SearchAnalysisPart, search_analysis_parts};
use archguard_core::{SearchField, SearchIndexDetail};
use std::collections::BTreeSet;

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

fn parent(fields: &[SearchField], i: usize) -> Option<usize> {
    let path = &fields[i].path;
    (0..i)
        .filter(|&j| {
            path.strip_prefix(&fields[j].path)
                .is_some_and(|r| r.starts_with('.'))
        })
        .max_by_key(|&j| fields[j].path.len())
}

/// Child of an `object`/`nested` field is a property, child of any other field a multi-field.
/// A parent is the longest path prefix among the fields before `i`.
pub fn search_field_role(fields: &[SearchField], i: usize) -> SearchFieldRole {
    match parent(fields, i).map(|j| fields[j].kind.as_str()) {
        None => SearchFieldRole::Field,
        Some("object" | "nested") => SearchFieldRole::Property,
        Some(_) => SearchFieldRole::MultiField,
    }
}

/// Number of ancestors of field `i` in `fields`.
pub fn search_field_depth(fields: &[SearchField], i: usize) -> usize {
    std::iter::successors(parent(fields, i), |&j| parent(fields, j)).count()
}

fn field_users(detail: &SearchIndexDetail, kind: &str, name: &str) -> Vec<SearchAnalysisUser> {
    let user = |f: &SearchField, role| SearchAnalysisUser {
        subject: f.path.clone(),
        role,
    };
    let mut out = vec![];
    for f in &detail.fields {
        let text = f.kind == "text";
        let roles: [(bool, &'static str); 5] = [
            (
                kind == "analyzer" && f.analyzer.as_deref() == Some(name),
                "index",
            ),
            (
                kind == "analyzer" && f.search_analyzer.as_deref() == Some(name),
                "search",
            ),
            (
                kind == "normalizer" && f.normalizer.as_deref() == Some(name),
                "normalizer",
            ),
            // The declared `default` is the index analyzer of a text field without one.
            (
                kind == "analyzer" && name == "default" && text && f.analyzer.is_none(),
                "default",
            ),
            // A field with its own `analyzer` searches with it (ES), so only both-less fields.
            (
                kind == "analyzer"
                    && name == "default_search"
                    && text
                    && f.analyzer.is_none()
                    && f.search_analyzer.is_none(),
                "default_search",
            ),
        ];
        out.extend(
            roles
                .into_iter()
                .filter(|(on, _)| *on)
                .map(|(_, r)| user(f, r)),
        );
    }
    out
}

fn chain_users(parts: &[SearchAnalysisPart], kind: &str, name: &str) -> Vec<SearchAnalysisUser> {
    let names = |c: &[ChainRef]| c.iter().any(|m| m.class == "declared" && m.name == name);
    parts
        .iter()
        .filter(|p| match kind {
            "char_filter" => names(&p.char_filters),
            "tokenizer" => p.tokenizer.iter().any(|t| names(std::slice::from_ref(t))),
            "filter" => names(&p.filters),
            _ => false,
        })
        .map(|p| SearchAnalysisUser {
            subject: format!("{}:{}", p.kind, p.name),
            role: "chain",
        })
        .collect()
}

/// Users of every declared part: fields (`index`, `search`, `normalizer`, `default`,
/// `default_search`) and parts naming it in their chain (`chain`). `default` and `default_search`
/// count only when declared (they are parts then).
pub fn search_analysis_users(detail: &SearchIndexDetail) -> Vec<SearchAnalysisUse> {
    let parts = search_analysis_parts(detail);
    parts
        .iter()
        .map(|p| SearchAnalysisUse {
            kind: p.kind.clone(),
            name: p.name.clone(),
            users: field_users(detail, &p.kind, &p.name)
                .into_iter()
                .chain(chain_users(&parts, &p.kind, &p.name))
                .collect(),
        })
        .collect()
}

fn stages(detail: &SearchIndexDetail, name: &str) -> (Option<String>, BTreeSet<String>) {
    let parts = search_analysis_parts(detail);
    let Some(p) = parts
        .iter()
        .find(|p| p.kind == "analyzer" && p.name == name)
    else {
        return (None, BTreeSet::from([name.to_string()])); // built in: one stage, its name
    };
    let set: BTreeSet<String> = p
        .char_filters
        .iter()
        .chain(&p.filters)
        .map(|c| c.name.clone())
        .collect();
    let tokenizer = p.tokenizer.as_ref().map(|t| t.name.clone());
    if tokenizer.is_none() && set.is_empty() {
        return (
            None,
            BTreeSet::from([p.kind_type.clone().unwrap_or_else(|| name.into())]),
        );
    }
    (tokenizer, set)
}

/// The difference of the index and the search chain of field `path`; `None` without an own search
/// analyzer or with equal stage names (names of the analyzers do not count).
pub fn search_analyzer_difference(
    detail: &SearchIndexDetail,
    path: &str,
) -> Option<SearchAnalyzerDifference> {
    let f = detail.fields.iter().find(|f| f.path == path)?;
    let search = f.search_analyzer.clone()?;
    let index = f.analyzer.clone().unwrap_or_else(|| "default".into());
    let ((it, is), (st, ss)) = (stages(detail, &index), stages(detail, &search));
    let list = |a: &BTreeSet<String>, b: &BTreeSet<String>| a.difference(b).cloned().collect();
    let d = SearchAnalyzerDifference {
        index,
        search,
        tokenizer_differs: it != st,
        only_index: list(&is, &ss),
        only_search: list(&ss, &is),
    };
    (d.tokenizer_differs || !d.only_index.is_empty() || !d.only_search.is_empty()).then_some(d)
}
