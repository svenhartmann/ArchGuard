//! One Python file as the DSL reader sees it: constants, imports and the class/def structure,
//! all from the shared scanner (`archguard_flow_common::lang_python`). A name counts as DSL only
//! when an import binds it below one of `ROOTS`; a class or call of the same name without such an
//! import does not match.
use archguard_flow_common::lang_pyscan::{dotted, skip_ws};
use archguard_flow_common::lang_python::{
    Def, DefKind, Stmt, assignment, code_statements, defs, imports, mask,
};
use std::collections::HashMap;

/// Import roots and the flavor of what they define. elasticsearch-dsl-py was archived on
/// 2026-05-05 and moved into `elasticsearch.dsl`, so both spellings match.
const ROOTS: &[(&str, &str)] = &[
    ("elasticsearch_dsl", ""),
    ("elasticsearch.dsl", ""),
    ("django_elasticsearch_dsl", ""),
    ("opensearch_dsl", "opensearch"),
    ("opensearchpy", "opensearch"),
];

/// Field classes of `elasticsearch.dsl.field` (elasticsearch-py main); the ES type is the
/// snake-case class name (`DateNanos` is `date_nanos`).
const FIELDS: &[&str] = &[
    "AggregateMetricDouble",
    "Alias",
    "Binary",
    "Boolean",
    "Byte",
    "Completion",
    "ConstantKeyword",
    "CountedKeyword",
    "Date",
    "DateNanos",
    "DateRange",
    "DenseVector",
    "Double",
    "DoubleRange",
    "ExponentialHistogram",
    "Flattened",
    "Float",
    "FloatRange",
    "GeoPoint",
    "GeoShape",
    "HalfFloat",
    "Histogram",
    "IcuCollationKeyword",
    "Integer",
    "IntegerRange",
    "Ip",
    "IpRange",
    "Join",
    "Keyword",
    "Long",
    "LongRange",
    "MatchOnlyText",
    "Murmur3",
    "Nested",
    "Object",
    "Passthrough",
    "Percolator",
    "Point",
    "RankFeature",
    "RankFeatures",
    "RankVectors",
    "ScaledFloat",
    "SearchAsYouType",
    "SemanticText",
    "Shape",
    "Short",
    "SparseVector",
    "Text",
    "TokenCount",
    "UnsignedLong",
    "Version",
    "Wildcard",
];

/// What a call to a DSL name builds.
pub(super) enum Callee {
    /// `analyzer(…)` and friends: the key below `analysis`.
    Analysis(&'static str),
    /// A field class: the ES type.
    Field(String),
    /// django `ListField(field)`: the field itself.
    List,
}

pub(super) struct Module<'a> {
    pub(super) src: &'a str,
    pub(super) masked: String,
    pub(super) defs: Vec<Def>,
    stmts: Vec<Stmt>,
    consts: HashMap<String, (usize, usize)>,
    /// Local name -> dotted path it is bound to by an import.
    bound: HashMap<String, String>,
}

impl<'a> Module<'a> {
    pub(super) fn parse(src: &'a str) -> Self {
        let masked = mask(src);
        let stmts = code_statements(src, &masked);
        let consts = (stmts.iter().filter(|s| s.indent == 0))
            .filter_map(|s| assignment(src, &masked, s))
            .filter(|a| !a.augmented)
            .map(|a| {
                (
                    a.target.to_owned(),
                    (a.value_at, a.value_at + a.value.len()),
                )
            })
            .collect();
        let bound = (imports(&masked).into_iter().filter(|i| i.local != "*"))
            .map(|i| {
                let path = i
                    .name
                    .map_or(i.module.clone(), |n| format!("{}.{n}", i.module));
                (i.local, path)
            })
            .collect();
        Self {
            src,
            defs: defs(&masked),
            masked,
            stmts,
            consts,
            bound,
        }
    }

    /// The value range of a top-level `NAME = …`.
    pub(super) fn constant(&self, name: &str) -> Option<(usize, usize)> {
        self.consts.get(name).copied()
    }

    /// The last segment and flavor of a DSL name as the file imports it.
    pub(super) fn dsl(&self, name: &str) -> Option<(String, &'static str)> {
        let (head, rest) = name.split_once('.').unzip();
        let base = self.bound.get(head.unwrap_or(name))?;
        let path = rest.map_or(base.clone(), |r| format!("{base}.{r}"));
        let flavor = ROOTS
            .iter()
            .find(|(r, _)| path.strip_prefix(r).is_some_and(|t| t.starts_with('.')))?
            .1;
        Some((path.rsplit('.').next()?.to_owned(), flavor))
    }

    pub(super) fn classify(&self, name: &str) -> Option<Callee> {
        let (last, _) = self.dsl(name)?;
        Some(match last.as_str() {
            "analyzer" => Callee::Analysis("analyzer"),
            "tokenizer" => Callee::Analysis("tokenizer"),
            "token_filter" => Callee::Analysis("filter"),
            "char_filter" => Callee::Analysis("char_filter"),
            "normalizer" => Callee::Analysis("normalizer"),
            "ListField" => Callee::List,
            other => Callee::Field(field_type(other)?),
        })
    }

    /// Whether the expression `a..e` is a call of a DSL field class.
    pub(super) fn is_field(&self, a: usize, e: usize) -> bool {
        let b = self.masked.as_bytes();
        let (name, j) = dotted(b, a);
        let call = b.get(skip_ws(b, j)) == Some(&b'(') && j < e;
        call && matches!(self.classify(&name), Some(Callee::Field(_) | Callee::List))
    }

    /// The flavor of a class that derives from a DSL `Document`.
    pub(super) fn document_flavor(&self, class: &Def) -> Option<&'static str> {
        let (from, to) = class.head?;
        let b = self.masked.as_bytes();
        archguard_flow_common::lang_python::split_commas(&self.masked, from, to)
            .into_iter()
            .find_map(|(x, _)| {
                let (name, _) = dotted(b, skip_ws(b, x));
                let (last, flavor) = self.dsl(&name)?;
                (last == "Document").then_some(flavor)
            })
    }

    /// The index of the local class `name`.
    pub(super) fn class_index(&self, name: &str) -> Option<usize> {
        (self.defs.iter()).position(|d| d.kind == DefKind::Class && d.name == name)
    }

    /// The class `name` nested directly in class `of`.
    pub(super) fn nested(&self, of: usize, name: &str) -> Option<&Def> {
        (self.defs.iter())
            .find(|d| d.kind == DefKind::Class && d.parent == Some(of) && d.name == name)
    }

    /// `target = value` statements of the class body itself (not of nested classes or methods):
    /// target and value range.
    pub(super) fn body_assigns(&self, class: &Def) -> Vec<(String, usize, usize)> {
        let mut inside = (self.stmts.iter())
            .filter(|s| s.at >= class.body.0 && s.end <= class.body.1)
            .peekable();
        let Some(indent) = inside.peek().map(|s| s.indent) else {
            return vec![];
        };
        (inside.filter(|s| s.indent == indent))
            .filter_map(|s| assignment(self.src, &self.masked, s))
            .filter(|a| !a.augmented)
            .map(|a| (a.target.to_owned(), a.value_at, a.value_at + a.value.len()))
            .collect()
    }
}

/// `TextField` is `Text`; django's `TimeField` is a keyword and its `FileField` a text.
fn field_type(class: &str) -> Option<String> {
    let base = match class {
        "TimeField" => "Keyword",
        "FileField" => "Text",
        c => c.strip_suffix("Field").unwrap_or(c),
    };
    FIELDS.contains(&base).then(|| snake(base))
}

fn snake(camel: &str) -> String {
    let mut out = String::new();
    for (i, c) in camel.char_indices() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}
