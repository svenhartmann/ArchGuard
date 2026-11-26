//! Analysis building blocks of an index or template, read from their compact JSON definitions
//! (ADR 0023, ticket es-classdiagram/01). View-neutral: the desktop, CLI and MCP share it.
//!
//! A chain member is `declared` (same kind defined in the same detail), `builtin` (table below),
//! `undeclared` (neither: a plugin such as ICU, or an Elasticsearch default; never a warning) or
//! `inline` (an object instead of a name).
//!
//! Built-in names, Elasticsearch reference 8.x:
//! - analyzers: <https://www.elastic.co/guide/en/elasticsearch/reference/8.19/analysis-analyzers.html>
//!   and <https://www.elastic.co/guide/en/elasticsearch/reference/8.19/analysis-lang-analyzer.html>
//! - tokenizers: <https://www.elastic.co/guide/en/elasticsearch/reference/8.19/analysis-tokenizers.html>
//! - char filters: <https://www.elastic.co/guide/en/elasticsearch/reference/8.19/analysis-charfilters.html>
//! - token filters: the files under `docs/reference/analysis/tokenfilters/` of
//!   <https://github.com/elastic/elasticsearch/tree/8.19> (the guide page lists none in text);
//!   plugin filters (`phonetic`, `icu_*`) are left out on purpose.
//! - normalizer: only `lowercase` is built in.
use archguard_core::{SearchAnalysis, SearchIndexDetail};
use serde_json::Value;

const ANALYZERS: &[&str] = &[
    "standard",
    "simple",
    "whitespace",
    "stop",
    "keyword",
    "pattern",
    "fingerprint",
    "arabic",
    "armenian",
    "basque",
    "bengali",
    "brazilian",
    "bulgarian",
    "catalan",
    "cjk",
    "czech",
    "danish",
    "dutch",
    "english",
    "estonian",
    "finnish",
    "french",
    "galician",
    "german",
    "greek",
    "hindi",
    "hungarian",
    "indonesian",
    "irish",
    "italian",
    "latvian",
    "lithuanian",
    "norwegian",
    "persian",
    "portuguese",
    "romanian",
    "russian",
    "serbian",
    "sorani",
    "spanish",
    "swedish",
    "turkish",
    "thai",
];
const TOKENIZERS: &[&str] = &[
    "standard",
    "letter",
    "lowercase",
    "whitespace",
    "uax_url_email",
    "classic",
    "thai",
    "ngram",
    "edge_ngram",
    "keyword",
    "pattern",
    "simple_pattern",
    "char_group",
    "simple_pattern_split",
    "path_hierarchy",
];
const CHAR_FILTERS: &[&str] = &["html_strip", "mapping", "pattern_replace"];
const FILTERS: &[&str] = &[
    "apostrophe",
    "asciifolding",
    "cjk_bigram",
    "cjk_width",
    "classic",
    "common_grams",
    "condition",
    "decimal_digit",
    "delimited_payload",
    "dictionary_decompounder",
    "edge_ngram",
    "elision",
    "fingerprint",
    "flatten_graph",
    "hunspell",
    "hyphenation_decompounder",
    "keep_types",
    "keep",
    "keyword_marker",
    "keyword_repeat",
    "kstem",
    "length",
    "limit",
    "lowercase",
    "min_hash",
    "multiplexer",
    "ngram",
    "arabic_normalization",
    "german_normalization",
    "hindi_normalization",
    "indic_normalization",
    "sorani_normalization",
    "persian_normalization",
    "scandinavian_normalization",
    "scandinavian_folding",
    "serbian_normalization",
    "pattern_capture",
    "pattern_replace",
    "porter_stem",
    "predicate_token_filter",
    "remove_duplicates",
    "reverse",
    "shingle",
    "snowball",
    "stemmer_override",
    "stemmer",
    "stop",
    "synonym_graph",
    "synonym",
    "trim",
    "truncate",
    "unique",
    "uppercase",
    "word_delimiter_graph",
    "word_delimiter",
];

/// Whether `name` is built in for `stage` (`analyzer`, `tokenizer`, `filter`, `char_filter`,
/// `normalizer`).
pub fn search_builtin(stage: &str, name: &str) -> bool {
    let table = match stage {
        "analyzer" => ANALYZERS,
        "tokenizer" => TOKENIZERS,
        "filter" => FILTERS,
        "char_filter" => CHAR_FILTERS,
        "normalizer" => &["lowercase"],
        _ => &[],
    };
    table.contains(&name)
}

/// A member of a chain. `class` is `declared`, `builtin`, `undeclared` or `inline`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainRef {
    pub name: String,
    pub class: &'static str,
}

/// One declared analyzer, tokenizer, filter, char filter or normalizer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchAnalysisPart {
    pub kind: String,
    pub name: String,
    /// `type`; an analyzer with a `tokenizer` and no `type` is `custom`.
    pub kind_type: Option<String>,
    pub char_filters: Vec<ChainRef>,
    pub tokenizer: Option<ChainRef>,
    pub filters: Vec<ChainRef>,
    /// Every key except `type`, `tokenizer`, `filter`, `char_filter`; inline members add
    /// `<type>.<key>`.
    pub params: Vec<(String, String)>,
    pub definition: String,
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(text).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

fn items(v: Option<&Value>) -> Vec<&Value> {
    match v {
        Some(Value::Array(a)) => a.iter().collect(),
        Some(other) => vec![other],
        None => vec![],
    }
}

fn member(
    v: &Value,
    stage: &str,
    detail: &SearchIndexDetail,
    params: &mut Vec<(String, String)>,
) -> ChainRef {
    if let Value::Object(o) = v {
        let name = o.get("type").map(text).unwrap_or_default();
        for (k, x) in o.iter().filter(|(k, _)| *k != "type") {
            params.push((format!("{name}.{k}"), text(x)));
        }
        return ChainRef {
            name,
            class: "inline",
        };
    }
    let name = text(v);
    let declared = |a: &SearchAnalysis| a.kind == stage && a.name == name;
    let class = if detail.analysis.iter().any(declared) {
        "declared"
    } else if search_builtin(stage, &name) {
        "builtin"
    } else {
        "undeclared"
    };
    ChainRef { name, class }
}

fn part(a: &SearchAnalysis, detail: &SearchIndexDetail) -> SearchAnalysisPart {
    let mut p = SearchAnalysisPart {
        kind: a.kind.clone(),
        name: a.name.clone(),
        kind_type: None,
        char_filters: vec![],
        tokenizer: None,
        filters: vec![],
        params: vec![],
        definition: a.definition.clone(),
    };
    let Ok(Value::Object(o)) = serde_json::from_str::<Value>(&a.definition) else {
        return p;
    };
    let mut inline = vec![];
    let mut chain = |key: &str, stage: &str| -> Vec<ChainRef> {
        items(o.get(key))
            .into_iter()
            .map(|v| member(v, stage, detail, &mut inline))
            .collect()
    };
    p.char_filters = chain("char_filter", "char_filter");
    p.tokenizer = chain("tokenizer", "tokenizer").into_iter().next();
    p.filters = chain("filter", "filter");
    p.kind_type = match o.get("type") {
        Some(t) => Some(text(t)),
        None => (a.kind == "analyzer" && p.tokenizer.is_some()).then(|| "custom".into()),
    };
    const CHAIN: [&str; 4] = ["type", "tokenizer", "filter", "char_filter"];
    p.params = o
        .iter()
        .filter(|(k, _)| !CHAIN.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), text(v)))
        .chain(inline)
        .collect();
    p
}

/// The parts of an index or template detail, in declaration order.
pub fn search_analysis_parts(detail: &SearchIndexDetail) -> Vec<SearchAnalysisPart> {
    detail.analysis.iter().map(|a| part(a, detail)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(defs: &[(&str, &str, &str)]) -> SearchIndexDetail {
        SearchIndexDetail {
            analysis: defs
                .iter()
                .map(|(k, n, d)| SearchAnalysis {
                    kind: (*k).into(),
                    name: (*n).into(),
                    definition: (*d).into(),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn builtin_has_doppelganger() {
        assert!(!search_builtin("filter", "standard"));
        assert!(search_builtin("analyzer", "standard"));
        assert!(!search_builtin("analyzer", "standard_x"));
        assert!(!search_builtin("filter", "icu_folding"));
        assert!(search_builtin("normalizer", "lowercase"));
    }

    #[test]
    fn common_analysis_plugin_names_are_builtin() {
        for n in [
            "arabic_stem",
            "bengali_normalization",
            "german_stem",
            "type_as_payload",
        ] {
            assert!(search_builtin("filter", n), "{n}");
        }
        for n in ["chinese", "snowball"] {
            assert!(search_builtin("analyzer", n), "{n}");
        }
    }

    #[test]
    fn plugin_name_is_undeclared_and_same_name_other_kind_is_not_declared() {
        let d = detail(&[(
            "analyzer",
            "a",
            r#"{"tokenizer":"standard","filter":["icu_folding","a"]}"#,
        )]);
        let p = &search_analysis_parts(&d)[0];
        let classes: Vec<_> = p.filters.iter().map(|c| c.class).collect();
        assert_eq!(classes, ["undeclared", "undeclared"]);
        assert_eq!(p.kind_type.as_deref(), Some("custom"));
    }

    #[test]
    fn invalid_or_empty_definition_gives_an_empty_part() {
        for def in ["\"x\"", "", "[1]", "null", "{}"] {
            let p = &search_analysis_parts(&detail(&[("filter", "f", def)]))[0];
            assert!(p.filters.is_empty() && p.char_filters.is_empty() && p.params.is_empty());
            assert!(p.tokenizer.is_none() && p.kind_type.is_none(), "{def}");
        }
    }

    #[test]
    fn params_join_lists_and_keep_objects_compact() {
        let d = detail(&[(
            "filter",
            "f",
            r#"{"type":"x","a":["p","q"],"b":{"c":1},"d":true}"#,
        )]);
        let p = &search_analysis_parts(&d)[0];
        assert_eq!(
            p.params,
            [
                ("a".into(), "p, q".into()),
                ("b".into(), r#"{"c":1}"#.into()),
                ("d".into(), "true".into())
            ]
        );
    }
}
