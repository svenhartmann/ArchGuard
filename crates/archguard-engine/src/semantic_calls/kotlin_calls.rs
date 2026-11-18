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

const LIMITATION: &str = archguard_core::CallAnalyzer::KOTLIN_TOKENS;

#[tracing::instrument(skip_all)]
pub(crate) fn analyze_kotlin_calls(root: &Path, config: &Config) -> CallAnalysis {
    let scanned = RefCell::new(Vec::new());
    let mut report = super::token_calls::token_calls(
        root,
        config,
        (Lang::Kotlin, "kotlin", LIMITATION),
        |file, path, content| {
            // The graph names a Kotlin file after its package, as the pipeline does.
            let declarations = KotlinExtractor.extract(path, content).declarations;
            let node = crate::pipeline::file_node_name(file, path, &declarations);
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
    let declarations = if java.types.is_empty() {
        index::JavaDeclarations::default()
    } else {
        java_declarations(root, config)
    };
    let index = index::Index::new(&files, &report.methods, java, &declarations, complete);
    report.calls = resolve::resolve(&index);
    let coverage = coverage(&report.calls);
    let at = report.diagnostics.len() - 1;
    report.diagnostics.insert(at, coverage.clone());
    report.analyzers[0].diagnostics.insert(0, coverage);
    report
}

/// The Java classes and methods of the javac helper (cached, so the Java
/// backend runs once per analysis); empty when no JDK is available.
/// Local and anonymous classes cannot be named from Kotlin.
fn java_declarations(root: &Path, config: &Config) -> index::JavaDeclarations {
    let report = crate::java_calls::analyze_java_calls(root, config);
    let ids: std::collections::HashSet<&str> =
        report.classes.iter().map(|c| c.id.as_str()).collect();
    let classes = report
        .classes
        .iter()
        .filter(|c| !c.id.contains(['$', '@']))
        .map(|c| {
            let qualified = c.id.rsplit("::").next().unwrap_or(&c.id);
            let bodiless = report
                .methods
                .iter()
                .filter(|m| m.class_id == c.id && m.complexity.is_none())
                .map(|m| m.name.clone())
                .collect();
            archguard_extract::kotlin::KotlinClassFacts {
                id: c.id.clone(),
                name: qualified.rsplit('.').next().unwrap_or(qualified).into(),
                // A record is constructed like a class.
                kind: if c.kind == "record" { "class" } else { &c.kind }.into(),
                outer: c
                    .id
                    .rsplit_once('.')
                    .map(|(outer, _)| outer)
                    .filter(|outer| ids.contains(outer))
                    .map(Into::into),
                companion: false,
                supertypes: c
                    .supertypes
                    .iter()
                    .map(|s| s.rsplit("::").next().unwrap_or(s).into())
                    .collect(),
                modifiers: if c.is_abstract == Some(true) {
                    vec!["abstract".into()]
                } else {
                    Vec::new()
                },
                bodiless,
                values: Vec::new(),
            }
        })
        .collect();
    index::JavaDeclarations {
        classes,
        methods: report.methods,
    }
}

/// Java types of the project, `package.FileStem`, and their packages:
/// never external. `None` when a Java file cannot be read.
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
        "Calls: {} sites, {} project, {} external, {} unresolved (variables, properties, lambdas, extension functions, overloads, Java classes without javac declarations or unknown supertypes).",
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
