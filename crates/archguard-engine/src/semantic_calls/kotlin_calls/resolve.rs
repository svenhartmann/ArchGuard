//! The target of each Kotlin call site: a project function or constructor,
//! an external name, or unresolved.

use archguard_core::{CallMethod, MethodCall};
use archguard_extract::kotlin::{KotlinCallSite, KotlinTarget};

use super::hierarchy::{class_scope, companion, inherited, member, nested, outer, supertypes};
use super::index::{ClassInfo, Hit, Index, unique_class};
use super::names;

/// (resolution, dispatch, to_class, to_method) of one call.
type Target = (&'static str, &'static str, Option<String>, Option<String>);
const UNRESOLVED: Target = ("unresolved", "unknown", None, None);

/// The call edges of all scanned files.
pub(super) fn resolve(ix: &Index) -> Vec<MethodCall> {
    let mut calls = Vec::new();
    for (file, (path, symbols)) in ix.files.iter().enumerate() {
        for site in &symbols.sites {
            let (resolution, dispatch, to_class, to_method) = target(ix, file, site);
            calls.push(MethodCall {
                from_class: site.from_class.clone(),
                from_method: site.from_method.clone(),
                to_class,
                to_method,
                target_label: site.label.clone(),
                dispatch: dispatch.into(),
                resolution: resolution.into(),
                file: path.clone(),
                line: site.line,
                column: site.column,
                code: site.code.clone(),
            });
        }
    }
    calls
}

fn target(ix: &Index, file: usize, site: &KotlinCallSite) -> Target {
    let class = ix.classes.get(site.from_class.as_str());
    match (&site.target, class) {
        (KotlinTarget::Name(name), _) => bare_call(ix, file, class, name),
        (KotlinTarget::Path(segments), _) => path_call(ix, file, class, segments),
        (KotlinTarget::This(name), Some(class)) => method(member(ix, class, name, 0)),
        (KotlinTarget::Super { qualifier, name }, Some(class)) => {
            super_call(ix, class, qualifier.as_deref(), name)
        }
        _ => UNRESOLVED,
    }
}

/// What a bare `name` binds to inside `class` (or at the top level).
fn bare<'i>(
    ix: &'i Index<'i>,
    file: usize,
    class: Option<&'i ClassInfo<'i>>,
    name: &str,
) -> Hit<'i> {
    let mut scope = class;
    while let Some(k) = scope {
        let hit = class_scope(ix, k, name, 0);
        if !hit.is_missing() {
            return hit;
        }
        scope = outer(ix, k);
    }
    names::top(ix, name, file)
}

/// `name(…)`: a function, a member or a constructor.
fn bare_call(ix: &Index, file: usize, class: Option<&ClassInfo>, name: &str) -> Target {
    match bare(ix, file, class, name) {
        Hit::Method(found) => function(found),
        Hit::Class(found) => constructor(ix, found),
        // `Missing` here means: declared nowhere in a fully scanned project.
        Hit::External(_) | Hit::Missing => ("external", "unknown", None, None),
        Hit::Unknown => UNRESOLVED,
    }
}

/// `a.b.C.m(…)` or `C.m(…)` or `C.Nested(…)`.
fn path_call(ix: &Index, file: usize, class: Option<&ClassInfo>, segments: &[String]) -> Target {
    let receiver = segments[1..segments.len() - 1]
        .iter()
        .map(|s| format!(".{s}"));
    match bare(ix, file, class, &segments[0]) {
        Hit::Class(found) => walk(ix, found, &segments[1..]),
        Hit::External(Some(name)) => {
            external_class(name.to_string() + &receiver.collect::<String>())
        }
        Hit::External(None) | Hit::Missing => qualified_call(ix, segments),
        Hit::Method(_) | Hit::Unknown => UNRESOLVED,
    }
}

/// A fully qualified call: the longest prefix that names a project class
/// or function.
fn qualified_call(ix: &Index, segments: &[String]) -> Target {
    let n = segments.len();
    for k in (1..=n).rev() {
        let prefix = segments[..k].join(".");
        if k == n {
            match names::declared_at(ix, &prefix) {
                Hit::Class(found) => return constructor(ix, found),
                Hit::Method(found) => return function(found),
                Hit::Missing => {}
                _ => return UNRESOLVED,
            }
        } else if let Some(found) = unique_class(ix, &prefix) {
            return walk(ix, found, &segments[k..]);
        }
        if names::in_project_package(ix, &prefix) {
            return UNRESOLVED;
        }
    }
    external_class(segments[..n - 1].join("."))
}

/// `names` after the class `class`: nested classes, then the called name.
fn walk(ix: &Index, class: &ClassInfo, names: &[String]) -> Target {
    let Some((last, middle)) = names.split_last() else {
        return UNRESOLVED;
    };
    let mut current = class;
    for segment in middle {
        match nested(ix, current, segment) {
            Some(inner) => current = inner,
            // A property or enum entry: its type is unknown.
            None => return UNRESOLVED,
        }
    }
    if let Some(inner) = nested(ix, current, last) {
        return constructor(ix, inner);
    }
    let owner = if current.facts.kind == "object" {
        Some(current)
    } else {
        companion(ix, current)
    };
    owner.map_or(UNRESOLVED, |owner| method(member(ix, owner, last, 0)))
}

/// `super.m(…)` names the one supertype that has `m`; `super<I>.m(…)` the
/// supertype `I`.
fn super_call(ix: &Index, class: &ClassInfo, qualifier: Option<&str>, name: &str) -> Target {
    let Some(qualifier) = qualifier else {
        return method(inherited(ix, class, name, 0, member));
    };
    let mut chosen = class
        .facts
        .supertypes
        .iter()
        .zip(supertypes(ix, class))
        .filter(|(written, _)| written.rsplit('.').next() == Some(qualifier));
    match (chosen.next(), chosen.next()) {
        (Some((_, Some(supertype))), None) => method(member(ix, supertype, name, 0)),
        _ => UNRESOLVED,
    }
}

/// A constructor call of `class`; objects, interfaces, enum, abstract and
/// sealed classes cannot be constructed, and a companion `invoke` competes.
fn constructor(ix: &Index, class: &ClassInfo) -> Target {
    let facts = class.facts;
    let abstract_class = facts
        .modifiers
        .iter()
        .any(|m| m == "abstract" || m == "sealed");
    let invoke = companion(ix, class).is_some_and(|c| !member(ix, c, "invoke", 0).is_missing());
    if facts.kind != "class" || abstract_class || invoke {
        return UNRESOLVED;
    }
    ("project", "constructor", Some(facts.id.clone()), None)
}

fn method(hit: Hit) -> Target {
    match hit {
        Hit::Method(found) => function(found),
        _ => UNRESOLVED,
    }
}

fn function(method: &CallMethod) -> Target {
    let dispatch = if method.is_static {
        "static"
    } else {
        "instance"
    };
    (
        "project",
        dispatch,
        Some(method.class_id.clone()),
        Some(method.id.clone()),
    )
}

fn external_class(name: String) -> Target {
    ("external", "unknown", Some(name), None)
}
