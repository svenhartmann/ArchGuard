//! Token-based PHP measurements from `archguard_extract::scan_php_structure`.
//! Nothing is executed and no PHP installation is needed. Calls and types stay
//! unresolved, so the report is `partial`.
use std::path::Path;

use archguard_core::{CallAnalysis, Config};
use archguard_extract::{Extractor, Lang, PhpExtractor, scan_php_structure};

const LIMITATION: &str = "Token-based analysis: classes, functions, complexity and Halstead counts. Calls and types are not resolved; abstractness is read from interface and abstract class declarations.";

pub(crate) fn analyze_php_calls(root: &Path, config: &Config) -> CallAnalysis {
    crate::semantic_calls::token_calls::token_calls(
        root,
        config,
        (Lang::Php, "php", LIMITATION),
        |path, content| {
            // The graph names a PHP file after its namespace, as the pipeline does.
            let declarations = PhpExtractor.extract(path, content).declarations;
            let node = crate::pipeline::file_node_name(path, &declarations);
            let found = scan_php_structure(path, &node, content);
            (
                found.classes,
                found.methods,
                found.metrics,
                found.diagnostics,
            )
        },
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
    fn php_classes_reach_their_graph_nodes() {
        let dir = project(&[
            (
                "src/Domain/User.php",
                "<?php\nnamespace App\\Domain;\n\nabstract class User {\n    public function name($a) { return $a ? 'x' : 'y'; }\n}\n",
            ),
            ("src/broken.php", "<?php\nfunction broken() { if (\n"),
        ]);
        let report = analyze_php_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "partial");
        assert_eq!(report.files, 2);
        let analyzer = &report.analyzers[0];
        assert_eq!(
            (
                analyzer.language.as_str(),
                analyzer.status.as_str(),
                analyzer.files
            ),
            ("php", "partial", 2)
        );
        let user = report.classes.iter().find(|c| c.name == "User").unwrap();
        assert_eq!(user.is_abstract, Some(true));
        // tests/php.rs checks `graph_node` against the full pipeline graph.
        let name = report.methods.iter().find(|m| m.name == "name").unwrap();
        assert_eq!(name.complexity.as_ref().unwrap().cyclomatic, 2);
        assert!(
            report.diagnostics.iter().any(|d| d.contains("broken.php")),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn projects_without_php_are_not_applicable() {
        let dir = project(&[("a.py", "x = 1\n")]);
        let report = analyze_php_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "not_applicable");
        assert!(report.analyzers.is_empty());
    }
}
