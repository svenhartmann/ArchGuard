//! The detail models as Slint data (`es_class_details.slint`), ticket 12; called by `view`.

use archguard_engine::SearchGraph;
use slint::ModelRc;

use super::details::{Details, FieldRow, Hint, IndexDetails, PartDetails, StageRow};
use super::types::Key;
use crate::es_ui::index_page::UsageRow;
use crate::tab_support::model;
use crate::ui::{
    EsDetailField, EsDetailHead, EsDetailHint, EsDetailPair, EsDetailRow, EsDetailUse, EsDetails,
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

/// The names of a list, or a dash when it is empty (the hint sentence needs a word).
fn names(items: &[String]) -> String {
    if items.is_empty() {
        "–".into()
    } else {
        items.join(", ")
    }
}

fn label(g: &SearchGraph, key: &Key) -> String {
    match key {
        Key::Node(i) => g.nodes.get(*i).map(|n| n.label.clone()),
        Key::Part { name, .. } => Some(name.clone()),
    }
    .unwrap_or_default()
}

pub fn hint(h: &Hint) -> EsDetailHint {
    EsDetailHint {
        field: h.field.as_str().into(),
        search_only: names(&h.only_search).into(),
        index_only: names(&h.only_index).into(),
        ..Default::default()
    }
}

fn index_mode(g: &SearchGraph, key: &Key, d: &IndexDetails) -> EsDetails {
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
        fields: model(d.fields.iter().map(|f| EsDetailField {
            name: f.path.as_str().into(),
            kind: f.kind.as_str().into(),
            analysis: analysis_text(f).into(),
        })),
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

/// The details column for `key`; `d` is `details::details(t, g, key)`.
pub fn slint_details(g: &SearchGraph, key: &Key, d: &Details) -> EsDetails {
    match d {
        Details::Index(i) => index_mode(g, key, i),
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
