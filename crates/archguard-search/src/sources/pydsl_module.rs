//! One Python file as the DSL reader sees it: constants, imports and the class/def structure,
//! all from the shared scanner (`archguard_flow_common::lang_python`). A name counts as DSL only
//! when an import binds it below one of `ROOTS`; a class or call of the same name without such an
//! import does not match.
//!
//! Class bodies are read member by member: a field is a call of a field class, a
//! `mapped_field(Field())` call or a `M[...]` annotation (typed documents).
use archguard_flow_common::lang_pyscan::{dotted, skip_ws};
use archguard_flow_common::lang_python::{
    Def, DefKind, Import, Stmt, assignment, call_args, code_statements, defs, imports, mask,
    split_commas,
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

type Range = (usize, usize);

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
    /// Every import of the file, for following a base class into another file.
    pub(super) imports: Vec<Import>,
}

/// What a class body member says about a field.
pub(super) enum Member {
    /// A field class call, to be evaluated: its range.
    Call(usize, usize),
    /// A typed member (`x: M[Keyword]`) with its ES type; `None` when the annotation names no
    /// field class and the type would have to be inferred.
    Typed(Option<String>),
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
        let all = imports(&masked);
        let bound = (all.iter().filter(|i| i.local != "*"))
            .map(|i| {
                let path =
                    (i.name.as_ref()).map_or(i.module.clone(), |n| format!("{}.{n}", i.module));
                (i.local.clone(), path)
            })
            .collect();
        Self {
            src,
            defs: defs(&masked),
            masked,
            stmts,
            consts,
            bound,
            imports: all,
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

    /// Whether the file imports anything below a DSL root.
    pub(super) fn imports_dsl(&self) -> bool {
        self.bound.keys().any(|n| self.dsl(n).is_some())
    }

    /// The base class names of `class` as written (`Base`, `models.Base`).
    pub(super) fn bases(&self, class: &Def) -> Vec<String> {
        let Some((from, to)) = class.head else {
            return vec![];
        };
        let b = self.masked.as_bytes();
        (split_commas(&self.masked, from, to).into_iter())
            .map(|(x, _)| dotted(b, skip_ws(b, x)).0)
            .filter(|n| !n.is_empty())
            .collect()
    }

    /// The field-like members of the class body: name and what they say.
    pub(super) fn members(&self, class: &Def) -> Vec<(String, Member)> {
        let mut inside = (self.stmts.iter())
            .filter(|s| s.at >= class.body.0 && s.end <= class.body.1)
            .peekable();
        let indent = inside.peek().map(|s| s.indent);
        (inside.filter(|s| Some(s.indent) == indent))
            .filter_map(|s| {
                let (name, ann, val) = self.member(s)?;
                Some((name, self.member_kind(ann, val)?))
            })
            .collect()
    }

    /// `name`, annotation range and value range of `name[: ann] [= value]`.
    fn member(&self, s: &Stmt) -> Option<(String, Option<Range>, Option<Range>)> {
        let b = self.masked.as_bytes();
        let (name, j) = dotted(b, s.at);
        let k = skip_ws(b, j);
        if name.is_empty() || name.contains('.') || k >= s.end {
            return None;
        }
        let eq = |i: usize| {
            b[i] == b'=' && b.get(i + 1) != Some(&b'=') && !b"=!<>+-".contains(&b[i - 1])
        };
        let (ann, eq_at) = match b[k] {
            b':' => (Some(k + 1), (k..s.end).find(|&i| eq(i))),
            b'=' => (None, eq(k).then_some(k)),
            _ => return None,
        };
        let end = |to: usize| to - (self.masked[..to].len() - self.masked[..to].trim_end().len());
        let val = eq_at.map(|e| (skip_ws(b, e + 1), end(s.end)));
        let ann = ann.map(|a| (skip_ws(b, a), end(eq_at.unwrap_or(s.end))));
        Some((name, ann, val))
    }

    fn member_kind(&self, ann: Option<Range>, val: Option<Range>) -> Option<Member> {
        let call = |name: &str, (a, e): Range| {
            let b = self.masked.as_bytes();
            let (n, j) = dotted(b, a);
            let open = skip_ws(b, j);
            let hit = b.get(open) == Some(&b'(') && self.dsl(&n).is_some_and(|d| d.0 == name);
            hit.then(|| call_args(self.src, &self.masked, open))?
                .filter(|(_, close)| *close == e - 1)
        };
        if let Some(r) = val.filter(|&(a, e)| self.is_field(a, e)) {
            return Some(Member::Call(r.0, r.1));
        }
        let mapped = val.and_then(|r| call("mapped_field", r));
        let field = mapped.as_ref().and_then(|(args, _)| {
            let first = args.iter().find(|a| a.name.is_none())?;
            let r = (first.at, first.at + first.value.len());
            self.is_field(r.0, r.1).then_some(Member::Call(r.0, r.1))
        });
        field
            .or_else(|| ann.and_then(|r| self.annotated(r)))
            .or_else(|| mapped.map(|_| Member::Typed(None)))
    }

    /// `M[Keyword]` and `Keyword` name a field class; `M[str]` needs inference; `str` is no field.
    fn annotated(&self, (a, e): Range) -> Option<Member> {
        let b = self.masked.as_bytes();
        let (name, j) = dotted(b, a);
        let open = skip_ws(b, j);
        let mapped = self
            .dsl(&name)
            .is_some_and(|d| matches!(d.0.as_str(), "M" | "Mapped"));
        if !(mapped && b.get(open) == Some(&b'[') && e > open + 1) {
            return self.field_type_of(&name).map(|t| Member::Typed(Some(t)));
        }
        let inner = dotted(b, skip_ws(b, open + 1)).0;
        Some(Member::Typed(self.field_type_of(&inner)))
    }

    fn field_type_of(&self, name: &str) -> Option<String> {
        match self.classify(name)? {
            Callee::Field(t) => Some(t),
            _ => None,
        }
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
        "NumpyDenseVector" => "DenseVector",
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
