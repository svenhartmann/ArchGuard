//! Which supertype names a source can mean (package and imports).
use crate::source::Source;

/// `package x.y;` of a source ("" for none).
fn package_of(text: &str) -> &str {
    (text.lines())
        .find_map(|l| l.trim().strip_prefix("package "))
        .map_or("", |p| p.trim().trim_end_matches(';').trim())
}

/// Whether the class in source `imp` can name interface `name` of source
/// `api`: same package, or an import of it (single or `*`).
pub fn names_interface(imp: &str, api: &str, name: &str) -> bool {
    let pkg = package_of(api);
    pkg == package_of(imp)
        || imp
            .lines()
            .filter_map(|l| l.trim().strip_prefix("import "))
            .any(|i| {
                let i = i.trim().trim_end_matches(';').trim();
                i == format!("{pkg}.{name}") || i == format!("{pkg}.*")
            })
}

/// Whether supertype `name`, as `src` names it, is a JDK or Kotlin type: it
/// declares no routes (an `Object`-like name, or imported from `java.`,
/// `javax.` or `kotlin.`).
pub fn jdk_type(src: &Source, name: &str) -> bool {
    ["Object", "Any", "Record", "Enum"].contains(&name)
        || (src.text.lines())
            .filter_map(|l| l.trim().strip_prefix("import "))
            .map(|i| i.trim().trim_end_matches(';').trim())
            .any(|i| {
                ["java.", "javax.", "kotlin."]
                    .iter()
                    .any(|p| i.starts_with(p))
                    && i.rsplit('.').next() == Some(name)
            })
}
