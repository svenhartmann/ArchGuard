//! Token-based C/C++ measurements from `archguard_extract::cpp` (ADR 0017).
//! No compiler or preprocessor runs. Calls and types stay unresolved, so
//! the report is `partial`. Out-of-class definitions `A::f` are bound here
//! to the one project class named `A`.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archguard_core::{CallAnalysis, CallMethod, Config};
use archguard_extract::{Lang, scan_cpp_structure};

const LIMITATION: &str = archguard_core::CallAnalyzer::CPP_TOKENS;

#[tracing::instrument(skip_all)]
pub(crate) fn analyze_cpp_calls(root: &Path, config: &Config) -> CallAnalysis {
    let mut report = super::token_calls::token_calls(
        root,
        config,
        (Lang::Cpp, "cpp", LIMITATION),
        |file, path, content| {
            let node = crate::pipeline::file_node_name(file, path, &[]);
            let found = scan_cpp_structure(path, &node, content);
            (
                found.classes,
                found.methods,
                found.metrics,
                found.diagnostics,
            )
        },
    );
    let classes: Vec<(&str, &str)> = report
        .classes
        .iter()
        .filter(|class| class.kind != "module")
        .map(|class| (class.id.as_str(), &class.id[class.file.len() + 2..]))
        .collect();
    let owners = owners(&classes, &report.methods);
    for method in &mut report.methods {
        if let Some(owner) = owners.get(&method.id) {
            method.class_id = owner.clone();
        }
    }
    report
}

/// The class id of every method whose class its file does not declare:
/// the one project class with that qualified name, else the one whose
/// qualified name ends with it (`using namespace`), else the file's
/// `(module)`.
fn owners(classes: &[(&str, &str)], methods: &[CallMethod]) -> BTreeMap<String, String> {
    let ids: BTreeSet<&str> = classes.iter().map(|(id, _)| *id).collect();
    let unique = |found: Vec<&str>| (found.len() == 1).then(|| found[0].to_string());
    let mut out = BTreeMap::new();
    for method in methods {
        let Some(qualified) = method
            .class_id
            .strip_prefix(&format!("{}::", method.file))
            .filter(|q| *q != "(module)" && !ids.contains(method.class_id.as_str()))
        else {
            continue;
        };
        let suffix = format!("::{qualified}");
        let exact = classes.iter().filter(|(_, q)| *q == qualified);
        let tail = classes.iter().filter(|(_, q)| q.ends_with(&suffix));
        let owner = unique(exact.map(|(id, _)| *id).collect())
            .or_else(|| unique(tail.map(|(id, _)| *id).collect()))
            .unwrap_or_else(|| format!("{}::(module)", method.file));
        out.insert(method.id.clone(), owner);
    }
    out
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
    fn classes_are_measured_and_out_of_class_definitions_bound() {
        let dir = project(&[
            (
                "include/shop/shape.h",
                "#pragma once\nnamespace shop {\nclass Shape {\npublic:\n    virtual double area() const = 0;\n    int sides() const;\n};\nclass Square : public Shape {\n    double area() const override;\n};\n}\n",
            ),
            (
                "src/shape.cpp",
                "#include \"shop/shape.h\"\nusing namespace shop;\nint Shape::sides() const { return 0; }\ndouble Square::area() const { return 1.0; }\nint Unknown::f() { return 1; }\nint main(int argc, char** argv) { return argc > 1 ? 1 : 0; }\n",
            ),
            ("src/broken.c", "void broken() { if (\n"),
        ]);
        let report = analyze_cpp_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "partial");
        assert_eq!(report.files, 3);
        let analyzer = &report.analyzers[0];
        assert_eq!(
            (
                analyzer.language.as_str(),
                analyzer.backend.as_str(),
                analyzer.files
            ),
            ("cpp", "archguard tokens", 3)
        );
        let shape = report.classes.iter().find(|c| c.name == "Shape").unwrap();
        assert_eq!(shape.is_abstract, Some(true));
        assert_eq!(shape.id, "include/shop/shape.h::shop::Shape");
        assert_eq!(shape.package, "include/shop");
        let owner = |name: &str| {
            let method = report.methods.iter().find(|m| m.name == name).unwrap();
            method.class_id.as_str()
        };
        // `using namespace shop;` leaves `Shape::sides` unqualified: the one
        // class whose name ends in `::Shape` owns it.
        assert_eq!(owner("sides"), "include/shop/shape.h::shop::Shape");
        assert_eq!(owner("area"), "include/shop/shape.h::shop::Square");
        assert_eq!(owner("f"), "shape.cpp::(module)");
        assert_eq!(owner("main"), "shape.cpp::(module)");
        let main = report.methods.iter().find(|m| m.name == "main").unwrap();
        assert_eq!(main.complexity.as_ref().unwrap().cyclomatic, 2);
        let details = report.details.as_ref().unwrap();
        assert_eq!(details.enumeration_complete, Some(false));
        assert_eq!(details.method_metrics.len(), 4);
        assert!(
            report.diagnostics.iter().any(|d| d.contains("broken.c")),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn projects_without_c_or_cpp_are_not_applicable() {
        let dir = project(&[("a.py", "x = 1\n")]);
        let report = analyze_cpp_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "not_applicable");
    }
}
