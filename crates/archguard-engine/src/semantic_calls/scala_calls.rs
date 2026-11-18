//! Token-based Scala measurements and static call targets from
//! `archguard_extract::scala` (ADR 0020). No Scala compiler or sbt run is
//! needed. The scanner records call sites with names as written; this
//! module binds them across the project ([`resolve`]). Types stay
//! unresolved, so the report is `partial`.
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;

use archguard_core::{CallAnalysis, Config, MethodCall};
use archguard_extract::scala::scan_scala_structure;
use archguard_extract::{Extractor, Lang, ScalaExtractor, extractor_for};

mod resolve;

const LIMITATION: &str = archguard_core::CallAnalyzer::SCALA_TOKENS;

#[tracing::instrument(skip_all)]
pub(crate) fn analyze_scala_calls(root: &Path, config: &Config) -> CallAnalysis {
    let scanned = RefCell::new(Vec::new());
    let mut report = super::token_calls::token_calls(
        root,
        config,
        (Lang::Scala, "scala", LIMITATION),
        |file, path, content| {
            // The graph names a Scala file after its package, as the pipeline does.
            let declarations = ScalaExtractor.extract(path, content).declarations;
            let node = crate::pipeline::file_node_name(file, path, &declarations);
            let found = scan_scala_structure(path, &node, content);
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
    // Only a fully scanned project can tell that a package is not its own.
    let complete = files.len() == report.files && files.iter().all(|(_, f)| f.measured);
    let jvm = jvm_packages(root, config).filter(|_| complete);
    report.calls = resolve::resolve(&files, &report.methods, jvm.as_ref());
    let coverage = coverage(&report.calls);
    let at = report.diagnostics.len() - 1;
    report.diagnostics.insert(at, coverage.clone());
    report.analyzers[0].diagnostics.insert(0, coverage);
    report
}

/// Packages of the project's Java and Kotlin files: Scala calls into them
/// stay unresolved, but they are never external. `None` when a file cannot
/// be read, so that no call is called external.
fn jvm_packages(root: &Path, config: &Config) -> Option<HashSet<String>> {
    const SOURCE_LIMIT: u64 = 8 * 1024 * 1024;
    let mut packages = HashSet::new();
    let files = crate::pipeline::walk_files(root, config).ok()?;
    for file in files
        .iter()
        .filter(|file| matches!(file.lang, Lang::Java | Lang::Kotlin))
    {
        let bytes =
            crate::project_files::read_repo_file(root, Path::new(&file.rel_path), SOURCE_LIMIT)
                .ok()?;
        let content = String::from_utf8(bytes).ok()?;
        let extractor = extractor_for(file.lang)?;
        for declaration in extractor.extract(&file.rel_path, &content).declarations {
            packages.insert(declaration.module_path.replace('/', "."));
        }
    }
    Some(packages)
}

/// How many call sites the static resolver could attribute.
fn coverage(calls: &[MethodCall]) -> String {
    let count = |kind: &str| calls.iter().filter(|c| c.resolution == kind).count();
    format!(
        "Calls: {} sites, {} project, {} external, {} unresolved (values, implicits, givens, extension methods, overloads, Java or Kotlin classes or unknown supertypes).",
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
    fn scala_classes_are_measured_and_broken_files_reported() {
        let dir = project(&[
            (
                "src/com/shop/User.scala",
                "package com.shop\n\nabstract class User:\n  def name(a: Boolean) = if a then \"x\" else \"y\"\n",
            ),
            ("src/broken.scala", "def broken() = { if (\n"),
        ]);
        let report = analyze_scala_calls(dir.path(), &Config::default());
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
            ("scala", "archguard tokens", "partial", 2)
        );
        let user = report.classes.iter().find(|c| c.name == "User").unwrap();
        assert_eq!(user.is_abstract, Some(true));
        assert_eq!(user.graph_node, "com/shop/User.scala");
        let name = report.methods.iter().find(|m| m.name == "name").unwrap();
        assert_eq!(name.complexity.as_ref().unwrap().cyclomatic, 2);
        let details = report.details.as_ref().unwrap();
        assert_eq!(details.enumeration_complete, Some(false));
        assert_eq!(details.method_metrics.len(), 1);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.contains("broken.scala")),
            "{:?}",
            report.diagnostics
        );
    }

    fn resolution<'a>(report: &'a CallAnalysis, code: &str) -> &'a str {
        let call = report.calls.iter().find(|c| c.code.contains(code));
        call.unwrap_or_else(|| panic!("{code}: {:?}", report.calls))
            .resolution
            .as_str()
    }

    #[test]
    fn a_new_of_an_unknown_single_name_stays_unresolved() {
        let dir = project(&[(
            "src/A.scala",
            "class A { def run(): Int = { throw new Exception(\"x\") } }\n",
        )]);
        let report = analyze_scala_calls(dir.path(), &Config::default());
        assert_eq!(resolution(&report, "new Exception"), "unresolved");
    }

    #[test]
    fn an_unscanned_file_turns_external_into_unresolved() {
        let dir = project(&[
            (
                "src/shop/order/Order.scala",
                "package shop.order\n\nimport shop.pay.Payment\n\nclass Order {\n  def a() = Payment.make(2)\n  def b() = Payment(3)\n}\n",
            ),
            (
                "src/shop/pay/Payment.scala",
                "package shop.pay\n\nobject Payment {\n  def make(n: Int) = { if (\n}\n",
            ),
        ]);
        let report = analyze_scala_calls(dir.path(), &Config::default());
        assert_eq!(resolution(&report, "Payment.make(2)"), "unresolved");
        assert_eq!(resolution(&report, "Payment(3)"), "unresolved");
    }

    #[test]
    fn projects_without_scala_are_not_applicable() {
        let dir = project(&[("a.py", "x = 1\n")]);
        let report = analyze_scala_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "not_applicable");
        assert!(report.analyzers.is_empty());
    }
}
