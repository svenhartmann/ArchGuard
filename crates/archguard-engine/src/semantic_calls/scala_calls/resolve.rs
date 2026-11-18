//! The target of each Scala call site: a project def, `apply` or
//! constructor, an external name, or unresolved. Names bind in Scala's
//! order of a static reading: members of the enclosing classes and their
//! project supertypes, top-level defs, then types (nested, imported, same
//! package, wildcard imports with a single candidate). A class and its
//! companion object share one id; `Name(…)` on an object is its `apply`,
//! on a concrete class its constructor.

use std::collections::{HashMap, HashSet};

use archguard_core::{CallMethod, MethodCall};
use archguard_extract::scala::{ScalaCallSite, ScalaClassFacts, ScalaSymbols, ScalaTarget};

/// (resolution, dispatch, to_class, to_method) of one call.
type Target = (&'static str, &'static str, Option<String>, Option<String>);
const UNRESOLVED: Target = ("unresolved", "unknown", None, None);
/// How deep the resolver follows supertypes.
const MAX_DEPTH: usize = 8;

/// The call edges of all scanned files. `jvm` holds the packages of the
/// project's Java and Kotlin files; `None` when they are unknown.
pub(super) fn resolve(
    files: &[(String, ScalaSymbols)],
    methods: &[CallMethod],
    jvm: Option<&HashSet<String>>,
) -> Vec<MethodCall> {
    let ix = Index::new(files, methods, jvm);
    let mut calls = Vec::new();
    for (file, (path, symbols)) in files.iter().enumerate() {
        for site in &symbols.sites {
            let (resolution, dispatch, to_class, to_method) = ix.target(file, site);
            calls.push(MethodCall {
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
            });
        }
    }
    calls
}

/// A class-like declaration and the index of its file.
type Decl<'a> = (&'a ScalaClassFacts, usize);

struct Index<'a> {
    files: &'a [(String, ScalaSymbols)],
    /// Qualified name to the class and companion object declaring it.
    by_name: HashMap<String, Vec<Decl<'a>>>,
    /// Class id to its declarations and qualified name.
    by_id: HashMap<&'a str, (Vec<Decl<'a>>, String)>,
    methods: HashMap<&'a str, Vec<&'a CallMethod>>,
    /// Qualified top-level def name to its overloads.
    top: HashMap<String, Vec<&'a CallMethod>>,
    /// Scala, Java and Kotlin packages of the project and all their parent
    /// packages; `None` when the Java and Kotlin packages are unknown.
    packages: Option<HashSet<String>>,
}

fn qualify(package: &str, name: &str) -> String {
    if package.is_empty() {
        name.into()
    } else {
        format!("{package}.{name}")
    }
}

impl<'a> Index<'a> {
    fn new(
        files: &'a [(String, ScalaSymbols)],
        methods: &'a [CallMethod],
        jvm: Option<&HashSet<String>>,
    ) -> Self {
        let mut ix = Index {
            files,
            by_name: HashMap::new(),
            by_id: HashMap::new(),
            methods: HashMap::new(),
            top: HashMap::new(),
            packages: jvm.cloned(),
        };
        for method in methods {
            ix.methods.entry(&method.class_id).or_default().push(method);
        }
        for (file, (_, symbols)) in files.iter().enumerate() {
            if let Some(packages) = &mut ix.packages {
                packages.insert(symbols.package.clone());
            }
            for facts in &symbols.classes {
                ix.add(file, facts);
            }
        }
        if let Some(packages) = &mut ix.packages {
            *packages = with_parents(packages);
        }
        ix
    }

    fn add(&mut self, file: usize, facts: &'a ScalaClassFacts) {
        let package = &self.files[file].1.package;
        if facts.kind == "module" {
            for method in self.methods.get(facts.id.as_str()).into_iter().flatten() {
                let name = qualify(package, &method.name);
                self.top.entry(name).or_default().push(method);
            }
            return;
        }
        let name = match facts.outer.as_deref().and_then(|o| self.by_id.get(o)) {
            Some((_, outer)) => format!("{outer}.{}", facts.name),
            None => qualify(package, &facts.name),
        };
        self.by_name
            .entry(name.clone())
            .or_default()
            .push((facts, file));
        let entry = self.by_id.entry(&facts.id).or_insert((Vec::new(), name));
        entry.0.push((facts, file));
    }

    fn target(&self, file: usize, site: &ScalaCallSite) -> Target {
        let class = self
            .by_id
            .get(site.from_class.as_str())
            .map(|(_, q)| q.as_str());
        match &site.target {
            ScalaTarget::Name(name) => self.name_call(file, class, name),
            ScalaTarget::Path(segments) => self.path_call(file, class, segments),
            ScalaTarget::New(segments) => self.new_call(file, class, segments),
            ScalaTarget::This(name) => method(self.members(&site.from_class, name, 0)),
            ScalaTarget::Super { qualifier, name } => {
                self.super_call(&site.from_class, qualifier.as_deref(), name)
            }
            ScalaTarget::Unresolved => UNRESOLVED,
        }
    }

    /// `name(…)`: a member, a top-level def, an `apply` or a constructor.
    fn name_call(&self, file: usize, class: Option<&str>, name: &str) -> Target {
        let mut scope = class.map(str::to_string);
        while let Some(qualified) = scope {
            let id = self
                .by_name
                .get(&qualified)
                .map(|decls| decls[0].0.id.as_str());
            let found = id.map(|id| self.members(id, name, 0)).unwrap_or_default();
            if !found.is_empty() {
                return method(found);
            }
            scope = qualified
                .rsplit_once('.')
                .map(|(outer, _)| outer.to_string());
            scope = scope.filter(|outer| self.by_name.contains_key(outer));
        }
        if let Some(found) = self.top_level(file, name) {
            return method(found);
        }
        match self.type_named(file, class, name) {
            Some(qualified) => self.create(&qualified),
            None => self.external(file, name, &[]),
        }
    }

    /// `T.m(…)`, `T.Nested(…)` or `a.b.T.m(…)`.
    fn path_call(&self, file: usize, class: Option<&str>, segments: &[String]) -> Target {
        let (last, middle) = segments.split_last().expect("a path has two segments");
        let full = segments.join(".");
        if self.by_name.contains_key(&full) {
            // `a.b.C(…)`: the fully qualified type itself.
            return self.create(&full);
        }
        let leading = match self.type_named(file, class, &segments[0]) {
            Some(found) => Some((found, 1)),
            None => (2..segments.len())
                .rev()
                .map(|k| (segments[..k].join("."), k))
                .find(|(prefix, _)| self.by_name.contains_key(prefix)),
        };
        let Some((mut qualified, used)) = leading else {
            return self.external(file, &segments[0], &middle[1..]);
        };
        for segment in &middle[used..] {
            qualified = format!("{qualified}.{segment}");
            if !self.by_name.contains_key(&qualified) {
                // A value inside the object: its type is unknown.
                return UNRESOLVED;
            }
        }
        let nested = format!("{qualified}.{last}");
        if self.by_name.contains_key(&nested) {
            return self.create(&nested);
        }
        let id = &self.by_name[&qualified][0].0.id;
        method(self.statics(id, last))
    }

    /// `new T(…)`: the constructor of a concrete project class.
    fn new_call(&self, file: usize, class: Option<&str>, segments: &[String]) -> Target {
        let found = match segments {
            [name] => self.type_named(file, class, name),
            _ => Some(segments.join(".")).filter(|q| self.by_name.contains_key(q)),
        };
        match found {
            Some(qualified) => self.constructor(&qualified).unwrap_or(UNRESOLVED),
            // `new T(…)` has no receiver segments.
            None => {
                let receiver = segments.get(1..segments.len() - 1).unwrap_or_default();
                self.external(file, &segments[0], receiver)
            }
        }
    }

    /// `super.m(…)` names the one supertype chain that has `m`;
    /// `super[T].m(…)` the supertype `T`.
    fn super_call(&self, class_id: &str, qualifier: Option<&str>, name: &str) -> Target {
        let mut found = Vec::new();
        for (written, supertype) in self.supertypes(class_id) {
            if qualifier.is_none_or(|q| written.rsplit('.').next() == Some(q)) {
                found.extend(self.members(supertype, name, 1));
            }
        }
        method(found)
    }

    /// `T(…)` on the type `qualified`: the object's `apply`, else the
    /// constructor of a concrete class.
    fn create(&self, qualified: &str) -> Target {
        let decls = &self.by_name[qualified];
        let id = &decls[0].0.id;
        if decls.iter().any(|(facts, _)| facts.kind == "object") {
            let apply = self.statics(id, "apply");
            if !apply.is_empty() {
                return method(apply);
            }
        }
        self.constructor(qualified).unwrap_or(UNRESOLVED)
    }

    fn constructor(&self, qualified: &str) -> Option<Target> {
        let (facts, _) = self.by_name[qualified]
            .iter()
            .find(|(facts, _)| facts.kind == "class")?;
        let concrete = !facts.modifiers.iter().any(|m| m == "abstract");
        concrete.then(|| ("project", "constructor", Some(facts.id.clone()), None))
    }

    /// Defs named `name` of the class `id` or, if it has none, of its
    /// project supertypes.
    fn members(&self, id: &str, name: &str, depth: usize) -> Vec<&'a CallMethod> {
        let own: Vec<_> = self
            .methods
            .get(id)
            .into_iter()
            .flatten()
            .filter(|m| m.name == name)
            .copied()
            .collect();
        if !own.is_empty() || depth >= MAX_DEPTH {
            return own;
        }
        self.supertypes(id)
            .into_iter()
            .flat_map(|(_, supertype)| self.members(supertype, name, depth + 1))
            .collect()
    }

    /// Defs named `name` of the object `id`.
    fn statics(&self, id: &str, name: &str) -> Vec<&'a CallMethod> {
        let found = self.methods.get(id).into_iter().flatten();
        found
            .filter(|m| m.name == name && m.is_static)
            .copied()
            .collect()
    }

    /// The project supertypes of the class `id`, as written and by id.
    fn supertypes(&self, id: &str) -> Vec<(&'a str, &'a str)> {
        let Some((decls, qualified)) = self.by_id.get(id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (facts, file) in decls {
            for written in &facts.supertypes {
                let found = self
                    .type_named(*file, Some(qualified), written)
                    .or_else(|| Some(written.clone()).filter(|w| self.by_name.contains_key(w)));
                if let Some(found) = found {
                    out.push((written.as_str(), self.by_name[&found][0].0.id.as_str()));
                }
            }
        }
        out
    }

    /// Top-level defs named `name` visible in `file`.
    fn top_level(&self, file: usize, name: &str) -> Option<Vec<&'a CallMethod>> {
        let symbols = &self.files[file].1;
        let mut candidates = vec![qualify(&symbols.package, name)];
        for import in &symbols.imports {
            match import.name.as_deref() {
                Some(bound) if bound == name => candidates.insert(0, import.path.clone()),
                None => candidates.push(qualify(&import.path, name)),
                _ => {}
            }
        }
        candidates.iter().find_map(|c| self.top.get(c)).cloned()
    }

    /// The qualified name of the project type `name` in `file` inside the
    /// class `class`: nested, imported, same package or wildcard import.
    fn type_named(&self, file: usize, class: Option<&str>, name: &str) -> Option<String> {
        let mut scope = class;
        while let Some(qualified) = scope {
            let nested = format!("{qualified}.{name}");
            if self.by_name.contains_key(&nested) {
                return Some(nested);
            }
            scope = qualified.rsplit_once('.').map(|(outer, _)| outer);
        }
        let symbols = &self.files[file].1;
        let (head, rest) = name.split_once('.').unwrap_or((name, ""));
        let suffix = |q: String| {
            if rest.is_empty() {
                q
            } else {
                format!("{q}.{rest}")
            }
        };
        let explicit = symbols
            .imports
            .iter()
            .find(|i| i.name.as_deref() == Some(head))
            .map(|i| suffix(i.path.clone()));
        if let Some(explicit) = explicit {
            return Some(explicit).filter(|q| self.by_name.contains_key(q));
        }
        let local = suffix(qualify(&symbols.package, head));
        if self.by_name.contains_key(&local) {
            return Some(local);
        }
        let mut wildcard = symbols
            .imports
            .iter()
            .filter(|i| i.name.is_none())
            .map(|i| suffix(qualify(&i.path, head)))
            .filter(|q| self.by_name.contains_key(q));
        match (wildcard.next(), wildcard.next()) {
            (Some(found), None) => Some(found),
            _ => None,
        }
    }

    /// A name the file imports explicitly from outside the project, with
    /// the `receiver` segments after it; unresolved for everything else.
    fn external(&self, file: usize, head: &str, receiver: &[String]) -> Target {
        let Some(packages) = &self.packages else {
            return UNRESOLVED;
        };
        let imported = self.files[file]
            .1
            .imports
            .iter()
            .find(|i| i.name.as_deref() == Some(head));
        let Some(import) = imported else {
            return UNRESOLVED;
        };
        let package = import.path.rsplit_once('.').map_or("", |(p, _)| p);
        if packages.contains(package) {
            return UNRESOLVED;
        }
        let name = std::iter::once(import.path.clone())
            .chain(receiver.iter().cloned())
            .collect::<Vec<_>>()
            .join(".");
        ("external", "unknown", Some(name), None)
    }
}

/// Each package and every prefix of it that ends before a `.`.
fn with_parents(packages: &HashSet<String>) -> HashSet<String> {
    let mut out = HashSet::new();
    for package in packages {
        out.extend(
            package
                .match_indices('.')
                .map(|(i, _)| package[..i].to_string()),
        );
        out.insert(package.clone());
    }
    out
}

/// A unique def; overloads are not told apart.
fn method(found: Vec<&CallMethod>) -> Target {
    match found.as_slice() {
        [method] => (
            "project",
            if method.is_static {
                "static"
            } else {
                "instance"
            },
            Some(method.class_id.clone()),
            Some(method.id.clone()),
        ),
        _ => UNRESOLVED,
    }
}
