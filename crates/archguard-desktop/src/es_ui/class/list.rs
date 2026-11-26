#![cfg_attr(not(test), allow(dead_code))]
// Unused until ticket 12.
//! List model (groups, search, kind filter), ticket 06.

use archguard_engine::core::{SearchIndexDetail, ServiceTopology};
use archguard_engine::{SearchGraph, SearchNode, search_analysis_parts, short_scope};

use super::types::{Group, Key};
use crate::services_ui::search_origin::node_label;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupTitle {
    Services,
    Indices(String),
    IndicesUndeclared,
    Aliases,
    Templates,
    Analysis,
    Processing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListRow {
    pub key: Key,
    pub label: String,
    /// The full label (tooltip): `label` without the root suffix inside a root group.
    pub title: String,
    /// The raw kind (analysis, processing); empty elsewhere.
    pub side: String,
    pub writes: usize,
    pub reads: usize,
    pub warn: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListGroup {
    pub title: GroupTitle,
    pub rows: Vec<ListRow>,
}

/// Analysis kinds in list order.
const STAGES: [&str; 5] = [
    "analyzer",
    "normalizer",
    "tokenizer",
    "filter",
    "char_filter",
];

fn title_of(n: &SearchNode) -> GroupTitle {
    match n.kind.as_str() {
        "service" => GroupTitle::Services,
        "index" if n.declared && n.foreign => GroupTitle::Indices(format!(
            "[{}] {}",
            n.projects.join(", "),
            short_scope(&n.scope)
        )),
        "index" if n.declared => GroupTitle::Indices(short_scope(&n.scope).to_string()),
        "index" => GroupTitle::IndicesUndeclared,
        "alias" => GroupTitle::Aliases,
        "index_template" | "component_template" => GroupTitle::Templates,
        _ => GroupTitle::Processing,
    }
}

fn selected(title: &GroupTitle, group: Group) -> bool {
    matches!(
        (group, title),
        (Group::All, _)
            | (Group::Services, GroupTitle::Services)
            | (
                Group::Indices,
                GroupTitle::Indices(_) | GroupTitle::IndicesUndeclared
            )
            | (Group::Aliases, GroupTitle::Aliases)
            | (Group::Templates, GroupTitle::Templates)
            | (Group::Analysis, GroupTitle::Analysis)
            | (Group::Processing, GroupTitle::Processing)
    )
}

/// `label` without the ` (<folder tail>)` the graph adds for equal names in different roots;
/// the tail (possibly cut in the middle with `…`) must belong to `scope`.
fn without_root<'a>(label: &'a str, scope: &str) -> &'a str {
    let Some(body) = label.strip_suffix(')') else {
        return label;
    };
    let Some(at) = body.rfind(" (") else {
        return label;
    };
    let tail = &body[at + 2..];
    let (head, end) = tail.split_once('…').unwrap_or(("", tail));
    if scope.ends_with(end) && scope.contains(head) {
        &body[..at]
    } else {
        label
    }
}

fn node_row(g: &SearchGraph, i: usize) -> ListRow {
    let n = &g.nodes[i];
    let count = |relation: &str| {
        g.edges
            .iter()
            .filter(|e| e.relation == relation && (e.from == i || e.to == i))
            .count()
    };
    let project = n.foreign.then(|| n.projects.join(", "));
    let title = title_of(n);
    let processing = title == GroupTitle::Processing;
    let name = match title {
        GroupTitle::Indices(_) => without_root(&n.label, short_scope(&n.scope)),
        _ => &n.label,
    };
    let project = project.as_deref().filter(|p| !p.is_empty());
    ListRow {
        key: Key::Node(i),
        title: node_label(&n.label, false, project),
        label: node_label(name, false, project),
        side: if processing {
            n.kind.clone()
        } else {
            String::new()
        },
        writes: count("write"),
        reads: count("read"),
        warn: n.warn,
    }
}

/// One merged building block: the first owner, every owner's name, its definition.
struct Block {
    owner: usize,
    owner_label: String,
    kind: String,
    name: String,
    definition: String,
}

fn blocks(t: &ServiceTopology, g: &SearchGraph) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for (i, n) in g.nodes.iter().enumerate() {
        let detail: Option<&SearchIndexDetail> = n
            .artifact
            .and_then(|a| t.elasticsearch.artifacts[a].detail.as_ref());
        for p in detail.map(search_analysis_parts).unwrap_or_default() {
            let same =
                |b: &&Block| b.kind == p.kind && b.name == p.name && b.definition == p.definition;
            if out.iter().find(same).is_none() {
                out.push(Block {
                    owner: i,
                    owner_label: n.label.clone(),
                    kind: p.kind,
                    name: p.name,
                    definition: p.definition,
                });
            }
        }
    }
    let rank = |k: &str| STAGES.iter().position(|s| *s == k).unwrap_or(STAGES.len());
    out.sort_by(|a, b| (rank(&a.kind), &a.name).cmp(&(rank(&b.kind), &b.name)));
    out
}

fn analysis_rows(t: &ServiceTopology, g: &SearchGraph) -> Vec<ListRow> {
    let all = blocks(t, g);
    all.iter()
        .map(|b| {
            let clash = all
                .iter()
                .filter(|o| o.kind == b.kind && o.name == b.name)
                .count()
                > 1;
            let label = if clash {
                format!("{} · {}", b.name, b.owner_label)
            } else {
                b.name.clone()
            };
            ListRow {
                key: Key::Part {
                    owner: b.owner,
                    kind: b.kind.clone(),
                    name: b.name.clone(),
                },
                title: label.clone(),
                label,
                side: b.kind.clone(),
                writes: 0,
                reads: 0,
                warn: false,
            }
        })
        .collect()
}

pub fn list(t: &ServiceTopology, g: &SearchGraph, query: &str, group: Group) -> Vec<ListGroup> {
    let mut out: Vec<ListGroup> = Vec::new();
    let mut place =
        |title: GroupTitle, row: ListRow| match out.iter_mut().find(|x| x.title == title) {
            Some(x) => x.rows.push(row),
            None => out.push(ListGroup {
                title,
                rows: vec![row],
            }),
        };
    for i in 0..g.nodes.len() {
        place(title_of(&g.nodes[i]), node_row(g, i));
    }
    for r in analysis_rows(t, g) {
        place(GroupTitle::Analysis, r);
    }
    let rank = |t: &GroupTitle| match t {
        GroupTitle::Services => 0,
        GroupTitle::Indices(_) => 1,
        GroupTitle::IndicesUndeclared => 2,
        GroupTitle::Aliases => 3,
        GroupTitle::Templates => 4,
        GroupTitle::Analysis => 5,
        GroupTitle::Processing => 6,
    };
    out.sort_by_key(|x| rank(&x.title));
    let q = query.to_lowercase();
    out.retain(|x| selected(&x.title, group));
    for x in &mut out {
        x.rows.retain(|r| r.label.to_lowercase().contains(&q));
    }
    out.retain(|x| !x.rows.is_empty());
    out
}

pub fn members(groups: &[ListGroup]) -> Vec<Key> {
    groups
        .iter()
        .flat_map(|x| x.rows.iter().map(|r| r.key.clone()))
        .collect()
}
