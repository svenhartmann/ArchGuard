//! Token-based Swift measurements and the few call targets that syntax
//! alone can name (ADR 0018). No Swift compiler or `swift build` is needed.
//! A call is resolved only as `Type(…)`, `Type.init(…)`, `Type.Nested(…)` or
//! `Type.staticMethod(…)` of a unique top-level project type that the
//! calling target can see (its own target or an imported one); everything
//! else stays unresolved, and nothing is claimed external. Types stay
//! unresolved, so the report is `partial`.
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archguard_core::{CallAnalysis, CallClass, CallMethod, Config, MethodCall};
use archguard_extract::swift::{
    SwiftCallSite, SwiftTarget, attach_extensions, scan_swift_structure,
};
use archguard_extract::{Extractor, Lang, SwiftExtractor, TargetSpec};

const LIMITATION: &str = archguard_core::CallAnalyzer::SWIFT_TOKENS;

/// What one scanned file contributes to resolution.
struct File {
    /// The target (Swift module) the file belongs to; `None` outside any
    /// `Package.swift` target.
    module: Option<String>,
    imports: BTreeSet<String>,
    sites: Vec<SwiftCallSite>,
}

#[tracing::instrument(skip_all)]
pub(crate) fn analyze_swift_calls(root: &Path, config: &Config) -> CallAnalysis {
    let walked = crate::pipeline::walk_files(root, config).unwrap_or_default();
    let targets = crate::swift_sources::targets(root, &walked);
    let scanned = RefCell::new(BTreeMap::new());
    let mut report = super::token_calls::token_calls(
        root,
        config,
        (Lang::Swift, "swift", LIMITATION),
        |file, path, content| {
            let target = crate::swift_sources::target_of(&targets, &file.rel_path);
            let package = match target {
                // The target name, then the directory inside it.
                Some((name, dir)) => {
                    let inner = file.rel_path[dir.len() + 1..].rsplit_once('/');
                    inner.map_or(name.clone(), |(sub, _)| format!("{name}/{sub}"))
                }
                None => path.rsplit_once('/').map_or("", |(dir, _)| dir).to_string(),
            };
            let node = crate::pipeline::file_node_name(file, path, &[]);
            let found = scan_swift_structure(path, &node, &package, content);
            let imports = SwiftExtractor
                .extract(path, content)
                .imports
                .into_iter()
                .filter_map(|import| match import.target {
                    TargetSpec::Symbol { path, .. } => Some(path),
                    _ => None,
                })
                .collect();
            scanned.borrow_mut().insert(
                path.to_string(),
                File {
                    module: target.map(|(name, _)| name.clone()),
                    imports,
                    sites: found.sites,
                },
            );
            (
                found.classes,
                found.methods,
                found.metrics,
                found.diagnostics,
            )
        },
    );
    if report.files == 0 {
        return report;
    }
    let files = scanned.into_inner();
    attach_extensions(&mut report.classes, &mut report.methods);
    report.calls = resolve(&files, &report.classes, &report.methods);
    let outside = files
        .iter()
        .filter(|(path, f)| f.module.is_none() && !path.ends_with("Package.swift"))
        .count();
    let mut notes = vec![coverage(&report.calls)];
    if outside > 0 {
        notes.push(format!(
            "{outside} Swift files belong to no literal Package.swift target (Xcode projects are not read); they form one module, and their imports of project targets stay unresolved."
        ));
    }
    let at = report.diagnostics.len() - 1;
    for (offset, note) in notes.into_iter().enumerate() {
        report.diagnostics.insert(at + offset, note.clone());
        report.analyzers[0].diagnostics.insert(offset, note);
    }
    report
}

/// (resolution, dispatch, to_class, to_method) of one call.
type Target = (&'static str, &'static str, Option<String>, Option<String>);
const UNRESOLVED: Target = ("unresolved", "unknown", None, None);

/// The call edges of all scanned files.
fn resolve(
    files: &BTreeMap<String, File>,
    classes: &[CallClass],
    methods: &[CallMethod],
) -> Vec<MethodCall> {
    let owner: BTreeMap<&str, &str> = methods
        .iter()
        .map(|m| (m.id.as_str(), m.class_id.as_str()))
        .collect();
    let mut calls = Vec::new();
    for (path, file) in files {
        for site in &file.sites {
            let (resolution, dispatch, to_class, to_method) =
                target(files, file, classes, methods, &site.target);
            calls.push(MethodCall {
                // An extension's members belong to the type it extends.
                from_class: owner
                    .get(site.from_method.as_str())
                    .map_or(site.from_class.clone(), |id| id.to_string()),
                from_method: Some(site.from_method.clone()),
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

fn target(
    files: &BTreeMap<String, File>,
    caller: &File,
    classes: &[CallClass],
    methods: &[CallMethod],
    target: &SwiftTarget,
) -> Target {
    // The one type at `path` (`Outer.Inner` for nested types) the caller sees.
    let unique = |path: &str| {
        let mut found = classes.iter().filter(|class| {
            class.id.split_once("::").map(|(_, p)| p) == Some(path)
                && class.kind != "extension"
                && files.get(&class.file).is_some_and(|file| {
                    file.module == caller.module
                        || file
                            .module
                            .as_ref()
                            .is_some_and(|m| caller.imports.contains(m))
                })
        });
        match (found.next(), found.next()) {
            (Some(class), None) => Some(class),
            _ => None,
        }
    };
    let construct = |class: &CallClass| match class.kind.as_str() {
        "class" | "struct" | "actor" => ("project", "constructor", Some(class.id.clone()), None),
        _ => UNRESOLVED,
    };
    match target {
        SwiftTarget::Init(name) => unique(name).map_or(UNRESOLVED, construct),
        SwiftTarget::Static { owner, name } => {
            let Some(class) = unique(owner) else {
                return UNRESOLVED;
            };
            if let Some(nested) = unique(&format!("{owner}.{name}")) {
                return construct(nested);
            }
            let mut found = methods
                .iter()
                .filter(|m| m.class_id == class.id && &m.name == name && m.is_static);
            match (found.next(), found.next()) {
                (Some(method), None) => (
                    "project",
                    "static",
                    Some(class.id.clone()),
                    Some(method.id.clone()),
                ),
                _ => UNRESOLVED,
            }
        }
        SwiftTarget::Unresolved => UNRESOLVED,
    }
}

/// How many call sites the static resolver could attribute.
fn coverage(calls: &[MethodCall]) -> String {
    let count = |kind: &str| calls.iter().filter(|c| c.resolution == kind).count();
    format!(
        "Calls: {} sites, {} project, {} unresolved (values, self, bare functions, overloads or types the target cannot see).",
        calls.len(),
        count("project"),
        count("unresolved")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, source) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source).unwrap();
        }
        dir
    }

    #[test]
    fn swift_types_are_measured_and_safe_calls_resolved() {
        let dir = project(&[
            (
                "Package.swift",
                "import PackageDescription\nlet package = Package(name: \"Shop\", targets: [\n    .target(name: \"Core\"),\n    .executableTarget(name: \"App\", dependencies: [\"Core\"]),\n])\n",
            ),
            (
                "Sources/Core/Price.swift",
                "public struct Price {\n    public static func zero() -> Price { Price(cents: 0) }\n    public struct Tax {}\n}\npublic protocol Pricing { func price() -> Price }\n",
            ),
            (
                "Sources/Core/Price+Format.swift",
                "extension Price: CustomStringConvertible {\n    public var description: String { \"\" }\n    func format(_ a: Bool) -> String { a ? \"x\" : \"y\" }\n}\n",
            ),
            (
                "Sources/App/main.swift",
                "import Core\n\nfunc run() {\n    let p = Price.zero()\n    let t = Price.Tax()\n    _ = Price(cents: 1)\n    _ = Pricing()\n    print(p, t)\n}\n",
            ),
            (
                "Sources/App/Local.swift",
                "struct Helper { func go() { _ = Missing() } }\n",
            ),
            ("Scripts/broken.swift", "func broken() { if (\n"),
        ]);
        let report = analyze_swift_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "partial");
        assert_eq!(report.files, 6);
        let analyzer = &report.analyzers[0];
        assert_eq!(
            (
                analyzer.language.as_str(),
                analyzer.backend.as_str(),
                analyzer.status.as_str()
            ),
            ("swift", "archguard tokens", "partial")
        );
        let class = |id: &str| report.classes.iter().find(|c| c.id == id).unwrap();
        let price = class("Sources/Core/Price.swift::Price");
        assert_eq!(price.package, "Core");
        assert_eq!(price.graph_node, "Sources/Core/Price.swift");
        assert_eq!(price.supertypes, ["CustomStringConvertible"]);
        assert_eq!(
            class("Sources/Core/Price.swift::Pricing").is_abstract,
            Some(true)
        );
        // The extension's function belongs to the extended type.
        let format = report.methods.iter().find(|m| m.name == "format").unwrap();
        assert_eq!(format.class_id, price.id);
        assert!(!report.classes.iter().any(|c| c.kind == "extension"));
        let calls: Vec<_> = report
            .calls
            .iter()
            .map(|c| {
                (
                    c.target_label.as_str(),
                    c.resolution.as_str(),
                    c.dispatch.as_str(),
                    c.to_class.as_deref(),
                )
            })
            .collect();
        let price_id = Some(price.id.as_str());
        assert_eq!(
            calls,
            [
                ("Missing", "unresolved", "unknown", None),
                ("Price.zero", "project", "static", price_id),
                (
                    "Price.Tax",
                    "project",
                    "constructor",
                    Some("Sources/Core/Price.swift::Price.Tax")
                ),
                ("Price", "project", "constructor", price_id),
                // A protocol cannot be constructed.
                ("Pricing", "unresolved", "unknown", None),
                ("print", "unresolved", "unknown", None),
                ("Price", "project", "constructor", price_id),
            ]
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.contains("broken.swift")),
            "{:?}",
            report.diagnostics
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.contains("1 Swift files belong to no")),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn a_type_of_a_target_that_is_not_imported_stays_unresolved() {
        let dir = project(&[
            (
                "Package.swift",
                ".target(name: \"Core\")\n.target(name: \"Other\")\n",
            ),
            ("Sources/Core/Price.swift", "struct Price {}\n"),
            ("Sources/Other/Use.swift", "func use() { _ = Price() }\n"),
        ]);
        let report = analyze_swift_calls(dir.path(), &Config::default());
        let call = &report.calls[0];
        assert_eq!(
            (call.target_label.as_str(), call.resolution.as_str()),
            ("Price", "unresolved")
        );
    }

    #[test]
    fn projects_without_swift_are_not_applicable() {
        let dir = project(&[("a.py", "x = 1\n")]);
        let report = analyze_swift_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "not_applicable");
        assert!(report.analyzers.is_empty());
    }
}
