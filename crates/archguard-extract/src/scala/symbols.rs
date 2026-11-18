//! What the Scala scanner hands to the engine's call resolver: the package,
//! imports, class-like declarations and call sites of one file, with names
//! as written. Which declaration a name binds to depends on the whole
//! project, so the engine resolves them (`archguard_engine` Scala calls).

/// The declarations and call sites of one file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScalaSymbols {
    /// False when the file could not be scanned (unbalanced brackets).
    pub measured: bool,
    /// The dotted package of all `package` clauses, empty for none.
    pub package: String,
    pub imports: Vec<ScalaImport>,
    pub classes: Vec<ScalaClassFacts>,
    pub sites: Vec<ScalaCallSite>,
}

/// One imported name or wildcard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalaImport {
    /// `a.b.C` for a name, `a.b` for a wildcard (`_`, `*`, `given`).
    pub path: String,
    /// The name the import binds (the alias of `C => D` / `C as D`);
    /// `None` for a wildcard.
    pub name: Option<String>,
}

/// A class, case class, trait, object, enum, given instance or the file's
/// `(module)` of top-level definitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalaClassFacts {
    /// The `CallClass` id.
    pub id: String,
    pub name: String,
    /// `class`, `trait`, `object`, `enum` or `module`.
    pub kind: String,
    /// The enclosing class id of a nested declaration.
    pub outer: Option<String>,
    /// Supertypes as written after `extends`/`with`, type arguments dropped.
    pub supertypes: Vec<String>,
    /// `abstract`, `sealed`, `case` and similar modifiers.
    pub modifiers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalaCallSite {
    pub from_class: String,
    pub from_method: Option<String>,
    pub target: ScalaTarget,
    pub label: String,
    pub line: u32,
    pub column: u32,
    pub code: String,
}

/// What the source says a call targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalaTarget {
    /// `f(…)` or `C(…)`: a method, a top-level function or a constructor
    /// (`apply`).
    Name(String),
    /// `a.b.C.m(…)`, `O.m(…)` or `a.b.C(…)`: the segments, the called name last.
    Path(Vec<String>),
    /// `new a.b.C(…)` or `new C`: the segments of the class name.
    New(Vec<String>),
    /// `this.m(…)`.
    This(String),
    /// `super.m(…)`, or `super[T].m(…)` with the qualifier `T`.
    Super {
        qualifier: Option<String>,
        name: String,
    },
    /// Calls on values, call results, extension methods and every call
    /// inside an `extension` block.
    Unresolved,
}
