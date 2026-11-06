//! The Kotlin declarations of all scanned files, indexed for name lookup.

use std::collections::{HashMap, HashSet};

use archguard_core::CallMethod;
use archguard_extract::kotlin::{KotlinClassFacts, KotlinFunctionFacts, KotlinSymbols};

/// Java types of the project: Kotlin calls to them stay unresolved, but a
/// name declared there is never external.
#[derive(Debug, Default)]
pub(super) struct JavaTypes {
    /// `package.FileStem`.
    pub(super) types: HashSet<String>,
    pub(super) names: HashSet<String>,
    pub(super) packages: HashSet<String>,
}

/// A Kotlin class-like declaration with its file and qualified name.
pub(super) struct ClassInfo<'a> {
    pub(super) facts: &'a KotlinClassFacts,
    pub(super) file: usize,
}

/// What a name binds to.
#[derive(Clone, Copy)]
pub(super) enum Hit<'a> {
    Method(&'a CallMethod),
    Class(&'a ClassInfo<'a>),
    /// Not declared in the fully scanned project; the qualified name an
    /// explicit import gives, if any.
    External(Option<&'a str>),
    /// Declared but ambiguous, or possibly declared where the scanner does
    /// not look (unknown supertypes, Java, unscanned files).
    Unknown,
    /// Not found at this scope level.
    Missing,
}

impl Hit<'_> {
    pub(super) fn is_missing(&self) -> bool {
        matches!(self, Hit::Missing)
    }
}

pub(super) struct Index<'a> {
    pub(super) files: &'a [(String, KotlinSymbols)],
    pub(super) classes: HashMap<&'a str, ClassInfo<'a>>,
    /// Qualified class name to class ids.
    pub(super) by_name: HashMap<String, Vec<&'a str>>,
    /// Outer class id to the ids of its nested classes.
    pub(super) nested: HashMap<&'a str, Vec<&'a str>>,
    pub(super) methods: HashMap<&'a str, Vec<&'a CallMethod>>,
    pub(super) method_ids: HashMap<&'a str, &'a CallMethod>,
    /// Qualified top-level function name to its overloads.
    pub(super) functions: HashMap<String, Vec<&'a KotlinFunctionFacts>>,
    /// Qualified names of top-level properties and type aliases.
    pub(super) values: HashSet<String>,
    /// Every simple name a Kotlin or Java declaration uses.
    pub(super) declared: HashSet<&'a str>,
    pub(super) packages: HashSet<&'a str>,
    pub(super) java: JavaTypes,
    pub(super) complete: bool,
}

/// `package.name`, or `name` in the default package.
pub(crate) fn qualify(package: &str, name: &str) -> String {
    if package.is_empty() {
        name.into()
    } else {
        format!("{package}.{name}")
    }
}

impl<'a> Index<'a> {
    pub(super) fn new(
        files: &'a [(String, KotlinSymbols)],
        methods: &'a [CallMethod],
        java: JavaTypes,
        complete: bool,
    ) -> Self {
        let mut index = Index {
            files,
            classes: HashMap::new(),
            by_name: HashMap::new(),
            nested: HashMap::new(),
            methods: HashMap::new(),
            method_ids: methods.iter().map(|m| (m.id.as_str(), m)).collect(),
            functions: HashMap::new(),
            values: HashSet::new(),
            declared: HashSet::new(),
            packages: HashSet::new(),
            java,
            complete,
        };
        for method in methods {
            let owner = index.methods.entry(method.class_id.as_str()).or_default();
            owner.push(method);
        }
        for (file, (_, symbols)) in files.iter().enumerate() {
            index.add_file(file, symbols);
        }
        index
    }

    fn add_file(&mut self, file: usize, symbols: &'a KotlinSymbols) {
        self.packages.insert(&symbols.package);
        // Outer classes come before their nested classes.
        let mut qualified: HashMap<&str, String> = HashMap::new();
        for class in &symbols.classes {
            let name = match class.outer.as_deref().and_then(|o| qualified.get(o)) {
                Some(outer) => format!("{outer}.{}", class.name),
                None => qualify(&symbols.package, &class.name),
            };
            self.by_name
                .entry(name.clone())
                .or_default()
                .push(&class.id);
            qualified.insert(&class.id, name);
            if let Some(outer) = &class.outer {
                self.nested.entry(outer).or_default().push(&class.id);
            }
            self.declared.insert(&class.name);
            self.classes
                .insert(&class.id, ClassInfo { facts: class, file });
        }
        for function in &symbols.functions {
            let name = qualify(&symbols.package, &function.name);
            self.functions.entry(name).or_default().push(function);
            self.declared.insert(&function.name);
        }
        for value in &symbols.values {
            self.values.insert(qualify(&symbols.package, value));
            self.declared.insert(value);
        }
    }

    pub(super) fn symbols(&self, file: usize) -> &'a KotlinSymbols {
        &self.files[file].1
    }

    /// A name some project declaration uses: never external.
    pub(super) fn is_declared(&self, name: &str) -> bool {
        self.declared.contains(name) || self.java.names.contains(name)
    }
}

/// The hits of one scope level: one target (possibly reached twice), none,
/// or unknown.
pub(super) fn combine<'i>(hits: impl IntoIterator<Item = Hit<'i>>) -> Hit<'i> {
    let mut found = Hit::Missing;
    for hit in hits {
        found = match (found, hit) {
            (found, Hit::Missing) => found,
            (Hit::Missing, hit) => hit,
            (Hit::Method(a), Hit::Method(b)) if a.id == b.id => found,
            (Hit::Class(a), Hit::Class(b)) if a.facts.id == b.facts.id => found,
            _ => return Hit::Unknown,
        };
    }
    found
}

/// The class with the qualified name `name`, if exactly one.
pub(super) fn unique_class<'i>(ix: &'i Index<'i>, name: &str) -> Option<&'i ClassInfo<'i>> {
    match ix.by_name.get(name).map(Vec::as_slice) {
        Some([id]) => ix.classes.get(id),
        _ => None,
    }
}
