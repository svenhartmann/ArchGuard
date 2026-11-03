//! Syntax-only Python measurements from the standard `ast` module of the local
//! Python 3. The bundled helper parses sources and never imports or executes
//! project code. Calls and types stay unresolved, so the report is `partial`.
//! Counting rules mirror `rust_calls.rs` (`ast-decisions-v1`,
//! `source-maintainability/v1`), mapped to Python in the helper's docstring.
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

use archguard_core::{
    CallAnalysis, CallAnalyzer, CallClass, CallMethod, Config, SemanticDetails, SourceMethodMetrics,
};
use archguard_extract::{Lang, strip_root};
use serde::Deserialize;

const HELPER: &str = include_str!("ArchguardCallScanner.py");
const BACKEND: &str = "python ast";
const HINT: &str = "Install Python 3.9+ or set ARCHGUARD_PYTHON to its executable.";
const SOURCE_LIMIT: u64 = 8 * 1024 * 1024;
const INPUT_LIMIT: usize = 64 * 1024 * 1024;
const LIMITATION: &str = "Syntax-only analysis: classes, functions, complexity and Halstead counts. Calls and types are not resolved; abstractness is read from ABC/Protocol bases and @abstractmethod.";

/// What the helper writes to stdout.
#[derive(Deserialize)]
struct HelperOutput {
    version: String,
    classes: Vec<CallClass>,
    methods: Vec<CallMethod>,
    metrics: Vec<SourceMethodMetrics>,
    diagnostics: Vec<String>,
}

pub(crate) fn analyze_python_calls(root: &Path, config: &Config) -> CallAnalysis {
    analyze_with(root, config, &python_executable())
}

/// `ARCHGUARD_PYTHON`, else `python3` (`python` on Windows) from `PATH`.
fn python_executable() -> OsString {
    std::env::var_os("ARCHGUARD_PYTHON")
        .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into())
}

fn analyze_with(root: &Path, config: &Config, python: &OsStr) -> CallAnalysis {
    let files = match crate::pipeline::walk_files(root, config) {
        Ok(files) => files,
        Err(error) => return unavailable(0, error.to_string()),
    };
    let mut input = Vec::new();
    let mut diagnostics = Vec::new();
    let mut bytes = 0;
    let mut count = 0;
    for file in files.iter().filter(|file| file.lang == Lang::Py) {
        count += 1;
        let path = strip_root(&file.rel_path, &config.project.source_roots);
        let text =
            crate::project_files::read_repo_file(root, Path::new(&file.rel_path), SOURCE_LIMIT)
                .map_err(|error| error.to_string())
                .and_then(|bytes| String::from_utf8(bytes).map_err(|error| error.to_string()));
        match text {
            Ok(content) => {
                bytes += content.len();
                input.push(serde_json::json!({"path": path, "content": content}));
            }
            Err(error) => diagnostics.push(format!("{path}: {error}")),
        }
    }
    if count == 0 {
        return CallAnalysis {
            java_release: 0,
            ..Default::default()
        };
    }
    if bytes > INPUT_LIMIT {
        return unavailable(count, "Python source budget exceeded".into());
    }
    let output = match run(python, &input) {
        Ok(output) => output,
        Err(error) => return unavailable(count, error),
    };
    diagnostics.extend(output.diagnostics);
    diagnostics.push(LIMITATION.into());
    CallAnalysis {
        details: Some(SemanticDetails {
            enumeration_complete: Some(false),
            method_metrics: output.metrics,
            ..Default::default()
        }),
        status: "partial".into(),
        java_release: 0,
        analyzers: vec![CallAnalyzer {
            language: "python".into(),
            backend: BACKEND.into(),
            status: "partial".into(),
            files: count,
            version: Some(output.version),
            project: None,
            diagnostics: diagnostics.clone(),
        }],
        files: count,
        classes: output.classes,
        methods: output.methods,
        calls: Vec::new(),
        diagnostics,
    }
}

/// Run the helper isolated (`-I -S`: no user site, no environment, no script
/// directory on `sys.path`), so nothing from the project can be imported.
fn run(python: &OsStr, files: &[serde_json::Value]) -> Result<HelperOutput, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let script = dir.path().join("scanner.py");
    std::fs::write(&script, HELPER).map_err(|e| e.to_string())?;
    let input = dir.path().join("input.json");
    std::fs::write(
        &input,
        serde_json::to_vec(&serde_json::json!({"files": files})).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut command = Command::new(python);
    command.args(["-I", "-S"]).arg(&script);
    let bytes = crate::go_calls::execute(
        &mut command,
        dir.path(),
        Some(&input),
        "Python source analysis",
        HINT,
    )?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid Python analysis: {e}"))
}

fn unavailable(files: usize, message: String) -> CallAnalysis {
    CallAnalysis {
        status: "unavailable".into(),
        java_release: 0,
        files,
        analyzers: vec![CallAnalyzer {
            language: "python".into(),
            backend: BACKEND.into(),
            status: "unavailable".into(),
            files,
            version: None,
            project: None,
            diagnostics: vec![message.clone()],
        }],
        diagnostics: vec![message],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::CallMethod;

    fn analyze(files: &[(&str, &str)]) -> CallAnalysis {
        let dir = tempfile::tempdir().unwrap();
        for (path, source) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source).unwrap();
        }
        analyze_python_calls(dir.path(), &Config::default())
    }
    fn method<'a>(report: &'a CallAnalysis, name: &str) -> &'a CallMethod {
        report
            .methods
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("no method {name}: {:?}", report.diagnostics))
    }
    /// (cyclomatic, max nesting, cognitive) of one function.
    fn complexity(report: &CallAnalysis, name: &str) -> (u32, u32, u32) {
        let method = method(report, name);
        let measured = method.complexity.as_ref().unwrap();
        let cognitive = report
            .details
            .as_ref()
            .unwrap()
            .method_metrics
            .iter()
            .find(|m| m.method_id == method.id)
            .unwrap()
            .cognitive;
        (measured.cyclomatic, measured.max_nesting, cognitive)
    }

    #[test]
    fn counts_follow_the_shared_rules() {
        let report = analyze(&[(
            "a.py",
            r#"
def plain():
    """Docstrings are not code."""
    return 1

def branches(a, b):
    if a:
        if b:
            return 1
        else:
            return 2
    elif b:
        return 3
    else:
        return 4

def logic(a, b, c):
    return a and b or c

def arms(x):
    match x:
        case 0:
            return 1
        case n if n > 5:
            return 2
        case _:
            return 3

def guarded(items):
    try:
        for item in items:
            while item:
                item -= 1
    except ValueError:
        return 0
    except KeyError:
        return 1
    return 2

def comprehension(xs):
    return [x for x in xs if x > 0]

def ternary(a):
    f = lambda y: y if a else 0
    return f(1)

def outer():
    def inner(a):
        if a:
            return 1
    return inner
"#,
        )]);
        assert_eq!(complexity(&report, "plain"), (1, 0, 0));
        // Same shape and numbers as the Rust `branches` case.
        assert_eq!(complexity(&report, "branches"), (4, 2, 6));
        assert_eq!(complexity(&report, "logic"), (3, 0, 2));
        // Two counted cases and one guard; the final `case _` does not count.
        assert_eq!(complexity(&report, "arms"), (4, 1, 1));
        // for, while and two handlers; `try` itself neither counts nor nests.
        assert_eq!(complexity(&report, "guarded"), (5, 2, 5));
        // Each generator and filter is a decision, but adds no cognitive load.
        assert_eq!(complexity(&report, "comprehension"), (3, 0, 0));
        // The lambda body is one level deeper, like a Rust closure.
        assert_eq!(complexity(&report, "ternary"), (2, 2, 2));
        // A nested `def` is its own scope and, as in Rust, not measured.
        assert_eq!(complexity(&report, "outer"), (1, 0, 0));
        assert!(report.methods.iter().all(|m| m.name != "inner"));
        let plain = report
            .details
            .as_ref()
            .unwrap()
            .method_metrics
            .iter()
            .find(|m| m.method_id == method(&report, "plain").id)
            .unwrap();
        // `return 1`: one operator, one operand, one line; no docstring.
        assert_eq!(
            (plain.operators, plain.operands, plain.source_lines),
            (1, 1, 1)
        );
    }

    #[test]
    fn classes_methods_and_abstractness_come_from_syntax() {
        let report = analyze(&[(
            "shop/ports.py",
            r#"
from abc import ABC, abstractmethod
from typing import Protocol

class Port(ABC):
    @abstractmethod
    def send(self, msg): ...

class Reader(Protocol):
    def read(self) -> str: ...

class Marked:
    @abstractmethod
    def run(self): ...

class Adapter(Port, base.Logger):
    class _Config:
        pass

    def send(self, msg):
        return msg

    @staticmethod
    def build():
        return Adapter()

if TYPE_CHECKING:
    def typed() -> None: ...

def helper():
    pass
"#,
        )]);
        assert_eq!(report.status, "partial");
        assert_eq!(report.files, 1);
        let class = |name: &str| {
            report
                .classes
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("no class {name}: {:?}", report.classes))
        };
        for abstract_name in ["Port", "Reader", "Marked"] {
            assert_eq!(
                class(abstract_name).is_abstract,
                Some(true),
                "{abstract_name}"
            );
        }
        let adapter = class("Adapter");
        assert_eq!(adapter.is_abstract, Some(false));
        assert_eq!(adapter.supertypes, ["Port", "base.Logger"]);
        assert_eq!(adapter.visibility.as_deref(), Some("public"));
        assert_eq!(adapter.id, "shop/ports.py::Adapter");
        assert_eq!(adapter.package, "shop");
        assert_eq!(adapter.graph_node, "shop/ports.py");
        assert_eq!(adapter.language.as_deref(), Some("python"));
        assert_eq!(
            class("Adapter._Config").visibility.as_deref(),
            Some("private")
        );
        assert_eq!(class("(module)").kind, "module");
        let send = report
            .methods
            .iter()
            .find(|m| m.name == "send" && m.class_id == adapter.id)
            .unwrap();
        assert!(!send.is_static);
        assert_eq!(send.signature, "def send(self, msg)");
        assert!(method(&report, "build").is_static);
        assert_eq!(
            method(&report, "helper").class_id,
            "shop/ports.py::(module)"
        );
        // Definitions under module-level `if`/`try` blocks still count.
        assert_eq!(method(&report, "typed").class_id, "shop/ports.py::(module)");
        let analyzer = &report.analyzers[0];
        assert_eq!(
            (
                analyzer.language.as_str(),
                analyzer.status.as_str(),
                analyzer.files
            ),
            ("python", "partial", 1)
        );
        assert!(
            analyzer.version.is_some(),
            "the interpreter version is reported"
        );
    }

    #[test]
    fn a_syntax_error_is_a_diagnostic_not_a_failed_run() {
        let report = analyze(&[
            ("ok.py", "def fine():\n    return 1\n"),
            ("bad.py", "def broken(:\n"),
        ]);
        assert_eq!(report.status, "partial");
        assert_eq!(report.files, 2);
        assert!(report.methods.iter().any(|m| m.name == "fine"));
        assert!(
            report.diagnostics.iter().any(|d| d.starts_with("bad.py:")),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn a_missing_interpreter_is_unavailable_with_a_hint() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
        let report = analyze_with(
            dir.path(),
            &Config::default(),
            OsStr::new("/nonexistent/python3"),
        );
        assert_eq!(report.status, "unavailable");
        assert_eq!(report.files, 1);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.contains("ARCHGUARD_PYTHON")),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn projects_without_python_are_not_applicable() {
        let report = analyze(&[("a.rs", "fn main() {}\n")]);
        assert_eq!(report.status, "not_applicable");
        assert!(report.analyzers.is_empty());
    }
}
