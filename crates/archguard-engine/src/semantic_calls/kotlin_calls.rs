//! Token-based Kotlin measurements and static call targets from
//! `archguard_extract::kotlin`. No Kotlin compiler or Gradle run is needed.
//! The scanner records call sites with names as written; this module binds
//! them across the project ([`resolve`]). Types stay unresolved, so the
//! report is `partial`.
use std::cell::RefCell;
use std::path::Path;

use archguard_core::{CallAnalysis, Config, MethodCall};
use archguard_extract::kotlin::scan_kotlin_structure;
use archguard_extract::{Extractor, JavaExtractor, KotlinExtractor, Lang};

mod hierarchy;
mod index;
mod names;
mod resolve;

const LIMITATION: &str = "Token-based analysis: classes, objects, functions, complexity and Halstead counts. Call targets are resolved statically from the package, imports (aliases; star imports with a single candidate), Kotlin classes, objects, companion objects, top-level functions, this, super and the project class hierarchy; calls on variables, properties and call results, extension functions, every call inside lambdas, local and extension functions, and Java classes of the project stay unresolved; overloads are not told apart. Types are not resolved; abstractness is read from interface, abstract class and sealed declarations. Property accessors, constructors, init blocks and property initializers are not measured, and calls in them are not counted.";

pub(crate) fn analyze_kotlin_calls(root: &Path, config: &Config) -> CallAnalysis {
    let scanned = RefCell::new(Vec::new());
    let mut report = super::token_calls::token_calls(
        root,
        config,
        (Lang::Kotlin, "kotlin", LIMITATION),
        |path, content| {
            // The graph names a Kotlin file after its package, as the pipeline does.
            let declarations = KotlinExtractor.extract(path, content).declarations;
            let node = crate::pipeline::file_node_name(path, &declarations);
            let found = scan_kotlin_structure(path, &node, content);
            scanned.borrow_mut().push((path.to_string(), found.symbols));
            (
                found.classes,
                found.methods,
                found.metrics,
                found.diagnostics,
            )
        },
    );
    let files = scanned.into_inner();
    if report.files == 0 {
        return report;
    }
    let java = java_types(root, config);
    // Only a fully scanned project can tell that a name is declared nowhere.
    let complete =
        java.is_some() && files.len() == report.files && files.iter().all(|(_, f)| f.measured);
    let java = java.unwrap_or_default();
    let index = index::Index::new(&files, &report.methods, java, complete);
    report.calls = resolve::resolve(&index);
    let coverage = coverage(&report.calls);
    let at = report.diagnostics.len() - 1;
    report.diagnostics.insert(at, coverage.clone());
    report.analyzers[0].diagnostics.insert(0, coverage);
    report
}

/// Java types of the project, `package.FileStem`, and their packages.
/// Kotlin calls to them stay unresolved, but they are never external.
/// `None` when a Java file cannot be read.
fn java_types(root: &Path, config: &Config) -> Option<index::JavaTypes> {
    const SOURCE_LIMIT: u64 = 8 * 1024 * 1024;
    let mut java = index::JavaTypes::default();
    let files = crate::pipeline::walk_files(root, config).ok()?;
    for file in files.iter().filter(|file| file.lang == Lang::Java) {
        let bytes =
            crate::project_files::read_repo_file(root, Path::new(&file.rel_path), SOURCE_LIMIT)
                .ok()?;
        let content = String::from_utf8(bytes).ok()?;
        let package = JavaExtractor
            .extract(&file.rel_path, &content)
            .declarations
            .first()
            .map(|d| d.module_path.replace('/', "."))
            .unwrap_or_default();
        let stem = Path::new(&file.rel_path).file_stem()?.to_string_lossy();
        java.types.insert(index::qualify(&package, &stem));
        java.names.insert(stem.into_owned());
        java.packages.insert(package);
    }
    Some(java)
}

/// How many call sites the static resolver could attribute.
fn coverage(calls: &[MethodCall]) -> String {
    let count = |kind: &str| calls.iter().filter(|c| c.resolution == kind).count();
    format!(
        "Calls: {} sites, {} project, {} external, {} unresolved (variables, properties, lambdas, extension functions, Java classes or unknown supertypes).",
        calls.len(),
        count("project"),
        count("external"),
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
    fn kotlin_classes_are_measured_and_broken_files_reported() {
        let dir = project(&[
            (
                "src/com/shop/User.kt",
                "package com.shop\n\nsealed class User {\n    fun name(a: Boolean) = if (a) \"x\" else \"y\"\n}\n",
            ),
            ("src/broken.kt", "fun broken() { if (\n"),
        ]);
        let report = analyze_kotlin_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "partial");
        assert_eq!(report.files, 2);
        let analyzer = &report.analyzers[0];
        assert_eq!(
            (
                analyzer.language.as_str(),
                analyzer.backend.as_str(),
                analyzer.status.as_str(),
                analyzer.files
            ),
            ("kotlin", "archguard tokens", "partial", 2)
        );
        let user = report.classes.iter().find(|c| c.name == "User").unwrap();
        assert_eq!(user.is_abstract, Some(true));
        assert_eq!(user.graph_node, "com/shop/User.kt");
        let name = report.methods.iter().find(|m| m.name == "name").unwrap();
        assert_eq!(name.complexity.as_ref().unwrap().cyclomatic, 2);
        let details = report.details.as_ref().unwrap();
        assert_eq!(details.enumeration_complete, Some(false));
        assert_eq!(details.method_metrics.len(), 1);
        assert!(
            report.diagnostics.iter().any(|d| d.contains("broken.kt")),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn projects_without_kotlin_are_not_applicable() {
        let dir = project(&[("a.py", "x = 1\n")]);
        let report = analyze_kotlin_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "not_applicable");
        assert!(report.analyzers.is_empty());
    }
}
