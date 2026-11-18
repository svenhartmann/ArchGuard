//! What the Ruby scanner hands to the engine: declarations, constant
//! references and call sites of one file, with names as written plus the
//! lexical scope they were written in. Which class a constant names depends
//! on the whole project, so the engine resolves them.

/// The declarations and call sites of one file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RubySymbols {
    /// False when the file could not be scanned (unbalanced blocks).
    pub measured: bool,
    pub classes: Vec<RubyClassFacts>,
    /// Qualified constants assigned in the file (`Point = Struct.new(…)`):
    /// project constants, but no class the scanner could follow.
    pub constants: Vec<String>,
    pub sites: Vec<RubyCallSite>,
    /// Every constant reference in code, with its line.
    pub references: Vec<(u32, RubyConst)>,
}

/// A constant as written (`Billing::Invoice`, `::Invoice`) and the lexical
/// scope it was written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RubyConst {
    pub path: String,
    /// Written with a leading `::`: looked up at the top level only.
    pub absolute: bool,
    /// Qualified names of the enclosing `class`/`module` bodies, innermost
    /// last (Ruby's `Module.nesting` reversed).
    pub nesting: Vec<String>,
}

impl RubyConst {
    /// The qualified names Ruby's lexical lookup tries, innermost scope
    /// first. Ancestors of the enclosing classes are not searched.
    pub fn candidates(&self) -> Vec<String> {
        let mut out: Vec<String> = if self.absolute {
            Vec::new()
        } else {
            self.nesting
                .iter()
                .rev()
                .map(|scope| format!("{scope}::{}", self.path))
                .collect()
        };
        out.push(self.path.clone());
        out
    }
}

/// A `class` or `module` declaration. A class reopened in the same file is
/// one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RubyClassFacts {
    /// The `CallClass` id, `file::Qualified::Name`.
    pub id: String,
    /// The qualified name, `Billing::Invoice`.
    pub name: String,
    /// `class`, `module`, or `module` for the file's top-level `(module)`.
    pub kind: String,
    pub superclass: Superclass,
    pub includes: Vec<RubyConst>,
    pub prepends: Vec<RubyConst>,
    pub extends: Vec<RubyConst>,
    pub methods: Vec<RubyMethodFacts>,
}

/// The superclass a `class` header names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Superclass {
    /// No `<` clause: `Object` for a class; modules have none.
    None,
    Const(RubyConst),
    /// Any other expression (`Struct.new(:a)`): not known statically.
    Expression,
}

/// A method defined in a class body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RubyMethodFacts {
    pub name: String,
    pub method_id: String,
    /// `def self.x`, or any `def` inside `class << self`.
    pub singleton: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RubyCallSite {
    pub from_class: String,
    pub from_method: Option<String>,
    /// The call is made from a singleton method, where `self` is the class.
    pub singleton: bool,
    pub target: RubyTarget,
    pub label: String,
    pub line: u32,
    pub column: u32,
    pub code: String,
}

/// What the source says a call targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RubyTarget {
    /// `Const.m(…)` or `Const.new(…)`.
    Const { constant: RubyConst, method: String },
    /// `self.m(…)` or a bare `m(…)`: a method of `self`.
    SelfCall(String),
    /// `super` in the method of that name.
    Super(String),
    /// `send`, `public_send`, `define_method`, `method_missing` and the
    /// other reflective calls: the target is only known at runtime.
    Dynamic,
    /// Calls on variables, call results and other receivers.
    Unresolved,
}
