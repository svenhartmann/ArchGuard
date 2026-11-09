//! Rust type relations for `rust_calls`. Traits are abstract; structs, enums and
//! unions concrete. `impl Trait for Type` and supertraits (`trait A: B`) become
//! supertypes, resolved from syntax within the crate (`CrateItems`) and, once
//! the repository is trusted, from the rust-analyzer SCIP index, which also
//! supplies each type's references in fields, signatures and impl bodies.
//! Rust has no implementation inheritance, so no inheritance depth is recorded.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;

use archguard_core::{CallAnalysis, CallClass, CallSupertypeSite, SemanticTypeUse};

use crate::pipeline::{CrateItems, ItemTarget};
use crate::rust_scip::{ScipIndex, line_at, line_starts};

/// A trait path in an `impl Trait for Type` header or a supertrait bound.
pub(crate) struct TraitSite {
    pub class_id: String,
    pub rel_path: String,
    /// Inline modules around the site inside its file.
    pub inline: Vec<String>,
    pub path: Vec<String>,
    /// 1-based line and 0-based character column of the last path segment.
    pub line: u32,
    pub column: u32,
    /// The `impl` or `trait` line, the evidence for the relation.
    pub evidence_line: u32,
}

/// A source range whose type references belong to `class_id`.
pub(crate) struct Region {
    pub class_id: String,
    pub method_id: Option<String>,
    /// `field`, `parameter`, `return` or `local`.
    pub role: &'static str,
    pub rel_path: String,
    /// 1-based line and 0-based character column; `end` is exclusive.
    pub start: (u32, u32),
    pub end: (u32, u32),
}

#[derive(Default)]
pub(crate) struct TypeSites {
    pub traits: Vec<TraitSite>,
    pub regions: Vec<Region>,
}

/// Parsed sources by walked path, and the graph-relative path of each.
#[derive(Default)]
pub(crate) struct Parsed {
    pub syntax: BTreeMap<String, (String, syn::File)>,
    pub stripped: BTreeMap<String, String>,
    /// Line starts per walked path, built on the first [`Parsed::line`].
    starts: OnceLock<HashMap<String, Vec<usize>>>,
}
impl Parsed {
    /// The 1-based `line` of a source; line 0 reads as line 1.
    fn line(&self, rel_path: &str, line: u32) -> &str {
        let starts = self.starts.get_or_init(|| {
            let texts = self.syntax.iter();
            texts
                .map(|(path, (text, _))| (path.clone(), line_starts(text)))
                .collect()
        });
        match (self.syntax.get(rel_path), starts.get(rel_path)) {
            (Some((text, _)), Some(starts)) => line_at(text, starts, line.max(1)),
            _ => "",
        }
    }
}

/// Types a SCIP definition can name (not impls, modules or unknown
/// markers), by file and last name segment.
struct Classes<'a>(HashMap<(&'a str, &'a str), Vec<&'a CallClass>>);

impl<'a> Classes<'a> {
    fn new(report: &'a CallAnalysis) -> Self {
        let mut map: HashMap<_, Vec<_>> = HashMap::new();
        let named = report
            .classes
            .iter()
            .filter(|class| !matches!(class.kind.as_str(), "impl" | "module" | "unknown"));
        for class in named {
            let name = class.name.rsplit("::").next().unwrap_or(&class.name);
            map.entry((class.file.as_str(), name))
                .or_default()
                .push(class);
        }
        Classes(map)
    }
}

/// Set the supertypes of every class with a trait site: `scip` decides where
/// it has a symbol for the trait, syntax everywhere else.
#[tracing::instrument(skip_all)]
pub(crate) fn supertypes(
    report: &mut CallAnalysis,
    parsed: &Parsed,
    sites: &[TraitSite],
    scip: Option<&ScipIndex>,
) {
    // Syntax resolution only for the sites SCIP leaves open.
    let items = std::cell::OnceCell::new();
    let classes = Classes::new(report);
    // Class → (target, evidence file, evidence line).
    let mut found = BTreeMap::<&str, BTreeSet<(String, String, u32)>>::new();
    for site in sites {
        let from_scip = scip.and_then(|index| {
            let text = parsed.line(&site.rel_path, site.line);
            let symbol = index.symbol_at(&site.rel_path, text, site.line, site.column)?;
            match target(index, &classes, parsed, symbol)? {
                Target::Class(id) => Some(id),
                Target::External(label) => Some(format!("external:{label}")),
                Target::Unmatched(_) => None,
            }
        });
        let target = from_scip.unwrap_or_else(|| {
            let items = items.get_or_init(|| {
                tracing::info_span!("crate_items").in_scope(|| CrateItems::new(&parsed.syntax))
            });
            match items.resolve(&site.rel_path, &site.inline, &site.path) {
                ItemTarget::Project { file, name } => {
                    format!("{}::{name}", parsed.stripped[&file])
                }
                ItemTarget::External(path) => format!("external:{path}"),
                ItemTarget::Unresolved => format!("unresolved:{}", site.path.join("::")),
            }
        });
        found.entry(&site.class_id).or_default().insert((
            target,
            parsed.stripped[&site.rel_path].clone(),
            site.evidence_line,
        ));
    }
    for class in &mut report.classes {
        if let Some(sites) = found.remove(class.id.as_str()) {
            let targets: BTreeSet<_> = sites.iter().map(|site| site.0.clone()).collect();
            class.supertypes = targets.into_iter().collect();
            let sites = sites
                .into_iter()
                .map(|(target, file, line)| CallSupertypeSite { target, file, line });
            class.supertype_sites = Some(sites.collect());
        }
    }
}

enum Target {
    /// A project type with a measured declaration.
    Class(String),
    /// Defined in the project but not measured (a type alias, test or generated code).
    Unmatched(String),
    External(String),
}

/// `crate::path::Name` of a SCIP type symbol. rust-analyzer writes
/// `<scheme> <manager> <crate> <version> <target>/path/Name#`, for example
/// `rust-analyzer cargo core <url> lib/fmt/Debug#`; the target namespace
/// (`lib`) is not part of the Rust path.
fn type_label(symbol: &str) -> Option<String> {
    let mut parts = symbol.splitn(5, ' ');
    let package = parts.nth(2)?;
    let path = parts.nth(1)?.strip_suffix('#')?;
    if path.contains(['#', '(', '[', '!', '.']) {
        return None;
    }
    let path = path.split_once('/').map_or(path, |(_, inner)| inner);
    Some(format!("{package}::{}", path.replace('/', "::")))
}

/// What a SCIP symbol names, if it is a type (struct, enum, union, trait or alias).
fn target(
    index: &ScipIndex,
    classes: &Classes<'_>,
    parsed: &Parsed,
    symbol: &str,
) -> Option<Target> {
    let label = type_label(symbol)?;
    let Some((document, line)) = index.definition(symbol) else {
        return Some(Target::External(label));
    };
    let file = parsed.stripped.get(document).unwrap_or(document);
    let name = label.rsplit("::").next().unwrap_or(&label);
    let class = classes
        .0
        .get(&(file.as_str(), name))
        .into_iter()
        .flatten()
        .filter(|class| class.line <= line + 1)
        .max_by_key(|class| class.line);
    Some(match class {
        Some(class) => Target::Class(class.id.clone()),
        None => Target::Unmatched(label),
    })
}

/// Type references inside each region, once per region owner, method, role and target.
#[tracing::instrument(skip_all)]
pub(crate) fn type_uses(
    index: &ScipIndex,
    report: &CallAnalysis,
    parsed: &Parsed,
    regions: &[Region],
) -> Vec<SemanticTypeUse> {
    let classes = Classes::new(report);
    let mut seen = BTreeSet::new();
    let mut uses = Vec::new();
    for region in regions {
        let position = |(line, column): (u32, u32)| {
            let text = parsed.line(&region.rel_path, line);
            index.position(&region.rel_path, text, line, column)
        };
        let (start, end) = (position(region.start), position(region.end));
        for (line, symbol) in index.symbols_between(&region.rel_path, start, end) {
            let Some(target) = target(index, &classes, parsed, symbol) else {
                continue;
            };
            let (resolution, to_class, label) = match target {
                Target::Class(id) => ("project", Some(id.clone()), id),
                Target::Unmatched(label) => ("project", None, label),
                Target::External(label) => ("external", None, label),
            };
            if to_class.as_deref() == Some(region.class_id.as_str())
                || !seen.insert((
                    &region.class_id,
                    &region.method_id,
                    region.role,
                    symbol.clone(),
                ))
            {
                continue;
            }
            uses.push(SemanticTypeUse {
                from_class: region.class_id.clone(),
                from_method: region.method_id.clone(),
                to_class,
                target_label: label,
                role: region.role.into(),
                resolution: resolution.into(),
                file: parsed.stripped[&region.rel_path].clone(),
                line: line + 1,
                code: parsed
                    .line(&region.rel_path, line + 1)
                    .trim()
                    .chars()
                    .take(200)
                    .collect(),
            });
        }
    }
    uses
}
