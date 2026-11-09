//! Token-based PHP measurements and static call targets from
//! `archguard_extract::scan_php_structure`. Nothing is executed and no PHP
//! installation is needed. The extractor resolves names per file (namespace,
//! `use` imports); this module looks them up across the project. Types stay
//! unresolved, so the report is `partial`.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use archguard_core::{CallAnalysis, CallMethod, Config, MethodCall};
use archguard_extract::{
    Extractor, Lang, PhpClassFacts, PhpClassRef, PhpExtractor, PhpSymbols, PhpTarget,
    scan_php_structure,
};

const LIMITATION: &str = archguard_core::CallAnalyzer::PHP_TOKENS;

#[tracing::instrument(skip_all)]
pub(crate) fn analyze_php_calls(root: &Path, config: &Config) -> CallAnalysis {
    let scanned = RefCell::new(Vec::new());
    let mut report = crate::semantic_calls::token_calls::token_calls(
        root,
        config,
        (Lang::Php, "php", LIMITATION),
        |path, content| {
            // The graph names a PHP file after its namespace, as the pipeline does.
            let declarations = PhpExtractor.extract(path, content).declarations;
            let node = crate::pipeline::file_node_name(path, &declarations);
            let found = scan_php_structure(path, &node, content);
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
    // Only a fully scanned project can tell that a name is declared nowhere.
    let complete = files.len() == report.files && files.iter().all(|(_, f)| f.measured);
    report.calls = resolve(&files, &report.methods, complete);
    let coverage = coverage(&report.calls);
    let at = report.diagnostics.len() - 1;
    report.diagnostics.insert(at, coverage.clone());
    report.analyzers[0].diagnostics.insert(0, coverage);
    report
}

/// How many call sites the static resolver could attribute.
fn coverage(calls: &[MethodCall]) -> String {
    let count = |kind: &str| calls.iter().filter(|c| c.resolution == kind).count();
    format!(
        "Calls: {} sites, {} project, {} external, {} unresolved (other receivers, dynamic names, closures or unknown classes).",
        calls.len(),
        count("project"),
        count("external"),
        count("unresolved")
    )
}

/// The call edges of all scanned files, `(path, symbols)`.
fn resolve(
    files: &[(String, PhpSymbols)],
    methods: &[CallMethod],
    complete: bool,
) -> Vec<MethodCall> {
    let project = Project::new(files, methods, complete);
    files
        .iter()
        .flat_map(|(path, symbols)| {
            symbols.sites.iter().map(|site| {
                let (resolution, dispatch, to_class, to_method) =
                    project.target(&site.target, &site.from_class);
                MethodCall {
                    from_class: site.from_class.clone(),
                    from_method: site.from_method.clone(),
                    to_class,
                    to_method,
                    target_label: site.label.clone(),
                    dispatch: dispatch.into(),
                    resolution: resolution.into(),
                    file: path.clone(),
                    line: site.line,
                    column: site.column,
                    code: site.code.clone(),
                }
            })
        })
        .collect()
}

/// (resolution, dispatch, to_class, to_method) of one call.
type Target = (&'static str, &'static str, Option<String>, Option<String>);
const UNRESOLVED: Target = ("unresolved", "unknown", None, None);

/// A class name looked up in the project.
enum Class<'a> {
    Project(&'a PhpClassFacts),
    /// Declared nowhere in a fully scanned project.
    External(String),
    /// Declared twice, inside an unscanned body, or the project is not
    /// fully scanned.
    Unknown,
}

/// A method looked up along a class, its traits and its parents.
enum Member<'a> {
    Found(&'a CallMethod),
    Missing,
    Unknown,
}

/// Declarations of all files, keyed by lowercase name: PHP class, function
/// and method names are case-insensitive.
struct Project<'a> {
    classes: HashMap<String, Vec<&'a PhpClassFacts>>,
    by_id: HashMap<&'a str, &'a PhpClassFacts>,
    methods: HashMap<&'a str, Vec<&'a CallMethod>>,
    method_ids: HashMap<&'a str, &'a CallMethod>,
    functions: HashMap<String, Vec<&'a str>>,
    hidden_classes: HashSet<String>,
    hidden_functions: HashSet<String>,
    complete: bool,
}

impl<'a> Project<'a> {
    fn new(files: &'a [(String, PhpSymbols)], methods: &'a [CallMethod], complete: bool) -> Self {
        let mut project = Project {
            classes: HashMap::new(),
            by_id: HashMap::new(),
            methods: HashMap::new(),
            method_ids: methods.iter().map(|m| (m.id.as_str(), m)).collect(),
            functions: HashMap::new(),
            hidden_classes: HashSet::new(),
            hidden_functions: HashSet::new(),
            complete,
        };
        for method in methods {
            project
                .methods
                .entry(method.class_id.as_str())
                .or_default()
                .push(method);
        }
        for (_, symbols) in files {
            for class in &symbols.classes {
                project
                    .classes
                    .entry(class.name.to_ascii_lowercase())
                    .or_default()
                    .push(class);
                project.by_id.insert(&class.id, class);
            }
            for function in &symbols.functions {
                project
                    .functions
                    .entry(function.name.to_ascii_lowercase())
                    .or_default()
                    .push(&function.method_id);
            }
            let lower = |names: &[String]| {
                names
                    .iter()
                    .map(|n| n.to_ascii_lowercase())
                    .collect::<Vec<_>>()
            };
            project
                .hidden_classes
                .extend(lower(&symbols.hidden_classes));
            project
                .hidden_functions
                .extend(lower(&symbols.hidden_functions));
        }
        project
    }

    fn class(&self, name: &str) -> Class<'a> {
        let key = name.to_ascii_lowercase();
        match self.classes.get(&key).map(Vec::as_slice) {
            Some([class]) => Class::Project(class),
            Some(_) => Class::Unknown,
            None if self.complete && !self.hidden_classes.contains(&key) => {
                Class::External(name.into())
            }
            None => Class::Unknown,
        }
    }

    /// The class `reference` names from inside `from_class`.
    fn class_ref(&self, reference: &PhpClassRef, from_class: &str) -> Class<'a> {
        let Some(&current) = self.by_id.get(from_class) else {
            return Class::Unknown;
        };
        match reference {
            PhpClassRef::Named(name) => self.class(name),
            // In a trait `self` names the class that uses it.
            PhpClassRef::Current if current.kind == "trait" => Class::Unknown,
            PhpClassRef::Current => Class::Project(current),
            PhpClassRef::Parent => current
                .parent
                .as_deref()
                .map_or(Class::Unknown, |parent| self.class(parent)),
        }
    }

    /// `name` in `class`: its own methods, then its traits, then its parent.
    fn member(&self, class: &'a PhpClassFacts, name: &str, depth: usize) -> Member<'a> {
        if depth > 32 {
            return Member::Unknown;
        }
        let own: Vec<_> = self
            .methods
            .get(class.id.as_str())
            .into_iter()
            .flatten()
            .filter(|m| m.name.eq_ignore_ascii_case(name))
            .collect();
        match own.as_slice() {
            [method] => return Member::Found(method),
            [] => {}
            _ => return Member::Unknown,
        }
        if class.bodiless.contains(&name.to_ascii_lowercase()) {
            return Member::Unknown;
        }
        match self.trait_member(class, name, depth) {
            Member::Missing => {}
            known => return known,
        }
        match class.parent.as_deref().map(|parent| self.class(parent)) {
            None => Member::Missing,
            Some(Class::Project(parent)) => self.member(parent, name, depth + 1),
            Some(_) => Member::Unknown,
        }
    }

    /// `name` in the traits `class` uses: found in exactly one of them, or
    /// unknown when a trait is not a project trait or `insteadof`/`as`
    /// rules apply.
    fn trait_member(&self, class: &'a PhpClassFacts, name: &str, depth: usize) -> Member<'a> {
        if class.trait_rules && !class.traits.is_empty() {
            return Member::Unknown;
        }
        let mut found = Member::Missing;
        for name_of_trait in &class.traits {
            let Class::Project(used) = self.class(name_of_trait) else {
                return Member::Unknown;
            };
            match (self.member(used, name, depth + 1), &found) {
                (Member::Missing, _) => {}
                (Member::Found(method), Member::Missing) => found = Member::Found(method),
                _ => return Member::Unknown,
            }
        }
        found
    }

    fn method_call(&self, class: Class<'a>, name: &str) -> Target {
        match class {
            Class::Project(class) => match self.member(class, name, 0) {
                Member::Found(method) => found(method),
                _ => UNRESOLVED,
            },
            Class::External(name_of_class) => ("external", "unknown", Some(name_of_class), None),
            Class::Unknown => UNRESOLVED,
        }
    }

    fn function(&self, name: &str) -> Option<Target> {
        let key = name.to_ascii_lowercase();
        if self.hidden_functions.contains(&key) {
            return Some(UNRESOLVED);
        }
        match self.functions.get(&key)?.as_slice() {
            [id] => Some(found(self.method_ids[id])),
            _ => Some(UNRESOLVED),
        }
    }

    fn target(&self, target: &PhpTarget, from_class: &str) -> Target {
        match target {
            // In a trait this finds only the trait's own (and used) methods.
            PhpTarget::This(name) | PhpTarget::Static(PhpClassRef::Current, name) => {
                match self.by_id.get(from_class) {
                    Some(&current) => self.method_call(Class::Project(current), name),
                    None => UNRESOLVED,
                }
            }
            PhpTarget::Static(reference, name) => {
                self.method_call(self.class_ref(reference, from_class), name)
            }
            PhpTarget::New(reference) => match self.class_ref(reference, from_class) {
                Class::Project(class) => {
                    let constructor = match self.member(class, "__construct", 0) {
                        Member::Found(method) => Some(method.id.clone()),
                        _ => None,
                    };
                    (
                        "project",
                        "constructor",
                        Some(class.id.clone()),
                        constructor,
                    )
                }
                Class::External(name) => ("external", "constructor", Some(name), None),
                Class::Unknown => UNRESOLVED,
            },
            PhpTarget::Function { name, fallback } => self
                .function(name)
                .or_else(|| fallback.as_deref().and_then(|global| self.function(global)))
                .unwrap_or(if self.complete {
                    ("external", "unknown", None, None)
                } else {
                    UNRESOLVED
                }),
            PhpTarget::Unresolved => UNRESOLVED,
        }
    }
}

fn found(method: &CallMethod) -> Target {
    let dispatch = if method.is_static {
        "static"
    } else {
        "instance"
    };
    (
        "project",
        dispatch,
        Some(method.class_id.clone()),
        Some(method.id.clone()),
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
    fn analyze(files: &[(&str, &str)]) -> CallAnalysis {
        analyze_php_calls(project(files).path(), &Config::default())
    }
    /// The single call whose source text is `code`.
    fn call<'a>(report: &'a CallAnalysis, code: &str) -> &'a MethodCall {
        let found: Vec<_> = report.calls.iter().filter(|c| c.code == code).collect();
        assert_eq!(found.len(), 1, "call {code}: {:#?}", report.calls);
        found[0]
    }
    /// (resolution, dispatch, target class, target method name) of `code`.
    fn target(
        report: &CallAnalysis,
        code: &str,
    ) -> (String, String, Option<String>, Option<String>) {
        let call = call(report, code);
        let name = call.to_method.as_ref().map(|id| {
            report
                .methods
                .iter()
                .find(|m| &m.id == id)
                .map_or_else(|| id.clone(), |m| m.name.clone())
        });
        (
            call.resolution.clone(),
            call.dispatch.clone(),
            call.to_class.clone(),
            name,
        )
    }
    fn project_call(
        dispatch: &str,
        class: &str,
        method: Option<&str>,
    ) -> (String, String, Option<String>, Option<String>) {
        (
            "project".into(),
            dispatch.into(),
            Some(class.into()),
            method.map(Into::into),
        )
    }
    fn unresolved() -> (String, String, Option<String>, Option<String>) {
        ("unresolved".into(), "unknown".into(), None, None)
    }

    #[test]
    fn php_classes_reach_their_graph_nodes() {
        let report = analyze(&[
            (
                "src/Domain/User.php",
                "<?php\nnamespace App\\Domain;\n\nabstract class User {\n    public function name($a) { return $a ? 'x' : 'y'; }\n}\n",
            ),
            ("src/broken.php", "<?php\nfunction broken() { if (\n"),
        ]);
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

    const BASE: &str = r#"<?php
namespace App\Domain;

abstract class Base {
    use Logs;
    public function __construct() {}
    public function save() { return 1; }
    protected static function make() { return new static(); }
    abstract public function id();
}
"#;
    const LOGS: &str =
        "<?php\nnamespace App\\Domain;\n\ntrait Logs {\n    public function log() {}\n}\n";
    const SUPPORT: &str = r#"<?php
namespace App\Support;

class Clock {}

class Format {
    public static function date() {}
}

function tick() {}
"#;
    const USER: &str = r#"<?php
namespace App\Domain;

use Lib\Mailer;
use App\Support\{Clock, Format as Fmt};

class User extends Base {
    public function __construct() {
        parent::__construct();
    }

    public function run($repo) {
        $this->save();
        $this->log();
        self::make();
        static::make();
        $this->id();
        $this->missing();
        new Mailer();
        Mailer::send();
        Fmt::date();
        new Clock();
        new User();
        helper();
        local();
        strlen('x');
        \App\Support\tick();
        $repo->find();
    }
}
"#;

    fn shop() -> CallAnalysis {
        analyze(&[
            ("src/Domain/Base.php", BASE),
            ("src/Domain/Logs.php", LOGS),
            ("src/Domain/User.php", USER),
            (
                "src/Domain/functions.php",
                "<?php\nnamespace App\\Domain;\n\nfunction local() {}\n",
            ),
            ("src/Support/Clock.php", SUPPORT),
            ("src/functions.php", "<?php\nfunction helper() {}\n"),
        ])
    }

    #[test]
    fn resolves_members_along_the_class_hierarchy_and_traits() {
        let report = shop();
        let base = "Domain/Base.php::Base";
        assert_eq!(
            target(&report, "$this->save()"),
            project_call("instance", base, Some("save"))
        );
        // A method of a trait the parent class uses.
        assert_eq!(
            target(&report, "$this->log()"),
            project_call("instance", "Domain/Logs.php::Logs", Some("log"))
        );
        assert_eq!(
            target(&report, "self::make()"),
            project_call("static", base, Some("make"))
        );
        assert_eq!(
            target(&report, "static::make()"),
            project_call("static", base, Some("make"))
        );
        assert_eq!(
            target(&report, "parent::__construct()"),
            project_call("instance", base, Some("__construct"))
        );
        // Abstract: the implementation is not known statically.
        assert_eq!(target(&report, "$this->id()"), unresolved());
        // Missing everywhere (`__call` or an error at runtime).
        assert_eq!(target(&report, "$this->missing()"), unresolved());
        assert_eq!(target(&report, "$repo->find()"), unresolved());
        let edge = call(&report, "$this->save()");
        assert_eq!(edge.from_class, "Domain/User.php::User");
        assert_eq!(edge.file, "Domain/User.php");
        assert_eq!((edge.line, edge.column), (13, 9));
        assert_eq!(edge.target_label, "$this->save");
    }

    #[test]
    fn resolves_imported_classes_and_namespaced_functions() {
        let report = shop();
        assert_eq!(
            target(&report, "Fmt::date()"),
            project_call("static", "Support/Clock.php::Format", Some("date"))
        );
        // No constructor of its own: the class itself is the target.
        assert_eq!(
            target(&report, "new Clock()"),
            project_call("constructor", "Support/Clock.php::Clock", None)
        );
        assert_eq!(
            target(&report, "new User()"),
            project_call("constructor", "Domain/User.php::User", Some("__construct"))
        );
        // `new static()` in Base names Base (late static binding aside).
        assert_eq!(
            target(&report, "new static()"),
            project_call("constructor", "Domain/Base.php::Base", Some("__construct"))
        );
        assert_eq!(
            target(&report, "local()"),
            project_call("static", "Domain/functions.php::(module)", Some("local"))
        );
        // Not in the namespace: PHP falls back to the global project function.
        assert_eq!(
            target(&report, "helper()"),
            project_call("static", "functions.php::(module)", Some("helper"))
        );
        assert_eq!(
            target(&report, "\\App\\Support\\tick()"),
            project_call("static", "Support/Clock.php::(module)", Some("tick"))
        );
        // Declared nowhere in the project: a library or builtin.
        let external = |class: Option<&str>, dispatch: &str| {
            (
                "external".to_string(),
                dispatch.to_string(),
                class.map(Into::into),
                None,
            )
        };
        assert_eq!(target(&report, "strlen('x')"), external(None, "unknown"));
        assert_eq!(
            target(&report, "new Mailer()"),
            external(Some("Lib\\Mailer"), "constructor")
        );
        assert_eq!(
            target(&report, "Mailer::send()"),
            external(Some("Lib\\Mailer"), "unknown")
        );
        let coverage = "Calls: 18 sites, 12 project, 3 external, 3 unresolved";
        assert!(
            report.analyzers[0]
                .diagnostics
                .iter()
                .any(|d| d.starts_with(coverage)),
            "{:?}",
            report.analyzers[0].diagnostics
        );
        assert_eq!(report.status, "partial");
    }

    #[test]
    fn unknown_bases_trait_rules_and_duplicates_stay_unresolved() {
        let report = analyze(&[
            (
                "src/Page.php",
                r#"<?php
namespace App;

class Page extends \Framework\Controller {
    use A, B { A::x insteadof B; }
    public function show() {
        $this->render();
        $this->own();
        $this->x();
        new Twice();
        twice();
        inner();
    }
    public function own() {
        function inner() {}
    }
}
trait A { public function x() {} }
trait B { public function x() {} }
"#,
            ),
            (
                "src/One.php",
                "<?php\nnamespace App;\nclass Twice {}\nfunction twice() {}\n",
            ),
            (
                "src/Two.php",
                "<?php\nnamespace App;\nclass Twice {}\nfunction twice() {}\n",
            ),
        ]);
        // Defined in the class itself: certain whatever the external base holds.
        assert_eq!(
            target(&report, "$this->own()"),
            project_call("instance", "Page.php::Page", Some("own"))
        );
        // May come from the external base: unknown, not external.
        assert_eq!(target(&report, "$this->render()"), unresolved());
        // `insteadof`/`as` rules are not evaluated.
        assert_eq!(target(&report, "$this->x()"), unresolved());
        // Declared twice in the project: ambiguous.
        assert_eq!(target(&report, "new Twice()"), unresolved());
        assert_eq!(target(&report, "twice()"), unresolved());
        // Declared inside a function body, which is not scanned.
        assert_eq!(target(&report, "inner()"), unresolved());
    }

    #[test]
    fn an_unscanned_file_turns_external_into_unresolved() {
        let report = analyze(&[
            ("src/a.php", "<?php\nfunction a() { strlen('x'); b(); }\n"),
            ("src/broken.php", "<?php\nfunction b() { if (\n"),
        ]);
        // `b` may be declared in the file that could not be scanned.
        assert_eq!(target(&report, "b()"), unresolved());
        assert_eq!(target(&report, "strlen('x')"), unresolved());
    }

    #[test]
    fn projects_without_php_are_not_applicable() {
        let dir = project(&[("a.py", "x = 1\n")]);
        let report = analyze_php_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "not_applicable");
        assert!(report.analyzers.is_empty());
    }
}
