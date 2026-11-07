//! File-level name lookup in Kotlin's order: explicit imports (with
//! aliases), the file's package, star imports, then the default imports,
//! which are outside the project.

use super::index::{Hit, Index, combine, qualify, unique_class};

/// What the qualified `name` declares at the top level of the project.
/// Extension functions, overloads, duplicates, properties, type aliases and
/// Java types are unknown.
pub(super) fn declared_at<'i>(ix: &'i Index<'i>, name: &str) -> Hit<'i> {
    let mut hits = Vec::new();
    match ix.by_name.get(name).map(Vec::as_slice) {
        Some([id]) => hits.push(Hit::Class(&ix.classes[id])),
        Some(_) => hits.push(Hit::Unknown),
        None => {}
    }
    match ix.functions.get(name).map(Vec::as_slice) {
        Some([function]) if !function.extension => {
            hits.push(Hit::Method(ix.method_ids[function.method_id.as_str()]));
        }
        Some(_) => hits.push(Hit::Unknown),
        None => {}
    }
    if ix.values.contains(name) || ix.java.types.contains(name) {
        hits.push(Hit::Unknown);
    }
    combine(hits)
}

/// `name` (or a prefix of it) is a Java type or package, or a Kotlin
/// package: declared in the project, but not as a Kotlin declaration.
pub(super) fn in_project_package(ix: &Index, name: &str) -> bool {
    let mut prefix = name;
    loop {
        if ix.java.types.contains(prefix) || ix.java.packages.contains(prefix) {
            return true;
        }
        match prefix.rsplit_once('.') {
            Some((parent, _)) => prefix = parent,
            None => break,
        }
    }
    name.rsplit_once('.')
        .is_some_and(|(parent, _)| ix.packages.contains(parent))
}

/// What an explicit `import path` binds.
fn imported<'i>(ix: &'i Index<'i>, path: &'i str) -> Hit<'i> {
    let hit = declared_at(ix, path);
    if !hit.is_missing() {
        return hit;
    }
    // `import a.Obj.m` imports a member of an object; only one the object
    // declares itself, once, is resolved.
    if let Some((owner, name)) = path.rsplit_once('.')
        && let Some(class) = unique_class(ix, owner)
    {
        let facts = class.facts;
        let mut own = ix
            .methods
            .get(facts.id.as_str())
            .into_iter()
            .flatten()
            .filter(|m| m.name == name);
        let shadowed = facts
            .values
            .iter()
            .chain(&facts.bodiless)
            .any(|v| v == name);
        return match (own.next(), own.next()) {
            (Some(method), None) if facts.kind == "object" && !shadowed => Hit::Method(method),
            _ => Hit::Unknown,
        };
    }
    if in_project_package(ix, path) || !ix.complete {
        Hit::Unknown
    } else {
        Hit::External(Some(path))
    }
}

/// What a simple `name` binds at the top level of `file`; `Missing` only
/// when no project declaration uses the name and every file was scanned.
pub(super) fn top<'i>(ix: &'i Index<'i>, name: &str, file: usize) -> Hit<'i> {
    let symbols = ix.symbols(file);
    let explicit: Vec<_> = symbols
        .imports
        .iter()
        .filter(|i| i.name.as_deref() == Some(name))
        .map(|i| imported(ix, &i.path))
        .collect();
    if !explicit.is_empty() {
        return combine(explicit);
    }
    let local = declared_at(ix, &qualify(&symbols.package, name));
    if !local.is_missing() {
        return local;
    }
    let stars: Vec<_> = symbols
        .imports
        .iter()
        .filter(|i| i.name.is_none())
        .map(|i| {
            if ix.java.types.contains(&i.path) {
                // Static members of a Java class.
                Hit::Unknown
            } else {
                declared_at(ix, &format!("{}.{name}", i.path))
            }
        })
        .filter(|hit| !hit.is_missing())
        .collect();
    match stars.as_slice() {
        [] if !ix.is_declared(name) && ix.complete => Hit::Missing,
        [one] => *one,
        _ => Hit::Unknown,
    }
}
