//! What the PHP scanner hands to the engine's call resolver: declarations
//! and call sites of one file, with names fully qualified. Which
//! declarations exist is a project question the engine answers across files
//! (`archguard_engine::php_calls`).

/// The declarations and call sites of one file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PhpSymbols {
    /// False when the file could not be scanned (unbalanced brackets).
    pub measured: bool,
    pub classes: Vec<PhpClassFacts>,
    pub functions: Vec<PhpFunctionFacts>,
    /// Fully qualified names of classes and functions declared inside
    /// function bodies, which are not scanned.
    pub hidden_classes: Vec<String>,
    pub hidden_functions: Vec<String>,
    pub sites: Vec<PhpCallSite>,
}

/// A class, interface, trait or enum: what member lookup needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhpClassFacts {
    /// The `CallClass` id.
    pub id: String,
    /// Fully qualified name without a leading `\`.
    pub name: String,
    pub kind: String,
    /// The fully qualified `extends` class of a `class`.
    pub parent: Option<String>,
    /// Fully qualified names of the traits a `use` in the body imports.
    pub traits: Vec<String>,
    /// A `use T { … }` block with `insteadof`/`as` rules.
    pub trait_rules: bool,
    /// Lowercase names of methods declared without a body (abstract or
    /// interface methods).
    pub bodiless: Vec<String>,
}

/// A top-level function and the `CallMethod` that measures it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhpFunctionFacts {
    /// Fully qualified name without a leading `\`.
    pub name: String,
    pub class_id: String,
    pub method_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhpCallSite {
    pub from_class: String,
    pub from_method: Option<String>,
    pub target: PhpTarget,
    pub label: String,
    pub line: u32,
    pub column: u32,
    pub code: String,
}

/// What the source says a call targets, with names fully qualified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhpTarget {
    /// `new C(…)`.
    New(PhpClassRef),
    /// `C::m(…)`, `self::m(…)`, `static::m(…)`, `parent::m(…)`.
    Static(PhpClassRef, String),
    /// `$this->m(…)` in a method.
    This(String),
    /// `f(…)`: `name` first; an unqualified name without `use function`
    /// falls back to the global `fallback` at runtime.
    Function {
        name: String,
        fallback: Option<String>,
    },
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhpClassRef {
    Named(String),
    /// `self`, and `static` read as `self` (late static binding may pick a
    /// subclass at runtime).
    Current,
    Parent,
}
