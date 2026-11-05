//! Python measurements and static call targets from the standard `ast` module
//! of the local Python 3. The bundled helper parses sources and never imports
//! or executes project code. Calls are resolved only where imports and
//! definitions bind them (rules in the helper's docstring); everything else is
//! reported as `unresolved`. Types stay unresolved, so the report is `partial`.
//! Counting rules mirror `rust_calls.rs` (`ast-decisions-v1`,
//! `source-maintainability/v1`), mapped to Python in the helper's docstring.
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

use archguard_core::{
    CallAnalysis, CallAnalyzer, CallClass, CallMethod, Config, MethodCall, SemanticDetails,
    SourceMethodMetrics,
};
use archguard_extract::{Lang, strip_root};
use serde::Deserialize;

const HELPER: &str = include_str!("ArchguardCallScanner.py");
const BACKEND: &str = "python ast";
const HINT: &str = "Install Python 3.9+ or set ARCHGUARD_PYTHON to its executable.";
const SOURCE_LIMIT: u64 = 8 * 1024 * 1024;
const INPUT_LIMIT: usize = 64 * 1024 * 1024;
const LIMITATION: &str = "Syntax-only analysis: classes, functions, complexity and Halstead counts. Call targets are resolved statically from imports, module functions, classes, self/cls and super(); calls on other values, rebound names and star imports stay unresolved. Types are not resolved; abstractness is read from ABC/Protocol bases and @abstractmethod.";

/// What the helper writes to stdout.
#[derive(Deserialize)]
struct HelperOutput {
    version: String,
    classes: Vec<CallClass>,
    methods: Vec<CallMethod>,
    metrics: Vec<SourceMethodMetrics>,
    calls: Vec<MethodCall>,
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
    diagnostics.push(coverage(&output.calls));
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
        calls: output.calls,
        diagnostics,
    }
}

/// How many call sites the static resolver could attribute.
fn coverage(calls: &[MethodCall]) -> String {
    let count = |kind: &str| calls.iter().filter(|c| c.resolution == kind).count();
    format!(
        "Calls: {} sites, {} project, {} external, {} unresolved (dynamic receivers, call results, subscripts or rebound names).",
        calls.len(),
        count("project"),
        count("external"),
        count("unresolved")
    )
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

    /// The single call whose source text is `code`.
    fn call<'a>(report: &'a CallAnalysis, code: &str) -> &'a archguard_core::MethodCall {
        let found: Vec<_> = report.calls.iter().filter(|c| c.code == code).collect();
        assert_eq!(found.len(), 1, "call {code}: {:#?}", report.calls);
        found[0]
    }
    /// (resolution, dispatch, target method name) of the call `code`.
    fn target(report: &CallAnalysis, code: &str) -> (String, String, Option<String>) {
        let call = call(report, code);
        let name = call.to_method.as_ref().map(|id| {
            report
                .methods
                .iter()
                .find(|m| &m.id == id)
                .map_or_else(|| id.clone(), |m| m.name.clone())
        });
        (call.resolution.clone(), call.dispatch.clone(), name)
    }
    fn project(dispatch: &str, name: &str) -> (String, String, Option<String>) {
        ("project".into(), dispatch.into(), Some(name.into()))
    }

    #[test]
    fn resolves_calls_to_functions_in_the_same_module() {
        let report = analyze(&[(
            "app.py",
            "def run():\n    return step(1)\n\ndef step(x):\n    return x\n\nrun()\n",
        )]);
        let edge = call(&report, "step(1)");
        assert_eq!(target(&report, "step(1)"), project("static", "step"));
        assert_eq!(edge.from_method.as_ref(), Some(&method(&report, "run").id));
        assert_eq!(edge.to_class.as_deref(), Some("app.py::(module)"));
        assert_eq!(
            (edge.file.as_str(), edge.line, edge.column),
            ("app.py", 2, 12)
        );
        assert_eq!(edge.target_label, "step");
        // Module-level statements call from the module, not from a function.
        let top = call(&report, "run()");
        assert_eq!(
            (top.from_class.as_str(), top.from_method.as_ref()),
            ("app.py::(module)", None)
        );
        assert_eq!(top.resolution, "project");
    }

    #[test]
    fn resolves_self_super_and_inherited_methods() {
        let report = analyze(&[
            (
                "shop/base.py",
                "class Base:\n    def __init__(self):\n        pass\n\n    def save(self):\n        return 1\n\n    def log(self):\n        return 2\n",
            ),
            (
                "shop/order.py",
                r#"
from abc import ABC
from shop.base import Base

class Order(Base, ABC):
    def __init__(self):
        super().__init__()

    def save(self):
        self.log()
        self.check()
        return super().save()

    def check(this):
        return Order.make()

    @staticmethod
    def make():
        return Order()
"#,
            ),
        ]);
        assert_eq!(
            target(&report, "self.check()"),
            project("instance", "check")
        );
        // Inherited from the project base class in another module.
        let log = call(&report, "self.log()");
        assert_eq!(log.to_class.as_deref(), Some("shop/base.py::Base"));
        assert_eq!(log.resolution, "project");
        let parent_save = call(&report, "super().save()");
        assert_eq!(parent_save.to_class.as_deref(), Some("shop/base.py::Base"));
        assert_eq!(parent_save.dispatch, "instance");
        assert_eq!(
            call(&report, "super().__init__()").to_class.as_deref(),
            Some("shop/base.py::Base")
        );
        assert_eq!(target(&report, "Order.make()"), project("static", "make"));
        // A constructor call targets the class and its own `__init__`.
        let new = call(&report, "Order()");
        assert_eq!(new.to_class.as_deref(), Some("shop/order.py::Order"));
        assert_eq!(
            target(&report, "Order()"),
            project("constructor", "__init__")
        );
    }

    #[test]
    fn resolves_functions_imported_from_other_modules() {
        let report = analyze(&[
            ("pkg/__init__.py", "from .util import helper as exported\n"),
            (
                "pkg/util.py",
                "def helper():\n    return 1\n\nclass Service:\n    pass\n",
            ),
            (
                "pkg/app.py",
                r#"
import pkg.util
import pkg.util as u
from . import util
from .util import helper, Service
from pkg import exported

def main():
    helper()
    util.helper()
    u.helper()
    pkg.util.helper()
    exported()
    return Service()
"#,
            ),
        ]);
        for code in [
            "helper()",
            "util.helper()",
            "u.helper()",
            "pkg.util.helper()",
            "exported()",
        ] {
            assert_eq!(target(&report, code), project("static", "helper"), "{code}");
            assert_eq!(
                call(&report, code).to_class.as_deref(),
                Some("pkg/util.py::(module)")
            );
        }
        // No `__init__` of its own: the class itself is the target.
        let new = call(&report, "Service()");
        assert_eq!(
            (
                new.resolution.as_str(),
                new.dispatch.as_str(),
                new.to_class.as_deref(),
                new.to_method.as_ref()
            ),
            ("project", "constructor", Some("pkg/util.py::Service"), None)
        );
    }

    #[test]
    fn dynamic_and_external_calls_are_reported_not_dropped() {
        let report = analyze(&[(
            "dyn.py",
            r#"
import os.path
from json import loads

def target():
    pass

def twice():
    pass

def twice():
    pass

def run(obj, name, handlers):
    getattr(obj, name)()
    obj.method()
    handlers[0]()
    target = obj
    target()
    twice()
    loads("1")
    os.path.join("a", "b")
    return len(handlers)
"#,
        )]);
        for code in [
            "getattr(obj, name)()",
            "obj.method()",
            "handlers[0]()",
            "target()",
            "twice()",
        ] {
            let edge = call(&report, code);
            assert_eq!(
                (
                    edge.resolution.as_str(),
                    edge.dispatch.as_str(),
                    edge.to_class.as_ref()
                ),
                ("unresolved", "unknown", None),
                "{code}"
            );
        }
        let external = call(&report, "loads(\"1\")");
        assert_eq!(external.resolution, "external");
        assert_eq!(external.to_method.as_deref(), Some("json::(module)#loads"));
        assert_eq!(
            call(&report, r#"os.path.join("a", "b")"#).resolution,
            "external"
        );
        assert_eq!(
            call(&report, "len(handlers)").to_class.as_deref(),
            Some("builtins::(module)")
        );
        // `getattr(...)` itself is a builtin call; the call of its result is not.
        assert_eq!(call(&report, "getattr(obj, name)").resolution, "external");
        assert_eq!(report.status, "partial");
        let coverage = "Calls: 9 sites, 0 project, 4 external, 5 unresolved";
        assert!(
            report.analyzers[0]
                .diagnostics
                .iter()
                .any(|d| d.starts_with(coverage)),
            "{:?}",
            report.analyzers[0].diagnostics
        );
    }

    #[test]
    fn unknown_bases_and_import_cycles_stay_unresolved() {
        let report = analyze(&[
            ("a.py", "from b import loop\n"),
            ("b.py", "from a import loop\n"),
            (
                "c.py",
                r#"
from a import loop
from django.views import View

class Page(View):
    def get(self):
        self.render()
        self.dispatch()
        return loop()

    def post(self):
        self = None
        return self.get()

    def render(self):
        pass
"#,
            ),
        ]);
        // Defined in the class itself: certain whatever the external base holds.
        assert_eq!(
            target(&report, "self.render()"),
            project("instance", "render")
        );
        // May come from the external base (or be overridden there): unknown.
        assert_eq!(target(&report, "self.dispatch()").0, "unresolved");
        assert_eq!(target(&report, "loop()").0, "unresolved");
        assert_eq!(target(&report, "self.get()").0, "unresolved");
    }

    #[test]
    fn projects_without_python_are_not_applicable() {
        let report = analyze(&[("a.rs", "fn main() {}\n")]);
        assert_eq!(report.status, "not_applicable");
        assert!(report.analyzers.is_empty());
    }
}
