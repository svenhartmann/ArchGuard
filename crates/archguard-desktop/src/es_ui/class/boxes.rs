//! Box contents (header, compartments, rows, chips), ticket 05.

use archguard_engine::core::{SearchField, SearchIndexDetail, ServiceTopology};
use archguard_engine::{
    ChainRef, SearchAnalysisPart, SearchEdge, SearchFieldRole, SearchGraph, SearchNode,
    is_detail_gap, search_analysis_parts, search_analysis_users, search_field_analyzers,
    search_field_depth, search_field_role, short_scope,
};

use super::types::{Key, Switches};
use crate::es_ui::index_page::{DetailNote, IndexPage, index_page};

/// Compartment titles; Slint translates them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Title {
    Fields,
    Settings,
    Analysis,
    Chain,
    Parameters,
    UsedBy,
    Patterns,
    PointsTo,
    Access,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Prefix {
    #[default]
    None,
    /// `+`, a top-level field.
    Field,
    /// `›`, a multi-field.
    Multi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    CharFilter,
    Tokenizer,
    Filter,
}

/// Engine class of a chain member (`ChainRef.class`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainClass {
    Declared,
    Builtin,
    Undeclared,
    Inline,
}

/// `summary` > 0: that many collapsed fields; < 0: that many more rows. A hint row has the
/// identifier `not_read`, `mapping_not_read` or `not_declared` as `text`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub depth: usize,
    pub prefix: Prefix,
    /// A field line: the field's full path (what a click selects), else empty.
    pub path: String,
    pub text: String,
    pub kind_text: String,
    pub note: String,
    pub summary: Option<isize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub text: String,
    pub stage: Stage,
    pub class: ChainClass,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compartment {
    pub title: Title,
    pub lines: Vec<Line>,
    pub chips: Vec<Chip>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoxContent {
    pub stereotype: String,
    pub name: String,
    pub root: String,
    pub ghost: bool,
    pub compartments: Vec<Compartment>,
}

/// Glyphs of the Access compartment (ticket 12 may restyle them).
pub const WRITE_GLYPH: &str = "✎";
// 👁 (U+1F441) draws nothing in the canvas and list fonts (snapshot 2026-11-27); spec fallback "↓".
pub const READ_GLYPH: &str = "↓";
const MAX_SETTINGS: usize = 6;
const DETAIL_KINDS: [&str; 3] = ["index", "index_template", "component_template"];
const ANALYSIS_KINDS: [&str; 5] = [
    "analyzer",
    "normalizer",
    "char_filter",
    "tokenizer",
    "filter",
];

fn comp(title: Title, lines: Vec<Line>) -> Compartment {
    Compartment {
        title,
        lines,
        chips: Vec::new(),
    }
}

fn text_line(text: String) -> Line {
    Line {
        text,
        ..Default::default()
    }
}

/// The content of the box `key` selects; an unknown key gives an empty box.
pub fn content(t: &ServiceTopology, g: &SearchGraph, key: &Key, sw: &Switches) -> BoxContent {
    let owner = match key {
        Key::Node(i) => *i,
        Key::Part { owner, .. } => *owner,
    };
    let Some(node) = g.nodes.get(owner) else {
        return BoxContent::default();
    };
    let artifact = node.artifact.and_then(|i| t.elasticsearch.artifacts.get(i));
    let detail = artifact.and_then(|a| a.detail.as_ref());
    let mut b = BoxContent {
        stereotype: node.kind.clone(),
        name: node.short_label.clone(),
        ghost: !node.declared,
        ..Default::default()
    };
    if let Key::Part { kind, name, .. } = key {
        b.name = name.clone();
        b.root = owner_root(g, node);
        if let Some(d) = detail {
            part_box(&mut b, d, kind, name);
        }
        return b;
    }
    if b.ghost {
        return b;
    }
    if let Some(a) = artifact {
        let line = a.evidence.first();
        b.root = match line.and_then(|e| e.line.map(|l| (e, l))) {
            Some((e, l)) => format!("{} · {}:{l}", short_scope(&a.scope), e.file),
            None => short_scope(&a.scope).to_string(),
        };
    }
    node_box(&mut b, t, g, owner, sw);
    b
}

/// Whether the index's mapping was not read (a `DetailNotLiteral` gap names `mappings` or `body`)
/// although other parts of its detail were.
pub fn mapping_unread(t: &ServiceTopology, node: &SearchNode) -> bool {
    let Some(a) = node.artifact.and_then(|i| t.elasticsearch.artifacts.get(i)) else {
        return false;
    };
    t.diagnostics.iter().any(|d| {
        let missed = d.message.rsplit("not read: ").next().unwrap_or_default();
        is_detail_gap(d, &a.address)
            && missed
                .trim_end_matches('.')
                .split(", ")
                .any(|m| m == "mappings" || m == "body")
    })
}

/// The owner line of a building block: the owner's short label, with its root only when another
/// node has the same short label.
fn owner_root(g: &SearchGraph, node: &SearchNode) -> String {
    let same = g.nodes.iter().filter(|n| n.short_label == node.short_label);
    if same.count() > 1 {
        node.label.clone()
    } else {
        node.short_label.clone()
    }
}

fn node_box(b: &mut BoxContent, t: &ServiceTopology, g: &SearchGraph, i: usize, sw: &Switches) {
    let node = &g.nodes[i];
    let artifact = node.artifact.and_then(|a| t.elasticsearch.artifacts.get(a));
    match (node.kind.as_str(), artifact) {
        (k, Some(a)) if DETAIL_KINDS.contains(&k) => {
            let page = index_page(t, node);
            index_compartments(b, a.detail.as_ref(), (&page, mapping_unread(t, node)), sw);
            if k == "index_template" {
                let lines = a.patterns.iter().cloned().map(text_line).collect();
                b.compartments.push(comp(Title::Patterns, lines));
            }
        }
        ("alias", _) => {
            let lines = edge_lines(g, |e| {
                (e.from == i && e.relation == "reference").then(|| g.nodes[e.to].label.clone())
            });
            b.compartments.push(comp(Title::PointsTo, lines));
        }
        ("service", _) => {
            // A write or unknown edge leaves the service, a read edge points at it.
            let lines = edge_lines(g, |e| match e.relation {
                "write" | "unknown" if e.from == i => Some(access_text(e.relation, &g.nodes[e.to])),
                "read" if e.to == i => Some(access_text(e.relation, &g.nodes[e.from])),
                _ => None,
            });
            b.compartments.push(comp(Title::Access, lines));
        }
        _ => {}
    }
}

fn edge_lines(g: &SearchGraph, text: impl Fn(&SearchEdge) -> Option<String>) -> Vec<Line> {
    g.edges.iter().filter_map(text).map(text_line).collect()
}

fn access_text(relation: &str, other: &SearchNode) -> String {
    let glyph = match relation {
        "write" => WRITE_GLYPH,
        "read" => READ_GLYPH,
        _ => "?",
    };
    format!("{glyph} {}", other.label)
}

fn index_compartments(
    b: &mut BoxContent,
    detail: Option<&SearchIndexDetail>,
    (page, unread): (&IndexPage, bool),
    sw: &Switches,
) {
    let Some(d) = detail else {
        let id = match page.note {
            Some(DetailNote::NotRead) => "not_read",
            _ => "not_declared",
        };
        b.compartments
            .push(comp(Title::Fields, vec![text_line(id.into())]));
        return;
    };
    let fields = field_lines(d, sw);
    let fields = if sw.collapse_fields && !fields.is_empty() {
        vec![Line {
            summary: Some(fields.len() as isize),
            ..Default::default()
        }]
    } else if fields.is_empty() && unread {
        vec![text_line("mapping_not_read".into())]
    } else {
        fields
    };
    let mut settings: Vec<Line> = page
        .settings
        .iter()
        .map(|(k, v)| text_line(format!("{k} = {v}")))
        .collect();
    if settings.len() > MAX_SETTINGS {
        let rest = settings.len() - MAX_SETTINGS;
        settings.truncate(MAX_SETTINGS);
        settings.push(Line {
            summary: Some(-(rest as isize)),
            ..Default::default()
        });
    }
    let analysis = ANALYSIS_KINDS
        .iter()
        .filter_map(|k| {
            let names: Vec<&str> = d
                .analysis
                .iter()
                .filter(|a| a.kind == *k)
                .map(|a| a.name.as_str())
                .collect();
            (!names.is_empty()).then(|| text_line(format!("{k} {}", names.join(", "))))
        })
        .collect();
    for (title, lines) in [
        (Title::Fields, fields),
        (Title::Settings, settings),
        (Title::Analysis, analysis),
    ] {
        if !lines.is_empty() {
            b.compartments.push(comp(title, lines));
        }
    }
}

/// The effective index and search analyzer of the field from the engine (one order rule, spec
/// 2026-11-26); `None` when analysis does not apply or both are `standard`.
fn analyzer_note(d: &SearchIndexDetail, f: &SearchField) -> Option<String> {
    let (index, search) = search_field_analyzers(d, &f.path)?;
    match (index == search, index == "standard") {
        (true, true) => None,
        (true, false) => Some(index),
        _ => Some(format!("{index} / ⌕ {search}")),
    }
}

fn field_lines(d: &SearchIndexDetail, sw: &Switches) -> Vec<Line> {
    let mut out = Vec::new();
    for (i, f) in d.fields.iter().enumerate() {
        let role = search_field_role(&d.fields, i);
        if role == SearchFieldRole::MultiField && !sw.multi_fields {
            continue;
        }
        let prefix = match role {
            SearchFieldRole::Field => Prefix::Field,
            SearchFieldRole::MultiField => Prefix::Multi,
            SearchFieldRole::Property => Prefix::None,
        };
        let analyzers = analyzer_note(d, f);
        let parts: Vec<String> = analyzers
            .into_iter()
            .chain(f.normalizer.as_ref().map(|n| format!("norm {n}")))
            .collect();
        out.push(Line {
            depth: search_field_depth(&d.fields, i),
            prefix,
            path: f.path.clone(),
            text: f.path.rsplit('.').next().unwrap_or_default().into(),
            kind_text: f.kind.clone(),
            note: if parts.is_empty() {
                String::new()
            } else {
                format!("{{{}}}", parts.join(", "))
            },
            summary: None,
        });
    }
    out
}

fn chip_class(c: &str) -> ChainClass {
    match c {
        "declared" => ChainClass::Declared,
        "builtin" => ChainClass::Builtin,
        "inline" => ChainClass::Inline,
        _ => ChainClass::Undeclared,
    }
}

fn chips(p: &SearchAnalysisPart) -> Vec<Chip> {
    let chip = |c: &ChainRef, stage| Chip {
        text: c.name.clone(),
        stage,
        class: chip_class(c.class),
    };
    let cf = p.char_filters.iter().map(|c| chip(c, Stage::CharFilter));
    let tk = p.tokenizer.iter().map(|c| chip(c, Stage::Tokenizer));
    let f = p.filters.iter().map(|c| chip(c, Stage::Filter));
    cf.chain(tk).chain(f).collect()
}

fn part_box(b: &mut BoxContent, d: &SearchIndexDetail, kind: &str, name: &str) {
    let Some(p) = search_analysis_parts(d)
        .into_iter()
        .find(|p| p.kind == kind && p.name == name)
    else {
        return;
    };
    b.stereotype = match &p.kind_type {
        Some(ty) => format!("{kind} · {ty}"),
        None => kind.to_string(),
    };
    let chained = matches!(kind, "analyzer" | "normalizer");
    if chained {
        b.compartments.push(Compartment {
            title: Title::Chain,
            lines: Vec::new(),
            chips: chips(&p),
        });
    }
    if !chained || p.kind_type.as_deref().is_some_and(|t| t != "custom") {
        let ty = p.kind_type.iter().map(|t| format!("type = {t}"));
        let params = p.params.iter().map(|(k, v)| format!("{k} = {v}"));
        let lines: Vec<Line> = ty.chain(params).map(text_line).collect();
        if !lines.is_empty() {
            b.compartments.push(comp(Title::Parameters, lines));
        }
    }
    let users = search_analysis_users(d)
        .into_iter()
        .find(|u| u.kind == kind && u.name == name)
        .map(|u| u.users)
        .unwrap_or_default();
    let lines = users
        .into_iter()
        .map(|u| Line {
            note: if u.role == "search" {
                "(search)".into()
            } else {
                String::new()
            },
            ..text_line(u.subject)
        })
        .collect();
    b.compartments.push(comp(Title::UsedBy, lines));
}
