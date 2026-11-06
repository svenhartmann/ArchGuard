//! What the Kotlin scanner hands to the engine's call resolver: the package,
//! imports, declarations and call sites of one file, with names as written.
//! Which declaration a name binds to depends on the whole project (other
//! files, the class hierarchy), so the engine resolves them
//! (`archguard_engine` Kotlin calls).

/// The declarations and call sites of one file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KotlinSymbols {
    /// False when the file could not be scanned (unbalanced brackets).
    pub measured: bool,
    /// The dotted `package`, empty for the default package.
    pub package: String,
    pub imports: Vec<KotlinImport>,
    pub classes: Vec<KotlinClassFacts>,
    /// Top-level functions.
    pub functions: Vec<KotlinFunctionFacts>,
    /// Names of top-level properties and type aliases: a call of such a
    /// name is no function or constructor the resolver could name.
    pub values: Vec<String>,
    pub sites: Vec<KotlinCallSite>,
}

/// `import a.b.C`, `import a.b.C as D` or `import a.b.*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KotlinImport {
    /// The dotted path without `.*`.
    pub path: String,
    /// The name the import binds; `None` for a star import.
    pub name: Option<String>,
}

/// A class, interface, object or companion object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KotlinClassFacts {
    /// The `CallClass` id.
    pub id: String,
    /// The simple name (`Companion` for an unnamed companion object).
    pub name: String,
    /// `class`, `interface`, `object`, `enum` or `annotation`.
    pub kind: String,
    /// The enclosing class id of a nested declaration.
    pub outer: Option<String>,
    pub companion: bool,
    /// Supertypes as written in the header, type arguments dropped.
    pub supertypes: Vec<String>,
    /// `abstract`, `sealed`, `data`, `value` and similar modifiers.
    pub modifiers: Vec<String>,
    /// Names of functions declared without a body (abstract or interface).
    pub bodiless: Vec<String>,
    /// Names of properties, constructor properties and member extension
    /// functions: calls of them are no member function the resolver could
    /// name.
    pub values: Vec<String>,
}

/// A top-level function and the `CallMethod` that measures it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KotlinFunctionFacts {
    pub name: String,
    pub method_id: String,
    /// An extension function `fun T.name()`: callable only on a receiver of
    /// a type the scanner does not know.
    pub extension: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KotlinCallSite {
    pub from_class: String,
    pub from_method: Option<String>,
    pub target: KotlinTarget,
    pub label: String,
    pub line: u32,
    pub column: u32,
    pub code: String,
}

/// What the source says a call targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KotlinTarget {
    /// `f(…)` or `C(…)`: a function, a member or a constructor.
    Name(String),
    /// `a.b.C.m(…)` or `a.b.C(…)`: the segments, the called name last.
    Path(Vec<String>),
    /// `this.m(…)` in a method.
    This(String),
    /// `super.m(…)`, or `super<I>.m(…)` with the qualifier `I`.
    Super {
        qualifier: Option<String>,
        name: String,
    },
    /// Calls on other receivers, call results and local variables, and every
    /// call inside lambdas, local functions and extension functions, where an
    /// implicit receiver of unknown type is in scope.
    Unresolved,
}
