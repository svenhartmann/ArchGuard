//! The index page model as Slint data (`es_index_page.slint`).

use slint::SharedString;

use super::index_page::{DetailNote, IndexPage, MappingRow, UsageRow, index_page};
use super::state::EsState;
use crate::tab_support::model;
use crate::ui::{EsMapRow, EsPage, EsPair, EsUse};

/// Kinds with a page; the others keep the detail bar.
const KINDS: [&str; 3] = ["index", "index_template", "component_template"];

/// The focused node's page: indices, templates and undeclared indices have one.
pub fn page(s: &EsState) -> Option<EsPage> {
    let node = &s.graph.nodes[s.focus?];
    let t = (*s.topology).as_ref().ok()?;
    KINDS
        .contains(&node.kind.as_str())
        .then(|| convert(&index_page(t, node), &s.problem))
}

fn at(file: &str, line: Option<u32>) -> String {
    line.map_or_else(|| file.to_string(), |l| format!("{file}:{l}"))
}

fn pair(key: &str, value: &str) -> EsPair {
    EsPair {
        key: key.into(),
        value: value.into(),
    }
}

fn map_row(r: &MappingRow) -> EsMapRow {
    let analyzers: Vec<String> = [
        r.analyzer.clone(),
        r.search_analyzer.as_ref().map(|a| format!("search: {a}")),
        r.normalizer.as_ref().map(|n| format!("normalizer: {n}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    EsMapRow {
        depth: r.depth as i32,
        name: r.name.as_str().into(),
        kind: r.kind.as_str().into(),
        analyzers: analyzers.join(" · ").into(),
        parent: r.parent,
    }
}

fn usage(u: &UsageRow) -> EsUse {
    EsUse {
        text: format!("{} · {} · {}", u.service, u.access, at(&u.file, u.line)).into(),
        file: u.file.as_str().into(),
        line: u.line.unwrap_or(0) as i32,
        clickable: u.clickable && !u.file.is_empty(),
    }
}

fn convert(p: &IndexPage, problem: &str) -> EsPage {
    EsPage {
        declaration: p
            .overview
            .declaration
            .as_ref()
            .map(|(f, l)| at(f, *l))
            .unwrap_or_default()
            .into(),
        scope: p.overview.scope.as_str().into(),
        address: p.overview.address.as_str().into(),
        origins: p.overview.origins.join(", ").into(),
        mapping: model(p.mapping.iter().map(map_row)),
        settings: model(p.settings.iter().map(|(k, v)| pair(k, v))),
        analysis: model(
            p.analysis
                .iter()
                .map(|a| pair(&format!("{} {}", a.kind, a.name), &a.definition)),
        ),
        note: match p.note {
            None => 0,
            Some(DetailNote::NotRead) => 1,
            Some(DetailNote::NotDeclared) => 2,
        },
        links: model(p.links.iter().map(|l| pair(&l.kind, &l.name))),
        usages: model(p.usages.iter().map(usage)),
        problem: SharedString::from(problem),
    }
}
