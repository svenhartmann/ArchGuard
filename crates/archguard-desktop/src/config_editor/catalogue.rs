//! Every field `archguard.toml` accepts, described once: its path, value
//! kind, form group and a short description. Defaults are not repeated
//! here; [`super::defaults`] reads them from `Config::default()`.
//!
//! Paths use `.` between keys and `[]` for "every entry of a table
//! array": `layers[].name`. Concrete paths carry the index:
//! `layers[0].name`. A test probes `archguard_core::Config` with
//! `deny_unknown_fields` so a new config field cannot be forgotten.

/// The form section a field is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Project,
    Modules,
    Layers,
    DependencyRules,
    OwnershipRules,
    Rules,
    Review,
    Performance,
    Architecture,
    Semantic,
    Store,
}

impl Group {
    pub const ALL: [Group; 11] = [
        Group::Project,
        Group::Modules,
        Group::Layers,
        Group::DependencyRules,
        Group::OwnershipRules,
        Group::Rules,
        Group::Review,
        Group::Performance,
        Group::Architecture,
        Group::Semantic,
        Group::Store,
    ];

    /// English heading of the form section.
    pub fn label(self) -> &'static str {
        match self {
            Group::Project => "Project",
            Group::Modules => "Modules",
            Group::Layers => "Layers",
            Group::DependencyRules => "Dependency rules",
            Group::OwnershipRules => "Ownership rules",
            Group::Rules => "Rules and thresholds",
            Group::Review => "Review",
            Group::Performance => "Performance",
            Group::Architecture => "Architecture",
            Group::Semantic => "Semantic analysis",
            Group::Store => "Store",
        }
    }
}

/// The value a field holds in TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    /// A non-negative integer.
    Int,
    Float,
    Text,
    /// A string from a fixed set.
    Enum(&'static [&'static str]),
    /// A list of strings (globs, paths, names).
    List,
    /// A table of name → number (`review.health_weights`).
    Map,
    /// A section header such as `[rules]`; its fields are listed separately.
    Table,
    /// A `[[…]]` table array; its entry fields are listed as `path[].field`.
    TableArray,
}

impl Kind {
    /// Whether the field holds a value itself (not a section or array).
    pub fn is_value(self) -> bool {
        !matches!(self, Kind::Table | Kind::TableArray)
    }
}

/// One field of the configuration.
#[derive(Debug, PartialEq, Eq)]
pub struct Field {
    pub path: &'static str,
    pub kind: Kind,
    pub group: Group,
    /// Absent means "off" or "not set"; there is no default value.
    pub optional: bool,
    pub description: &'static str,
}

pub const SEVERITIES: &[&str] = &["error", "warning", "info"];
pub const DENY_CYCLES: &[&str] = &["module", "file", "off"];
pub const ORPHANS: &[&str] = &["off", "info", "warning", "error"];

const fn f(path: &'static str, kind: Kind, group: Group, description: &'static str) -> Field {
    Field {
        path,
        kind,
        group,
        optional: false,
        description,
    }
}

const fn opt(path: &'static str, kind: Kind, group: Group, description: &'static str) -> Field {
    Field {
        path,
        kind,
        group,
        optional: true,
        description,
    }
}

use Group as G;
use Kind as K;

/// The catalogue, in form order.
pub const FIELDS: &[Field] = &[
    // Project
    f(
        "project",
        K::Table,
        G::Project,
        "Where to look for sources and what to skip. Derived from the project when not written.",
    ),
    f(
        "project.source_roots",
        K::List,
        G::Project,
        "Directories holding the sources, relative to the project root.",
    ),
    f(
        "project.exclude",
        K::List,
        G::Project,
        "Globs excluded from the analysis. An explicit list replaces the defaults.",
    ),
    f(
        "project.languages",
        K::List,
        G::Project,
        "\"auto\" or explicit language ids, e.g. ts, py, rs, java, cs, php, go, kt.",
    ),
    // Modules
    f(
        "modules",
        K::Table,
        G::Modules,
        "The zoom level at which modules are named.",
    ),
    f(
        "modules.depth",
        K::Int,
        G::Modules,
        "Module depth: with 2, app/billing/x.ts is seen as module app/billing. Derived when not written.",
    ),
    // Layers
    f(
        "derive_layers",
        K::Bool,
        G::Layers,
        "Derive layers from the dependency ranking; off keeps only the declared layers.",
    ),
    f(
        "layers",
        K::TableArray,
        G::Layers,
        "Declared layers; a declared layer wins over a derived one of the same name.",
    ),
    f("layers[].name", K::Text, G::Layers, "Unique layer name."),
    f(
        "layers[].contains",
        K::List,
        G::Layers,
        "Globs matching module names (source-root-relative).",
    ),
    f(
        "layers[].may_depend_on",
        K::List,
        G::Layers,
        "Layers this one may depend on; empty means bottom layer.",
    ),
    // Dependency rules
    f(
        "dependency_rules",
        K::TableArray,
        G::DependencyRules,
        "Forbidden dependency directions between source modules.",
    ),
    f(
        "dependency_rules[].name",
        K::Text,
        G::DependencyRules,
        "Unique rule name.",
    ),
    f(
        "dependency_rules[].from",
        K::List,
        G::DependencyRules,
        "Globs over source module names the rule applies to, independent of the view depth.",
    ),
    f(
        "dependency_rules[].to",
        K::List,
        G::DependencyRules,
        "Globs over the forbidden targets.",
    ),
    f(
        "dependency_rules[].severity",
        K::Enum(SEVERITIES),
        G::DependencyRules,
        "Severity of a violation; error fails the check.",
    ),
    f(
        "dependency_rules[].message",
        K::Text,
        G::DependencyRules,
        "Explanation shown with each violation.",
    ),
    // Ownership rules
    f(
        "ownership_rules",
        K::TableArray,
        G::OwnershipRules,
        "Forbidden dependency directions between CODEOWNERS owners.",
    ),
    f(
        "ownership_rules[].from",
        K::Text,
        G::OwnershipRules,
        "Exact CODEOWNERS token of the depending side, e.g. @example/frontend.",
    ),
    f(
        "ownership_rules[].to",
        K::Text,
        G::OwnershipRules,
        "Exact CODEOWNERS token of the forbidden target.",
    ),
    f(
        "ownership_rules[].severity",
        K::Enum(SEVERITIES),
        G::OwnershipRules,
        "Severity of a violation; error fails the check.",
    ),
    f(
        "ownership_rules[].message",
        K::Text,
        G::OwnershipRules,
        "Explanation shown with each violation.",
    ),
    // Rules and thresholds
    f(
        "rules",
        K::Table,
        G::Rules,
        "Cycle policy and quality gates.",
    ),
    f(
        "rules.deny_cycles",
        K::Enum(DENY_CYCLES),
        G::Rules,
        "Forbid cycles between modules, even between files, or allow them.",
    ),
    f(
        "rules.max_scc_size",
        K::Int,
        G::Rules,
        "Largest allowed cycle group; 0 and 1 both reject every cycle.",
    ),
    f(
        "rules.baseline",
        K::Text,
        G::Rules,
        "Path of the baseline file with accepted cycles.",
    ),
    f(
        "rules.max_acd",
        K::Float,
        G::Rules,
        "Gate for the average component dependency of the module view; 0 disables it.",
    ),
    f(
        "rules.max_instability_core",
        K::Float,
        G::Rules,
        "Gate for the instability (0 to 1) of bottom-layer modules; 0 disables it.",
    ),
    // Review
    f(
        "review",
        K::Table,
        G::Review,
        "Review settings and key-figure gates.",
    ),
    f(
        "review.git_since",
        K::Text,
        G::Review,
        "History window for change counts: a date (2026-01-31) or e.g. 180.days.",
    ),
    f(
        "review.entry_points",
        K::List,
        G::Review,
        "Globs of entry files that are never orphan candidates.",
    ),
    f(
        "review.orphans",
        K::Enum(ORPHANS),
        G::Review,
        "Opt-in file-level orphan gate; a candidate is not proof of dead code.",
    ),
    opt(
        "review.health_weights",
        K::Map,
        G::Review,
        "Weights of the health index parts (name = weight); unlisted parts weigh 1.",
    ),
    opt(
        "review.max_sdp_violations",
        K::Int,
        G::Review,
        "Gate: maximum number of stable-dependencies violations; absent = off.",
    ),
    opt(
        "review.max_tangledness",
        K::Float,
        G::Review,
        "Gate: maximum share (0 to 1) of modules in cycles; absent = off.",
    ),
    opt(
        "review.max_propagation_cost",
        K::Float,
        G::Review,
        "Gate: maximum propagation cost (0 to 1) of a cycle group; absent = off.",
    ),
    f(
        "review.adr_required_on_accept_risk",
        K::Bool,
        G::Review,
        "Accept-risk review decisions must link an ADR.",
    ),
    // Performance
    f(
        "performance",
        K::Table,
        G::Performance,
        "Structural performance hints.",
    ),
    f(
        "performance.heavy_imports",
        K::List,
        G::Performance,
        "External module prefixes flagged for review, not measured costs.",
    ),
    f(
        "performance.min_barrel_exports",
        K::Int,
        G::Performance,
        "Re-exports from which a file counts as a barrel; at least 1.",
    ),
    f(
        "performance.min_barrel_ratio",
        K::Float,
        G::Performance,
        "Share (0 to 1) of re-exporting lines from which a file counts as a barrel.",
    ),
    // Architecture
    f(
        "architecture",
        K::Table,
        G::Architecture,
        "Explicit organization intent: goals, constraints, decisions, target model.",
    ),
    f(
        "architecture.goals",
        K::TableArray,
        G::Architecture,
        "Quality goals of the organization.",
    ),
    f(
        "architecture.goals[].name",
        K::Text,
        G::Architecture,
        "Unique goal name.",
    ),
    f(
        "architecture.goals[].priority",
        K::Text,
        G::Architecture,
        "The organization's own priority wording; empty means unspecified.",
    ),
    f(
        "architecture.goals[].rationale",
        K::Text,
        G::Architecture,
        "Why the goal matters.",
    ),
    f(
        "architecture.constraints",
        K::TableArray,
        G::Architecture,
        "Constraints the architecture must respect.",
    ),
    f(
        "architecture.constraints[].name",
        K::Text,
        G::Architecture,
        "Unique constraint name.",
    ),
    f(
        "architecture.constraints[].rationale",
        K::Text,
        G::Architecture,
        "The contract or other evidence behind it.",
    ),
    f(
        "architecture.decisions",
        K::TableArray,
        G::Architecture,
        "Recorded architecture decisions.",
    ),
    f(
        "architecture.decisions[].id",
        K::Text,
        G::Architecture,
        "Unique decision id, e.g. ADR-1.",
    ),
    f(
        "architecture.decisions[].title",
        K::Text,
        G::Architecture,
        "Decision title.",
    ),
    f(
        "architecture.decisions[].status",
        K::Text,
        G::Architecture,
        "User-supplied status, e.g. proposed, accepted, superseded.",
    ),
    f(
        "architecture.decisions[].chosen_option",
        K::Text,
        G::Architecture,
        "The chosen alternative.",
    ),
    f(
        "architecture.decisions[].alternatives",
        K::List,
        G::Architecture,
        "Other options considered.",
    ),
    f(
        "architecture.decisions[].trade_offs",
        K::List,
        G::Architecture,
        "Explicitly accepted disadvantages.",
    ),
    f(
        "architecture.decisions[].links",
        K::List,
        G::Architecture,
        "References; never followed automatically.",
    ),
    opt(
        "architecture.model",
        K::Table,
        G::Architecture,
        "Target architecture: logical components and their allowed directions.",
    ),
    f(
        "architecture.model.name",
        K::Text,
        G::Architecture,
        "Name of the target architecture.",
    ),
    f(
        "architecture.model.severity",
        K::Enum(SEVERITIES),
        G::Architecture,
        "Severity of a forbidden component dependency.",
    ),
    f(
        "architecture.model.require_assignment",
        K::Bool,
        G::Architecture,
        "Every source module must belong to a component.",
    ),
    f(
        "architecture.model.components",
        K::TableArray,
        G::Architecture,
        "Logical components, assigned from source module names.",
    ),
    f(
        "architecture.model.components[].name",
        K::Text,
        G::Architecture,
        "Component name.",
    ),
    f(
        "architecture.model.components[].include",
        K::List,
        G::Architecture,
        "Globs of the source modules in this component.",
    ),
    opt(
        "architecture.model.components[].public",
        K::List,
        G::Architecture,
        "Globs other components may use; absent = all, empty = none.",
    ),
    opt(
        "architecture.model.components[].max_incoming_components",
        K::Int,
        G::Architecture,
        "Maximum number of components depending on this one.",
    ),
    opt(
        "architecture.model.components[].max_outgoing_components",
        K::Int,
        G::Architecture,
        "Maximum number of components this one depends on.",
    ),
    f(
        "architecture.model.allowed_dependencies",
        K::TableArray,
        G::Architecture,
        "Allowed directions between components; all others are denied.",
    ),
    f(
        "architecture.model.allowed_dependencies[].from",
        K::Text,
        G::Architecture,
        "Depending component.",
    ),
    f(
        "architecture.model.allowed_dependencies[].to",
        K::Text,
        G::Architecture,
        "Component it may depend on.",
    ),
    f(
        "architecture.asyncapi",
        K::List,
        G::Architecture,
        "Extra local AsyncAPI documents; root asyncapi files are read anyway.",
    ),
    // Semantic analysis
    f(
        "semantic",
        K::Table,
        G::Semantic,
        "Compiler-backed call analysis.",
    ),
    f(
        "semantic.java",
        K::Table,
        G::Semantic,
        "Java compiler settings.",
    ),
    opt(
        "semantic.java.release",
        K::Int,
        G::Semantic,
        "Java release (at least 6); otherwise build metadata, then the running JDK.",
    ),
    f(
        "semantic.java.classpath",
        K::List,
        G::Semantic,
        "Existing jars or class directories, relative to the project root.",
    ),
    f(
        "semantic.csharp",
        K::Table,
        G::Semantic,
        "C# compiler settings.",
    ),
    opt(
        "semantic.csharp.project",
        K::Text,
        G::Semantic,
        "Project file to analyze, e.g. src/App/App.csproj.",
    ),
    f(
        "semantic.csharp.configuration",
        K::Text,
        G::Semantic,
        "Build configuration.",
    ),
    opt(
        "semantic.csharp.target_framework",
        K::Text,
        G::Semantic,
        "Target framework of a multi-target project, e.g. net8.0.",
    ),
    f(
        "semantic.typescript",
        K::Table,
        G::Semantic,
        "TypeScript compiler settings.",
    ),
    opt(
        "semantic.typescript.project",
        K::Text,
        G::Semantic,
        "Existing tsconfig; defaults to the root tsconfig.json.",
    ),
    opt(
        "semantic.typescript.compiler",
        K::Text,
        G::Semantic,
        "Installed typescript/lib/typescript.js; never downloaded.",
    ),
    // Store
    f("store", K::Table, G::Store, "The graph database mirror."),
    f("store.db", K::Text, G::Store, "Path of the graph database."),
    f(
        "store.sync_on_check",
        K::Bool,
        G::Store,
        "Refresh the database on every check.",
    ),
];

/// The field of a pattern (`layers[].name`) or concrete path
/// (`layers[3].name`).
pub fn field(path: &str) -> Option<&'static Field> {
    let pattern = pattern_of(path);
    FIELDS.iter().find(|f| f.path == pattern)
}

/// The catalogue field an error path belongs to: the path itself or the
/// nearest enclosing field (`layers[0].contains[1]` → `layers[].contains`,
/// `review.health_weights[hubs]` → `review.health_weights`).
pub fn resolve(path: &str) -> Option<&'static Field> {
    let mut pattern = pattern_of(path);
    loop {
        if let Some(found) = FIELDS.iter().find(|f| f.path == pattern) {
            return Some(found);
        }
        let cut = pattern.rfind(['.', '['])?;
        pattern.truncate(cut);
    }
}

/// `layers[3].contains[0]` → `layers[].contains[]`.
pub fn pattern_of(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut in_index = false;
    for c in path.chars() {
        match c {
            '[' => {
                in_index = true;
                out.push('[');
            }
            ']' => {
                in_index = false;
                out.push(']');
            }
            _ if in_index => {}
            c => out.push(c),
        }
    }
    out
}

/// The fields directly inside `scope` (`""` for the root, `rules`,
/// `layers[]`), in catalogue order.
pub fn children(scope: &str) -> impl Iterator<Item = &'static Field> + '_ {
    FIELDS.iter().filter(move |f| parent_of(f.path) == scope)
}

/// `layers[].name` → `layers[]`, `rules.max_acd` → `rules`, `rules` → ``.
pub fn parent_of(path: &str) -> &str {
    path.rfind('.').map_or("", |i| &path[..i])
}
