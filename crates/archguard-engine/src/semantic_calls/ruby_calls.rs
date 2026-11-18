//! Token-based Ruby measurements and static call targets from
//! `archguard_extract::ruby` (ADR 0019). Nothing is executed and no Ruby
//! installation is needed. The scanner records constants with their lexical
//! scope and call sites with names as written; this module binds them across
//! the project. Types stay unresolved, so the report is `partial`.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use archguard_core::{CallAnalysis, Config, MethodCall};
use archguard_extract::Lang;
use archguard_extract::ruby::{
    RubyCallSite, RubyClassFacts, RubyConst, RubyMethodFacts, RubySymbols, RubyTarget, Superclass,
    scan_ruby_structure,
};

const LIMITATION: &str = archguard_core::CallAnalyzer::RUBY_TOKENS;

#[tracing::instrument(skip_all)]
pub(crate) fn analyze_ruby_calls(root: &Path, config: &Config) -> CallAnalysis {
    let scanned = RefCell::new(Vec::new());
    let mut report = super::token_calls::token_calls(
        root,
        config,
        (Lang::Ruby, "ruby", LIMITATION),
        |file, path, content| {
            // Ruby files declare no module: the graph names them by path.
            let node = crate::pipeline::file_node_name(file, path, &[]);
            let found = scan_ruby_structure(path, &node, content);
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
    // Only a fully scanned project can tell that a constant is declared nowhere.
    let complete = files.len() == report.files && files.iter().all(|(_, f)| f.measured);
    let project = Project::new(&files, complete);
    for class in &mut report.classes {
        if let Some(facts) = project.by_id.get(class.id.as_str()) {
            class.supertypes = project.supertypes(facts);
        }
    }
    report.calls = files
        .iter()
        .flat_map(|(path, symbols)| symbols.sites.iter().map(|site| project.call(path, site)))
        .collect();
    let dynamic = files
        .iter()
        .flat_map(|(_, symbols)| &symbols.sites)
        .filter(|site| site.target == RubyTarget::Dynamic)
        .count();
    let coverage = coverage(&report.calls, dynamic);
    let at = report.diagnostics.len() - 1;
    report.diagnostics.insert(at, coverage.clone());
    report.analyzers[0].diagnostics.insert(0, coverage);
    report
}

/// How many call sites the static resolver could attribute.
fn coverage(calls: &[MethodCall], dynamic: usize) -> String {
    let count = |kind: &str| calls.iter().filter(|c| c.resolution == kind).count();
    format!(
        "Calls: {} sites, {} project, {} external, {} unresolved (variables, other receivers, ambiguous or unknown classes; {dynamic} of them dynamic: send, define_method, method_missing and other reflection).",
        calls.len(),
        count("project"),
        count("external"),
        count("unresolved")
    )
}

/// (resolution, dispatch, to_class, to_method) of one call.
type Target = (&'static str, &'static str, Option<String>, Option<String>);
const UNRESOLVED: Target = ("unresolved", "unknown", None, None);

/// A constant looked up in the project.
enum Class<'a> {
    Project(&'a RubyClassFacts),
    /// Declared nowhere in a fully scanned project: a gem or the standard
    /// library.
    External(String),
    /// Declared in several files, assigned rather than declared, or the
    /// project is not fully scanned.
    Unknown,
}

/// A method looked up along a class, its modules and superclasses.
enum Member<'a> {
    Found(&'a RubyClassFacts, &'a RubyMethodFacts),
    Missing,
    /// An ancestor is not a unique project class or module.
    Unknown,
}

struct Project<'a> {
    by_name: HashMap<&'a str, Vec<&'a RubyClassFacts>>,
    by_id: HashMap<&'a str, &'a RubyClassFacts>,
    /// Constants assigned in the project (`Point = Struct.new`).
    assigned: HashSet<&'a str>,
    complete: bool,
}

impl<'a> Project<'a> {
    fn new(files: &'a [(String, RubySymbols)], complete: bool) -> Self {
        let mut project = Project {
            by_name: HashMap::new(),
            by_id: HashMap::new(),
            assigned: HashSet::new(),
            complete,
        };
        for (_, symbols) in files {
            for class in &symbols.classes {
                project.by_name.entry(&class.name).or_default().push(class);
                project.by_id.insert(&class.id, class);
            }
            project
                .assigned
                .extend(symbols.constants.iter().map(String::as_str));
        }
        project
    }

    fn known(&self, name: &str) -> bool {
        self.by_name.contains_key(name) || self.assigned.contains(name)
    }

    /// Ruby's lexical lookup of `constant`, ancestors of the enclosing
    /// classes aside.
    fn constant(&self, constant: &RubyConst) -> Class<'a> {
        for candidate in constant.candidates() {
            match self.by_name.get(candidate.as_str()).map(Vec::as_slice) {
                Some([class]) => return Class::Project(class),
                Some(_) => return Class::Unknown,
                None if self.assigned.contains(candidate.as_str()) => return Class::Unknown,
                None => {}
            }
        }
        // `Billing::Missing` below a project namespace is no gem constant.
        let head = RubyConst {
            path: constant.path.split("::").next().unwrap_or("").to_string(),
            ..constant.clone()
        };
        let inside_project = head.candidates().iter().any(|name| self.known(name));
        if self.complete && !inside_project {
            Class::External(constant.path.clone())
        } else {
            Class::Unknown
        }
    }

    /// Superclass, included, prepended and extended modules: project class
    /// ids where they resolve, the names as written otherwise.
    fn supertypes(&self, class: &RubyClassFacts) -> Vec<String> {
        let superclass = match &class.superclass {
            Superclass::Const(constant) => Some(constant),
            _ => None,
        };
        let mut out: Vec<String> = superclass
            .into_iter()
            .chain(&class.includes)
            .chain(&class.prepends)
            .chain(&class.extends)
            .map(|constant| match self.constant(constant) {
                Class::Project(found) => found.id.clone(),
                _ => constant.path.clone(),
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// The last definition of `name` in `class` itself.
    fn own(class: &'a RubyClassFacts, name: &str, singleton: bool) -> Option<&'a RubyMethodFacts> {
        class
            .methods
            .iter()
            .rev()
            .find(|m| m.name == name && m.singleton == singleton)
    }

    /// `name` in the first of `modules` (searched last to first) that has it.
    fn modules(&self, modules: &'a [RubyConst], name: &str, depth: usize) -> Member<'a> {
        for module in modules.iter().rev() {
            let Class::Project(found) = self.constant(module) else {
                return Member::Unknown;
            };
            match self.instance(found, name, depth + 1) {
                Member::Missing => {}
                known => return known,
            }
        }
        Member::Missing
    }

    /// An instance method: prepended modules, the class, included modules,
    /// then the superclass.
    fn instance(&self, class: &'a RubyClassFacts, name: &str, depth: usize) -> Member<'a> {
        if depth > 32 {
            return Member::Unknown;
        }
        match self.modules(&class.prepends, name, depth) {
            Member::Missing => {}
            known => return known,
        }
        if let Some(method) = Self::own(class, name, false) {
            return Member::Found(class, method);
        }
        self.after_own(class, name, false, depth)
    }

    /// A singleton (class) method: the class, extended modules, then the
    /// superclass's singleton methods.
    fn singleton(&self, class: &'a RubyClassFacts, name: &str, depth: usize) -> Member<'a> {
        if depth > 32 {
            return Member::Unknown;
        }
        if let Some(method) = Self::own(class, name, true) {
            return Member::Found(class, method);
        }
        self.after_own(class, name, true, depth)
    }

    /// The lookup past the class's own methods, where `super` starts.
    fn after_own(
        &self,
        class: &'a RubyClassFacts,
        name: &str,
        singleton: bool,
        depth: usize,
    ) -> Member<'a> {
        let modules = if singleton {
            &class.extends
        } else {
            &class.includes
        };
        match self.modules(modules, name, depth) {
            Member::Missing => {}
            known => return known,
        }
        match &class.superclass {
            Superclass::None => Member::Missing,
            Superclass::Expression => Member::Unknown,
            Superclass::Const(constant) => match self.constant(constant) {
                Class::Project(parent) if singleton => self.singleton(parent, name, depth + 1),
                Class::Project(parent) => self.instance(parent, name, depth + 1),
                _ => Member::Unknown,
            },
        }
    }

    fn target(&self, site: &RubyCallSite) -> Target {
        let current = self.by_id.get(site.from_class.as_str()).copied();
        match (&site.target, current) {
            // `new` in a class method constructs the class itself.
            (RubyTarget::SelfCall(name), Some(class))
                if site.singleton && name == "new" && class.kind == "class" =>
            {
                self.construct(class)
            }
            (RubyTarget::SelfCall(name), Some(class)) if site.singleton => {
                found(self.singleton(class, name, 0))
            }
            (RubyTarget::SelfCall(name), Some(class)) => found(self.instance(class, name, 0)),
            (RubyTarget::Super(name), Some(class)) => {
                found(self.after_own(class, name, site.singleton, 0))
            }
            (RubyTarget::Const { constant, method }, _) => self.on_constant(constant, method),
            _ => UNRESOLVED,
        }
    }

    /// `Const.new` or `Const.method`.
    fn on_constant(&self, constant: &RubyConst, method: &str) -> Target {
        match (self.constant(constant), method) {
            (Class::Project(class), "new") if class.kind == "class" => self.construct(class),
            (Class::Project(class), _) => found(self.singleton(class, method, 0)),
            (Class::External(name), "new") => ("external", "constructor", Some(name), None),
            (Class::External(name), _) => ("external", "unknown", Some(name), None),
            (Class::Unknown, _) => UNRESOLVED,
        }
    }

    /// `new` on a project class: the class, with its `initialize` if known.
    fn construct(&self, class: &'a RubyClassFacts) -> Target {
        let initialize = match self.instance(class, "initialize", 0) {
            Member::Found(_, method) => Some(method.method_id.clone()),
            _ => None,
        };
        ("project", "constructor", Some(class.id.clone()), initialize)
    }

    fn call(&self, path: &str, site: &RubyCallSite) -> MethodCall {
        let (resolution, dispatch, to_class, to_method) = self.target(site);
        let target_label = if site.target == RubyTarget::Dynamic {
            format!("{} (dynamic)", site.label)
        } else {
            site.label.clone()
        };
        MethodCall {
            from_class: site.from_class.clone(),
            from_method: site.from_method.clone(),
            to_class,
            to_method,
            target_label,
            dispatch: dispatch.into(),
            resolution: resolution.into(),
            file: path.to_string(),
            line: site.line,
            column: site.column,
            code: site.code.clone(),
        }
    }
}

fn found(member: Member) -> Target {
    match member {
        Member::Found(class, method) => (
            "project",
            if method.singleton {
                "static"
            } else {
                "instance"
            },
            Some(class.id.clone()),
            Some(method.method_id.clone()),
        ),
        _ => UNRESOLVED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze(files: &[(&str, &str)]) -> CallAnalysis {
        let dir = tempfile::tempdir().unwrap();
        for (path, source) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source).unwrap();
        }
        analyze_ruby_calls(dir.path(), &Config::default())
    }

    /// (resolution, dispatch, target class, target method name) of the
    /// single call whose source text is `code`.
    fn target(
        report: &CallAnalysis,
        code: &str,
    ) -> (String, String, Option<String>, Option<String>) {
        let found: Vec<_> = report.calls.iter().filter(|c| c.code == code).collect();
        assert_eq!(found.len(), 1, "call {code}: {:#?}", report.calls);
        let call = found[0];
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
    fn project(
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

    const BASE: &str = r#"module Shop
  class Base
    def initialize(id)
      @id = id
    end

    def save
      true
    end

    def self.table
      "base"
    end
  end
end
"#;
    const AUDITED: &str = r#"module Shop
  module Audited
    def audit
      log
    end
  end
end
"#;
    const ORDER: &str = r#"module Shop
  class Order < Base
    include Audited

    def self.build(id)
      new(id)
    end

    def place(repo)
      save
      self.audit
      total
      Order.table
      Order.new(1)
      Billing::Invoice.new
      Tally.count
      JSON.parse("{}")
      repo.find(1)
      send(:total)
      missing
      super
    end

    def total
      Twice.new
    end
  end
end
"#;

    fn shop() -> CallAnalysis {
        analyze(&[
            ("app/models/shop/base.rb", BASE),
            ("app/models/shop/audited.rb", AUDITED),
            ("app/models/shop/order.rb", ORDER),
            (
                "app/models/billing/invoice.rb",
                "module Billing\n  class Invoice\n  end\nend\n",
            ),
            ("lib/tally.rb", "Tally = Struct.new(:count)\n"),
            ("lib/a.rb", "class Twice\nend\n"),
            ("lib/b.rb", "class Twice\nend\n"),
        ])
    }

    #[test]
    fn resolves_self_calls_along_modules_and_superclasses() {
        let report = shop();
        let base = "app/models/shop/base.rb::Shop::Base";
        let order = "app/models/shop/order.rb::Shop::Order";
        assert_eq!(
            target(&report, "save"),
            project("instance", base, Some("save"))
        );
        assert_eq!(
            target(&report, "self.audit"),
            project(
                "instance",
                "app/models/shop/audited.rb::Shop::Audited",
                Some("audit")
            )
        );
        assert_eq!(
            target(&report, "total"),
            project("instance", order, Some("total"))
        );
        // Singleton methods inherit; `new` in a class method is `self.new`.
        assert_eq!(
            target(&report, "Order.table"),
            project("static", base, Some("table"))
        );
        assert_eq!(
            target(&report, "new(id)"),
            project("constructor", order, Some("initialize"))
        );
        assert_eq!(
            target(&report, "Order.new(1)"),
            project("constructor", order, Some("initialize"))
        );
        // No `initialize` of its own anywhere: the class is the target.
        assert_eq!(
            target(&report, "Billing::Invoice.new"),
            project(
                "constructor",
                "app/models/billing/invoice.rb::Billing::Invoice",
                None
            )
        );
        // `super` in `place`: Base has no `place`, Object is the end.
        assert_eq!(target(&report, "super"), unresolved());
        // Missing everywhere: `method_missing` may answer at runtime.
        assert_eq!(target(&report, "missing"), unresolved());
        assert_eq!(target(&report, "repo.find(1)"), unresolved());
        let edge = report.calls.iter().find(|c| c.code == "save").unwrap();
        assert_eq!(edge.from_class, order);
        assert_eq!(edge.file, "app/models/shop/order.rb");
        assert_eq!((edge.line, edge.column), (10, 7));
    }

    #[test]
    fn constants_are_project_external_or_unknown() {
        let report = shop();
        assert_eq!(
            target(&report, "JSON.parse(\"{}\")"),
            (
                "external".into(),
                "unknown".into(),
                Some("JSON".into()),
                None
            )
        );
        // Assigned, not declared: no class to follow.
        assert_eq!(target(&report, "Tally.count"), unresolved());
        // Declared in two files: ambiguous.
        assert_eq!(target(&report, "Twice.new"), unresolved());
        let dynamic = report
            .calls
            .iter()
            .find(|c| c.code == "send(:total)")
            .unwrap();
        assert_eq!(dynamic.resolution, "unresolved");
        assert_eq!(dynamic.target_label, "send (dynamic)");
        let coverage = &report.analyzers[0].diagnostics[0];
        assert!(
            coverage.starts_with("Calls: 15 sites, 7 project, 1 external, 7 unresolved")
                && coverage.contains("1 of them dynamic"),
            "{coverage}"
        );
        assert_eq!(report.status, "partial");
        assert_eq!(report.analyzers[0].language, "ruby");
    }

    #[test]
    fn supertypes_name_project_classes_and_modules() {
        let report = shop();
        let order = report
            .classes
            .iter()
            .find(|c| c.name == "Shop::Order")
            .unwrap();
        assert_eq!(
            order.supertypes,
            [
                "app/models/shop/audited.rb::Shop::Audited",
                "app/models/shop/base.rb::Shop::Base",
            ]
        );
        assert_eq!(order.is_abstract, None);
    }

    #[test]
    fn an_unscanned_file_turns_external_into_unresolved() {
        let report = analyze(&[
            (
                "lib/a.rb",
                "class A\n  def run\n    JSON.parse(1)\n  end\nend\n",
            ),
            ("lib/broken.rb", "class B\n  def x\n    if y\n  end\nend\n"),
        ]);
        assert_eq!(target(&report, "JSON.parse(1)"), unresolved());
        assert!(
            report.diagnostics.iter().any(|d| d.contains("broken.rb")),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn projects_without_ruby_are_not_applicable() {
        let report = analyze(&[("a.py", "x = 1\n")]);
        assert_eq!(report.status, "not_applicable");
        assert!(report.analyzers.is_empty());
    }
}
