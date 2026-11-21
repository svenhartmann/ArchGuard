//! The comparison as the view shows it: a package tree whose packages and
//! files are marked added / removed / changed / unchanged, package
//! relations with the same marks, and cycle changes per package and
//! relation. The desktop form of the web viewer's `prepareDiffView` and
//! `buildDiffRows`.

use std::collections::BTreeSet;

use archguard_engine::core::{ArchitectureDiffDetails, ArchitectureEdgeChange, Ref};

use super::packages::Packages;

/// Sorted so that changes come before unchanged entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Added,
    Removed,
    /// On both sides, with other evidence (a relation) or a changed
    /// relation or file (a package or file).
    Changed,
    Unchanged,
}

/// A package's or relation's place in a cycle, before and after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cycle {
    None,
    /// In a cycle on both sides.
    Kept,
    New,
    Removed,
}

impl Cycle {
    fn of(before: bool, after: bool) -> Self {
        match (before, after) {
            (false, false) => Cycle::None,
            (true, true) => Cycle::Kept,
            (false, true) => Cycle::New,
            (true, false) => Cycle::Removed,
        }
    }
}

/// A dependency between two packages, or between two files.
#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub from: String,
    pub to: String,
    pub status: Status,
    pub cycle: Cycle,
    /// The target side's evidence; the base side's for a removed relation.
    pub evidence: Vec<Ref>,
}

/// A row of the package tree: a package, or one of its changed files.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: String,
    pub file: bool,
    pub status: Status,
    pub cycle: Cycle,
}

/// What the summary counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub packages_added: usize,
    pub packages_removed: usize,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    pub cycles_new: usize,
    pub cycles_removed: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Comparison {
    /// The two sides as the user chose them ("main", "Git index").
    pub from: String,
    pub to: String,
    pub rows: Vec<Row>,
    /// Package relations, both sides.
    pub relations: Vec<Relation>,
    /// File relations that changed (the engine's file-level diff).
    pub file_relations: Vec<Relation>,
    /// No node or file relation changed at all.
    pub empty: bool,
}

impl Comparison {
    pub fn build(details: &ArchitectureDiffDetails, from: &str, to: &str) -> Self {
        let before = Packages::of(&details.base_graph);
        let after = Packages::of(&details.target_graph);
        let relations = package_relations(&before, &after);
        let file_relations = file_relations(details);
        let added: BTreeSet<&String> = details.added_nodes.iter().collect();
        let removed: BTreeSet<&String> = details.removed_nodes.iter().collect();
        let mut rows = Vec::new();
        let names: BTreeSet<&String> = before.files.keys().chain(after.files.keys()).collect();
        for name in names {
            let files = changed_files(name, &before, &after, &file_relations, &added, &removed);
            let touched = relations
                .iter()
                .any(|r| r.status != Status::Unchanged && (&r.from == name || &r.to == name));
            let status = match (
                before.files.contains_key(name),
                after.files.contains_key(name),
            ) {
                (false, _) => Status::Added,
                (_, false) => Status::Removed,
                _ if touched || !files.is_empty() => Status::Changed,
                _ => Status::Unchanged,
            };
            let cycle = Cycle::of(before.in_cycle(name), after.in_cycle(name));
            rows.push(Row {
                name: name.clone(),
                file: false,
                status,
                cycle,
            });
            rows.extend(files);
        }
        let empty = details.added_nodes.is_empty()
            && details.removed_nodes.is_empty()
            && file_relations.is_empty();
        Comparison {
            from: from.into(),
            to: to.into(),
            rows,
            relations,
            file_relations,
            empty,
        }
    }

    pub fn counts(&self) -> Counts {
        let packages = self.rows.iter().filter(|r| !r.file);
        let rel = |s| self.relations.iter().filter(|r| r.status == s).count();
        Counts {
            packages_added: packages
                .clone()
                .filter(|r| r.status == Status::Added)
                .count(),
            packages_removed: packages
                .clone()
                .filter(|r| r.status == Status::Removed)
                .count(),
            added: rel(Status::Added),
            removed: rel(Status::Removed),
            changed: rel(Status::Changed),
            cycles_new: packages.clone().filter(|r| r.cycle == Cycle::New).count(),
            cycles_removed: packages.filter(|r| r.cycle == Cycle::Removed).count(),
        }
    }

    /// The relations of row `i`: a package's relations, changes first, or
    /// a file's changed relations.
    pub fn relations_of(&self, i: usize) -> Vec<&Relation> {
        let Some(row) = self.rows.get(i) else {
            return Vec::new();
        };
        let list = if row.file {
            &self.file_relations
        } else {
            &self.relations
        };
        let mut out: Vec<&Relation> = list
            .iter()
            .filter(|r| r.from == row.name || r.to == row.name)
            .collect();
        out.sort_by_key(|r| r.status);
        out
    }
}

fn package_relations(before: &Packages, after: &Packages) -> Vec<Relation> {
    let keys: BTreeSet<&(String, String)> = before.edges.keys().chain(after.edges.keys()).collect();
    keys.into_iter()
        .map(|key| {
            let (old, new) = (before.edges.get(key), after.edges.get(key));
            let status = match (old, new) {
                (None, _) => Status::Added,
                (_, None) => Status::Removed,
                (a, b) if a != b => Status::Changed,
                _ => Status::Unchanged,
            };
            let (from, to) = key;
            Relation {
                from: from.clone(),
                to: to.clone(),
                status,
                cycle: Cycle::of(
                    before.edge_in_cycle(from, to),
                    after.edge_in_cycle(from, to),
                ),
                evidence: new.or(old).cloned().unwrap_or_default(),
            }
        })
        .collect()
}

fn file_relations(d: &ArchitectureDiffDetails) -> Vec<Relation> {
    let of = |list: &[ArchitectureEdgeChange], status| {
        list.iter()
            .map(|e| Relation {
                from: e.from.clone(),
                to: e.to.clone(),
                status,
                cycle: Cycle::None,
                evidence: if e.after.is_empty() {
                    e.before.clone()
                } else {
                    e.after.clone()
                },
            })
            .collect::<Vec<_>>()
    };
    let mut all = of(&d.added_edges, Status::Added);
    all.extend(of(&d.removed_edges, Status::Removed));
    all.extend(of(&d.changed_edges, Status::Changed));
    all
}

/// The files of `package` that were added, removed or have a changed
/// relation, as rows below it.
fn changed_files(
    package: &str,
    before: &Packages,
    after: &Packages,
    relations: &[Relation],
    added: &BTreeSet<&String>,
    removed: &BTreeSet<&String>,
) -> Vec<Row> {
    let in_package = |f: &String| {
        after
            .owner
            .get(f)
            .or_else(|| before.owner.get(f))
            .is_some_and(|p| p == package)
    };
    let touched = relations.iter().flat_map(|r| [&r.from, &r.to]);
    let names: BTreeSet<&String> = added
        .iter()
        .chain(removed.iter())
        .copied()
        .chain(touched)
        .filter(|f| in_package(f))
        .collect();
    names
        .into_iter()
        .map(|name| Row {
            name: name.clone(),
            file: true,
            status: if added.contains(name) {
                Status::Added
            } else if removed.contains(name) {
                Status::Removed
            } else {
                Status::Changed
            },
            cycle: Cycle::None,
        })
        .collect()
}
