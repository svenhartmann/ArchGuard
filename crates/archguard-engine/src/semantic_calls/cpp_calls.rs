//! Token-based C/C++ measurements from `archguard_extract::cpp` (ADR 0017).
//! No compiler or preprocessor runs. Calls and types stay unresolved, so
//! the report is `partial`. Out-of-class definitions `A::f` are bound here
//! to the one project class named `A`.
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
    // Indexed once: by qualified name, and by last segment for the tail match.
    let mut exact: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut by_last: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
    for &(id, qualified) in classes {
        exact.entry(qualified).or_default().push(id);
        by_last
            .entry(last_segment(qualified))
            .or_default()
            .push((id, qualified));
    }
    let unique = |found: Vec<&str>| (found.len() == 1).then(|| found[0].to_string());
    let mut out = BTreeMap::new();
    for method in methods {
        let Some(qualified) = method
            .class_id
            .strip_prefix(method.file.as_str())
            .and_then(|rest| rest.strip_prefix("::"))
            .filter(|q| *q != "(module)" && !ids.contains(method.class_id.as_str()))
        else {
            continue;
        };
        // `q` ends with `::{qualified}`.
        let tail = |q: &str| q.strip_suffix(qualified).is_some_and(|r| r.ends_with("::"));
        let tails = by_last.get(last_segment(qualified)).into_iter().flatten();
        let owner = unique(exact.get(qualified).cloned().unwrap_or_default())
            .or_else(|| unique(tails.filter(|(_, q)| tail(q)).map(|(id, _)| *id).collect()))
            .unwrap_or_else(|| format!("{}::(module)", method.file));
        out.insert(method.id.clone(), owner);
    }
    out
}

fn last_segment(qualified: &str) -> &str {
    qualified.rsplit("::").next().unwrap_or(qualified)
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

    fn definition(file: &str, class: &str, name: &str) -> CallMethod {
        CallMethod {
            id: format!("{file}::{class}::{name}"),
            class_id: format!("{file}::{class}"),
            name: name.into(),
            signature: String::new(),
            file: file.into(),
            line: 1,
            end_line: None,
            is_static: false,
            complexity: None,
        }
    }

    #[test]
    fn owners_prefer_one_exact_name_then_one_tail_else_the_module() {
        let classes = [
            ("a.h::B", "B"),
            ("a.h::x::B", "x::B"),
            ("a.h::x::C", "x::C"),
            ("b.h::y::C", "y::C"),
            ("a.h::x::y::D", "x::y::D"),
            ("a.h::x::Bx", "x::Bx"),
        ];
        let methods = [
            definition("s.cpp", "B", "exact"),
            definition("s.cpp", "y::D", "tail"),
            definition("s.cpp", "C", "ambiguous"),
            definition("s.cpp", "x", "unknown"),
            definition("s.cpp", "(module)", "free"),
            definition("a.h", "x::C", "declared"),
        ];
        let owners = owners(&classes, &methods);
        let owner = |id: &str| owners.get(id).map(String::as_str);
        assert_eq!(owner("s.cpp::B::exact"), Some("a.h::B"));
        assert_eq!(owner("s.cpp::y::D::tail"), Some("a.h::x::y::D"));
        assert_eq!(owner("s.cpp::C::ambiguous"), Some("s.cpp::(module)"));
        assert_eq!(owner("s.cpp::x::unknown"), Some("s.cpp::(module)"));
        assert_eq!(owner("s.cpp::(module)::free"), None);
        assert_eq!(owner("a.h::x::C::declared"), None);
        assert_eq!(owners.len(), 4);
    }

    #[test]
    fn binding_many_definitions_is_not_quadratic() {
        let ids: Vec<(String, String)> = (0..6_000)
            .map(|n| (format!("a.h::ns::K{n}"), format!("ns::K{n}")))
            .collect();
        let classes: Vec<(&str, &str)> =
            ids.iter().map(|(i, q)| (i.as_str(), q.as_str())).collect();
        let methods: Vec<CallMethod> = (0..60_000)
            .map(|n| definition("s.cpp", &format!("K{}", n % 6_000), &format!("f{n}")))
            .collect();
        let start = std::time::Instant::now();
        let owners = owners(&classes, &methods);
        assert_eq!(owners["s.cpp::K7::f7"], "a.h::ns::K7");
        // Quadratic scanning took about 17 s here in a debug build.
        assert!(
            start.elapsed() < std::time::Duration::from_secs(3),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn projects_without_c_or_cpp_are_not_applicable() {
        let dir = project(&[("a.py", "x = 1\n")]);
        let report = analyze_cpp_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "not_applicable");
    }
}
