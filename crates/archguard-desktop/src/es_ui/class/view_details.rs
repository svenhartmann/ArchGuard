//! The detail models as Slint data (`es_class_details.slint`), ticket 12; called by `view`.

use archguard_engine::SearchGraph;
use slint::{ModelRc, SharedString};

use archguard_engine::SearchFieldRole;

use super::details::{
    Details, FieldChain, FieldDetails, FieldRow, Hint, IndexDetails, PartDetails, StageRow,
};
use super::types::Key;
use crate::es_ui::index_page::UsageRow;
use crate::tab_support::model;
use crate::ui::{
    EsDetailField, EsDetailFocus, EsDetailHead, EsDetailHint, EsDetailPair, EsDetailRow,
    EsDetailUse, EsDetails,
};

fn at(file: &str, line: Option<u32>) -> String {
    line.map_or_else(|| file.to_string(), |l| format!("{file}:{l}"))
}

fn pair((k, v): &(String, String)) -> EsDetailPair {
    EsDetailPair {
        key: k.as_str().into(),
        value: v.as_str().into(),
    }
}

fn pairs(items: &[(String, String)]) -> ModelRc<EsDetailPair> {
    model(items.iter().map(pair))
}

/// One row of an analysis chain: stage 0 char filter, 1 tokenizer, 2 filter; class 0 declared (and
/// inline), 1 built in, 2 not declared.
fn stage_row(r: &StageRow) -> EsDetailRow {
    let params: Vec<String> = r.params.iter().map(|(k, v)| format!("{k} = {v}")).collect();
    EsDetailRow {
        stage: match r.stage {
            "char_filter" => 0,
            "tokenizer" => 1,
            _ => 2,
        },
        n: r.n as i32,
        chip: r.name.as_str().into(),
        class: match r.class {
            "builtin" => 1,
            "undeclared" => 2,
            _ => 0,
        },
        params: params.join(" · ").into(),
    }
}

/// "index / ⌕ search · norm normalizer", the analyzers of a field as the box notes show them.
fn analysis_text(f: &FieldRow) -> String {
    let mut out: Vec<String> = Vec::new();
    match (&f.analyzer, &f.search_analyzer) {
        (Some(a), Some(s)) => out.push(format!("{a} / ⌕ {s}")),
        (Some(a), None) => out.push(a.clone()),
        (None, Some(s)) => out.push(format!("⌕ {s}")),
        (None, None) => {}
    }
    out.extend(f.normalizer.iter().map(|n| format!("norm {n}")));
    out.join(" · ")
}

fn usage(u: &UsageRow) -> EsDetailUse {
    EsDetailUse {
        text: format!("{} · {} · {}", u.service, u.access, at(&u.file, u.line)).into(),
        file: u.file.as_str().into(),
        line: u.line.unwrap_or(0) as i32,
        // Foreign usages (another project of a set) are not clickable.
        clickable: u.clickable && !u.file.is_empty(),
    }
}

fn label(g: &SearchGraph, key: &Key) -> String {
    match key {
        Key::Node(i) => g.nodes.get(*i).map(|n| n.short_label.clone()),
        Key::Part { name, .. } => Some(name.clone()),
    }
    .unwrap_or_default()
}

pub fn hint(h: &Hint) -> EsDetailHint {
    EsDetailHint {
        field: h.field.as_str().into(),
        search_only: h.only_search.join(", ").into(),
        index_only: h.only_index.join(", ").into(),
        tokenizer: h.tokenizer_differs,
    }
}

/// A tree row: the last path segment, "\u{203A}" before a multi-field.
fn field_row(f: &FieldRow, selected: Option<&str>) -> EsDetailField {
    let last = f.path.rsplit('.').next().unwrap_or_default();
    let mark = if f.role == SearchFieldRole::MultiField {
        "\u{203A} "
    } else {
        ""
    };
    EsDetailField {
        path: f.path.as_str().into(),
        depth: f.depth as i32,
        selected: selected == Some(f.path.as_str()),
        name: format!("{mark}{last}").into(),
        kind: f.kind.as_str().into(),
        analysis: analysis_text(f).into(),
    }
}

fn chain_rows(c: &Option<FieldChain>) -> (SharedString, ModelRc<EsDetailRow>) {
    let name = c.as_ref().map(|c| c.name.as_str()).unwrap_or_default();
    (
        name.into(),
        model(c.iter().flat_map(|c| c.stages.iter().map(stage_row))),
    )
}

fn focus(f: &FieldDetails) -> EsDetailFocus {
    let (index_name, index_chain) = chain_rows(&f.index);
    let (search_name, search_chain) = chain_rows(&f.search);
    let (norm_name, norm_chain) = chain_rows(&f.normalizer);
    EsDetailFocus {
        path: f.path.as_str().into(),
        kind: f.kind.as_str().into(),
        role: match f.role {
            SearchFieldRole::Field => 0,
            SearchFieldRole::MultiField => 1,
            SearchFieldRole::Property => 2,
        },
        index_name,
        index_chain,
        search_name,
        search_chain,
        norm_name,
        norm_chain,
        hint: f.hint.as_ref().map(hint).unwrap_or_default(),
    }
}

fn index_mode(g: &SearchGraph, key: &Key, d: &IndexDetails, selected: Option<&str>) -> EsDetails {
    let p = &d.page;
    let kind = match key {
        Key::Node(i) => g.nodes.get(*i).map(|n| n.kind.as_str()).unwrap_or_default(),
        Key::Part { .. } => "",
    };
    let first = d.chains.first();
    let hint = d.hints.first().map(hint);
    EsDetails {
        mode: 0,
        head: EsDetailHead {
            title: label(g, key).into(),
            subtitle: format!("«{kind}» · {}", p.overview.scope).into(),
            location: p
                .overview
                .declaration
                .as_ref()
                .map(|(f, l)| at(f, *l))
                .unwrap_or_default()
                .into(),
        },
        fields: model(d.fields.iter().map(|f| field_row(f, selected))),
        chain_name: first
            .map(|c| c.analyzer.as_str())
            .unwrap_or_default()
            .into(),
        chain: model(first.iter().flat_map(|c| c.stages.iter().map(stage_row))),
        hint: hint.unwrap_or_default(),
        settings: pairs(&p.settings),
        links: model(p.links.iter().map(|l| EsDetailPair {
            key: l.kind.as_str().into(),
            value: l.name.as_str().into(),
        })),
        usages: model(p.usages.iter().map(usage)),
        ..Default::default()
    }
}

fn part_mode(p: &PartDetails) -> EsDetails {
    let kind = match &p.kind_type {
        Some(t) => format!("{} · {t}", p.kind),
        None => p.kind.clone(),
    };
    let definition: Vec<String> = p
        .definition
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect();
    EsDetails {
        mode: 1,
        head: EsDetailHead {
            title: p.name.as_str().into(),
            subtitle: format!("«{kind}» · {}", p.owner).into(),
            location: p
                .declaration
                .as_ref()
                .map(|(f, l)| at(f, *l))
                .unwrap_or_default()
                .into(),
        },
        chain: model(p.stages.iter().map(stage_row)),
        params: pairs(&p.params),
        used_by: model(p.used_by.iter().map(|u| u.as_str().into())),
        definition: definition.join("\n").into(),
        ..Default::default()
    }
}

/// The details column for `key`; `d` is `details::details(t, g, key)`. An index with a field in
/// focus (`field`, the details of the field `selected`) shows that field (mode 3).
pub fn slint_details(
    g: &SearchGraph,
    key: &Key,
    d: &Details,
    field: Option<&FieldDetails>,
    selected: Option<&str>,
) -> EsDetails {
    match d {
        Details::Index(i) => EsDetails {
            mode: if field.is_some() { 3 } else { 0 },
            field: field.map(focus).unwrap_or_default(),
            ..index_mode(g, key, i, selected)
        },
        Details::Part(p) => part_mode(p),
        Details::Other(o) => EsDetails {
            mode: 2,
            head: EsDetailHead {
                title: label(g, key).into(),
                ..Default::default()
            },
            other: o.clone(),
            ..Default::default()
        },
    }
}
