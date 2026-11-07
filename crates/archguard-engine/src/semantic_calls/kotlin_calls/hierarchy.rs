//! Member lookup along Kotlin classes, their companions, nested classes and
//! supertypes. A supertype outside the scanned Kotlin code (a library or
//! Java class, and the implicit `Enum` of an enum class) may declare any
//! member, so a name not declared in the class itself is unknown there.

use super::index::{ClassInfo, Hit, Index, combine, unique_class};
use super::names;

/// Deeper hierarchies are cut off as unknown (and guard against cycles).
const DEPTH: usize = 32;

type Lookup = for<'i> fn(&'i Index<'i>, &'i ClassInfo<'i>, &str, usize) -> Hit<'i>;

pub(super) fn outer<'i>(ix: &'i Index<'i>, class: &ClassInfo) -> Option<&'i ClassInfo<'i>> {
    ix.classes.get(class.facts.outer.as_deref()?)
}

/// The nested class `name` declared directly in `class`.
pub(super) fn nested<'i>(
    ix: &'i Index<'i>,
    class: &ClassInfo,
    name: &str,
) -> Option<&'i ClassInfo<'i>> {
    let ids = ix.nested.get(class.facts.id.as_str())?;
    let mut found = ids
        .iter()
        .filter_map(|id| ix.classes.get(id))
        .filter(|c| c.facts.name == name);
    match (found.next(), found.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

pub(super) fn companion<'i>(ix: &'i Index<'i>, class: &ClassInfo) -> Option<&'i ClassInfo<'i>> {
    let ids = ix.nested.get(class.facts.id.as_str())?;
    ids.iter()
        .filter_map(|id| ix.classes.get(id))
        .find(|c| c.facts.companion)
}

/// A function `name` declared in `class` itself. Properties (which may hold
/// a function), member extensions, abstract declarations, overloads and
/// the generated `copy`/`componentN` of a data class are unknown.
fn own<'i>(ix: &'i Index<'i>, class: &ClassInfo, name: &str) -> Hit<'i> {
    let facts = class.facts;
    let data = facts.modifiers.iter().any(|m| m == "data");
    if facts
        .values
        .iter()
        .chain(&facts.bodiless)
        .any(|v| v == name)
        || (data && (name == "copy" || name.starts_with("component")))
    {
        return Hit::Unknown;
    }
    let mut own = ix
        .methods
        .get(facts.id.as_str())
        .into_iter()
        .flatten()
        .filter(|m| m.name == name);
    match (own.next(), own.next()) {
        (Some(method), None) => Hit::Method(method),
        (None, _) => Hit::Missing,
        _ => Hit::Unknown,
    }
}

/// The resolved supertypes of `class`; `None` for one outside the scanned
/// Kotlin code.
pub(super) fn supertypes<'i>(
    ix: &'i Index<'i>,
    class: &ClassInfo,
) -> Vec<Option<&'i ClassInfo<'i>>> {
    let mut out: Vec<_> = class
        .facts
        .supertypes
        .iter()
        .map(|written| type_path(ix, written, class))
        .collect();
    if class.facts.kind == "enum" {
        out.push(None);
    }
    out
}

/// The class a supertype `written` in the header of `class` names.
fn type_path<'i>(ix: &'i Index<'i>, written: &str, class: &ClassInfo) -> Option<&'i ClassInfo<'i>> {
    let segments: Vec<&str> = written.split('.').collect();
    let mut scope = outer(ix, class);
    let mut head = None;
    while let (Some(k), None) = (scope, head) {
        head = nested(ix, k, segments[0]);
        scope = outer(ix, k);
    }
    let head = head.or_else(|| match names::top(ix, segments[0], class.file) {
        Hit::Class(found) => Some(found),
        _ => None,
    });
    head.and_then(|c| segments[1..].iter().try_fold(c, |c, s| nested(ix, c, s)))
        .or_else(|| unique_class(ix, written))
}

/// The function `name` callable on an instance of `class`: its own, then
/// inherited ones.
pub(super) fn member<'i>(
    ix: &'i Index<'i>,
    class: &'i ClassInfo<'i>,
    name: &str,
    depth: usize,
) -> Hit<'i> {
    if depth > DEPTH {
        return Hit::Unknown;
    }
    match own(ix, class, name) {
        Hit::Missing => inherited(ix, class, name, depth, member),
        hit => hit,
    }
}

/// What a bare `name` inside the body of `class` binds to: its own
/// functions, nested classes and companion members, then those of its
/// supertypes.
pub(super) fn class_scope<'i>(
    ix: &'i Index<'i>,
    class: &'i ClassInfo<'i>,
    name: &str,
    depth: usize,
) -> Hit<'i> {
    if depth > DEPTH {
        return Hit::Unknown;
    }
    let mut hits = vec![own(ix, class, name)];
    hits.extend(nested(ix, class, name).map(Hit::Class));
    hits.extend(companion(ix, class).map(|c| member(ix, c, name, depth + 1)));
    match combine(hits) {
        Hit::Missing => inherited(ix, class, name, depth, class_scope),
        hit => hit,
    }
}

/// `lookup` over the supertypes of `class`: one target, reached on one or
/// more paths, or unknown.
pub(super) fn inherited<'i>(
    ix: &'i Index<'i>,
    class: &'i ClassInfo<'i>,
    name: &str,
    depth: usize,
    lookup: Lookup,
) -> Hit<'i> {
    let mut hits = Vec::new();
    for supertype in supertypes(ix, class) {
        let Some(supertype) = supertype else {
            return Hit::Unknown;
        };
        hits.push(lookup(ix, supertype, name, depth + 1));
    }
    combine(hits)
}
