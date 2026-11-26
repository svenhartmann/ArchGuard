//! One Python file as the DSL reader sees it: constants, imports and the class/def structure,
//! all from the shared scanner (`archguard_flow_common::lang_python`). A name counts as DSL only
//! when an import binds it below one of `ROOTS`; a class or call of the same name without such an
//! import does not match.
//!
//! Class bodies are read member by member: a field is a call of a field class, a
//! `mapped_field(Field())` call or a `M[...]` annotation (typed documents). An assigned value
//! wins over the annotation, as in the DSL. `Union[X, None]` and `Annotated[...]` end as gaps
//! although the DSL reads them.
use archguard_flow_common::lang_pyscan::{dotted, skip_ws};
use archguard_flow_common::lang_python::{
    Arg, Def, DefKind, Import, Stmt, assignment, call_args, code_statements, defs, imports, mask,
    split_commas, string_value,
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
    /// A typed member (`x: M[int]`) with its ES type; `None` when the annotation is not in the
    /// DSL type table.
    Typed(Option<String>),
    /// An `InnerDoc` class of the file as `object`, or `nested` in a list.
    Doc(usize, bool),
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
                let mut renamed = None;
                let kind = self.member_kind(ann, val, &mut renamed)?;
                Some((renamed.unwrap_or(name), kind))
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

    fn member_kind(
        &self,
        ann: Option<Range>,
        val: Option<Range>,
        renamed: &mut Option<String>,
    ) -> Option<Member> {
        let call = |name: &str, (a, e): Range| {
            let b = self.masked.as_bytes();
            let (n, j) = dotted(b, a);
            let open = skip_ws(b, j);
            let hit = b.get(open) == Some(&b'(') && self.dsl(&n).is_some_and(|d| d.0 == name);
            hit.then(|| call_args(self.src, &self.masked, open))?
                .filter(|(_, close)| *close == e - 1)
        };
        let Some((va, ve)) = val else {
            return ann.and_then(|(a, e)| self.annotation(&self.masked[a..e], false));
        };
        if self.is_field(va, ve) {
            return Some(Member::Call(va, ve));
        }
        // The DSL lets an assigned value win over the annotation.
        if let Some((args, _)) = call("mapped_field", (va, ve)) {
            return self.mapped_member(&args, ann, renamed);
        }
        let b = self.masked.as_bytes();
        let (mut name, j) = dotted(b, va);
        if name.is_empty() || b.get(skip_ws(b, j)) != Some(&b'(') {
            return ann.and_then(|(a, e)| self.annotation(&self.masked[a..e], false)); // a plain value
        }
        // A call: a local subclass chain of a field class, or a gap.
        let (open, mut subclass) = (skip_ws(b, j), false);
        for _ in 0..8 {
            if let Some(ty) = self.dsl(&name).and_then(|d| field_type(&d.0)) {
                // shortcut: a subclass call without arguments maps to its base type; with
                // arguments (or an overridden `__init__`) the DSL result differs, so it is a gap.
                let bare =
                    call_args(self.src, &self.masked, open).is_some_and(|(a, _)| a.is_empty());
                return Some(Member::Typed((bare || !subclass).then_some(ty)));
            }
            subclass = true;
            let class = self.class_index(&name).map(|c| self.bases(&self.defs[c]));
            match class.and_then(|bs| bs.into_iter().next()) {
                Some(base) => name = base,
                None => break,
            }
        }
        Some(Member::Typed(None))
    }

    /// `mapped_field(...)`: a readable `field=`/positional field class wins, `exclude=True` drops
    /// the member, an unreadable field is a gap; without a field the annotation decides.
    fn mapped_member(
        &self,
        args: &[Arg],
        ann: Option<Range>,
        renamed: &mut Option<String>,
    ) -> Option<Member> {
        // `exclude` is checked before the field, as in `DocumentOptions.__init__`.
        if (args.iter()).any(|a| a.name == Some("exclude") && a.value == "True") {
            return None;
        }
        // `es_name` (or a positional string) is the index name of the field.
        let text = |a: &&Arg| string_value(a.value);
        *renamed = (args.iter())
            .find(|a| a.name == Some("es_name") || (a.name.is_none() && text(a).is_some()))
            .and_then(|a| text(&a));
        let field = args.iter().find(|a| {
            a.name == Some("field") || (a.name.is_none() && string_value(a.value).is_none())
        });
        // `value` is a slice of `src`; `at` would start at the argument name.
        let r = field.map(|f| {
            let at = f.value.as_ptr() as usize - self.src.as_ptr() as usize;
            (at, at + f.value.len())
        });
        if let Some(r) = r.filter(|&(a, e)| self.is_field(a, e)) {
            return Some(Member::Call(r.0, r.1));
        }
        match field {
            None => ann
                .and_then(|(a, e)| self.annotation(&self.masked[a..e], false))
                .or(Some(Member::Typed(None))),
            Some(_) => Some(Member::Typed(None)),
        }
    }

    /// The annotation as the DSL reads it (`DocumentOptions`): `M`/`Mapped`/`Optional`/`X | None`
    /// are unwrapped, `list[...]` makes it multi, `str`/`int`/... map through a fixed table and a
    /// local `InnerDoc` is an object (nested in a list). `ClassVar` is no field; anything else,
    /// a field class (`M[Keyword]`) included, is a gap: the DSL raises a `TypeError` for it.
    fn annotation(&self, text: &str, multi: bool) -> Option<Member> {
        let text = text.trim();
        let parts = union_parts(text);
        let mut rest = parts.iter().filter(|p| **p != "None");
        if let (Some(one), None) = (rest.next(), rest.next()) {
            if parts.len() > 1 {
                return self.annotation(one, multi);
            }
        } else if parts.len() > 1 {
            return Some(Member::Typed(None));
        }
        let generic = text.strip_suffix(']').and_then(|s| s.split_once('['));
        let last = |n: &str| n.trim().rsplit('.').next().unwrap_or("").to_owned();
        match generic {
            Some((head, inner)) => match last(head).as_str() {
                "ClassVar" => None,
                "M" | "Mapped" | "Optional" => self.annotation(inner, multi),
                "list" | "List" => self.annotation(inner, true),
                _ => Some(Member::Typed(None)),
            },
            None => {
                let name = last(text);
                if name == "ClassVar" {
                    return None;
                }
                let Some(ty) = annotation_type(&name) else {
                    return Some(self.inner_doc(&name, multi));
                };
                Some(Member::Typed(Some(ty.into())))
            }
        }
    }

    /// A class of this file that derives `InnerDoc`; else a gap. Bases of bases are not followed.
    fn inner_doc(&self, name: &str, nested: bool) -> Member {
        let found = self.class_index(name).filter(|&c| {
            (self.bases(&self.defs[c]).iter())
                .any(|b| self.dsl(b).is_some_and(|d| d.0 == "InnerDoc"))
        });
        found.map_or(Member::Typed(None), |c| Member::Doc(c, nested))
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

/// The top-level `|` parts of an annotation, trimmed.
fn union_parts(text: &str) -> Vec<&str> {
    let (mut depth, mut start, mut parts) = (0, 0, vec![]);
    for (i, c) in text.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            '|' if depth == 0 => parts.push(text[std::mem::replace(&mut start, i + 1)..i].trim()),
            _ => {}
        }
    }
    parts.push(text[start..].trim());
    parts
}

/// The DSL's `type_annotation_map`.
fn annotation_type(name: &str) -> Option<&'static str> {
    Some(match name {
        "str" => "text",
        "int" => "integer",
        "float" => "float",
        "bool" => "boolean",
        "bytes" => "binary",
        "datetime" | "date" => "date",
        _ => return None,
    })
}

/// `TextField` is `Text`; django's `TimeField` is a keyword and its `FileField` a text.
fn field_type(class: &str) -> Option<String> {
    let base = match class {
        "NumpyDenseVector" => "DenseVector",
        "TimeField" => "Keyword",
        "FileField" => "Text",
        c => c.strip_suffix("Field").unwrap_or(c),
    };
    FIELDS.contains(&base).then(|| {
        let mut out = String::new();
        for (i, c) in base.char_indices() {
            if c.is_uppercase() && i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        }
        out
    })
}
