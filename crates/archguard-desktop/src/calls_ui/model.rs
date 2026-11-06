//! The call snapshot indexed by id, its labels and the connection filter
//! (`prepareCallModel`, `callClassLabel`, `callRelations` in viewer.js).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use archguard_engine::core::{CallAnalysis, CallClass, CallMethod, MethodCall};

/// Translated fallback names Rust needs; English by default (tests).
#[derive(Clone, Debug, PartialEq)]
pub struct Labels {
    pub unresolved_target: String,
    pub initializer: String,
    pub unreported_language: String,
}

impl Default for Labels {
    fn default() -> Self {
        Self {
            unresolved_target: "Unresolved target".into(),
            initializer: "initializer".into(),
            unreported_language: "Unreported language".into(),
        }
    }
}

/// The analysis with id lookups. The analysis is shared with the other
/// tabs of the document.
pub struct Index {
    pub a: Arc<CallAnalysis>,
    classes: HashMap<String, usize>,
    methods: HashMap<String, usize>,
}

impl Index {
    pub fn new(a: impl Into<Arc<CallAnalysis>>) -> Self {
        let a = a.into();
        let classes = a
            .classes
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.clone(), i))
            .collect();
        let methods = a
            .methods
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id.clone(), i))
            .collect();
        Self {
            a,
            classes,
            methods,
        }
    }

    pub fn class(&self, id: Option<&str>) -> Option<&CallClass> {
        id.and_then(|id| self.classes.get(id))
            .map(|&i| &self.a.classes[i])
    }

    pub fn method(&self, id: Option<&str>) -> Option<&CallMethod> {
        id.and_then(|id| self.methods.get(id))
            .map(|&i| &self.a.methods[i])
    }

    /// The method id when it is declared on `owner` (a known class).
    pub fn owned<'a>(&self, m: &'a Option<String>, owner: Option<&str>) -> Option<&'a str> {
        let id = m.as_deref()?;
        (owner.is_some() && self.method(Some(id)).map(|m| m.class_id.as_str()) == owner)
            .then_some(id)
    }

    /// `callClassLanguage`.
    pub fn language(&self, cls: &CallClass) -> String {
        if let Some(l) = cls.language.as_deref() {
            return if l == "cs" { "csharp" } else { l }.to_string();
        }
        if cls.file.ends_with(".java") {
            return "java".into();
        }
        if cls.file.ends_with(".cs") {
            return "csharp".into();
        }
        match self.a.analyzers.as_slice() {
            [one] => one.language.clone(),
            _ if self.a.java_release > 0 => "java".into(),
            _ => String::new(),
        }
    }

    /// `callClassLabel`.
    pub fn class_label(&self, id: Option<&str>, l: &Labels) -> String {
        if let Some(c) = self.class(id).filter(|c| !c.name.is_empty()) {
            return class_name(c);
        }
        match id {
            Some(id) if id.starts_with("csharp:") => {
                let label = &id["csharp:".len()..];
                match label.rfind("::") {
                    Some(at) if label.starts_with("external:") => label[at + 2..].to_string(),
                    _ => label.to_string(),
                }
            }
            Some(id) if !id.is_empty() => id.to_string(),
            _ => l.unresolved_target.clone(),
        }
    }

    /// `callMethodLabel`.
    pub fn method_label(&self, id: Option<&str>, l: &Labels) -> String {
        match self.method(id).filter(|m| !m.signature.is_empty()) {
            Some(m) => m.signature.clone(),
            None => match id {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => l.initializer.clone(),
            },
        }
    }

    /// `callTargetLabel`.
    pub fn target_label(&self, call: &MethodCall, qualified: bool, l: &Labels) -> String {
        match self.method(call.to_method.as_deref()) {
            Some(m) if qualified => {
                format!("{}#{}", self.class_label(Some(&m.class_id), l), m.signature)
            }
            Some(m) => m.signature.clone(),
            None if !call.target_label.is_empty() => call.target_label.clone(),
            None => l.unresolved_target.clone(),
        }
    }

    /// A method's owner and signature, as the rankings and cycles show it.
    pub fn qualified(&self, kind: ElementKind, id: &str, l: &Labels) -> String {
        match (kind, self.method(Some(id))) {
            (ElementKind::Method, Some(m)) => {
                format!("{}#{}", self.class_label(Some(&m.class_id), l), m.signature)
            }
            _ => self.class_label(Some(id), l),
        }
    }

    /// Languages of the analyzers and classes, sorted.
    pub fn languages(&self) -> Vec<String> {
        let mut set: BTreeSet<String> = self
            .a
            .analyzers
            .iter()
            .map(|a| a.language.clone())
            .collect();
        set.extend(self.a.classes.iter().map(|c| self.language(c)));
        set.remove("");
        set.into_iter().collect()
    }

    pub fn in_language(&self, cls: &CallClass, language: Option<&str>) -> bool {
        language.is_none_or(|l| self.language(cls) == l)
    }

    /// Packages of the classes in a language scope, sorted.
    pub fn packages(&self, language: Option<&str>) -> Vec<String> {
        let set: BTreeSet<&str> = self
            .a
            .classes
            .iter()
            .filter(|c| self.in_language(c, language))
            .map(|c| c.package.as_str())
            .collect();
        set.into_iter().map(str::to_string).collect()
    }

    /// Without one resolved project target the web opens with all targets.
    pub fn default_resolution(&self) -> Resolution {
        let calls = &self.a.calls;
        if !calls.is_empty() && !calls.iter().any(|c| c.resolution == "project") {
            Resolution::All
        } else {
            Resolution::Project
        }
    }
}

/// The shown name of a type/module: module-level code ("(module)", one per
/// file of a script language) goes by its file name.
pub fn class_name(c: &CallClass) -> String {
    match c.file.rsplit('/').next() {
        Some(file) if file_module(c) && !file.is_empty() => file.to_string(),
        _ => c.name.clone(),
    }
}

/// Module-level code that [`class_name`] names by its file.
pub fn file_module(c: &CallClass) -> bool {
    c.kind == "module" && c.name == "(module)"
}

/// The web's language names (`callLanguageLabel`).
pub fn language_label(code: &str, l: &Labels) -> String {
    let name = match code {
        "go" => "Go",
        "java" => "Java",
        "csharp" | "cs" => "C#",
        "ts" | "typescript" => "TypeScript",
        "js" | "javascript" => "JavaScript",
        "py" | "python" => "Python",
        "php" => "PHP",
        "rust" => "Rust",
        "" => return l.unreported_language.clone(),
        other => other,
    };
    name.to_string()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Resolution {
    #[default]
    Project,
    External,
    Unresolved,
    All,
}

impl Resolution {
    pub fn matches(self, r: &str) -> bool {
        match self {
            Self::Project => r == "project",
            Self::External => r == "external",
            Self::Unresolved => r == "unresolved",
            Self::All => true,
        }
    }

    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Self::External,
            2 => Self::Unresolved,
            3 => Self::All,
            _ => Self::Project,
        }
    }

    pub fn index(self) -> i32 {
        self as i32
    }
}

/// The toolbar filters. `package: None` is "All packages".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub language: Option<String>,
    pub package: Option<String>,
    pub query: String,
    pub resolution: Resolution,
    pub internal: bool,
}

impl Filter {
    /// Identity of the filter scope (the web's `callRelationsCache.scope`).
    pub fn scope(&self) -> String {
        format!(
            "{:?}\u{0}{:?}\u{0}{}\u{0}{:?}\u{0}{}",
            self.language, self.package, self.query, self.resolution, self.internal
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementKind {
    Class,
    Method,
}

/// A selected type/module or function/method (`callUi.element`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    pub kind: ElementKind,
    pub id: String,
}

impl Element {
    /// `callMatchesElement`.
    pub fn matches(&self, c: &MethodCall) -> bool {
        let id = Some(self.id.as_str());
        match self.kind {
            ElementKind::Class => Some(c.from_class.as_str()) == id || c.to_class.as_deref() == id,
            ElementKind::Method => c.from_method.as_deref() == id || c.to_method.as_deref() == id,
        }
    }
}

/// One type/module connection with its call sites (indices into `calls`).
#[derive(Clone, Debug, PartialEq)]
pub struct Relation {
    pub key: String,
    pub from: String,
    pub to: Option<String>,
    pub label: String,
    pub resolution: String,
    pub calls: Vec<usize>,
    /// The function/method or type edge of a cycle relation.
    pub cycle: Option<(String, String)>,
}

impl Relation {
    /// Distinct target methods (`to_method || target_label`).
    pub fn target_methods(&self, a: &CallAnalysis) -> usize {
        self.calls
            .iter()
            .map(|&i| {
                let c = &a.calls[i];
                c.to_method
                    .as_deref()
                    .filter(|m| !m.is_empty())
                    .unwrap_or(&c.target_label)
            })
            .collect::<BTreeSet<_>>()
            .len()
    }
}

/// `JSON.stringify` of a string.
pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `JSON.stringify` of an array of strings.
pub fn json_list<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let parts: Vec<String> = items.into_iter().map(json_str).collect();
    format!("[{}]", parts.join(","))
}

fn passes(idx: &Index, f: &Filter, c: &MethodCall, q: &str, l: &Labels) -> bool {
    let Some(from) = idx.class(Some(&c.from_class)) else {
        return false;
    };
    let to = idx.class(c.to_class.as_deref());
    if let Some(p) = &f.package
        && from.package != *p
        && to.is_none_or(|t| t.package != *p)
    {
        return false;
    }
    if !idx.in_language(from, f.language.as_deref()) || !f.resolution.matches(&c.resolution) {
        return false;
    }
    if !f.internal && Some(c.from_class.as_str()) == c.to_class.as_deref() {
        return false;
    }
    q.is_empty()
        || [
            from.name.clone(),
            idx.class_label(c.to_class.as_deref(), l),
            idx.method_label(c.from_method.as_deref(), l),
            idx.method_label(c.to_method.as_deref(), l),
            c.target_label.clone(),
        ]
        .join(" ")
        .to_lowercase()
        .contains(q)
}

/// `callRelations({ignoreElement: true})`: matching sites grouped by
/// caller, target and resolution, sorted by caller and target name.
pub fn relations(idx: &Index, f: &Filter, l: &Labels) -> Vec<Relation> {
    let q = f.query.trim().to_lowercase();
    let mut groups: Vec<Relation> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for (i, c) in idx.a.calls.iter().enumerate() {
        if !passes(idx, f, c, &q, l) {
            continue;
        }
        let to = c
            .to_class
            .as_deref()
            .filter(|t| !t.is_empty())
            .unwrap_or(&c.target_label);
        let key = json_list([c.from_class.as_str(), to, c.resolution.as_str()]);
        let slot = *at.entry(key.clone()).or_insert_with(|| {
            groups.push(Relation {
                key,
                from: c.from_class.clone(),
                to: c.to_class.clone(),
                label: c.target_label.clone(),
                resolution: c.resolution.clone(),
                calls: Vec::new(),
                cycle: None,
            });
            groups.len() - 1
        });
        groups[slot].calls.push(i);
    }
    let names: Vec<(String, String)> = groups
        .iter()
        .map(|r| {
            (
                idx.class_label(Some(&r.from), l),
                idx.class_label(r.to.as_deref(), l),
            )
        })
        .collect();
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|&a, &b| {
        names[a]
            .cmp(&names[b])
            .then_with(|| groups[a].key.cmp(&groups[b].key))
    });
    let mut slots: Vec<Option<Relation>> = groups.into_iter().map(Some).collect();
    order.into_iter().filter_map(|i| slots[i].take()).collect()
}

/// The package with the most connections under `f` in all packages (ties:
/// the first by name); `None` when no package has one. A connection counts
/// for the caller's and the target's package, as the package filter does.
pub fn busiest_package(idx: &Index, f: &Filter, l: &Labels) -> Option<String> {
    let all = Filter {
        package: None,
        ..f.clone()
    };
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for r in relations(idx, &all, l) {
        let from = idx.class(Some(&r.from)).map(|c| c.package.as_str());
        let to = idx.class(r.to.as_deref()).map(|c| c.package.as_str());
        for p in from.into_iter().chain(to.filter(|&t| Some(t) != from)) {
            *counts.entry(p).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(p, _)| p.to_string())
}

/// The relations restricted to the sites of an element.
pub fn focus(rels: &[Relation], a: &CallAnalysis, el: Option<&Element>) -> Vec<Relation> {
    let Some(el) = el else {
        return rels.to_vec();
    };
    rels.iter()
        .filter_map(|r| {
            let calls: Vec<usize> = r
                .calls
                .iter()
                .copied()
                .filter(|&i| el.matches(&a.calls[i]))
                .collect();
            (!calls.is_empty()).then(|| Relation { calls, ..r.clone() })
        })
        .collect()
}

#[cfg(test)]
pub mod fixture {
    //! A small Java-like call snapshot: blue.Blue ⇄ red.Red form a cycle.
    use archguard_engine::core::{
        CallAnalysis, CallAnalyzer, CallClass, CallMethod, MethodCall, MethodComplexity,
    };

    pub fn class(id: &str, package: &str) -> CallClass {
        CallClass {
            language: Some("java".into()),
            id: id.into(),
            name: id.into(),
            package: package.into(),
            file: format!("{}.java", id.replace('.', "/")),
            graph_node: format!("{}.java", id.replace('.', "/")),
            line: 1,
            kind: "class".into(),
            is_abstract: None,
            visibility: None,
            supertypes: Vec::new(),
            supertype_sites: None,
            declaration_files: None,
        }
    }

    pub fn method(class: &str, sig: &str, cyclomatic: u32) -> CallMethod {
        CallMethod {
            id: format!("{class}#{sig}"),
            class_id: class.into(),
            name: sig.trim_end_matches("()").into(),
            signature: sig.into(),
            file: format!("{}.java", class.replace('.', "/")),
            line: 3,
            end_line: None,
            is_static: false,
            complexity: Some(MethodComplexity {
                cyclomatic,
                max_nesting: cyclomatic.saturating_sub(1),
                definition: "ast-decisions-v1".into(),
            }),
        }
    }

    pub fn call(from: &str, to: Option<&str>, resolution: &str, line: u32) -> MethodCall {
        let (fc, _) = from.split_once('#').unwrap();
        let to_class = to.map(|t| t.split_once('#').unwrap().0.to_string());
        MethodCall {
            from_class: fc.into(),
            from_method: Some(from.into()),
            to_class,
            to_method: to.map(str::to_string),
            target_label: to.unwrap_or("dyn.call").into(),
            dispatch: if to.is_some_and(|t| t.contains("<init>")) {
                "constructor"
            } else {
                "instance"
            }
            .into(),
            resolution: resolution.into(),
            file: format!("{}.java", fc.replace('.', "/")),
            line,
            column: 5,
            code: format!("call{line}()"),
        }
    }

    pub fn analysis() -> CallAnalysis {
        CallAnalysis {
            status: "ready".into(),
            java_release: 0,
            analyzers: vec![CallAnalyzer {
                language: "java".into(),
                backend: "javac".into(),
                status: "ready".into(),
                files: 4,
                version: Some("21".into()),
                project: None,
                diagnostics: Vec::new(),
            }],
            files: 4,
            classes: vec![
                class("blue.Blue", "blue"),
                class("blue.Util", "blue"),
                class("red.Red", "red"),
                class("green.Leaf", "green"),
            ],
            methods: vec![
                method("blue.Blue", "name()", 2),
                method("blue.Util", "hint()", 1),
                method("red.Red", "describe()", 4),
                method("green.Leaf", "text()", 1),
            ],
            calls: vec![
                call("blue.Blue#name()", Some("red.Red#describe()"), "project", 7),
                call("blue.Blue#name()", Some("red.Red#<init>()"), "project", 8),
                call("red.Red#describe()", Some("blue.Blue#name()"), "project", 9),
                call("blue.Util#hint()", Some("green.Leaf#text()"), "project", 10),
                call("blue.Util#hint()", None, "unresolved", 11),
                call("blue.Blue#name()", Some("blue.Blue#name()"), "project", 12),
            ],
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> Index {
        Index::new(fixture::analysis())
    }

    #[test]
    fn labels_fall_back_like_the_web() {
        let idx = index();
        let l = Labels::default();
        assert_eq!(idx.class_label(Some("red.Red"), &l), "red.Red");
        assert_eq!(idx.class_label(None, &l), "Unresolved target");
        assert_eq!(
            idx.class_label(Some("csharp:external:System::Console"), &l),
            "Console"
        );
        assert_eq!(
            idx.method_label(Some("red.Red#<init>()"), &l),
            "red.Red#<init>()"
        );
        assert_eq!(idx.method_label(None, &l), "initializer");
        assert_eq!(idx.languages(), vec!["java"]);
        assert_eq!(idx.packages(None), vec!["blue", "green", "red"]);
        assert_eq!(language_label("csharp", &l), "C#");
    }

    /// Module-level code of a script language is one "(module)" per file;
    /// the file name tells them apart.
    #[test]
    fn module_pseudo_classes_are_named_by_their_file() {
        let mut a = fixture::analysis();
        let mut module = fixture::class("api/http.py::(module)", "api");
        module.name = "(module)".into();
        module.kind = "module".into();
        module.file = "api/http.py".into();
        a.classes.push(module.clone());
        let idx = Index::new(a);
        let l = Labels::default();
        assert_eq!(idx.class_label(Some(&module.id), &l), "http.py");
        assert_eq!(class_name(&module), "http.py");
        assert_eq!(class_name(&idx.a.classes[0]), "blue.Blue");
    }

    #[test]
    fn relations_group_filter_and_sort() {
        let idx = index();
        let l = Labels::default();
        let f = Filter {
            package: Some("blue".into()),
            ..Default::default()
        };
        let rels = relations(&idx, &f, &l);
        let names: Vec<(String, Option<String>, usize)> = rels
            .iter()
            .map(|r| (r.from.clone(), r.to.clone(), r.calls.len()))
            .collect();
        // Project targets only, no within-class call, sorted by names.
        assert_eq!(
            names,
            vec![
                ("blue.Blue".into(), Some("red.Red".into()), 2),
                ("blue.Util".into(), Some("green.Leaf".into()), 1),
                ("red.Red".into(), Some("blue.Blue".into()), 1),
            ]
        );
        assert_eq!(rels[0].key, r#"["blue.Blue","red.Red","project"]"#);
        assert_eq!(rels[0].target_methods(&idx.a), 2);

        let all = Filter {
            package: None,
            resolution: Resolution::All,
            internal: true,
            ..Default::default()
        };
        assert_eq!(relations(&idx, &all, &l).len(), 5);
        let q = Filter {
            query: " LEAF ".into(),
            package: None,
            ..Default::default()
        };
        assert_eq!(relations(&idx, &q, &l).len(), 1);
    }

    #[test]
    fn focus_keeps_the_sites_of_an_element() {
        let idx = index();
        let l = Labels::default();
        let f = Filter::default();
        let rels = relations(&idx, &f, &l);
        let el = Element {
            kind: ElementKind::Method,
            id: "red.Red#describe()".into(),
        };
        let focused = focus(&rels, &idx.a, Some(&el));
        assert_eq!(focused.len(), 2);
        assert!(focused.iter().all(|r| r.calls.len() == 1));
    }

    #[test]
    fn default_resolution_opens_all_without_project_targets() {
        let mut a = fixture::analysis();
        assert_eq!(
            Index::new(a.clone()).default_resolution(),
            Resolution::Project
        );
        a.calls.retain(|c| c.resolution != "project");
        assert_eq!(Index::new(a).default_resolution(), Resolution::All);
    }

    #[test]
    fn json_matches_stringify() {
        assert_eq!(json_list(["a\"b", "c\\"]), r#"["a\"b","c\\"]"#);
    }
}
