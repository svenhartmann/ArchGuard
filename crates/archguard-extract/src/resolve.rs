//! Two-stage target resolution (ARCHITECTURE.md Kap. 6, Schritt 4–5, plus
//! Kap. 5.1 and 6.1): the pure functions that turn a raw import's
//! [`crate::extract::TargetSpec`] into graph node names — no IO, no
//! filesystem access, so the whole mechanism is table-testable.
//!
//! Stage one resolves `TargetSpec::Path` specs against the file tree:
//! [`strip_root`] cuts the configured source-root prefix off a walked path
//! and [`resolve_relative`] normalizes `./`/`../` specs against the
//! importing file's directory. Stage two resolves `TargetSpec::Symbol`
//! specs against the [`DeclMap`] — every known module path (from
//! `package`/`namespace` declarations plus the path-derived directory
//! chain of each source file) mapped to the files declared inside it.
//! Unique file → file edge; module-only or ambiguous → the caller creates
//! the `<module>/*` pseudo node (Kap. 5.1); unknown → external.

use std::collections::{BTreeMap, BTreeSet};

use crate::extract::Declaration;

/// What a symbolic import resolved to inside the known module universe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolTarget {
    /// Unique file matching the stem inside the declared module — the edge
    /// ends at this file node (root-relative, `/`-joined path).
    File(String),
    /// Module known, file ambiguous or no stem given — the caller creates
    /// the pseudo node `<module>/*` (Kap. 5.1) and edges to that.
    Module(String),
    /// Nothing found — the caller warns and drops the edge.
    External,
}

/// Remove the matching source-root prefix from a walked path.
///
/// `rel_path` is the [`crate::walk::SourceFile::rel_path`] (project-root
/// relative, `/`-joined) and `source_roots` are the configured
/// `[project]` source roots (Kap. 11) such as `["src"]`, so
/// `src/app/main.ts` becomes `app/main.ts`. The first root matching at a
/// path-component boundary wins; trailing slashes in a root entry are
/// ignored. Without a matching root the path is returned unchanged
/// (projects that already walk the source root itself).
pub fn strip_root(rel_path: &str, source_roots: &[String]) -> String {
    for root in source_roots {
        let prefix = root.trim_end_matches('/');
        if prefix.is_empty() {
            continue;
        }
        if let Some(stripped) = rel_path.strip_prefix(&format!("{prefix}/")) {
            return stripped.to_string();
        }
    }
    rel_path.to_string()
}

/// Resolve a `TargetSpec::Path` spec against the importing file.
///
/// A spec starting with `./` or `../` (or being exactly `.` / `..`) is
/// resolved against the directory of `source_rel_path`; a bare spec is
/// taken as already rooted and only normalized (Kap. 6.1: relative paths
/// against the file tree). The result is normalized — `.`/`..`/empty
/// components applied — and returned without a file extension on its last
/// component. `None` when a `..` component climbs beyond the root, or when
/// the target would be the root directory itself.
pub fn resolve_relative(source_rel_path: &str, spec: &str) -> Option<String> {
    let mut stack: Vec<&str> = match spec.split('/').next() {
        Some(".") | Some("..") => source_rel_path
            .rsplit_once('/')
            .map(|(dir, _)| {
                dir.split('/')
                    .filter(|c| !c.is_empty() && *c != "." && *c != "..")
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    for component in spec.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                stack.pop()?;
            }
            other => stack.push(other),
        }
    }
    if stack.is_empty() {
        return None;
    }
    let last = stack.len() - 1;
    stack[last] = file_stem(stack[last]);
    Some(stack.join("/"))
}

/// Name of the last path component without its file extension. Only a
/// non-empty stem is stripped, so a dot-file like `.hidden` keeps its name.
fn file_stem(path: &str) -> &str {
    let base = path.rsplit('/').next().unwrap_or(path);
    match base.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => base,
    }
}

/// Module path → known files of that module, plus the set of all known
/// module paths.
///
/// Built from every [`Declaration`] of all scanned files **plus** one
/// path-derived entry per source file: the file is registered under its
/// parent directory (so it can be matched by stem) and every ancestor
/// directory becomes a known module. This is what makes physical
/// hierarchies (TypeScript/JS, Python, Rust — Kap. 6.1) resolvable without
/// any declaration, while logical hierarchies (Java, C#, PHP) enter
/// through their `package`/`namespace` declarations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclMap {
    modules: BTreeSet<String>,
    files: BTreeMap<String, BTreeSet<String>>,
}

impl DeclMap {
    /// Build the map from all scanned files' root-relative paths and every
    /// declaration those files produced.
    pub fn new(declarations: &[Declaration], source_rel_paths: &[String]) -> DeclMap {
        let mut map = DeclMap::default();
        for path in source_rel_paths
            .iter()
            .map(String::as_str)
            .chain(declarations.iter().map(|d| d.file.as_str()))
        {
            map.add_file_path(path);
        }
        for declaration in declarations {
            map.modules.insert(declaration.module_path.clone());
            map.files
                .entry(declaration.module_path.clone())
                .or_default()
                .insert(declaration.file.clone());
        }
        map
    }

    /// Register `path` under its parent directory (files at the tree root
    /// belong to module `""`) and mark every ancestor directory known.
    fn add_file_path(&mut self, path: &str) {
        let dir = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        self.modules.insert(dir.to_string());
        let mut ancestor = String::new();
        for component in dir.split('/') {
            if !ancestor.is_empty() {
                ancestor.push('/');
            }
            ancestor.push_str(component);
            self.modules.insert(ancestor.clone());
        }
        self.files
            .entry(dir.to_string())
            .or_default()
            .insert(path.to_string());
    }

    /// Resolve a symbolic import (`TargetSpec::Symbol { path, stem }`).
    ///
    /// Unknown module → [`SymbolTarget::External`]. Known module without a
    /// stem → [`SymbolTarget::Module`] (the caller builds the `<module>/*`
    /// pseudo node). With a stem, the module's files are matched by file
    /// stem: exactly one match → [`SymbolTarget::File`]; no match or more
    /// than one → [`SymbolTarget::Module`] (Kap. 6.1: unique file if
    /// possible, otherwise the pseudo node — never external).
    pub fn resolve_symbol(&self, module_path: &str, stem: Option<&str>) -> SymbolTarget {
        if !self.modules.contains(module_path) {
            return SymbolTarget::External;
        }
        let Some(stem) = stem else {
            return SymbolTarget::Module(module_path.to_string());
        };
        let mut matching = self
            .files
            .get(module_path)
            .into_iter()
            .flatten()
            .filter(|file| file_stem(file) == stem);
        match (matching.next(), matching.next()) {
            (Some(file), None) => SymbolTarget::File(file.clone()),
            _ => SymbolTarget::Module(module_path.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Own the `&str` lists the table-driven tests are written with.
    fn owned(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    fn decl(module_path: &str, file: &str) -> Declaration {
        Declaration {
            module_path: module_path.to_string(),
            file: file.to_string(),
        }
    }

    #[test]
    fn strip_root_removes_the_matching_source_root_prefix() {
        for (rel_path, roots, expected) in [
            ("src/app/main.ts", &["src"][..], "app/main.ts"),
            ("src/app/main.ts", &["src/"][..], "app/main.ts"),
            ("packages/api/src/x.ts", &["packages/api"][..], "src/x.ts"),
            ("src/app/main.ts", &["packages/api"][..], "src/app/main.ts"),
            ("srcx/app.ts", &["src"][..], "srcx/app.ts"),
            ("src", &["src"][..], "src"),
            ("src/a.rs", &["docs", "src"][..], "a.rs"),
            ("app.ts", &[""][..], "app.ts"),
        ] {
            assert_eq!(
                strip_root(rel_path, &owned(roots)),
                expected,
                "rel_path {rel_path}, roots {roots:?}"
            );
        }
    }

    #[test]
    fn resolve_relative_resolves_nesting_against_the_source_directory() {
        for (source, spec, expected) in [
            ("app/billing/invoice.ts", "./util", Some("app/billing/util")),
            ("app/billing/invoice.ts", "../api/x", Some("app/api/x")),
            ("app/billing/invoice.ts", "./a/../b", Some("app/billing/b")),
            (
                "app/billing/invoice.ts",
                "./nested/./deep",
                Some("app/billing/nested/deep"),
            ),
            (
                "app/billing/invoice.ts",
                "./util.ts",
                Some("app/billing/util"),
            ),
            (
                "app/billing/invoice.ts",
                "../../domain/money",
                Some("domain/money"),
            ),
            ("a/b/c.ts", "./../d", Some("a/d")),
            ("app/main.js", "./deep/inner.d.ts", Some("app/deep/inner.d")),
            // A bare spec is taken as already rooted (Kap. 6.1).
            ("app/main.js", "shared/util", Some("shared/util")),
            ("app/x.ts", "a/b/../../c", Some("c")),
        ] {
            assert_eq!(
                resolve_relative(source, spec).as_deref(),
                expected,
                "source {source}, spec {spec}"
            );
        }
    }

    #[test]
    fn resolve_relative_returns_none_for_parent_traversal_beyond_root() {
        for (source, spec) in [
            ("a/b/c.ts", "../../.."),
            ("a.ts", "../x"),
            ("a.ts", ".."),
            ("a/b.ts", ".."),
            ("a/b.ts", "./.."),
            ("a/b.ts", "x/../../.."),
        ] {
            assert_eq!(
                resolve_relative(source, spec),
                None,
                "source {source}, spec {spec}"
            );
        }
    }

    #[test]
    fn resolve_symbol_finds_the_unique_java_class_in_its_declared_package() {
        let map = DeclMap::new(
            &[decl(
                "com/example/svc",
                "src/main/java/com/example/svc/Order.java",
            )],
            &owned(&["src/main/java/com/example/svc/Order.java"]),
        );
        assert_eq!(
            map.resolve_symbol("com/example/svc", Some("Order")),
            SymbolTarget::File("src/main/java/com/example/svc/Order.java".to_string())
        );
        // Unknown class inside a known package → pseudo node, not external
        // (Kap. 6.1 Java row: "sonst Pseudoknoten").
        assert_eq!(
            map.resolve_symbol("com/example/svc", Some("Missing")),
            SymbolTarget::Module("com/example/svc".to_string())
        );
        assert_eq!(
            map.resolve_symbol("com/example/other", Some("Order")),
            SymbolTarget::External
        );
    }

    #[test]
    fn resolve_symbol_namespace_only_import_yields_the_module() {
        // C#: namespace ≠ directory — the files live elsewhere and only the
        // declarations map them into the namespace (Kap. 6.1 C# row).
        let map = DeclMap::new(
            &[
                decl("Acme/Domain", "src/legacy/Order.cs"),
                decl("Acme/Domain", "src/more/Customer.cs"),
            ],
            &owned(&["src/legacy/Order.cs", "src/more/Customer.cs"]),
        );
        // `using Acme.Domain;` — namespace only, no stem → pseudo node.
        assert_eq!(
            map.resolve_symbol("Acme/Domain", None),
            SymbolTarget::Module("Acme/Domain".to_string())
        );
        // An explicit, unique type name resolves to its file even when the
        // file's directory and the namespace fall apart.
        assert_eq!(
            map.resolve_symbol("Acme/Domain", Some("Order")),
            SymbolTarget::File("src/legacy/Order.cs".to_string())
        );
        assert_eq!(
            map.resolve_symbol("Acme/Unknown", None),
            SymbolTarget::External
        );
    }

    #[test]
    fn resolve_symbol_distinguishes_python_module_file_from_package_init() {
        let map = DeclMap::new(
            &[],
            &owned(&["a/b.py", "pkg/util.py", "pkg/__init__.py", "standalone.py"]),
        );
        for (module, stem, expected) in [
            // `import a.b` against the plain module file a/b.py: found by
            // stem inside its parent module; the (non-existent) directory
            // a/b is not a known module.
            ("a", Some("b"), SymbolTarget::File("a/b.py".to_string())),
            ("a/b", None, SymbolTarget::External),
            ("a/b", Some("c"), SymbolTarget::External),
            // `import pkg` / `from pkg import x` against the package.
            ("pkg", None, SymbolTarget::Module("pkg".to_string())),
            (
                "pkg",
                Some("util"),
                SymbolTarget::File("pkg/util.py".to_string()),
            ),
            (
                "pkg",
                Some("__init__"),
                SymbolTarget::File("pkg/__init__.py".to_string()),
            ),
            ("pkg", Some("nope"), SymbolTarget::Module("pkg".to_string())),
            // Files at the tree root live in the known root module "".
            (
                "",
                Some("standalone"),
                SymbolTarget::File("standalone.py".to_string()),
            ),
            ("deep/unknown", None, SymbolTarget::External),
        ] {
            assert_eq!(
                map.resolve_symbol(module, stem),
                expected,
                "module {module:?}, stem {stem:?}"
            );
        }
    }

    #[test]
    fn resolve_symbol_ambiguous_stem_yields_the_module() {
        // Two files with stem `C` inside one declared module — possible
        // whenever the directory is not the module (C#, PHP).
        let map = DeclMap::new(
            &[decl("a/b", "a/b/C.java"), decl("a/b", "generated/C.cs")],
            &owned(&["a/b/C.java", "generated/C.cs"]),
        );
        assert_eq!(
            map.resolve_symbol("a/b", Some("C")),
            SymbolTarget::Module("a/b".to_string())
        );
    }
}
